//! Complete triangulation pipeline from vertex coordinates to search results.
//!
//! The pipeline projects to full dimension, computes hull facets, lattice
//! points, simplices and faces, builds the SAT formula and runs the selected
//! enumeration mode. Optional filters classify regular, flag or quadratic
//! triangulations. [`run_detailed`] retains geometry; [`run`] returns counts.
//!
//! [`Row`] uses a stable TSV format. Capped and timed-out searches return
//! lower bounds. Disabled classifications have zero counts. Dependency panics
//! become [`JobError::Panic`] at the per-polytope boundary.

use std::{
    any::Any,
    convert::Infallible,
    fmt,
    io::{self, Write},
    num::NonZeroU64,
    panic::{self, AssertUnwindSafe},
    time::{Duration, Instant},
};
use unitrisat_core::{
    cnf::{self, Cnf, CnfError},
    enumerate::{
        self, AnyTriangulation, Classify, EnumerateError, Enumeration, Goal, Keep, Options, Outcome,
    },
    flag::{Flag, FlagError},
    hull::{self, HullError},
    intersect::{self, IntersectError},
    lattice::{
        self, Central, CentralError, Faces, FacesError, PointsError, ProjectionError, Restriction,
        SimplicesError,
    },
    matrix::Matrix,
    quadratic::{Quadratic, QuadraticError},
    regular::{RegularError, Regularity},
    solve::Backend,
};

/// The header line of a TSV result file.
pub const TSV_HEADER: &str =
    "# idx\tstatus\tn_triangulations\tn_regular\tn_flag\tn_quadratic\tseconds";

/// How the triangulations are enumerated: [`enumerate::serial`],
/// [`enumerate::parallel_split`] or [`enumerate::incremental`]. The variant
/// docs are the `--mode` help texts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum Mode {
    /// One solver on the full formula.
    Serial,
    /// The full formula split by the central simplices over the threads.
    Split,
    /// One solver on a reduced formula; intersection clauses arrive while it solves.
    Incremental,
}

/// Where simplex intersections are computed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum IntersectionBackend {
    /// Exact CPU intersection.
    Cpu,
    /// Exact CUDA intersection, with automatic CPU fallback when unavailable.
    Gpu,
}

/// What a job computes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Intersection backend, including streamed clauses in incremental mode.
    pub intersection_backend: IntersectionBackend,
    /// Which simplices may be used.
    pub restriction: Restriction,
    /// How the triangulations are enumerated.
    pub mode: Mode,
    /// The SAT solver.
    pub backend: Backend,
    /// Stop at the first target triangulation, or enumerate all of them.
    pub goal: Goal,
    /// Test regularity.
    pub regular: bool,
    /// Learn exact nonregularity conflicts after failed regularity LPs.
    /// Requires `regular`; counts then cover only encountered candidates.
    pub regular_learn: bool,
    /// Test flagness.
    pub flag: bool,
    /// Restrict the SAT search to flag triangulations. Independent of `flag`,
    /// which still runs the classifier when requested. Counts and caps then
    /// refer to the flag-restricted search domain.
    pub flag_sat: bool,
    /// The solution cap of D1 (`--max-solutions`).
    pub max_solutions: Option<NonZeroU64>,
    /// The regularity-test cap of D1 (`--max-regularity-tests`); it only
    /// skips tests when both filters are on.
    pub max_regularity_tests: Option<NonZeroU64>,
    /// The per-job timeout of D1, counted from the start of the job; it
    /// interrupts the search.
    pub timeout: Option<Duration>,
}

/// Incompatible computation options, rejected before geometry or search starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    /// Conflict learning requires the regularity classifier.
    #[error("--regular-learn requires --regular")]
    RegularLearnRequiresRegular,
    /// The native Kissat backend is not available on Windows.
    #[cfg(windows)]
    #[error("Kissat is unavailable on Windows; use PicoSAT or CaDiCaL")]
    KissatUnavailable,
    /// Kissat cannot add clauses after its first solve.
    #[error(
        "Kissat is incompatible with {0}: it cannot add clauses after solving; use PicoSAT or CaDiCaL"
    )]
    KissatUnsupported(&'static str),
}

impl Settings {
    /// Validate solver compatibility before opening input or starting any jobs.
    /// Kissat supports first, unfiltered searches in serial and split modes.
    pub fn validate(&self) -> Result<(), SettingsError> {
        if self.regular_learn && !self.regular {
            return Err(SettingsError::RegularLearnRequiresRegular);
        }
        if self.backend == Backend::Kissat {
            let unsupported = if self.mode == Mode::Incremental {
                Some("incremental mode (--mode incremental)")
            } else if self.regular {
                Some("regularity filtering (--regular)")
            } else if self.flag {
                Some("flag filtering (--flag)")
            } else if self.goal == Goal::All {
                Some("enumerating all triangulations (--all / --find-all)")
            } else {
                None
            };
            if let Some(option) = unsupported {
                return Err(SettingsError::KissatUnsupported(option));
            }
            #[cfg(windows)]
            return Err(SettingsError::KissatUnavailable);
        }
        Ok(())
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            intersection_backend: IntersectionBackend::Cpu,
            restriction: Restriction::Unimodular,
            mode: Mode::Serial,
            backend: Backend::PicoSat,
            goal: Goal::First,
            regular: false,
            regular_learn: false,
            flag: false,
            flag_sat: false,
            max_solutions: None,
            max_regularity_tests: None,
            timeout: None,
        }
    }
}

/// The status column of a [`Row`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    /// The requested search completed. With regularity conflict learning,
    /// regular targets are exact but visited-candidate counts can omit
    /// nonregular triangulations pruned by learned clauses.
    Ok,
    /// The search was cut short by the solution cap or the timeout: the
    /// counts are lower bounds.
    Capped,
    /// The regularity-test cap skipped a test before any regular
    /// triangulation was seen: the regular count is a lower bound. Encountered
    /// triangulation/flag counts can also be lower bounds when learning prunes
    /// candidates; the completed quadratic target count remains exact.
    RegLimit,
    /// No result: the polytope was skipped or its job failed. The row has
    /// no counts (written as -1).
    Skipped,
}

impl Status {
    /// Every status, the closed vocabulary of the result rows.
    pub const ALL: [Self; 4] = [Self::Ok, Self::Capped, Self::RegLimit, Self::Skipped];

    /// The status as written in a result row.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Capped => "capped",
            Self::RegLimit => "reglimit",
            Self::Skipped => "skipped",
        }
    }

    /// The status written as `text`, if it is one.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == text)
    }
}

/// One per-polytope result row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    /// One-based index of the polytope in its file.
    pub index: u64,
    /// What the counts mean.
    pub status: Status,
    /// Triangulations encountered; learning can prune additional nonregular
    /// candidates that do not contribute to this count.
    pub triangulations: u64,
    /// Regular triangulations seen (0 without the regular filter).
    pub regular: u64,
    /// Flag triangulations seen (0 unless `flag` or `flag_sat` is enabled).
    pub flag: u64,
    /// Regular flag triangulations seen (requires regularity and either flag option).
    pub quadratic: u64,
    /// Wall-clock seconds of the job.
    pub seconds: f64,
}

impl Row {
    /// The `skipped` row of polytope `index` (all counts 0, written as -1).
    pub fn skipped(index: u64, seconds: f64) -> Self {
        Self {
            index,
            status: Status::Skipped,
            triangulations: 0,
            regular: 0,
            flag: 0,
            quadratic: 0,
            seconds,
        }
    }

    /// Writes the row with a trailing LF.
    pub fn write<W: Write>(&self, out: &mut W) -> io::Result<()> {
        writeln!(out, "{self}")
    }

    /// Parses one row without its line terminator. Only the exact text
    /// [`Row::write`] produces is accepted (positive index, a status of
    /// [`Status::ALL`], counts in canonical form, all -1 for `skipped`,
    /// finite non-negative seconds with four decimals); anything else, a
    /// torn or foreign row included, is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        let fields: Vec<&str> = text.split('\t').collect();
        let [index, status, counts @ .., seconds] = fields.as_slice() else {
            return None;
        };
        let [triangulations, regular, flag, quadratic] = counts else {
            return None;
        };
        let status = Status::parse(status)?;
        let count = |text: &str| -> Option<u64> {
            if status == Status::Skipped {
                (text == "-1").then_some(0)
            } else {
                text.parse().ok()
            }
        };
        let row = Self {
            index: index.parse().ok().filter(|&i| i >= 1)?,
            status,
            triangulations: count(triangulations)?,
            regular: count(regular)?,
            flag: count(flag)?,
            quadratic: count(quadratic)?,
            seconds: seconds
                .parse::<f64>()
                .ok()
                .filter(|s| s.is_finite() && *s >= 0.0)?,
        };
        (row.regular <= row.triangulations
            && row.flag <= row.triangulations
            && row.quadratic <= row.regular.min(row.flag)
            && row.to_string() == text)
            .then_some(row)
    }
}

/// The row without its LF: tab-separated, seconds with four decimals.
impl fmt::Display for Row {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\t{}\t", self.index, self.status.as_str())?;
        if self.status == Status::Skipped {
            f.write_str("-1\t-1\t-1\t-1\t")?;
        } else {
            write!(
                f,
                "{}\t{}\t{}\t{}\t",
                self.triangulations, self.regular, self.flag, self.quadratic
            )?;
        }
        write!(f, "{:.4}", self.seconds)
    }
}

/// Failure of a job.
#[derive(Debug, thiserror::Error)]
pub enum JobError {
    /// The requested solver cannot support these settings.
    #[error(transparent)]
    Settings(#[from] SettingsError),
    /// The projection to full dimension failed.
    #[error("projection: {0}")]
    Projection(#[from] ProjectionError),
    /// The hull facets could not be computed.
    #[error("hull: {0}")]
    Hull(#[from] HullError),
    /// The lattice points could not be computed.
    #[error("lattice points: {0}")]
    Points(#[from] PointsError),
    /// The simplices could not be computed.
    #[error("simplices: {0}")]
    Simplices(#[from] SimplicesError),
    /// The faces could not be computed.
    #[error("faces: {0}")]
    Faces(#[from] FacesError),
    /// The intersecting pairs could not be computed.
    #[error("intersections: {0}")]
    Intersect(#[from] IntersectError),
    /// The formula could not be built.
    #[error("formula: {0}")]
    Cnf(#[from] CnfError),
    /// The central simplices could not be computed.
    #[error("central simplices: {0}")]
    Central(#[from] CentralError),
    /// The regularity test could not be set up.
    #[error("regularity: {0}")]
    Regular(#[from] RegularError),
    /// The flag test could not be set up.
    #[error("flagness: {0}")]
    Flag(#[from] FlagError),
    /// The search failed.
    #[error("search: {0}")]
    Search(#[from] EnumerateError<Infallible>),
    /// The search with the regularity test failed.
    #[error("search: {0}")]
    SearchRegular(#[from] EnumerateError<RegularError>),
    /// The search with the flag test failed.
    #[error("search: {0}")]
    SearchFlag(#[from] EnumerateError<FlagError>),
    /// The search with both tests failed.
    #[error("search: {0}")]
    SearchQuadratic(#[from] EnumerateError<QuadraticError<RegularError>>),
    /// The job panicked: a bug, reported instead of ending the batch.
    #[error("panic: {0}")]
    Panic(String),
}

/// A completed stage or transition in one polytope's computation.
///
/// Observers run on the calling thread, including periodic snapshots during
/// enumeration. Keep callbacks short: their time is included in the job's wall
/// time and timeout. A failed job may omit later events.
#[derive(Clone, Copy, Debug)]
pub enum Progress {
    /// Lattice points have been computed.
    Points {
        /// Number of lattice points.
        count: usize,
    },
    /// Candidate simplices have been computed.
    Simplices {
        /// Number of candidate simplices.
        count: usize,
    },
    /// Formula construction is complete; classifier setup and search begin.
    /// Incremental intersection production happens during this stage.
    Solving,
    /// Live counters from incremental solving, excluding initial formula and
    /// model-blocking clauses. Each event runs on the calling thread.
    Incremental {
        /// Intersection clauses successfully inserted into the solver.
        clauses_added: usize,
        /// Solver calls that actually returned a stopped status.
        interruptions: u64,
    },
    /// Live classification counts. Rejections count only completed negative
    /// tests; skipped regularity tests are never counted as rejections.
    Classification {
        /// Triangulations reached by classification, including in-flight tests.
        triangulations: u64,
        /// Completed positive regularity tests.
        regular: u64,
        /// Completed negative regularity tests.
        non_regular: u64,
        /// Completed positive flag tests.
        flag: u64,
        /// Completed negative flag tests.
        non_flag: u64,
        /// Completed positive tests for both properties.
        quadratic: u64,
    },
    /// The job completed successfully, including a capped search.
    /// This follows `Simplices` directly when there are no simplices.
    Finished {
        /// Wall times of the consecutive pipeline stages.
        timings: StageTimes,
    },
}

/// Runs the job for polytope `index` with `vertices` (one vertex per row)
/// and returns its result row. A panic becomes [`JobError::Panic`].
pub fn run(index: u64, vertices: &Matrix, settings: &Settings) -> Result<Row, JobError> {
    run_counts::<false>(index, vertices, settings, &mut |_| {})
}

/// Runs the count-only pipeline with live stage notifications and coarse timing.
///
/// Unlike [`run_detailed`], this retains no triangulations or geometry. The
/// observer receives stage events, live classification counts and incremental
/// batch/interrupt counters;
/// its panics become [`JobError::Panic`].
/// Use [`run`] when no progress or stage timing is needed.
///
/// Filtered searches use persistent execution threads; callbacks stay on the
/// calling thread. Each caller caches one private Rayon pool matching the
/// current thread count, replacing it if that count changes. The cache retains
/// its workers until replacement or caller exit so native scratch can be reused.
/// It does not inherit another pool's identity or thread-local state.
pub fn run_with_progress(
    index: u64,
    vertices: &Matrix,
    settings: &Settings,
    observer: &mut impl FnMut(Progress),
) -> Result<Row, JobError> {
    run_counts::<true>(index, vertices, settings, observer)
}

fn run_counts<const PROGRESS: bool>(
    index: u64,
    vertices: &Matrix,
    settings: &Settings,
    observer: &mut impl FnMut(Progress),
) -> Result<Row, JobError> {
    settings.validate()?;
    let start = Instant::now();
    let (outcome, counts) = panic::catch_unwind(AssertUnwindSafe(|| {
        pipeline::<PROGRESS, PROGRESS, _>(
            vertices,
            settings,
            start,
            Keep::None,
            observer,
            |_, found, counts, _| (found.outcome, counts),
        )
    }))
    .map_err(|payload| JobError::Panic(panic_message(payload.as_ref())))??;
    Ok(Row {
        index,
        status: status(outcome, counts.limited),
        triangulations: counts.triangulations,
        regular: counts.regular,
        flag: counts.flag,
        quadratic: counts.quadratic,
        seconds: start.elapsed().as_secs_f64(),
    })
}

/// The row status of a search that ended with `outcome`; `limited` says
/// that the regularity-test cap skipped a test before any regular
/// triangulation was seen.
fn status(outcome: Outcome, limited: bool) -> Status {
    match outcome {
        Outcome::Complete | Outcome::Found if limited => Status::RegLimit,
        Outcome::Complete | Outcome::Found => Status::Ok,
        // A lower bound, never `ok` (REVIEW.md R28).
        Outcome::Capped | Outcome::TimedOut => Status::Capped,
    }
}

/// The message of a panic payload.
fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no message".to_owned())
}

/// The counts of a search, with zeros for the filters that are off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counts {
    triangulations: u64,
    regular: u64,
    flag: u64,
    quadratic: u64,
    /// The regularity-test cap skipped a test and no regular triangulation
    /// was seen.
    limited: bool,
}

/// The lattice data of one polytope (phase 2).
struct Lattice {
    points: Matrix,
    simplices: Vec<u32>,
    faces: Faces,
    /// Number of simplices.
    count: usize,
}

impl Lattice {
    fn new<const PROGRESS: bool>(
        vertices: &Matrix,
        restriction: Restriction,
        observer: &mut impl FnMut(Progress),
    ) -> Result<Self, JobError> {
        let projected = lattice::project_to_full_dimension(vertices)?;
        let full = projected.as_ref().unwrap_or(vertices);
        let facets = hull::facets(full)?;
        let points = lattice::lattice_points(full, &facets)?;
        if PROGRESS {
            observer(Progress::Points {
                count: points.rows(),
            });
        }
        let simplices = lattice::simplices(&points, restriction)?;
        let count = points
            .columns()
            .checked_add(1)
            .and_then(|width| simplices.len().checked_div(width))
            .unwrap_or(0);
        if PROGRESS {
            observer(Progress::Simplices { count });
        }
        let faces = lattice::faces(&points, &facets, &simplices)?;
        Ok(Self {
            points,
            simplices,
            faces,
            count,
        })
    }

    fn pairs(&self, backend: IntersectionBackend) -> Result<Vec<(u32, u32)>, IntersectError> {
        let search = match backend {
            IntersectionBackend::Cpu => intersect::intersecting_pairs,
            IntersectionBackend::Gpu => unitrisat_gpu::intersecting_pairs,
        };
        search(self.points.values(), self.points.columns(), &self.simplices)
    }
}

/// The formula of a [`Mode`].
enum Formula {
    Serial(Cnf),
    Split(Cnf, Central),
    Incremental(Cnf, IntersectionBackend),
}

impl Formula {
    fn new(
        lattice: &Lattice,
        mode: Mode,
        backend: IntersectionBackend,
        flag_sat: bool,
    ) -> Result<Self, JobError> {
        let central = || lattice::central(&lattice.points, &lattice.simplices, &lattice.faces);
        let constrain = |cnf| {
            if flag_sat {
                cnf::with_flag(
                    cnf,
                    &lattice.simplices,
                    lattice.points.columns().saturating_add(1),
                    lattice.points.rows(),
                )
            } else {
                Ok(cnf)
            }
        };
        Ok(match mode {
            Mode::Serial => Self::Serial(constrain(cnf::formula(
                lattice.count,
                &lattice.pairs(backend)?,
                &lattice.faces,
            )?)?),
            Mode::Split => Self::Split(
                constrain(cnf::formula(
                    lattice.count,
                    &lattice.pairs(backend)?,
                    &lattice.faces,
                )?)?,
                central()?,
            ),
            Mode::Incremental => Self::Incremental(
                constrain(cnf::reduced(lattice.count, &lattice.faces, &central()?)?)?,
                backend,
            ),
        })
    }

    fn search<const PROGRESS: bool, C: Classify>(
        &self,
        lattice: &Lattice,
        options: &Options,
        classify: &C,
        tests: classification_progress::Tests<'_>,
        observer: &mut impl FnMut(Progress),
    ) -> Result<Enumeration, EnumerateError<C::Error>> {
        if PROGRESS && (tests.regular.is_some() || tests.flag.is_some()) {
            if matches!(self, Self::Incremental(..)) {
                observer(Progress::Incremental {
                    clauses_added: 0,
                    interruptions: 0,
                });
            }
            classification_progress::observe(classify, tests, observer, |tracked, observer| {
                self.search_direct::<true, _>(lattice, options, tracked, observer)
            })
        } else {
            self.search_direct::<PROGRESS, _>(lattice, options, classify, observer)
        }
    }

    fn search_direct<const PROGRESS: bool, C: Classify>(
        &self,
        lattice: &Lattice,
        options: &Options,
        classify: &C,
        observer: &mut (impl FnMut(Progress) + ?Sized),
    ) -> Result<Enumeration, EnumerateError<C::Error>> {
        let mut incremental_observer = |progress: enumerate::IncrementalProgress| {
            observer(Progress::Incremental {
                clauses_added: progress.clauses_added,
                interruptions: progress.interruptions,
            });
        };
        match self {
            Self::Incremental(reduced, IntersectionBackend::Cpu) if PROGRESS => {
                enumerate::incremental_with_progress(
                    reduced,
                    &lattice.points,
                    &lattice.simplices,
                    options,
                    classify,
                    &mut incremental_observer,
                )
            }
            Self::Incremental(reduced, IntersectionBackend::Gpu) if PROGRESS => {
                enumerate::incremental_with_pairs_and_progress(
                    reduced,
                    &lattice.points,
                    &lattice.simplices,
                    options,
                    classify,
                    &unitrisat_gpu::stream_pairs,
                    &mut incremental_observer,
                )
            }
            Self::Serial(full) => enumerate::serial(full, options, classify),
            Self::Split(full, central) => {
                enumerate::parallel_split(full, central.simplices(), options, classify)
            }
            Self::Incremental(reduced, IntersectionBackend::Cpu) => enumerate::incremental(
                reduced,
                &lattice.points,
                &lattice.simplices,
                options,
                classify,
            ),
            Self::Incremental(reduced, IntersectionBackend::Gpu) => {
                enumerate::incremental_with_pairs(
                    reduced,
                    &lattice.points,
                    &lattice.simplices,
                    options,
                    classify,
                    &unitrisat_gpu::stream_pairs,
                )
            }
        }
    }
}

/// Detailed output for adapters that retain target triangulations.
#[derive(Debug)]
pub struct Detailed {
    /// Counts, status and total job wall time.
    pub row: Row,
    /// Exact search termination reason.
    pub outcome: Outcome,
    /// Lattice points in the effective full-dimensional coordinates.
    pub points: Matrix,
    /// Flat simplex vertex indices into `points`.
    pub simplices: Vec<u32>,
    /// Retained target triangulations, as zero-based simplex indices.
    pub triangulations: Vec<Vec<u32>>,
    /// Wall times of consecutive pipeline stages.
    pub timings: StageTimes,
}

/// Coarse timings; incremental intersection production is part of search.
#[derive(Clone, Copy, Debug, Default)]
pub struct StageTimes {
    /// Projection, hull, lattice points, simplices and faces.
    pub lattice: f64,
    /// Formula and (except incremental mode) intersections.
    pub formula: f64,
    /// Classifier setup, search and classification.
    pub search: f64,
}

/// Runs the same pipeline as [`run`], retaining the targets selected by `keep`.
/// Geometry is moved out of the pipeline, never recomputed or cloned.
pub fn run_detailed(
    index: u64,
    vertices: &Matrix,
    settings: &Settings,
    keep: Keep,
) -> Result<Detailed, JobError> {
    settings.validate()?;
    let start = Instant::now();
    panic::catch_unwind(AssertUnwindSafe(|| {
        pipeline::<true, false, _>(
            vertices,
            settings,
            start,
            keep,
            &mut |_| {},
            |lattice, found, counts, timings| Detailed {
                row: Row {
                    index,
                    status: status(found.outcome, counts.limited),
                    triangulations: counts.triangulations,
                    regular: counts.regular,
                    flag: counts.flag,
                    quadratic: counts.quadratic,
                    seconds: start.elapsed().as_secs_f64(),
                },
                outcome: found.outcome,
                points: lattice.points,
                simplices: lattice.simplices,
                triangulations: found.triangulations,
                timings,
            },
        )
    }))
    .map_err(|payload| JobError::Panic(panic_message(payload.as_ref())))?
}

/// The shared pipeline. Const false paths add no stage clocks or observer
/// calls to ordinary count-only jobs.
fn pipeline<const TIMED: bool, const PROGRESS: bool, T>(
    vertices: &Matrix,
    settings: &Settings,
    start: Instant,
    keep: Keep,
    observer: &mut impl FnMut(Progress),
    finish: impl FnOnce(Lattice, Enumeration, Counts, StageTimes) -> T,
) -> Result<T, JobError> {
    let mut stage = TIMED.then(Instant::now);
    let mut timings = StageTimes::default();
    let lattice = Lattice::new::<PROGRESS>(vertices, settings.restriction, observer)?;
    if let Some(clock) = stage {
        timings.lattice = clock.elapsed().as_secs_f64();
        stage = Some(Instant::now());
    }
    if lattice.count == 0 {
        // No simplex means no triangulation.
        if PROGRESS {
            observer(Progress::Finished { timings });
        }
        return Ok(finish(
            lattice,
            Enumeration {
                outcome: Outcome::Complete,
                found: 0,
                targets: 0,
                triangulations: Vec::new(),
            },
            Counts::default(),
            timings,
        ));
    }
    let formula = Formula::new(
        &lattice,
        settings.mode,
        settings.intersection_backend,
        settings.flag_sat,
    )?;
    if let Some(clock) = stage {
        timings.formula = clock.elapsed().as_secs_f64();
        stage = Some(Instant::now());
    }
    if PROGRESS {
        observer(Progress::Solving);
    }
    let options = Options {
        backend: settings.backend,
        goal: settings.goal,
        keep,
        max_solutions: settings.max_solutions,
        // An unrepresentable deadline is no deadline.
        deadline: settings.timeout.and_then(|t| start.checked_add(t)),
    };
    let width = lattice.points.columns().saturating_add(1);
    let flag = || Flag::new(&lattice.simplices, width, lattice.points.rows());
    let regularity = || {
        Regularity::new(&lattice.points, &lattice.simplices, &lattice.faces)
            .map(|regular| regular.with_learning(settings.regular_learn))
    };
    let (found, mut counts) = match (settings.regular, settings.flag) {
        (false, false) => {
            let found = formula.search::<PROGRESS, _>(
                &lattice,
                &options,
                &AnyTriangulation,
                classification_progress::Tests::default(),
                observer,
            )?;
            let counts = Counts {
                triangulations: found.found,
                ..Counts::default()
            };
            (found, counts)
        }
        (true, false) => {
            let regular = classification_progress::Counted::<_, PROGRESS>::new(regularity()?);
            let found = formula.search::<PROGRESS, _>(
                &lattice,
                &options,
                &regular,
                classification_progress::Tests {
                    regular: Some(&regular.tally),
                    flag: None,
                },
                observer,
            )?;
            let counts = Counts {
                triangulations: found.found,
                regular: found.targets,
                ..Counts::default()
            };
            (found, counts)
        }
        (false, true) => {
            let flag = classification_progress::Counted::<_, PROGRESS>::new(flag()?);
            let found = formula.search::<PROGRESS, _>(
                &lattice,
                &options,
                &flag,
                classification_progress::Tests {
                    regular: None,
                    flag: Some(&flag.tally),
                },
                observer,
            )?;
            let counts = Counts {
                triangulations: found.found,
                flag: found.targets,
                ..Counts::default()
            };
            (found, counts)
        }
        (true, true) => {
            let regular = classification_progress::Counted::<_, PROGRESS>::new(regularity()?);
            let flag = classification_progress::Counted::<_, PROGRESS>::new(flag()?);
            let quadratic = Quadratic::new(flag, &regular, settings.max_regularity_tests);
            let found = formula.search::<PROGRESS, _>(
                &lattice,
                &options,
                &quadratic,
                classification_progress::Tests {
                    regular: Some(&regular.tally),
                    flag: Some(&quadratic.flag_classifier().tally),
                },
                observer,
            )?;
            let counts = quadratic.counts();
            let counts = Counts {
                triangulations: found.found,
                regular: counts.regular,
                flag: counts.flag,
                quadratic: counts.quadratic,
                limited: counts.limited,
            };
            (found, counts)
        }
    };
    if settings.flag_sat && !settings.flag {
        counts.flag = counts.triangulations;
        if settings.regular {
            counts.quadratic = counts.regular;
        }
    }
    if let Some(clock) = stage {
        timings.search = clock.elapsed().as_secs_f64();
    }
    if PROGRESS {
        observer(Progress::Finished { timings });
    }
    Ok(finish(lattice, found, counts, timings))
}

mod classification_progress;

#[cfg(test)]
mod tests;

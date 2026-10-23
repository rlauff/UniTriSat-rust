//! Versioned, atomic interruption checkpoints for a file-based run.
//!
//! A checkpoint contains completed result rows, failed attempts, and the indices
//! active when Ctrl+C arrived. Active jobs are restarted from their beginning;
//! no incomplete SAT counts are mistaken for a completed result. Input is copied
//! once to an immutable temporary snapshot, hashed while copying, then parsed
//! for the record count. Computation reads that same snapshot.

use crate::job::{IntersectionBackend, Mode, Row, Settings, Status};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{self, BufReader, Read as _, Write as _},
    num::{NonZeroU64, NonZeroUsize},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant, SystemTime},
};
use unitrisat::{enumerate::Goal, input, lattice::Restriction, solve::Backend};

/// A checkpoint could not be created, validated, or written.
#[derive(Debug, thiserror::Error)]
pub enum ResumeError {
    /// Computation options are incompatible with the selected solver.
    #[error(transparent)]
    Settings(#[from] crate::job::SettingsError),
    /// A filesystem operation failed.
    #[error("checkpoint I/O: {0}")]
    Io(#[from] io::Error),
    /// The serialized checkpoint is malformed.
    #[error("invalid checkpoint JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Input parsing failed before the run started.
    #[error("{0}")]
    Input(#[from] input::InputError),
    /// The checkpoint is inconsistent with its input, settings, or schema.
    #[error("{0}")]
    Invalid(String),
    /// The operating system signal handler could not be installed.
    #[error("cannot install Ctrl+C handler: {0}")]
    Signal(#[from] ctrlc::Error),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedSettings {
    backend: String,
    simplices: String,
    mode: String,
    solver: String,
    all: bool,
    regular: bool,
    #[serde(default)]
    regular_learn: bool,
    flag: bool,
    #[serde(default)]
    flag_sat: bool,
    max_solutions: Option<NonZeroU64>,
    max_regularity_tests: Option<NonZeroU64>,
    timeout: Option<Duration>,
}

impl From<Settings> for SavedSettings {
    fn from(s: Settings) -> Self {
        Self {
            backend: match s.intersection_backend {
                IntersectionBackend::Cpu => "cpu",
                IntersectionBackend::Gpu => "gpu",
            }
            .into(),
            simplices: match s.restriction {
                Restriction::Unimodular => "unimodular",
                Restriction::Nondegenerate => "nondegenerate",
            }
            .into(),
            mode: match s.mode {
                Mode::Serial => "serial",
                Mode::Split => "split",
                Mode::Incremental => "incremental",
            }
            .into(),
            solver: match s.backend {
                Backend::PicoSat => "picosat",
                Backend::Cadical => "cadical",
                Backend::Kissat => "kissat",
            }
            .into(),
            all: s.goal == Goal::All,
            regular: s.regular,
            regular_learn: s.regular_learn,
            flag: s.flag,
            flag_sat: s.flag_sat,
            max_solutions: s.max_solutions,
            max_regularity_tests: s.max_regularity_tests,
            timeout: s.timeout,
        }
    }
}

impl SavedSettings {
    fn decode(&self) -> Result<Settings, ResumeError> {
        let invalid = || ResumeError::Invalid("checkpoint has unknown computation settings".into());
        let settings = Settings {
            intersection_backend: match self.backend.as_str() {
                "cpu" => IntersectionBackend::Cpu,
                "gpu" => IntersectionBackend::Gpu,
                _ => return Err(invalid()),
            },
            restriction: match self.simplices.as_str() {
                "unimodular" => Restriction::Unimodular,
                "nondegenerate" => Restriction::Nondegenerate,
                _ => return Err(invalid()),
            },
            mode: match self.mode.as_str() {
                "serial" => Mode::Serial,
                "split" => Mode::Split,
                "incremental" => Mode::Incremental,
                _ => return Err(invalid()),
            },
            backend: match self.solver.as_str() {
                "picosat" => Backend::PicoSat,
                "cadical" => Backend::Cadical,
                "kissat" => Backend::Kissat,
                _ => return Err(invalid()),
            },
            goal: if self.all { Goal::All } else { Goal::First },
            regular: self.regular,
            regular_learn: self.regular_learn,
            flag: self.flag,
            flag_sat: self.flag_sat,
            max_solutions: self.max_solutions,
            max_regularity_tests: self.max_regularity_tests,
            timeout: self.timeout,
        };
        Ok(settings)
    }
}

/// Runtime preferences restored separately from computation settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeOptions {
    /// Independent polytope workers; none uses the normal single-job driver.
    pub parallel: Option<NonZeroUsize>,
    /// Enabled terminal output sections.
    pub terminal_output: crate::display::TerminalOutput,
    /// Human or machine-readable output.
    pub output_format: crate::cli::OutputFormat,
}
impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            parallel: None,
            terminal_output: crate::display::TerminalOutput {
                initial: true,
                running: true,
                table: true,
                final_summary: true,
            },
            output_format: crate::cli::OutputFormat::Human,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedTimings {
    lattice: f64,
    formula: f64,
    search: f64,
}
impl From<crate::job::StageTimes> for SavedTimings {
    fn from(t: crate::job::StageTimes) -> Self {
        Self {
            lattice: t.lattice,
            formula: t.formula,
            search: t.search,
        }
    }
}
impl From<SavedTimings> for crate::job::StageTimes {
    fn from(t: SavedTimings) -> Self {
        Self {
            lattice: t.lattice,
            formula: t.formula,
            search: t.search,
        }
    }
}
fn same_query(a: Settings, b: Settings) -> bool {
    a.restriction == b.restriction
        && a.goal == b.goal
        && a.regular == b.regular
        && a.regular_learn == b.regular_learn
        && a.flag == b.flag
        && a.flag_sat == b.flag_sat
        && a.max_solutions == b.max_solutions
        && a.max_regularity_tests == b.max_regularity_tests
        && a.timeout == b.timeout
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum Attempt {
    Completed { row: String },
    Failed { index: u64, error: String },
}

impl Attempt {
    fn index(&self) -> Option<u64> {
        match self {
            Self::Completed { row } => Row::parse(row)
                .filter(|r| r.status != Status::Skipped)
                .map(|r| r.index),
            Self::Failed { index, error } => (!error.is_empty()).then_some(*index),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyCheckpoint {
    version: u32,
    input: PathBuf,
    input_sha256: String,
    total: u64,
    settings: SavedSettings,
    attempts: Vec<Attempt>,
    interrupted: Option<u64>,
}

impl LegacyCheckpoint {
    fn validate(&self) -> Result<Settings, ResumeError> {
        let invalid = || {
            ResumeError::Invalid(
                "checkpoint schema, indices, or result rows are inconsistent".into(),
            )
        };
        if self.version != 1
            || !self.input.is_absolute()
            || self.input_sha256.len() != 64
            || !self
                .input_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || u64::try_from(self.attempts.len()).unwrap_or(u64::MAX) > self.total
        {
            return Err(invalid());
        }
        for (index, attempt) in (1_u64..).zip(&self.attempts) {
            if attempt.index() != Some(index) {
                return Err(invalid());
            }
        }
        if let Some(index) = self.interrupted {
            let offset = offset(index)?;
            if index > self.total
                || offset > self.attempts.len()
                || matches!(self.attempts.get(offset), Some(Attempt::Completed { .. }))
            {
                return Err(invalid());
            }
        }
        self.settings.decode()
    }
}

/// Version two permits sparse attempts and tracks every in-flight job. Keep
/// separate wire types so legacy files cannot silently acquire v2 fields.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SparseCheckpoint {
    version: u32,
    input: PathBuf,
    input_sha256: String,
    total: u64,
    settings: SavedSettings,
    attempts: Vec<Attempt>,
    active: Vec<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CurrentCheckpoint {
    version: u32,
    input: PathBuf,
    input_sha256: String,
    total: u64,
    settings: SavedSettings,
    attempts: Vec<Attempt>,
    active: Vec<u64>,
    runtime: RuntimeOptions,
    elapsed: Duration,
    started_unix: Option<Duration>,
    legacy_timing: bool,
    #[serde(deserialize_with = "unique_map")]
    attempt_elapsed: BTreeMap<u64, Duration>,
    #[serde(deserialize_with = "unique_map")]
    timings: BTreeMap<u64, SavedTimings>,
    settings_history: Vec<SavedSettings>,
    #[serde(deserialize_with = "unique_map")]
    result_settings: BTreeMap<u64, usize>,
}

fn unique_map<'de, D, V>(deserializer: D) -> Result<BTreeMap<u64, V>, D::Error>
where
    D: serde::Deserializer<'de>,
    V: Deserialize<'de>,
{
    struct Unique<V>(std::marker::PhantomData<V>);
    impl<'de, V: Deserialize<'de>> serde::de::Visitor<'de> for Unique<V> {
        type Value = BTreeMap<u64, V>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a map with unique polytope indices")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut access: A,
        ) -> Result<Self::Value, A::Error> {
            let mut values = BTreeMap::new();
            while let Some((key, value)) = access.next_entry::<String, V>()? {
                let key: u64 = key.parse().map_err(serde::de::Error::custom)?;
                if values.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate polytope index"));
                }
            }
            Ok(values)
        }
    }
    deserializer.deserialize_map(Unique(std::marker::PhantomData))
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Checkpoint {
    Current(CurrentCheckpoint),
    Legacy(LegacyCheckpoint),
    Sparse(SparseCheckpoint),
}

/// Ordered maps provide indexed lookup without allocating for unstarted jobs.
/// Serialize map values as the schema's attempt array, borrowing all row text.
#[derive(Clone, Debug, Serialize)]
struct State {
    version: u32,
    input: PathBuf,
    input_sha256: String,
    total: u64,
    settings: SavedSettings,
    #[serde(serialize_with = "serialize_attempts")]
    attempts: BTreeMap<u64, Attempt>,
    active: BTreeSet<u64>,
    runtime: RuntimeOptions,
    #[serde(skip_serializing)]
    elapsed: Duration,
    started_unix: Option<Duration>,
    legacy_timing: bool,
    #[serde(skip_serializing)]
    attempt_elapsed: BTreeMap<u64, Duration>,
    timings: BTreeMap<u64, SavedTimings>,
    settings_history: Vec<SavedSettings>,
    result_settings: BTreeMap<u64, usize>,
    #[serde(skip)]
    running_since: Instant,
    #[serde(skip)]
    active_started: BTreeMap<u64, Instant>,
}

fn serialize_attempts<S: serde::Serializer>(
    attempts: &BTreeMap<u64, Attempt>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(attempts.values())
}

impl Checkpoint {
    fn into_state(self) -> Result<State, ResumeError> {
        let mut current = None;
        let sparse = match self {
            Self::Current(saved) => {
                if saved.version != 3 {
                    return Err(ResumeError::Invalid(
                        "unsupported checkpoint version".into(),
                    ));
                }
                let sparse = SparseCheckpoint {
                    version: 2,
                    input: saved.input.clone(),
                    input_sha256: saved.input_sha256.clone(),
                    total: saved.total,
                    settings: saved.settings.clone(),
                    attempts: saved.attempts.clone(),
                    active: saved.active.clone(),
                };
                current = Some(saved);
                sparse
            }
            Self::Legacy(legacy) => {
                legacy.validate()?;
                SparseCheckpoint {
                    version: 2,
                    input: legacy.input,
                    input_sha256: legacy.input_sha256,
                    total: legacy.total,
                    settings: legacy.settings,
                    attempts: legacy.attempts,
                    // A legacy retry can have both a failed attempt and an
                    // interrupted marker. Validation above checks that case;
                    // all interrupted work restarts, so discard the marker.
                    active: Vec::new(),
                }
            }
            Self::Sparse(sparse) => sparse,
        };
        let invalid = || {
            ResumeError::Invalid(
                "checkpoint schema, indices, or result rows are inconsistent".into(),
            )
        };
        if sparse.version != 2
            || !sparse.input.is_absolute()
            || sparse.input_sha256.len() != 64
            || !sparse
                .input_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid());
        }
        sparse.settings.decode()?;
        let mut attempts = BTreeMap::new();
        for attempt in sparse.attempts {
            let index = attempt.index().ok_or_else(invalid)?;
            if index == 0 || index > sparse.total || attempts.insert(index, attempt).is_some() {
                return Err(invalid());
            }
        }
        let mut active = BTreeSet::new();
        for index in sparse.active {
            if index == 0
                || index > sparse.total
                || attempts.contains_key(&index)
                || !active.insert(index)
            {
                return Err(invalid());
            }
        }
        let (
            runtime,
            elapsed,
            started_unix,
            legacy_timing,
            attempt_elapsed,
            timings,
            settings_history,
            result_settings,
        ) = if let Some(saved) = current {
            if saved.settings_history.is_empty() {
                return Err(invalid());
            }
            for settings in &saved.settings_history {
                settings.decode()?;
            }
            for (&index, &origin) in &saved.result_settings {
                if !matches!(attempts.get(&index), Some(Attempt::Completed { .. }))
                    || origin >= saved.settings_history.len()
                {
                    return Err(invalid());
                }
            }
            if attempts.iter().any(|(index, attempt)| {
                matches!(attempt, Attempt::Completed { .. })
                    && !saved.result_settings.contains_key(index)
            }) {
                return Err(invalid());
            }
            if saved.attempt_elapsed.keys().any(|index| {
                *index == 0
                    || *index > sparse.total
                    || matches!(attempts.get(index), Some(Attempt::Completed { .. }))
            }) {
                return Err(invalid());
            }
            for (&index, timing) in &saved.timings {
                if !matches!(attempts.get(&index), Some(Attempt::Completed { .. }))
                    || [timing.lattice, timing.formula, timing.search]
                        .into_iter()
                        .any(|v| Duration::try_from_secs_f64(v).is_err())
                {
                    return Err(invalid());
                }
            }
            (
                saved.runtime,
                saved.elapsed,
                saved.started_unix,
                saved.legacy_timing,
                saved.attempt_elapsed,
                saved.timings,
                saved.settings_history,
                saved.result_settings,
            )
        } else {
            let mut elapsed = Duration::ZERO;
            let mut origins = BTreeMap::new();
            for (&index, attempt) in &attempts {
                if let Attempt::Completed { row } = attempt {
                    let row = Row::parse(row).ok_or_else(invalid)?;
                    elapsed = elapsed.saturating_add(
                        Duration::try_from_secs_f64(row.seconds).map_err(|_| invalid())?,
                    );
                    origins.insert(index, 0);
                }
            }
            (
                RuntimeOptions::default(),
                elapsed,
                None,
                true,
                BTreeMap::new(),
                BTreeMap::new(),
                vec![sparse.settings.clone()],
                origins,
            )
        };
        Ok(State {
            version: 3,
            input: sparse.input,
            input_sha256: sparse.input_sha256,
            total: sparse.total,
            settings: sparse.settings,
            attempts,
            // Every active job is retried; completed rows alone are reused.
            active: BTreeSet::new(),
            runtime,
            elapsed,
            started_unix,
            legacy_timing,
            attempt_elapsed,
            timings,
            settings_history,
            result_settings,
            running_since: Instant::now(),
            active_started: BTreeMap::new(),
        })
    }
}

/// Object JSON is reserved for snapshots; malformed snapshots must fail strict
/// checkpoint decoding instead of falling back to the point-input parser.
pub(crate) fn is_checkpoint(path: &Path) -> Result<bool, ResumeError> {
    if !fs::metadata(path)?.is_file() {
        return Err(ResumeError::Invalid("input must be a regular file".into()));
    }
    let mut reader = BufReader::new(File::open(path)?);
    let mut byte = [0u8; 1];
    loop {
        if reader.read(&mut byte)? == 0 {
            return Ok(false);
        }
        if let Some(&value) = byte.first()
            && !value.is_ascii_whitespace()
        {
            return Ok(value == b'{');
        }
    }
}

fn offset(index: u64) -> Result<usize, ResumeError> {
    index
        .checked_sub(1)
        .and_then(|i| usize::try_from(i).ok())
        .ok_or_else(|| ResumeError::Invalid("invalid polytope index".into()))
}

/// A single run, with input provenance and retained preliminary results.
///
/// The session does not install a global signal handler until
/// [`Session::install_ctrlc`] is called by the executable.
pub struct Session {
    snapshot: tempfile::NamedTempFile,
    checkpoint: PathBuf,
    original_input: PathBuf,
    settings: Settings,
    total: u64,
    warnings: Vec<String>,
    eta_ids: Option<Arc<[u16]>>,
    state: Arc<Mutex<State>>,
    running: Arc<Mutex<bool>>,
}

impl Session {
    /// Open a new run or restore a validated checkpoint. `None` settings on
    /// resume restore the saved computation options; explicit mismatches fail.
    /// The default checkpoint path is `INPUT.resume.json`, or the resume path
    /// when resuming. All computation must read [`Self::snapshot_path`].
    pub fn open(
        input_path: Option<&Path>,
        resume_path: Option<&Path>,
        checkpoint_path: Option<&Path>,
        requested_settings: Option<Settings>,
    ) -> Result<Self, ResumeError> {
        Self::open_resolved(
            input_path,
            resume_path,
            checkpoint_path,
            |saved, runtime| {
                let settings = match (saved, requested_settings) {
                    (Some(saved), Some(requested)) if saved != requested => {
                        return Err(ResumeError::Invalid(
                            "computation settings differ from the resume file".into(),
                        ));
                    }
                    (Some(saved), _) => saved,
                    (None, Some(requested)) => requested,
                    (None, None) => {
                        return Err(ResumeError::Invalid(
                            "new runs require computation settings".into(),
                        ));
                    }
                };
                Ok((settings, runtime))
            },
        )
    }

    /// Open a positional point-input file or a checkpoint, merging explicitly
    /// selected options through `resolve` before validating the final settings.
    pub fn open_positional(
        path: &Path,
        checkpoint_path: Option<&Path>,
        resolve: impl FnOnce(
            Option<Settings>,
            RuntimeOptions,
        ) -> Result<(Settings, RuntimeOptions), ResumeError>,
    ) -> Result<Self, ResumeError> {
        if is_checkpoint(path)? {
            Self::open_resolved(None, Some(path), checkpoint_path, resolve)
        } else {
            Self::open_resolved(Some(path), None, checkpoint_path, resolve)
        }
    }

    fn open_resolved(
        input_path: Option<&Path>,
        resume_path: Option<&Path>,
        checkpoint_path: Option<&Path>,
        resolve: impl FnOnce(
            Option<Settings>,
            RuntimeOptions,
        ) -> Result<(Settings, RuntimeOptions), ResumeError>,
    ) -> Result<Self, ResumeError> {
        let saved = resume_path
            .map(|path| -> Result<Checkpoint, ResumeError> {
                Ok(serde_json::from_reader(BufReader::new(File::open(path)?))?)
            })
            .transpose()?
            .map(Checkpoint::into_state)
            .transpose()?;
        let saved_settings = saved.as_ref().map(|s| s.settings.decode()).transpose()?;
        let (settings, runtime) = resolve(
            saved_settings,
            saved
                .as_ref()
                .map_or_else(RuntimeOptions::default, |s| s.runtime),
        )?;
        settings.validate()?;
        if settings.max_regularity_tests.is_some() && !(settings.regular && settings.flag) {
            return Err(ResumeError::Invalid("--max-regularity-tests requires both regular and flag filtering in the resolved settings".into()));
        }
        let mut warnings = Vec::new();
        if settings.regular_learn {
            warnings.push("--regular-learn skips nonregular triangulations: triangulation and flag counts are observed lower bounds; only requested regular/quadratic target counts remain exact after a complete search".into());
        }
        if settings.flag_sat && settings.flag {
            warnings.push("--flag-sat already enforces flagness in SAT; --flag is redundant and will still independently check every triangulation".into());
        }
        if let Some(previous) = saved_settings {
            for (name, old, new) in [
                ("--regular", previous.regular, settings.regular),
                ("--flag", previous.flag, settings.flag),
                ("--flag-sat", previous.flag_sat, settings.flag_sat),
            ] {
                if !old && new {
                    warnings.push(format!("enabling {name} on a resumed run is almost never what you want: completed results are kept and were not checked under this new request"));
                }
            }
        }
        if let Some(saved) = &saved {
            if saved.result_settings.values().any(|&i| {
                saved
                    .settings_history
                    .get(i)
                    .and_then(|s| s.decode().ok())
                    .is_some_and(|s| !same_query(s, settings))
            }) {
                warnings.push("this run mixes different requests: completed results from earlier settings are kept unchanged; combined counts do not describe one uniform request".into());
            }
            if saved.legacy_timing {
                warnings.push("legacy snapshot timing is an estimate from completed job seconds (not exact parallel wall time); earlier interrupted work and stage timings were not recorded".into());
            }
            if !saved.attempt_elapsed.is_empty() {
                warnings.push("unfinished jobs restart from the beginning; previous attempt time is retained, but partial solver counters are not restored".into());
            }
        }
        let input_path = input_path
            .or_else(|| saved.as_ref().map(|s| s.input.as_path()))
            .ok_or_else(|| ResumeError::Invalid("an input or snapshot file is required".into()))?;
        let original_input = fs::canonicalize(input_path)?;
        if !original_input.is_file() {
            return Err(ResumeError::Invalid("input must be a regular file".into()));
        }
        if let Some(resume_path) = resume_path {
            reject_alias(&original_input, resume_path)?;
        }
        let default_checkpoint = {
            let mut name = original_input.as_os_str().to_owned();
            name.push(".resume.json");
            PathBuf::from(name)
        };
        let checkpoint = absolute_destination(
            checkpoint_path
                .or(resume_path)
                .unwrap_or(&default_checkpoint),
        )?;
        reject_alias(&original_input, &checkpoint)?;
        let mut source = File::open(&original_input)?;
        let mut snapshot = tempfile::NamedTempFile::new()?;
        let mut hasher = Sha256::new();
        let mut bytes = [0_u8; 64 * 1024];
        loop {
            let n = source.read(&mut bytes)?;
            if n == 0 {
                break;
            }
            let block = bytes
                .get(..n)
                .ok_or_else(|| ResumeError::Invalid("input read size is invalid".into()))?;
            snapshot.write_all(block)?;
            hasher.update(block);
        }
        snapshot.flush()?;
        let hash = format!("{:x}", hasher.finalize());
        if saved.as_ref().is_some_and(|s| s.input_sha256 != hash) {
            return Err(ResumeError::Invalid(
                "input content differs from the resume file; completed results cannot be reused"
                    .into(),
            ));
        }
        let mut eta_ids = runtime.terminal_output.running.then(Vec::new);
        let mut total = 0_u64;
        for record in input::open_points(snapshot.path())? {
            let record = record?;
            if let Some(ids) = &mut eta_ids {
                ids.push(crate::eta::descriptor(&record.vertices));
            }
            total = total
                .checked_add(1)
                .ok_or_else(|| ResumeError::Invalid("too many input records".into()))?;
        }
        let state = if let Some(mut saved) = saved {
            if saved.total != total {
                return Err(ResumeError::Invalid(
                    "checkpoint record count differs from input".into(),
                ));
            }
            saved.input = original_input.clone();
            saved.settings = settings.into();
            saved.runtime = runtime;
            if !saved.settings_history.contains(&saved.settings) {
                saved.settings_history.push(saved.settings.clone());
            }
            saved.running_since = Instant::now();
            saved
        } else {
            State {
                version: 3,
                input: original_input.clone(),
                input_sha256: hash,
                total,
                settings: settings.into(),
                attempts: BTreeMap::new(),
                active: BTreeSet::new(),
                runtime,
                elapsed: Duration::ZERO,
                started_unix: SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .ok(),
                legacy_timing: false,
                attempt_elapsed: BTreeMap::new(),
                timings: BTreeMap::new(),
                settings_history: vec![settings.into()],
                result_settings: BTreeMap::new(),
                running_since: Instant::now(),
                active_started: BTreeMap::new(),
            }
        };
        Ok(Self {
            snapshot,
            checkpoint,
            original_input,
            settings,
            total,
            warnings,
            eta_ids: eta_ids.map(Arc::from),
            state: Arc::new(Mutex::new(state)),
            running: Arc::new(Mutex::new(true)),
        })
    }

    pub(crate) fn estimator(&self) -> Result<Option<crate::eta::Estimator>, ResumeError> {
        let Some(ids) = &self.eta_ids else {
            return Ok(None);
        };
        let state = self.state()?;
        let mut estimator = crate::eta::Estimator::new(Arc::clone(ids));
        for (&index, attempt) in &state.attempts {
            if matches!(attempt, Attempt::Completed { .. }) {
                // v1-v3 lack per-result execution topology (outer workers and
                // internal Rayon threads). Retire old work but calibrate anew.
                estimator.restore_completed(index);
            }
        }
        Ok(Some(estimator))
    }

    /// Restored or newly selected computation settings.
    pub fn settings(&self) -> Settings {
        self.settings
    }
    /// Canonical path of the original user input.
    pub fn input_path(&self) -> &Path {
        &self.original_input
    }
    /// The immutable input snapshot to use for all computation.
    pub fn snapshot_path(&self) -> &Path {
        self.snapshot.path()
    }
    /// Destination written when interrupted.
    pub fn checkpoint_path(&self) -> &Path {
        &self.checkpoint
    }
    /// Number of validated input records.
    pub fn total(&self) -> u64 {
        self.total
    }

    /// Runtime options after saved preferences and explicit overrides are merged.
    pub fn runtime(&self) -> Result<RuntimeOptions, ResumeError> {
        Ok(self.state()?.runtime)
    }
    /// Diagnostics which must be displayed even with terminal output disabled.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
    /// Active-session wall time, including previous sessions but excluding offline gaps.
    pub fn elapsed(&self) -> Result<Duration, ResumeError> {
        let state = self.state()?;
        Ok(state.elapsed.saturating_add(state.running_since.elapsed()))
    }
    /// Original run start as a duration since the Unix epoch; unknown for legacy files.
    pub fn started_unix(&self) -> Result<Option<Duration>, ResumeError> {
        Ok(self.state()?.started_unix)
    }
    /// Whether a retained result was computed for the current request. Execution
    /// backend, solver and mode changes do not change the requested result.
    pub(crate) fn result_matches_request(&self, index: u64) -> Result<bool, ResumeError> {
        let state = self.state()?;
        let saved = state
            .result_settings
            .get(&index)
            .and_then(|&origin| state.settings_history.get(origin))
            .ok_or_else(|| {
                ResumeError::Invalid("completed result has no request provenance".into())
            })?;
        Ok(same_query(saved.decode()?, self.settings))
    }

    /// All retained completed rows, with their saved completed-stage timings.
    pub fn completed_rows(
        &self,
    ) -> Result<Vec<(Row, Option<crate::job::StageTimes>)>, ResumeError> {
        let state = self.state()?;
        Ok(state
            .attempts
            .iter()
            .filter_map(|(index, a)| match a {
                Attempt::Completed { row } => {
                    Row::parse(row).map(|r| (r, state.timings.get(index).copied().map(Into::into)))
                }
                _ => None,
            })
            .collect())
    }

    fn state(&self) -> Result<MutexGuard<'_, State>, ResumeError> {
        self.state
            .lock()
            .map_err(|_| ResumeError::Invalid("checkpoint state lock was poisoned".into()))
    }

    /// Return a previously completed row; failures and interrupted work are
    /// intentionally absent and must be recomputed.
    pub fn completed(&self, index: u64) -> Result<Option<Row>, ResumeError> {
        let state = self.state()?;
        offset(index)?;
        match state.attempts.get(&index) {
            Some(Attempt::Completed { row }) => Ok(Row::parse(row)),
            _ => Ok(None),
        }
    }

    /// Mark a job running. Independent indices may start and finish in any
    /// order; completed rows must be replayed rather than started again.
    pub fn begin(&self, index: u64) -> Result<(), ResumeError> {
        let mut state = self.state()?;
        if index == 0
            || index > state.total
            || state.active.contains(&index)
            || matches!(state.attempts.get(&index), Some(Attempt::Completed { .. }))
        {
            return Err(ResumeError::Invalid(
                "job index must be valid, inactive, and not already completed".into(),
            ));
        }
        // A retried failure becomes active, never both failed and active.
        state.attempts.remove(&index);
        state.active.insert(index);
        state.active_started.insert(index, Instant::now());
        Ok(())
    }

    /// Retain a completed result. Capped and regularity-limited rows retain their
    /// explicit status and remain completed according to the original settings.
    pub fn finish(&self, row: Row) -> Result<Row, ResumeError> {
        self.finish_timed(row, None)
    }

    /// Complete a job and retain its stage timings. Prior interrupted/failed
    /// attempt time is added to the newly computed row's elapsed cost.
    pub fn finish_timed(
        &self,
        mut row: Row,
        timings: Option<crate::job::StageTimes>,
    ) -> Result<Row, ResumeError> {
        let mut state = self.state()?;
        let prior = state
            .attempt_elapsed
            .get(&row.index)
            .copied()
            .unwrap_or_default();
        row.seconds += prior.as_secs_f64();
        let text = row.to_string();
        if row.status == Status::Skipped || Row::parse(&text).is_none() {
            return Err(ResumeError::Invalid(
                "a completed job must have a valid result row".into(),
            ));
        }
        if let Some(t) = timings
            && [t.lattice, t.formula, t.search]
                .into_iter()
                .any(|v| Duration::try_from_secs_f64(v).is_err())
        {
            return Err(ResumeError::Invalid("invalid stage timings".into()));
        }
        if !state.active.remove(&row.index) {
            return Err(ResumeError::Invalid(
                "result does not belong to an active polytope".into(),
            ));
        }
        state.active_started.remove(&row.index);
        state.attempt_elapsed.remove(&row.index);
        let origin = state
            .settings_history
            .iter()
            .position(|s| s == &state.settings)
            .ok_or_else(|| ResumeError::Invalid("missing current settings provenance".into()))?;
        state.result_settings.insert(row.index, origin);
        if let Some(t) = timings {
            state.timings.insert(row.index, t.into());
        }
        state
            .attempts
            .insert(row.index, Attempt::Completed { row: text });
        Ok(row)
    }

    /// Retain an unsuccessful attempt for diagnosis; resume retries it.
    pub fn fail(&self, index: u64, error: String) -> Result<(), ResumeError> {
        if error.is_empty() {
            return Err(ResumeError::Invalid(
                "failed jobs require an error description".into(),
            ));
        }
        self.record(index, Attempt::Failed { index, error })
    }

    fn record(&self, index: u64, attempt: Attempt) -> Result<(), ResumeError> {
        let mut state = self.state()?;
        if !state.active.remove(&index) {
            return Err(ResumeError::Invalid(
                "result does not belong to an active polytope".into(),
            ));
        }
        if let Some(started) = state.active_started.remove(&index) {
            let carried = state.attempt_elapsed.entry(index).or_default();
            *carried = carried.saturating_add(started.elapsed());
        }
        state.attempts.insert(index, attempt);
        Ok(())
    }

    /// Atomically save a consistent preliminary-results snapshot. The old file
    /// remains intact if serialization or writing fails before replacement.
    pub fn save(&self) -> Result<(), ResumeError> {
        // Hold the state lock through replacement so simultaneous saves cannot
        // overwrite a newer snapshot with an older one. Row strings are borrowed.
        let state = self.state()?;
        save(&state, &self.checkpoint)
    }

    /// Install the executable's single process-wide Ctrl+C handler. It saves a
    /// coherent snapshot and exits 130 on success, or 1 with an error on failure.
    /// This does not introduce callbacks in any numerical inner loop.
    pub fn install_ctrlc(&self) -> Result<(), ResumeError> {
        let state = Arc::clone(&self.state);
        let running = Arc::clone(&self.running);
        let destination = self.checkpoint.clone();
        let snapshot = self.snapshot.path().to_owned();
        ctrlc::set_handler(move || {
            // Serialize saving against Session::drop, including early error
            // returns. Once saving starts the main thread cannot exit before
            // the checkpoint is durable; once the session ends, a late signal
            // cannot start a stale save. Hold this guard through process exit.
            let running = running.lock().unwrap_or_else(|error| error.into_inner());
            if !*running {
                return;
            }
            let result = state
                .lock()
                .map_err(|_| ResumeError::Invalid("checkpoint state lock was poisoned".into()))
                .and_then(|state| save(&state, &destination));
            let code = match result {
                Ok(()) => {
                    let _ = writeln!(
                        io::stderr(),
                        "\nInterrupted. Preliminary results saved to {}. Resume with: unitrisat {:?}",
                        destination.display(),
                        destination
                    );
                    130
                }
                Err(error) => {
                    let _ = writeln!(
                        io::stderr(),
                        "\nInterrupted; checkpoint could not be saved: {error}"
                    );
                    1
                }
            };
            let _ = fs::remove_file(&snapshot);
            std::process::exit(code);
        })?;
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // If the signal handler won the gate, this blocks until its save and
        // process exit. Otherwise this is the run's completion boundary.
        let mut running = self
            .running
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *running = false;
    }
}

fn absolute_destination(path: &Path) -> Result<PathBuf, ResumeError> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let parent = absolute
        .parent()
        .ok_or_else(|| ResumeError::Invalid("checkpoint needs a filename".into()))?;
    let name = absolute
        .file_name()
        .ok_or_else(|| ResumeError::Invalid("checkpoint needs a filename".into()))?;
    Ok(fs::canonicalize(parent)?.join(name))
}

fn reject_alias(input: &Path, destination: &Path) -> Result<(), ResumeError> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(ResumeError::Invalid(
                    "checkpoint/resume path must not be a symbolic link".into(),
                ));
            }
            if !metadata.is_file() || same_file::is_same_file(input, destination)? {
                return Err(ResumeError::Invalid(
                    "checkpoint/resume path must be a separate regular file, never the input"
                        .into(),
                ));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn save(state: &State, destination: &Path) -> Result<(), ResumeError> {
    reject_alias(&state.input, destination)?;
    let parent = destination
        .parent()
        .ok_or_else(|| ResumeError::Invalid("checkpoint needs a parent directory".into()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    #[derive(Serialize)]
    struct Snapshot<'a> {
        #[serde(flatten)]
        state: &'a State,
        elapsed: Duration,
        attempt_elapsed: BTreeMap<u64, Duration>,
    }
    let mut attempt_elapsed = state.attempt_elapsed.clone();
    for (&index, started) in &state.active_started {
        let carried = attempt_elapsed.entry(index).or_default();
        *carried = carried.saturating_add(started.elapsed());
    }
    let snapshot = Snapshot {
        state,
        elapsed: state.elapsed.saturating_add(state.running_since.elapsed()),
        attempt_elapsed,
    };
    serde_json::to_writer_pretty(&mut temporary, &snapshot)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(destination)
        .map_err(|error| ResumeError::Io(error.error))?;
    // Directory fsync makes the rename durable on Unix. Windows does not permit
    // opening ordinary directory handles through std::fs::File.
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests;

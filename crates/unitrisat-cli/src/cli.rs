//! Direct file-based triangulation searches with human or TSV output.

use crate::display::{Reporter, TerminalOutput};
use crate::job::{self, IntersectionBackend, Mode, Row, Settings, Status, TSV_HEADER};
use crate::resume::{ResumeError, RuntimeOptions, Session};
use clap::{Args, CommandFactory, FromArgMatches, Parser, ValueEnum, parser::ValueSource};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    io::{self, Write},
    num::{NonZeroU64, NonZeroUsize},
    path::PathBuf,
    time::Duration,
};
use unitrisat::{
    enumerate::Goal,
    input::{self, InputError},
    lattice::Restriction,
    solve::Backend,
};

/// Exact triangulation searches for every polytope in an input file.
#[derive(Clone, Debug, Parser)]
#[command(name = "unitrisat", version, about)]
pub struct Cli {
    /// Point-input file or saved run snapshot (detected automatically).
    #[arg(value_name = "FILE", required = true)]
    pub input: Option<PathBuf>,
    /// Write interrupted-run state to this file (default: FILE.resume.json).
    #[arg(long, value_name = "FILE")]
    pub checkpoint: Option<PathBuf>,
    /// Output sections: initial,running,table,final, or all/none.
    #[arg(long, value_name = "SECTIONS", default_value = "all", value_parser = TerminalOutput::parse)]
    pub terminal_output: TerminalOutput,
    /// Human-readable progress and summaries, or machine-readable TSV rows.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    pub output_format: OutputFormat,
    /// Process N polytopes concurrently (bare flag: 4; 0 disables). Results are emitted in
    /// completion order with their original input indices.
    #[arg(long, value_name = "N", num_args = 0..=1, default_missing_value = "4", require_equals = true)]
    pub parallel: Option<usize>,
    /// Search options.
    #[command(flatten)]
    pub search: SearchArgs,
    #[arg(skip)]
    explicit: BTreeSet<String>,
    #[arg(skip)]
    parsed_search: Option<SearchArgs>,
    #[arg(skip)]
    parsed_runtime: Option<RuntimeOptions>,
}

impl Cli {
    /// Parse direct file arguments without rewriting their meaning.
    pub fn try_parse_input_from<I, T>(args: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let matches = Self::command().try_get_matches_from(args)?;
        let mut cli = Self::from_arg_matches(&matches)?;
        cli.explicit = matches
            .ids()
            .filter(|id| matches.value_source(id.as_str()) == Some(ValueSource::CommandLine))
            .map(ToString::to_string)
            .collect();
        cli.parsed_search = Some(cli.search.clone());
        cli.parsed_runtime = Some(cli.runtime_values());
        let snapshot = cli
            .input
            .as_deref()
            .is_some_and(|p| crate::resume::is_checkpoint(p).unwrap_or(false));
        if !snapshot {
            let settings = cli.search.settings(Mode::Serial, Solver::Picosat);
            settings.validate().map_err(|error| {
                clap::Error::raw(clap::error::ErrorKind::ArgumentConflict, error)
            })?;
            if settings.max_regularity_tests.is_some() && !(settings.regular && settings.flag) {
                return Err(clap::Error::raw(
                    clap::error::ErrorKind::ArgumentConflict,
                    "--max-regularity-tests requires --regular and --flag",
                ));
            }
        }
        Ok(cli)
    }
    fn runtime_values(&self) -> RuntimeOptions {
        RuntimeOptions {
            parallel: self.parallel.and_then(NonZeroUsize::new),
            terminal_output: self.terminal_output,
            output_format: self.output_format,
        }
    }

    fn resolve(
        &self,
        saved: Option<Settings>,
        mut runtime: RuntimeOptions,
    ) -> Result<(Settings, RuntimeOptions), ResumeError> {
        let values = self.search.settings(Mode::Serial, Solver::Picosat);
        let baseline = self
            .parsed_search
            .as_ref()
            .map(|s| s.settings(Mode::Serial, Solver::Picosat))
            .unwrap_or_default();
        let mut settings = saved.unwrap_or(values);
        macro_rules! override_field {
            ($key:literal, $field:ident) => {
                if self.explicit.contains($key) || values.$field != baseline.$field {
                    settings.$field = values.$field;
                }
            };
        }
        override_field!("intersection_backend", intersection_backend);
        override_field!("simplices", restriction);
        override_field!("mode", mode);
        override_field!("solver", backend);
        override_field!("all", goal);
        override_field!("regular", regular);
        override_field!("regular_learn", regular_learn);
        override_field!("flag", flag);
        override_field!("flag_sat", flag_sat);
        override_field!("max_solutions", max_solutions);
        override_field!("max_regularity_tests", max_regularity_tests);
        override_field!("timeout", timeout);
        let values = self.runtime_values();
        let baseline = self.parsed_runtime.unwrap_or_default();
        if saved.is_none() {
            runtime = values;
        }
        macro_rules! runtime_field {
            ($key:literal, $field:ident) => {
                if self.explicit.contains($key) || values.$field != baseline.$field {
                    runtime.$field = values.$field;
                }
            };
        }
        runtime_field!("parallel", parallel);
        runtime_field!("terminal_output", terminal_output);
        runtime_field!("output_format", output_format);
        Ok((settings, runtime))
    }
}

/// Standard output format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    /// Live progress, timing table and final counts.
    Human,
    /// Tab-separated result rows; reports go to standard error.
    Tsv,
}

/// The SAT solver.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Solver {
    /// PicoSAT.
    Picosat,
    /// CaDiCaL.
    Cadical,
    /// Kissat (first, unfiltered searches in serial or split mode).
    Kissat,
}

/// Which simplices a triangulation may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Simplices {
    /// Unimodular simplices (normalized volume 1).
    Unimodular,
    /// Any full-dimensional simplex.
    Nondegenerate,
}

/// Typed search options.
#[derive(Clone, Debug, PartialEq, Eq, Args)]
#[group(id = "search_options", multiple = true)]
pub struct SearchArgs {
    /// Intersection backend; gpu falls back to CPU when CUDA is unavailable.
    #[arg(long = "backend", visible_alias = "intersection-backend", value_enum, default_value_t = IntersectionBackend::Cpu)]
    pub intersection_backend: IntersectionBackend,
    /// Enumerate every triangulation instead of stopping at the first one of
    /// the requested type.
    #[arg(long, visible_alias = "find-all", action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true", default_value_t = false, require_equals = true)]
    pub all: bool,
    /// Test the triangulations for regularity (exact LP).
    #[arg(long, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true", default_value_t = false, require_equals = true)]
    pub regular: bool,
    /// Learn exact LP conflicts to skip nonregular candidates (requires --regular);
    /// triangulation and flag counts then describe only encountered candidates.
    #[arg(long, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true", default_value_t = false, require_equals = true)]
    pub regular_learn: bool,
    /// Test the triangulations for flagness; with --regular the requested
    /// type is quadratic (regular and flag).
    #[arg(long, visible_alias = "flag-triangulation", action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true", default_value_t = false, require_equals = true)]
    pub flag: bool,
    /// Encode flagness in SAT; --flag additionally checks every returned triangulation.
    #[arg(long, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true", default_value_t = false, require_equals = true)]
    pub flag_sat: bool,
    /// Solving mode [default: serial].
    #[arg(long, value_enum)]
    pub mode: Option<Mode>,
    /// SAT solver [default: picosat].
    #[arg(long, value_enum)]
    pub solver: Option<Solver>,
    /// Which simplices a triangulation may use.
    #[arg(long, value_enum, default_value_t = Simplices::Unimodular)]
    pub simplices: Simplices,
    /// Stop a polytope's search after N triangulations, 0 for no limit; the
    /// row is then "capped" and its counts are lower bounds.
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub max_solutions: u64,
    /// With --regular --flag: stop testing regularity on non-flag
    /// triangulations after N tests, 0 for no limit; the row is then
    /// "reglimit" if no regular triangulation was seen.
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub max_regularity_tests: u64,
    /// Interrupt a polytope's search after this many seconds; the row is
    /// then "capped" and its counts are lower bounds.
    #[arg(long, value_name = "SECONDS", value_parser = timeout)]
    pub timeout: Option<Duration>,
}

impl SearchArgs {
    /// The job settings, with the subcommand's `mode` and `solver` where
    /// none was given.
    pub fn settings(&self, mode: Mode, solver: Solver) -> Settings {
        Settings {
            intersection_backend: self.intersection_backend,
            restriction: match self.simplices {
                Simplices::Unimodular => Restriction::Unimodular,
                Simplices::Nondegenerate => Restriction::Nondegenerate,
            },
            mode: self.mode.unwrap_or(mode),
            backend: match self.solver.unwrap_or(solver) {
                Solver::Picosat => Backend::PicoSat,
                Solver::Cadical => Backend::Cadical,
                Solver::Kissat => Backend::Kissat,
            },
            goal: if self.all { Goal::All } else { Goal::First },
            regular: self.regular,
            regular_learn: self.regular_learn,
            flag: self.flag,
            flag_sat: self.flag_sat,
            max_solutions: NonZeroU64::new(self.max_solutions),
            max_regularity_tests: NonZeroU64::new(self.max_regularity_tests),
            timeout: self.timeout.filter(|t| !t.is_zero()),
        }
    }
}

/// Nonnegative finite seconds; zero clears a saved timeout.
fn timeout(text: &str) -> Result<Duration, String> {
    text.parse::<f64>()
        .ok()
        .filter(|&seconds| seconds >= 0.0)
        .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
        .ok_or_else(|| "expected a nonnegative number of seconds".to_owned())
}

/// Failure that ends a run.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// Starting or communicating with a parallel worker failed.
    #[error("parallel worker: {0}")]
    Worker(String),
    /// Creating or restoring resumable run state failed.
    #[error(transparent)]
    Resume(#[from] ResumeError),
    /// The input file could not be read.
    #[error("{path}: {source}")]
    Input {
        /// Original input path.
        path: PathBuf,
        /// Input decoding failure.
        source: InputError,
    },
    /// Writing output failed.
    #[error("cannot write the output: {0}")]
    Output(#[from] io::Error),
}

/// The counts of a run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// Polytopes run, failed ones included.
    pub polytopes: u64,
    /// With at least one triangulation.
    pub triangulatable: u64,
    /// With at least one regular triangulation.
    pub regular: u64,
    /// With at least one flag triangulation.
    pub flag: u64,
    /// With at least one quadratic triangulation.
    pub quadratic: u64,
    /// Retained results from a different request, excluded from current counts.
    pub historical: u64,
    /// Rows with status `capped`.
    pub capped: u64,
    /// Rows with status `reglimit`.
    pub reglimited: u64,
    /// Polytopes whose job failed (no row).
    pub failed: u64,
}

impl Tally {
    /// Counts `row` (not in [`Tally::polytopes`]).
    pub(crate) fn add(&mut self, row: &Row) {
        let bump = |n: &mut u64, yes: bool| *n = n.saturating_add(u64::from(yes));
        bump(&mut self.triangulatable, row.triangulations > 0);
        bump(&mut self.regular, row.regular > 0);
        bump(&mut self.flag, row.flag > 0);
        bump(&mut self.quadratic, row.quadratic > 0);
        bump(&mut self.capped, row.status == Status::Capped);
        bump(&mut self.reglimited, row.status == Status::RegLimit);
    }
}

/// `hh:mm:ss` of whole seconds.
pub(crate) fn hms(duration: Duration) -> String {
    let s = duration.as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        s.checked_div(3600).unwrap_or(0),
        s.checked_rem(3600)
            .and_then(|r| r.checked_div(60))
            .unwrap_or(0),
        s.checked_rem(60).unwrap_or(0)
    )
}

/// Execute with plain text streams (also suitable for redirected output).
pub fn execute<O: Write, E: Write>(cli: &Cli, out: &mut O, err: &mut E) -> Result<Tally, CliError> {
    execute_with_terminal(cli, out, err, false, false)
}

/// Execute while enabling terminal redraws only on actual terminal streams.
pub fn execute_with_terminal<O: Write, E: Write>(
    cli: &Cli,
    out: &mut O,
    err: &mut E,
    out_terminal: bool,
    err_terminal: bool,
) -> Result<Tally, CliError> {
    execute_run(cli, out, err, out_terminal, err_terminal, false)
}

/// Execute a console run, saving a checkpoint on Ctrl+C.
pub fn execute_interactive<O: Write, E: Write>(
    cli: &Cli,
    out: &mut O,
    err: &mut E,
    out_terminal: bool,
    err_terminal: bool,
) -> Result<Tally, CliError> {
    execute_run(cli, out, err, out_terminal, err_terminal, true)
}

fn execute_run<O: Write, E: Write>(
    cli: &Cli,
    out: &mut O,
    err: &mut E,
    out_terminal: bool,
    err_terminal: bool,
    signals: bool,
) -> Result<Tally, CliError> {
    let input = cli
        .input
        .as_deref()
        .ok_or_else(|| ResumeError::Invalid("an input or snapshot file is required".into()))?;
    let session = Session::open_positional(input, cli.checkpoint.as_deref(), |saved, runtime| {
        cli.resolve(saved, runtime)
    })?;
    for warning in session.warnings() {
        writeln!(err, "warning: {warning}")?;
    }
    let mut resolved = cli.clone();
    let runtime = session.runtime()?;
    resolved.parallel = runtime.parallel.map(NonZeroUsize::get);
    resolved.terminal_output = runtime.terminal_output;
    resolved.output_format = runtime.output_format;
    let cli = &resolved;
    if signals {
        session.install_ctrlc()?;
    }
    let settings = session.settings();
    let total = session.total();
    let input_error = |source| CliError::Input {
        path: session.input_path().to_owned(),
        source,
    };
    if cli.output_format == OutputFormat::Tsv {
        let header = match (settings.flag_sat, settings.regular) {
            (true, true) => "# idx\tstatus\tn_flag\tn_quadratic\tseconds",
            (true, false) => "# idx\tstatus\tn_flag\tseconds",
            (false, _) => TSV_HEADER,
        };
        writeln!(out, "{header}")?;
    }
    let terminal = if cli.output_format == OutputFormat::Human {
        out_terminal
    } else {
        // Two terminal streams can share one screen. Redrawing the diagnostic
        // stream must never erase TSV rows printed to standard output.
        err_terminal && !out_terminal
    };
    if let Some(workers) = cli.parallel {
        return crate::parallel::execute(cli, &session, workers, terminal, out, err);
    }
    let mut report = Reporter::new(cli.terminal_output, terminal, total, settings);
    report.restore_clock(session.elapsed()?, session.started_unix()?);
    report.set_estimator(session.estimator()?);
    let mut tally = Tally::default();
    for (row, timings) in session.completed_rows()? {
        tally.polytopes = tally.polytopes.saturating_add(1);
        let compatible = session.result_matches_request(row.index)?;
        if compatible {
            tally.add(&row);
        } else {
            tally.historical = tally.historical.saturating_add(1);
        }
        report.restore_row(&row, timings, &tally, compatible);
        if cli.output_format == OutputFormat::Tsv {
            write_restored_result(&row, settings, compatible, out)?;
        }
    }
    {
        let stream: &mut dyn Write = if cli.output_format == OutputFormat::Human {
            out
        } else {
            err
        };
        report.initial(stream)?;
    }
    for (index, record) in
        (1_u64..).zip(input::open_points(session.snapshot_path()).map_err(input_error)?)
    {
        let record = record.map_err(input_error)?;
        if session.completed(index)?.is_some() {
            continue;
        }
        let mut stage_timings = None;
        let result;
        {
            let stream: &mut dyn Write = if cli.output_format == OutputFormat::Human {
                out
            } else {
                err
            };
            report.begin(index, stream)?;
            {
                session.begin(index)?;
                let mut write_error = None;
                result = if !cli.terminal_output.running {
                    job::run_detailed(
                        index,
                        &record.vertices,
                        &settings,
                        unitrisat::enumerate::Keep::None,
                    )
                    .map(|detailed| {
                        stage_timings = Some(detailed.timings);
                        write_error = report
                            .event(
                                job::Progress::Finished {
                                    timings: detailed.timings,
                                },
                                stream,
                            )
                            .err();
                        detailed.row
                    })
                } else {
                    job::run_with_progress(index, &record.vertices, &settings, &mut |event| {
                        if let job::Progress::Finished { timings } = event {
                            stage_timings = Some(timings);
                        }
                        if write_error.is_none() {
                            write_error = report.event(event, stream).err();
                        }
                    })
                };
                if let Some(error) = write_error {
                    return Err(error.into());
                }
            }
        }
        tally.polytopes = tally.polytopes.saturating_add(1);
        match result {
            Ok(row) => {
                report.eta_finished(index, crate::eta::sample(row.status, stage_timings));
                let row = session.finish_timed(row, stage_timings)?;
                tally.add(&row);
                report.row(&row);
                if cli.output_format == OutputFormat::Tsv {
                    write_result(&row, settings, out)?;
                    out.flush()?;
                }
            }
            Err(error) => {
                report.eta_finished(index, None);
                session.fail(index, error.to_string())?;
                tally.failed = tally.failed.saturating_add(1);
                report.diagnostic();
                writeln!(err, "error: polytope {index}: {error}")?;
            }
        }
        let stream: &mut dyn Write = if cli.output_format == OutputFormat::Human {
            out
        } else {
            err
        };
        report.completed(&tally, stream)?;
    }
    let stream: &mut dyn Write = if cli.output_format == OutputFormat::Human {
        out
    } else {
        err
    };
    report.finish(&tally, stream)?;
    Ok(tally)
}

/// CLI result projection; canonical library/checkpoint rows retain their schema.
pub(crate) fn write_result<W: Write>(row: &Row, settings: Settings, out: &mut W) -> io::Result<()> {
    if !settings.flag_sat {
        return row.write(out);
    }
    write!(out, "{}\t{}\t", row.index, row.status.as_str())?;
    if row.status == Status::Skipped {
        write!(out, "-1\t")?;
        if settings.regular {
            write!(out, "-1\t")?;
        }
    } else {
        write!(out, "{}\t", row.flag)?;
        if settings.regular {
            write!(out, "{}\t", row.quadratic)?;
        }
    }
    writeln!(out, "{:.4}", row.seconds)
}

/// A differently requested result is retained, but cannot answer the current query.
pub(crate) fn write_restored_result<W: Write>(
    row: &Row,
    settings: Settings,
    compatible: bool,
    out: &mut W,
) -> io::Result<()> {
    if compatible {
        return write_result(row, settings, out);
    }
    write!(out, "{}\thistorical\t", row.index)?;
    let columns = if settings.flag_sat {
        if settings.regular { 2 } else { 1 }
    } else {
        4
    };
    for _ in 0..columns {
        write!(out, "?\t")?;
    }
    writeln!(out, "{:.4}", row.seconds)
}

#[cfg(test)]
mod tests;

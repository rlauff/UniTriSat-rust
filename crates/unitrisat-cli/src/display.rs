//! Human-readable run output, updated only at coarse computation boundaries.

use crate::cli::{Tally, hms};
use crate::job::{IntersectionBackend, Mode, Progress, Row, Settings, StageTimes, Status};
use std::{
    fmt::Write as _,
    io::{self, Write},
    time::{Duration, Instant, SystemTime},
};
use unitrisat::{enumerate::Goal, lattice::Restriction, solve::Backend};

/// Independently selectable output sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalOutput {
    /// Configuration at the start of the run.
    pub initial: bool,
    /// Coalesced progress during the run.
    pub running: bool,
    /// Aggregate stage timings.
    pub table: bool,
    /// Final result counts.
    pub final_summary: bool,
}

impl TerminalOutput {
    /// Parse all, none, or a comma-separated list of section names.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim().to_ascii_lowercase();
        let all = text == "all";
        let mut output = Self {
            initial: all,
            running: all,
            table: all,
            final_summary: all,
        };
        if all || text == "none" {
            return Ok(output);
        }
        for section in text.split(',').map(str::trim) {
            match section {
                "initial" => output.initial = true,
                "running" => output.running = true,
                "table" => output.table = true,
                "final" => output.final_summary = true,
                _ => return Err("expected initial,running,table,final, or all/none".into()),
            }
        }
        Ok(output)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Timing {
    total: f64,
    maximum: f64,
    count: u64,
}
impl Timing {
    fn add(&mut self, seconds: f64) {
        self.total = (self.total + seconds).min(Duration::MAX.as_secs_f64());
        self.maximum = self.maximum.max(seconds);
        self.count = self.count.saturating_add(1);
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ClassificationCounts {
    triangulations: u64,
    regular: u64,
    non_regular: u64,
    flag: u64,
    non_flag: u64,
}

/// Mutable display state; contains no geometry or solver state.
pub(crate) struct Reporter {
    sections: TerminalOutput,
    terminal: bool,
    total: u64,
    settings: Settings,
    started: Instant,
    elapsed_before: Duration,
    original_start: Option<Duration>,
    last_update: Option<Instant>,
    drawn_lines: usize,
    index: u64,
    points: Option<usize>,
    simplices: Option<usize>,
    solving: bool,
    clauses_added: usize,
    interruptions: u64,
    incremental_available: bool,
    classification: Option<ClassificationCounts>,
    tally: Tally,
    no_triangulation: u64,
    targets: u64,
    timings: [Timing; 3],
    estimator: Option<crate::eta::Estimator>,
    active_started: Option<(u64, Instant)>,
}

impl Reporter {
    pub(crate) fn new(
        sections: TerminalOutput,
        terminal: bool,
        total: u64,
        settings: Settings,
    ) -> Self {
        Self {
            sections,
            terminal,
            total,
            settings,
            started: Instant::now(),
            elapsed_before: Duration::ZERO,
            original_start: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .ok(),
            last_update: None,
            drawn_lines: 0,
            index: 0,
            points: None,
            simplices: None,
            solving: false,
            clauses_added: 0,
            interruptions: 0,
            incremental_available: false,
            classification: None,
            tally: Tally::default(),
            no_triangulation: 0,
            targets: 0,
            timings: [Timing::default(); 3],
            estimator: None,
            active_started: None,
        }
    }

    pub(crate) fn set_estimator(&mut self, estimator: Option<crate::eta::Estimator>) {
        self.estimator = estimator;
    }
    pub(crate) fn eta_finished(&mut self, index: u64, sample: Option<f64>) {
        if let Some(estimator) = &mut self.estimator {
            estimator.finish(index, sample);
        }
        self.active_started = None;
    }
    fn eta(&self, active: impl IntoIterator<Item = (u64, f64)>, workers: usize) -> String {
        match self.estimator.as_ref().map(|e| e.estimate(active, workers)) {
            Some(crate::eta::Estimate::Ready { seconds, rough }) => {
                let clock = hms(Duration::try_from_secs_f64(seconds).unwrap_or(Duration::MAX));
                if rough {
                    format!("~{clock} (low confidence)")
                } else {
                    format!("~{clock}")
                }
            }
            Some(crate::eta::Estimate::Unpredictable) => {
                "unpredictable (insufficient timing evidence)".into()
            }
            _ => "learning (need completed job timings)".into(),
        }
    }

    fn elapsed(&self) -> Duration {
        self.elapsed_before.saturating_add(self.started.elapsed())
    }
    pub(crate) fn restore_clock(&mut self, elapsed: Duration, original_start: Option<Duration>) {
        self.elapsed_before = elapsed;
        self.original_start = original_start;
        self.started = Instant::now();
    }
    pub(crate) fn restore_row(
        &mut self,
        row: &Row,
        timings: Option<StageTimes>,
        tally: &Tally,
        compatible: bool,
    ) {
        if compatible {
            self.row(row);
        }
        if let Some(timings) = timings {
            self.add_timings(timings);
        }
        self.tally = *tally;
    }

    pub(crate) fn initial(&self, out: &mut dyn Write) -> io::Result<()> {
        if !self.sections.initial {
            return Ok(());
        }
        let s = self.settings;
        let solver = match s.backend {
            Backend::PicoSat => "picosat",
            Backend::Cadical => "cadical",
            Backend::Kissat => "kissat",
        };
        let backend = match s.intersection_backend {
            IntersectionBackend::Cpu => "cpu",
            IntersectionBackend::Gpu => "gpu (automatic CPU fallback)",
        };
        let mut text = String::new();
        for (label, value) in [
            (
                "Run started at:",
                self.original_start
                    .map_or_else(|| "unknown (legacy snapshot)".into(), epoch_text),
            ),
            (
                "Number of threads:",
                rayon::current_num_threads().to_string(),
            ),
            (
                "Solve mode:",
                if s.goal == Goal::All {
                    "Find All"
                } else {
                    "Find First"
                }
                .into(),
            ),
            ("Solver:", solver.into()),
            ("Split Solving:", (s.mode == Mode::Split).to_string()),
            (
                "Incremental Solving:",
                (s.mode == Mode::Incremental).to_string(),
            ),
            ("Intersection backend selected:", backend.into()),
            ("Number of polytopes found:", self.total.to_string()),
            (
                "Restricting to unimodular simplices:",
                (s.restriction == Restriction::Unimodular).to_string(),
            ),
            ("Looking for regular triangulations:", s.regular.to_string()),
            (
                "Learning regularity conflicts:",
                s.regular_learn.to_string(),
            ),
            (
                "Looking for flag triangulations:",
                (s.flag || s.flag_sat).to_string(),
            ),
            ("Encoding flagness in SAT:", s.flag_sat.to_string()),
        ] {
            let _ = writeln!(text, "{label:<37}{value}");
        }
        text.push('\n');
        out.write_all(text.as_bytes())?;
        out.flush()
    }

    pub(crate) fn begin(&mut self, index: u64, out: &mut dyn Write) -> io::Result<()> {
        self.active_started = Some((index, Instant::now()));
        self.index = index;
        self.points = None;
        self.simplices = None;
        self.solving = false;
        self.clauses_added = 0;
        self.interruptions = 0;
        self.incremental_available = false;
        self.classification = None;
        self.running(out, false)
    }

    pub(crate) fn event(&mut self, event: Progress, out: &mut dyn Write) -> io::Result<()> {
        // A solver call can be arbitrarily long. Flush changed counters at
        // these coarse boundaries before the next call, even inside the
        // normal redraw interval. No observer runs in model or pair loops.
        let mut entering_search = matches!(event, Progress::Solving);
        match event {
            Progress::Points { count } => self.points = Some(count),
            Progress::Simplices { count } => self.simplices = Some(count),
            Progress::Solving => {
                self.solving = true;
                self.incremental_available = true;
                self.classification = Some(ClassificationCounts::default());
            }
            Progress::Incremental {
                clauses_added,
                interruptions,
            } => {
                entering_search = self.settings.mode == Mode::Incremental
                    && (self.clauses_added != clauses_added || self.interruptions != interruptions);
                self.incremental_available = true;
                self.clauses_added = clauses_added;
                self.interruptions = interruptions;
            }
            Progress::Classification {
                triangulations,
                regular,
                non_regular,
                flag,
                non_flag,
                ..
            } => {
                self.classification = Some(ClassificationCounts {
                    triangulations,
                    regular,
                    non_regular,
                    flag,
                    non_flag,
                });
            }
            Progress::Finished { timings } => {
                self.solving = false;
                // The final classification snapshot precedes this event. Do
                // not lose a short job's rejection counts when the next job
                // resets its per-polytope state inside the redraw interval.
                entering_search = self.settings.regular || self.settings.flag;
                self.add_timings(timings);
            }
        }
        self.running(out, entering_search)
    }

    pub(crate) fn row(&mut self, row: &Row) {
        if row.status == Status::Ok && row.triangulations == 0 {
            self.no_triangulation = self.no_triangulation.saturating_add(1);
        }
        let targets = match (
            self.settings.regular,
            self.settings.flag || self.settings.flag_sat,
        ) {
            (true, true) => row.quadratic,
            (true, false) => row.regular,
            (false, true) => row.flag,
            (false, false) => row.triangulations,
        };
        self.targets = self.targets.saturating_add(targets);
    }

    fn add_timings(&mut self, timings: StageTimes) {
        for (stat, seconds) in
            self.timings
                .iter_mut()
                .zip([timings.lattice, timings.formula, timings.search])
        {
            stat.add(seconds);
        }
    }

    /// Keep diagnostics printed between frames from being erased by a redraw.
    pub(crate) fn diagnostic(&mut self) {
        self.drawn_lines = 0;
    }

    pub(crate) fn completed(&mut self, tally: &Tally, out: &mut dyn Write) -> io::Result<()> {
        self.tally = *tally;
        self.running(out, false)
    }

    fn number(&self, count: u64, positive: bool) -> String {
        if self.terminal {
            format!("\x1b[{}m{count}\x1b[0m", if positive { 32 } else { 31 })
        } else {
            count.to_string()
        }
    }

    fn counts(&self, text: &mut String) {
        let t = self.tally;
        let negative_label = match (self.settings.flag_sat, self.settings.restriction) {
            (true, Restriction::Unimodular) => "No unimodular flag triangulation:",
            (true, Restriction::Nondegenerate) => "No flag triangulation:",
            (false, Restriction::Unimodular) => "No unimodular triangulation:",
            (false, Restriction::Nondegenerate) => "Non-Triangulatable:",
        };
        let width = negative_label.len().saturating_add(1).max(30);
        if !self.settings.flag_sat {
            let _ = writeln!(
                text,
                "{:<width$}{}",
                "Triangulatable:",
                self.number(t.triangulatable, true)
            );
        }
        if self.settings.regular && !self.settings.flag_sat {
            let _ = writeln!(
                text,
                "{:<width$}{}",
                "Regularly Triangulatable:",
                self.number(t.regular, true)
            );
        }
        if self.settings.flag || self.settings.flag_sat {
            let _ = writeln!(
                text,
                "{:<width$}{}",
                "Flag Triangulatable:",
                self.number(t.flag, true)
            );
        }
        if self.settings.regular && (self.settings.flag || self.settings.flag_sat) {
            let _ = writeln!(
                text,
                "{:<width$}{}",
                "Quadratic Triangulatable:",
                self.number(t.quadratic, true)
            );
        }
        let _ = writeln!(
            text,
            "{:<width$}{}",
            negative_label,
            self.number(self.no_triangulation, false)
        );
        if t.historical > 0 {
            let _ = writeln!(text, "{:<width$}{}", "Other-request results:", t.historical);
        }
        let known = if self.settings.flag_sat {
            t.flag
        } else {
            t.triangulatable
        };
        let unknown = t
            .polytopes
            .saturating_sub(t.historical)
            .saturating_sub(known)
            .saturating_sub(self.no_triangulation);
        if unknown > 0 {
            let _ = writeln!(text, "{:<width$}{}", "Unknown / incomplete:", unknown);
        }
        if t.capped > 0 {
            let _ = writeln!(
                text,
                "{:<width$}{} (lower bounds)",
                "Capped or timed out:", t.capped
            );
        }
        if t.reglimited > 0 {
            let _ = writeln!(
                text,
                "{:<width$}{} (regularity unknown)",
                "Regularity limited:", t.reglimited
            );
        }
        if t.failed > 0 {
            let _ = writeln!(text, "{:<width$}{}", "Failed:", t.failed);
        }
    }

    fn running(&mut self, out: &mut dyn Write, force: bool) -> io::Result<()> {
        if !self.sections.running
            || (!force
                && self
                    .last_update
                    .is_some_and(|last| last.elapsed() < Duration::from_millis(100)))
        {
            return Ok(());
        }
        let elapsed = self.elapsed();
        let eta = self.eta(
            self.active_started
                .map(|(index, start)| (index, start.elapsed().as_secs_f64())),
            1,
        );
        let points = self.points.map_or_else(|| "?".into(), |n| n.to_string());
        let simplices = self.simplices.map_or_else(|| "?".into(), |n| n.to_string());
        let mut text = format!(
            "Polytope: {}/{}  |P|={}  |S|={}{}\nElapsed Time:                 {}\nEstimated Time Left:          {}\n",
            self.index,
            self.total,
            points,
            simplices,
            if self.solving { "  solving..." } else { "" },
            hms(elapsed),
            eta
        );
        if self.settings.mode == Mode::Incremental && self.incremental_available {
            let _ = writeln!(
                text,
                "{:<30}{}",
                "Intersection clauses added:", self.clauses_added
            );
            let _ = writeln!(
                text,
                "{:<30}{}",
                "Solver interruptions:", self.interruptions
            );
        }
        if let Some(c) = self.classification {
            for (enabled, rejected_label, pending_label, positive, negative) in [
                (
                    self.settings.regular,
                    "Non-regular triangulations:",
                    "Regularity pending/untested:",
                    c.regular,
                    c.non_regular,
                ),
                (
                    self.settings.flag,
                    "Non-flag triangulations:",
                    "Flagness pending/untested:",
                    c.flag,
                    c.non_flag,
                ),
            ] {
                if enabled {
                    let _ = writeln!(text, "{rejected_label:<30}{negative}");
                    let pending = c
                        .triangulations
                        .saturating_sub(positive)
                        .saturating_sub(negative);
                    if pending > 0 {
                        let _ = writeln!(text, "{pending_label:<30}{pending}");
                    }
                }
            }
        }
        self.counts(&mut text);
        let lines = text.lines().count();
        if self.terminal && self.drawn_lines > 0 {
            text = format!("\x1b[{}A\x1b[J{text}", self.drawn_lines);
        }
        out.write_all(text.as_bytes())?;
        out.flush()?;
        self.drawn_lines = lines;
        self.last_update = Some(Instant::now());
        Ok(())
    }

    pub(crate) fn finish(&mut self, tally: &Tally, out: &mut dyn Write) -> io::Result<()> {
        self.tally = *tally;
        self.solving = false;
        self.running(out, true)?;
        self.summary(tally, out)
    }

    fn summary(&self, tally: &Tally, out: &mut dyn Write) -> io::Result<()> {
        let mut text = String::new();
        if self.sections.final_summary {
            let _ = writeln!(
                text,
                "\nRun finished: {}\n----------------------------------------\nRun Summary\n----------------------------------------\n{:<30}{}",
                clock_text(),
                "Total Polytopes Processed:",
                tally.polytopes
            );
            self.counts(&mut text);
            if self.settings.goal == Goal::All {
                let average = if self.total == 0 {
                    0.0
                } else {
                    self.targets as f64 / self.total as f64
                };
                let _ = writeln!(text, "Average targets found/input: {average:.2} (observed)");
            }
            let _ = writeln!(
                text,
                "Total Run Time:               {}\n----------------------------------------",
                hms(self.elapsed())
            );
        }
        if self.sections.table {
            let _ = writeln!(
                text,
                "\nTiming Summary (completed jobs; restored stages included)\n{:<34} {:>12} {:>12} {:>12} {:>8}",
                "Stage", "Total (s)", "Mean (s)", "Max (s)", "Calls"
            );
            for (label, stat) in [
                "Lattice geometry",
                "Formula and intersections",
                "Search and classification",
            ]
            .into_iter()
            .zip(self.timings)
            {
                let mean = if stat.count == 0 {
                    0.0
                } else {
                    stat.total / stat.count as f64
                };
                let _ = writeln!(
                    text,
                    "{label:<34} {:>12.4} {:>12.4} {:>12.4} {:>8}",
                    stat.total, mean, stat.maximum, stat.count
                );
            }
        }
        out.write_all(text.as_bytes())?;
        out.flush()
    }
}

#[derive(Clone, Copy, Debug, Default)]
enum WorkerStage {
    #[default]
    Idle,
    Lattice,
    Simplices,
    Formula,
    Search,
    Completed,
    Done,
}

/// Latest state of one persistent worker. Updating it never writes output.
/// A new polytope resets every counter, including availability of live stats.
#[derive(Clone, Debug, Default)]
pub(crate) struct WorkerSnapshot {
    index: Option<u64>,
    started: Option<Instant>,
    stage: WorkerStage,
    points: Option<usize>,
    simplices: Option<usize>,
    incremental: Option<(usize, u64)>,
    classification: Option<ClassificationCounts>,
}

impl WorkerSnapshot {
    pub(crate) fn begin(&mut self, index: u64) {
        *self = Self {
            index: Some(index),
            started: Some(Instant::now()),
            stage: WorkerStage::Lattice,
            ..Self::default()
        };
    }

    pub(crate) fn event(&mut self, event: Progress) {
        match event {
            Progress::Points { count } => {
                self.points = Some(count);
                self.stage = WorkerStage::Simplices;
            }
            Progress::Simplices { count } => {
                self.simplices = Some(count);
                self.stage = WorkerStage::Formula;
            }
            Progress::Solving => {
                self.stage = WorkerStage::Search;
                self.incremental = Some((0, 0));
                self.classification = Some(ClassificationCounts::default());
            }
            Progress::Incremental {
                clauses_added,
                interruptions,
            } => self.incremental = Some((clauses_added, interruptions)),
            Progress::Classification {
                triangulations,
                regular,
                non_regular,
                flag,
                non_flag,
                ..
            } => {
                self.classification = Some(ClassificationCounts {
                    triangulations,
                    regular,
                    non_regular,
                    flag,
                    non_flag,
                });
            }
            // Timing aggregation belongs to committed results, never snapshots:
            // several short jobs may finish between two display ticks.
            Progress::Finished { .. } => self.complete(),
        }
    }

    pub(crate) fn complete(&mut self) {
        self.stage = WorkerStage::Completed;
    }

    pub(crate) fn done(&mut self) {
        self.stage = WorkerStage::Done;
    }
}

/// Preserve inline labels/order with a stable stage width and compact numeric columns.
fn worker_rows(workers: &[WorkerSnapshot], count: usize, total: u64, settings: Settings) -> String {
    let has_class = workers.iter().any(|w| w.classification.is_some());
    let has_incremental =
        settings.mode == Mode::Incremental && workers.iter().any(|w| w.incremental.is_some());
    let pending = |c: ClassificationCounts, regular: bool| {
        let (positive, negative) = if regular {
            (c.regular, c.non_regular)
        } else {
            (c.flag, c.non_flag)
        };
        c.triangulations
            .saturating_sub(positive)
            .saturating_sub(negative)
    };
    let rp = settings.regular
        && workers
            .iter()
            .filter_map(|w| w.classification)
            .any(|c| pending(c, true) > 0);
    let fp = settings.flag
        && workers
            .iter()
            .filter_map(|w| w.classification)
            .any(|c| pending(c, false) > 0);
    let value = |n: Option<u64>| n.map_or_else(|| "?".into(), |n| n.to_string());
    let mut rows = Vec::with_capacity(count);
    for worker in 0..count {
        let idle = WorkerSnapshot::default();
        let state = workers.get(worker).unwrap_or(&idle);
        let stage = match state.stage {
            WorkerStage::Idle => "idle",
            WorkerStage::Lattice => "lattice",
            WorkerStage::Simplices => "simplices",
            WorkerStage::Formula => "formula",
            WorkerStage::Search => "solving",
            WorkerStage::Completed => "completed",
            WorkerStage::Done => "done",
        };
        // prefix, value, suffix, left-aligned text (numeric fields align right).
        let mut fields = vec![
            (
                "Worker ",
                worker.saturating_add(1).to_string(),
                ":".to_owned(),
                false,
            ),
            (
                "#",
                state.index.map_or_else(|| "-".into(), |n| n.to_string()),
                format!("/{total}"),
                false,
            ),
            // Reserve the longest stage labels ("simplices" and "completed")
            // even when this frame only contains shorter stages like "solving".
            ("", format!("{stage:<9}"), String::new(), true),
            (
                "|P|=",
                state.points.map_or_else(|| "?".into(), |n| n.to_string()),
                String::new(),
                false,
            ),
            (
                "|S|=",
                state
                    .simplices
                    .map_or_else(|| "?".into(), |n| n.to_string()),
                String::new(),
                false,
            ),
        ];
        for (show, label, number) in [
            (
                settings.regular && has_class,
                "NR=",
                state.classification.map(|c| c.non_regular),
            ),
            (
                settings.flag && has_class,
                "NF=",
                state.classification.map(|c| c.non_flag),
            ),
            (
                has_incremental,
                "C=",
                state
                    .incremental
                    .map(|v| u64::try_from(v.0).unwrap_or(u64::MAX)),
            ),
            (has_incremental, "I=", state.incremental.map(|v| v.1)),
            (
                rp,
                "RP=",
                state
                    .classification
                    .map(|c| pending(c, true))
                    .filter(|&n| n > 0),
            ),
            (
                fp,
                "FP=",
                state
                    .classification
                    .map(|c| pending(c, false))
                    .filter(|&n| n > 0),
            ),
        ] {
            if show {
                if number.is_some() {
                    fields.push((label, value(number), String::new(), false));
                } else {
                    fields.push(("", String::new(), String::new(), false));
                }
            }
        }
        rows.push(fields);
    }
    let mut widths: Vec<usize> = Vec::new();
    let mut decorations: Vec<usize> = Vec::new();
    for row in &rows {
        for (column, (prefix, value, suffix, _)) in row.iter().enumerate() {
            let length = prefix.len().saturating_add(suffix.len());
            if let Some(width) = decorations.get_mut(column) {
                *width = (*width).max(length);
            } else {
                decorations.push(length);
            }
            if let Some(width) = widths.get_mut(column) {
                *width = (*width).max(value.len());
            } else {
                widths.push(value.len());
            }
        }
    }
    let mut text = String::new();
    for row in rows {
        let mut line = String::new();
        for (column, (prefix, value, suffix, left)) in row.into_iter().enumerate() {
            if column > 0 {
                line.push(' ');
            }
            let width = widths.get(column).copied().unwrap_or(value.len());
            if value.is_empty() {
                let width = width.saturating_add(decorations.get(column).copied().unwrap_or(0));
                let _ = write!(line, "{:width$}", "");
            } else if left {
                let _ = write!(line, "{prefix}{value:<width$}{suffix}");
            } else {
                let _ = write!(line, "{prefix}{value:>width$}{suffix}");
            }
        }
        text.push_str(line.trim_end());
        text.push('\n');
    }
    text
}

/// Coordinator-owned parallel output. Workers only mutate their snapshots;
/// the coordinator supplies the periodic redraw tick and completion accounting.
pub(crate) struct ParallelReporter {
    report: Reporter,
    workers: usize,
    terminal_width: Option<usize>,
}

impl ParallelReporter {
    pub(crate) fn new(
        sections: TerminalOutput,
        terminal: bool,
        total: u64,
        settings: Settings,
        workers: usize,
    ) -> Self {
        Self {
            report: Reporter::new(sections, terminal, total, settings),
            workers,
            terminal_width: None,
        }
    }

    pub(crate) fn set_estimator(&mut self, estimator: Option<crate::eta::Estimator>) {
        self.report.set_estimator(estimator);
    }
    pub(crate) fn eta_finished(&mut self, index: u64, sample: Option<f64>) {
        self.report.eta_finished(index, sample);
    }

    pub(crate) fn restore_clock(&mut self, elapsed: Duration, started: Option<Duration>) {
        self.report.restore_clock(elapsed, started);
    }

    pub(crate) fn initial(&self, out: &mut dyn Write) -> io::Result<()> {
        self.report.initial(out)?;
        if self.report.sections.initial {
            writeln!(out, "{:<37}{}\n", "Polytope workers:", self.workers)?;
            out.flush()?;
        }
        Ok(())
    }

    /// Restore prior timing without interpreting a differently requested row.
    pub(crate) fn restored(
        &mut self,
        row: &Row,
        timings: Option<StageTimes>,
        tally: &Tally,
        compatible: bool,
    ) {
        self.report.restore_row(row, timings, tally, compatible);
    }

    /// Call once per accepted result, including replayed rows and any saved timings.
    pub(crate) fn completed(&mut self, row: &Row, timings: Option<StageTimes>, tally: &Tally) {
        self.report.row(row);
        if let Some(timings) = timings {
            self.report.add_timings(timings);
        }
        self.report.tally = *tally;
    }

    pub(crate) fn diagnostic(&mut self) {
        self.report.diagnostic();
    }

    pub(crate) fn snapshot(
        &mut self,
        workers: &[WorkerSnapshot],
        tally: &Tally,
        out: &mut dyn Write,
    ) -> io::Result<()> {
        self.draw(workers, tally, out, false)
    }

    fn draw(
        &mut self,
        workers: &[WorkerSnapshot],
        tally: &Tally,
        out: &mut dyn Write,
        force: bool,
    ) -> io::Result<()> {
        let report = &mut self.report;
        report.tally = *tally;
        if !report.sections.running
            || (!force
                && report
                    .last_update
                    .is_some_and(|last| last.elapsed() < Duration::from_millis(100)))
        {
            return Ok(());
        }
        let mut text = String::new();
        text.push_str(&worker_rows(
            workers,
            self.workers,
            report.total,
            report.settings,
        ));
        text.push('\n');
        if report.settings.regular
            || report.settings.flag
            || report.settings.mode == Mode::Incremental
        {
            text.push_str(
                "NR/NF: non-regular/non-flag; RP/FP: pending/untested; C: intersections; I: stops\n",
            );
        }
        let elapsed = report.elapsed();
        let eta = report.eta(
            workers.iter().filter_map(|worker| {
                if matches!(
                    worker.stage,
                    WorkerStage::Completed | WorkerStage::Done | WorkerStage::Idle
                ) {
                    None
                } else {
                    worker
                        .index
                        .zip(worker.started)
                        .map(|(index, start)| (index, start.elapsed().as_secs_f64()))
                }
            }),
            self.workers,
        );
        let _ = writeln!(
            text,
            "Completed: {}/{}\nElapsed Time:                 {}\nEstimated Time Left:          {eta}",
            tally.polytopes,
            report.total,
            hms(elapsed)
        );
        report.counts(&mut text);
        let dimensions = terminal_size::terminal_size()
            .map(
                |(terminal_size::Width(width), terminal_size::Height(height))| {
                    (usize::from(width), usize::from(height))
                },
            )
            // Unknown viewport size cannot support safe relative erasure.
            .unwrap_or((80, 0));
        self.write_frame(&text, dimensions, out)
    }

    fn write_frame(
        &mut self,
        text: &str,
        (width, height): (usize, usize),
        out: &mut dyn Write,
    ) -> io::Result<()> {
        let report = &mut self.report;
        let lines = text.lines().count();
        let mut text = if report.terminal {
            // Keep every logical row on one terminal row. A resize may have
            // reflowed the old frame; start a new frame without erasing it.
            if self.terminal_width != Some(width) {
                report.drawn_lines = 0;
            }
            self.terminal_width = Some(width);
            clip_terminal_frame(text, width.saturating_sub(1))
        } else {
            text.to_owned()
        };
        // A frame taller than the viewport cannot be safely updated in place.
        // Append it instead of moving the cursor into unrelated earlier text.
        if report.terminal && report.drawn_lines > 0 && report.drawn_lines < height {
            text = format!("\x1b[{}A\x1b[J{text}", report.drawn_lines);
        }
        out.write_all(text.as_bytes())?;
        out.flush()?;
        report.drawn_lines = if lines < height { lines } else { 0 };
        report.last_update = Some(Instant::now());
        Ok(())
    }

    pub(crate) fn finish(
        &mut self,
        workers: &[WorkerSnapshot],
        tally: &Tally,
        out: &mut dyn Write,
    ) -> io::Result<()> {
        self.draw(workers, tally, out, true)?;
        self.report.summary(tally, out)
    }
}

/// The frame contains ASCII labels/numbers and SGR color sequences only.
/// Preserve complete color sequences, including resets beyond clipped text.
fn clip_terminal_frame(text: &str, columns: usize) -> String {
    let mut clipped = String::with_capacity(text.len());
    for line in text.lines() {
        let mut visible = 0usize;
        let mut escape = false;
        for c in line.chars() {
            if c == '\x1b' {
                escape = true;
            }
            if escape {
                clipped.push(c);
                if c == 'm' {
                    escape = false;
                }
            } else if visible < columns {
                clipped.push(c);
                visible = visible.saturating_add(1);
            }
        }
        clipped.push('\n');
    }
    clipped
}

fn clock_text() -> String {
    epoch_text(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default(),
    )
}
fn epoch_text(time: Duration) -> String {
    let seconds = time.as_secs().checked_rem(86_400).unwrap_or(0);
    format!("{} UTC", hms(Duration::from_secs(seconds)))
}

#[cfg(test)]
mod tests;

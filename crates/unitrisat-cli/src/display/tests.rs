use super::*;
use crate::cli::{Cli, Solver};

#[test]
fn section_selection_is_exact_and_composable() {
    let selected = TerminalOutput::parse(" initial, final ").unwrap();
    assert!(selected.initial && selected.final_summary);
    assert!(!selected.running && !selected.table);
    for bad in ["", "initially", "none,final", "all,table", "initial,"] {
        assert!(TerminalOutput::parse(bad).is_err());
    }
}

#[test]
fn progress_is_coalesced_but_final_update_is_forced() {
    let cli = Cli::try_parse_input_from(["unitrisat", "input"]).unwrap();
    let settings = cli.search.settings(Mode::Serial, Solver::Picosat);
    let mut report = Reporter::new(
        TerminalOutput::parse("running").unwrap(),
        false,
        1,
        settings,
    );
    let mut out = Vec::new();
    report.begin(1, &mut out).unwrap();
    let length = out.len();
    report
        .event(Progress::Points { count: 3 }, &mut out)
        .unwrap();
    report
        .event(Progress::Simplices { count: 1 }, &mut out)
        .unwrap();
    assert_eq!(out.len(), length);
    report.event(Progress::Solving, &mut out).unwrap();
    let live = String::from_utf8(out.clone()).unwrap();
    assert!(live.contains("|P|=3  |S|=1  solving..."));
    report
        .finish(
            &Tally {
                polytopes: 1,
                triangulatable: 1,
                ..Tally::default()
            },
            &mut out,
        )
        .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(text.matches("Elapsed Time:").count(), 3);
    assert!(text.contains("|P|=3  |S|=1"));
    assert!(!text.contains('\x1b'));
}

#[test]
fn incremental_counters_flush_changed_values_and_reset_for_each_polytope() {
    let cli = Cli::try_parse_input_from(["unitrisat", "input", "--mode", "incremental"]).unwrap();
    let settings = cli.search.settings(Mode::Serial, Solver::Picosat);
    let mut report = Reporter::new(
        TerminalOutput::parse("running").unwrap(),
        false,
        2,
        settings,
    );
    let mut out = Vec::new();
    report.begin(1, &mut out).unwrap();
    report.event(Progress::Solving, &mut out).unwrap();
    let unchanged = out.len();
    report
        .event(
            Progress::Incremental {
                clauses_added: 0,
                interruptions: 0,
            },
            &mut out,
        )
        .unwrap();
    assert_eq!(out.len(), unchanged);
    report
        .event(
            Progress::Incremental {
                clauses_added: 123,
                interruptions: 0,
            },
            &mut out,
        )
        .unwrap();
    let inserted = out.len();
    assert!(
        inserted > unchanged,
        "new inserted clauses must be visible before the next solve"
    );
    report
        .event(
            Progress::Incremental {
                clauses_added: 123,
                interruptions: 1,
            },
            &mut out,
        )
        .unwrap();
    assert!(
        out.len() > inserted,
        "an actual solver stop must be visible immediately"
    );
    let text = String::from_utf8(out.clone()).unwrap();
    let frame = text.rsplit("Polytope:").next().unwrap();
    assert!(frame.lines().any(|line| {
        line.strip_prefix("Intersection clauses added:")
            .is_some_and(|value| value.trim() == "123")
    }));
    assert!(frame.lines().any(|line| {
        line.strip_prefix("Solver interruptions:")
            .is_some_and(|value| value.trim() == "1")
    }));
    report.begin(2, &mut out).unwrap();
    report.event(Progress::Solving, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    let frame = text.rsplit("Polytope:").next().unwrap();
    assert!(frame.starts_with(" 2/2"));
    for label in ["Intersection clauses added:", "Solver interruptions:"] {
        assert!(frame.lines().any(|line| {
            line.strip_prefix(label)
                .is_some_and(|value| value.trim() == "0")
        }));
    }
    assert!(!text.contains('\x1b'));
}

#[test]
fn incremental_counters_are_not_shown_in_other_modes_or_without_running_output() {
    for (mode, sections) in [
        ("serial", "running"),
        ("split", "running"),
        ("incremental", "none"),
        ("incremental", "final"),
    ] {
        let cli = Cli::try_parse_input_from(["unitrisat", "input", "--mode", mode]).unwrap();
        let settings = cli.search.settings(Mode::Serial, Solver::Picosat);
        let mut report =
            Reporter::new(TerminalOutput::parse(sections).unwrap(), false, 1, settings);
        let mut out = Vec::new();
        report.begin(1, &mut out).unwrap();
        report
            .event(
                Progress::Incremental {
                    clauses_added: 57,
                    interruptions: 2,
                },
                &mut out,
            )
            .unwrap();
        report.finish(&Tally::default(), &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            !text.contains("Intersection clauses added:"),
            "{mode} {sections}"
        );
        assert!(!text.contains("Solver interruptions:"), "{mode} {sections}");
    }
}

fn frame_value<'a>(frame: &'a str, label: &str) -> Option<&'a str> {
    frame
        .lines()
        .find_map(|line| line.strip_prefix(label).map(str::trim))
}

#[test]
fn classification_rejections_are_throttled_exact_and_final_snapshot_is_visible() {
    let cli = Cli::try_parse_input_from(["unitrisat", "input", "--regular", "--flag"]).unwrap();
    let settings = cli.search.settings(Mode::Serial, Solver::Picosat);
    let mut report = Reporter::new(
        TerminalOutput::parse("running").unwrap(),
        false,
        2,
        settings,
    );
    let mut out = Vec::new();
    report.begin(1, &mut out).unwrap();
    report.event(Progress::Solving, &mut out).unwrap();
    let before = out.len();
    report
        .event(
            Progress::Classification {
                triangulations: 10,
                regular: 2,
                non_regular: 3,
                flag: 4,
                non_flag: 6,
                quadratic: 1,
            },
            &mut out,
        )
        .unwrap();
    assert_eq!(
        out.len(),
        before,
        "classification snapshots are coalesced, never forced per model"
    );
    report
        .event(
            Progress::Finished {
                timings: StageTimes::default(),
            },
            &mut out,
        )
        .unwrap();
    let text = String::from_utf8(out.clone()).unwrap();
    let frame = text.rsplit("Polytope:").next().unwrap();
    assert_eq!(frame_value(frame, "Non-regular triangulations:"), Some("3"));
    assert_eq!(
        frame_value(frame, "Regularity pending/untested:"),
        Some("5")
    );
    assert_eq!(frame_value(frame, "Non-flag triangulations:"), Some("6"));
    assert_eq!(frame_value(frame, "Flagness pending/untested:"), None);
    report.begin(2, &mut out).unwrap();
    report.event(Progress::Solving, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    let frame = text.rsplit("Polytope:").next().unwrap();
    assert_eq!(frame_value(frame, "Non-regular triangulations:"), Some("0"));
    assert_eq!(frame_value(frame, "Non-flag triangulations:"), Some("0"));
    assert_eq!(frame_value(frame, "Regularity pending/untested:"), None);
    assert!(!text.contains('\x1b'));
}

#[test]
fn live_rejection_snapshots_obey_filter_and_running_section_selection() {
    for (filters, section, expected) in [
        (&["--regular"][..], "running", (true, false)),
        (&["--flag"][..], "running", (false, true)),
        (&[][..], "running", (false, false)),
        (&["--regular", "--flag"][..], "final", (false, false)),
    ] {
        let cli = Cli::try_parse_input_from(
            ["unitrisat", "input"]
                .into_iter()
                .chain(filters.iter().copied()),
        )
        .unwrap();
        let settings = cli.search.settings(Mode::Serial, Solver::Picosat);
        let mut report = Reporter::new(TerminalOutput::parse(section).unwrap(), false, 1, settings);
        let mut out = Vec::new();
        report.begin(1, &mut out).unwrap();
        // Make the next snapshot eligible without sleeping or involving a solver.
        report.last_update = None;
        report
            .event(
                Progress::Classification {
                    triangulations: 8,
                    regular: 1,
                    non_regular: 2,
                    flag: 3,
                    non_flag: 5,
                    quadratic: 1,
                },
                &mut out,
            )
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.contains("Non-regular triangulations:"), expected.0);
        assert_eq!(text.contains("Non-flag triangulations:"), expected.1);
        if expected.0 {
            assert_eq!(
                frame_value(
                    text.rsplit("Polytope:").next().unwrap(),
                    "Non-regular triangulations:"
                ),
                Some("2")
            );
        }
        if expected.1 {
            assert_eq!(
                frame_value(
                    text.rsplit("Polytope:").next().unwrap(),
                    "Non-flag triangulations:"
                ),
                Some("5")
            );
        }
    }
}

#[test]
fn replayed_rows_do_not_fabricate_unavailable_live_counters() {
    let cli = Cli::try_parse_input_from([
        "unitrisat",
        "input",
        "--regular",
        "--flag",
        "--mode",
        "incremental",
    ])
    .unwrap();
    let settings = cli.search.settings(Mode::Serial, Solver::Picosat);
    let mut report = Reporter::new(TerminalOutput::parse("all").unwrap(), false, 1, settings);
    let mut out = Vec::new();
    report.begin(1, &mut out).unwrap();
    report.row(&Row {
        index: 1,
        status: Status::Ok,
        triangulations: 10,
        regular: 2,
        flag: 4,
        quadratic: 1,
        seconds: 1.0,
    });
    let tally = Tally {
        polytopes: 1,
        triangulatable: 1,
        ..Tally::default()
    };
    report.completed(&tally, &mut out).unwrap();
    report.finish(&tally, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    for label in [
        "Non-regular triangulations:",
        "Non-flag triangulations:",
        "Regularity pending/untested:",
        "Flagness pending/untested:",
        "Intersection clauses added:",
        "Solver interruptions:",
    ] {
        assert!(!text.contains(label), "replayed rows do not retain {label}");
    }
    assert!(text.contains("Total Polytopes Processed:"));
}

fn parallel_report(args: &[&str], sections: &str, terminal: bool) -> ParallelReporter {
    let cli = Cli::try_parse_input_from(
        ["unitrisat", "input"]
            .into_iter()
            .chain(args.iter().copied()),
    )
    .unwrap();
    ParallelReporter::new(
        TerminalOutput::parse(sections).unwrap(),
        terminal,
        3,
        cli.search.settings(Mode::Serial, Solver::Picosat),
        3,
    )
}

fn classified_worker(index: u64) -> WorkerSnapshot {
    let mut state = WorkerSnapshot::default();
    state.begin(index);
    state.event(Progress::Points { count: 7 });
    state.event(Progress::Simplices { count: 12 });
    state.event(Progress::Solving);
    state.event(Progress::Incremental {
        clauses_added: 123,
        interruptions: 2,
    });
    state.event(Progress::Classification {
        triangulations: 10,
        regular: 2,
        non_regular: 3,
        flag: 4,
        non_flag: 6,
        quadratic: 1,
    });
    state
}

#[test]
fn parallel_workers_are_independent_and_reset_every_live_stat() {
    let mut report = parallel_report(
        &["--mode", "incremental", "--regular", "--flag"],
        "running",
        false,
    );
    let mut workers = [classified_worker(1), WorkerSnapshot::default()];
    workers[1].begin(2);
    workers[1].event(Progress::Points { count: 99 });
    let mut out = Vec::new();
    report
        .snapshot(&workers, &Tally::default(), &mut out)
        .unwrap();
    let text = String::from_utf8(out.clone()).unwrap();
    let rows: Vec<_> = text.lines().take(3).collect();
    assert!(rows[0].contains("#1/3 solving   |P|= 7 |S|=12"));
    for field in ["C=123", "I=2", "NR=3", "RP=5", "NF=6"] {
        assert!(rows[0].contains(field), "{field}");
    }
    let at_eighty_columns = clip_terminal_frame(rows[0], 79);
    assert_eq!(at_eighty_columns.trim_end(), rows[0]);
    assert!(at_eighty_columns.contains("NR=3 NF=6 C=123 I=2 RP=5"));
    assert!(text.contains("NR/NF: non-regular/non-flag"));
    assert!(text.contains("RP/FP: pending/untested"));
    assert!(!rows[0].contains("FP="));
    assert!(rows[1].contains("#2/3 simplices |P|=99 |S|= ?"));
    assert!(!rows[1].contains("C="));
    assert!(!rows[1].contains("NR="));
    assert!(rows[2].contains("#-/3 idle"));
    workers[0].complete();
    workers[1].begin(3);
    report.report.last_update = None;
    out.clear();
    report
        .snapshot(&workers, &Tally::default(), &mut out)
        .unwrap();
    let text = String::from_utf8(out).unwrap();
    let rows: Vec<_> = text.lines().take(3).collect();
    assert!(rows[0].contains("completed"));
    assert!(rows[0].contains("NR=3"));
    assert!(rows[1].contains("#3/3 lattice   |P|=? |S|= ?"));
    assert!(!rows[1].contains("NR="));
    assert!(!text.contains('\x1b'));
}

#[test]
fn parallel_ticks_coalesce_all_progress_but_account_every_completion() {
    let mut report = parallel_report(&["--regular"], "running,table", false);
    let mut workers = [classified_worker(1)];
    let mut tally = Tally::default();
    let mut out = Vec::new();
    report.snapshot(&workers, &tally, &mut out).unwrap();
    let before = out.len();
    let timings = StageTimes {
        lattice: 1.0,
        formula: 2.0,
        search: 3.0,
    };
    for index in 1..=3 {
        workers[0].begin(index);
        workers[0].event(Progress::Solving);
        workers[0].event(Progress::Finished { timings });
        tally.polytopes += 1;
        let row = Row {
            index,
            status: Status::Ok,
            triangulations: 0,
            regular: 0,
            flag: 0,
            quadratic: 0,
            seconds: 6.0,
        };
        report.completed(&row, Some(timings), &tally);
        // Suppress ticks deterministically; no test sleep or solver needed.
        report.report.last_update = Some(Instant::now());
        report.snapshot(&workers, &tally, &mut out).unwrap();
        assert_eq!(out.len(), before);
    }
    assert_eq!(report.report.timings[0].count, 3);
    assert_eq!(report.report.timings[2].total, 9.0);
    assert_eq!(report.report.no_triangulation, 3);
    workers[0].done();
    report.finish(&workers, &tally, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("#3/3 done"));
    assert!(text.contains("Completed: 3/3"));
    assert_eq!(text.matches("Elapsed Time:").count(), 2);
    assert!(text.contains("Timing Summary (completed jobs; restored stages included)"));
}

#[test]
fn parallel_replay_does_not_invent_stats_or_attempt_timings() {
    let mut report = parallel_report(
        &["--regular", "--flag", "--mode", "incremental"],
        "all",
        false,
    );
    let mut worker = classified_worker(1);
    worker.begin(2);
    worker.complete();
    let row = Row {
        index: 2,
        status: Status::Ok,
        triangulations: 10,
        regular: 2,
        flag: 4,
        quadratic: 1,
        seconds: 5.0,
    };
    let tally = Tally {
        polytopes: 1,
        triangulatable: 1,
        ..Tally::default()
    };
    report.completed(&row, None, &tally);
    let mut out = Vec::new();
    report.finish(&[worker], &tally, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    for unavailable in ["C=", "I=", "NR=", "NF=", "RP=", "FP="] {
        assert!(!text.contains(unavailable), "{unavailable}");
    }
    assert_eq!(report.report.timings[0].count, 0);
    assert_eq!(report.report.targets, 1);
    assert!(!text.contains('\x1b'));
}

#[test]
fn parallel_stats_obey_filters_mode_and_output_sections() {
    for (args, section, expected) in [
        (&["--regular"][..], "running", (true, false, false)),
        (&["--flag"][..], "running", (false, true, false)),
        (
            &["--mode", "incremental"][..],
            "running",
            (false, false, true),
        ),
        (
            &["--regular", "--flag", "--mode", "incremental"][..],
            "none",
            (false, false, false),
        ),
        (
            &["--regular", "--flag", "--mode", "incremental"][..],
            "final,table",
            (false, false, false),
        ),
    ] {
        let mut report = parallel_report(args, section, false);
        let mut out = Vec::new();
        report.initial(&mut out).unwrap();
        report
            .snapshot(&[classified_worker(1)], &Tally::default(), &mut out)
            .unwrap();
        report
            .finish(&[classified_worker(1)], &Tally::default(), &mut out)
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.contains("NR="), expected.0);
        assert_eq!(text.contains("NF="), expected.1);
        assert_eq!(text.contains("C="), expected.2);
        assert!(!text.contains('\x1b'));
        if section == "none" {
            assert!(text.is_empty());
        }
    }
}

#[test]
fn parallel_terminal_redraw_keeps_diagnostics_outside_the_erased_frame() {
    let mut report = parallel_report(&["--regular", "--flag"], "running", true);
    let mut out = Vec::new();
    report
        .write_frame("worker\ntally\n", (80, 24), &mut out)
        .unwrap();
    assert_eq!(out, b"worker\ntally\n");
    out.clear();
    report
        .write_frame("worker\ntally\n", (80, 24), &mut out)
        .unwrap();
    assert!(out.starts_with(b"\x1b[2A\x1b[J"));
    report.diagnostic();
    out.clear();
    report
        .write_frame("worker\ntally\n", (80, 24), &mut out)
        .unwrap();
    assert_eq!(out, b"worker\ntally\n");
}

#[test]
fn parallel_terminal_clips_without_wrapping_and_preserves_color_resets() {
    for width in [40, 80] {
        let text = format!(
            "Worker 1: {}\nCount: \x1b[32m{}\x1b[0m\n",
            "x".repeat(120),
            "9".repeat(100)
        );
        let frame = clip_terminal_frame(&text, width - 1);
        let plain = frame.replace("\x1b[32m", "").replace("\x1b[0m", "");
        assert_eq!(plain.lines().count(), 2);
        assert!(plain.lines().all(|line| line.len() == width - 1));
        assert!(frame.ends_with("\x1b[0m\n"));
        assert!(plain.starts_with("Worker 1:"));
    }
}

#[test]
fn parallel_terminal_resize_and_oversized_frames_do_not_erase_previous_output() {
    let mut report = parallel_report(&[], "running", true);
    let mut out = Vec::new();
    report
        .write_frame("worker\ntally\n", (80, 24), &mut out)
        .unwrap();
    out.clear();
    report
        .write_frame("worker\ntally\n", (40, 24), &mut out)
        .unwrap();
    assert_eq!(
        out, b"worker\ntally\n",
        "resizing may reflow the previous frame"
    );
    out.clear();
    report
        .write_frame("worker\ntally\n", (40, 24), &mut out)
        .unwrap();
    assert!(out.starts_with(b"\x1b[2A\x1b[J"));
    out.clear();
    report
        .write_frame("worker\ntally\n", (40, 2), &mut out)
        .unwrap();
    assert_eq!(out, b"worker\ntally\n");
    out.clear();
    report
        .write_frame("worker\ntally\n", (40, 2), &mut out)
        .unwrap();
    assert_eq!(out, b"worker\ntally\n");
    out.clear();
    report
        .write_frame("worker\ntally\n", (80, 0), &mut out)
        .unwrap();
    assert_eq!(
        out, b"worker\ntally\n",
        "unknown height uses append-only output"
    );
}

#[test]
fn worker_inline_columns_align_with_minimum_frame_widths_and_no_header() {
    let settings = Settings {
        regular: true,
        flag: true,
        mode: Mode::Incremental,
        ..Settings::default()
    };
    let first = WorkerSnapshot {
        index: Some(1),
        stage: WorkerStage::Search,
        points: Some(3),
        simplices: Some(7),
        incremental: Some((10, 2)),
        classification: Some(ClassificationCounts {
            triangulations: 10,
            regular: 4,
            non_regular: 2,
            flag: 4,
            non_flag: 1,
        }),
        ..WorkerSnapshot::default()
    };
    let compact = worker_rows(std::slice::from_ref(&first), 1, 1234, settings);
    assert_eq!(
        compact,
        "Worker 1: #1/1234 solving   |P|=3 |S|=7 NR=2 NF=1 C=10 I=2 RP=4 FP=5\n"
    );
    let second = WorkerSnapshot {
        index: Some(123),
        stage: WorkerStage::Simplices,
        points: Some(12345),
        simplices: Some(123456),
        incremental: Some((99999, 12)),
        ..WorkerSnapshot::default()
    };
    let third = WorkerSnapshot {
        index: Some(9),
        stage: WorkerStage::Search,
        classification: Some(ClassificationCounts {
            triangulations: 100,
            regular: 0,
            non_regular: 100,
            flag: 0,
            non_flag: 100,
        }),
        ..WorkerSnapshot::default()
    };
    let text = worker_rows(&[first, second, third], 12, 1234, settings);
    let rows: Vec<_> = text.lines().collect();
    assert_eq!(rows.len(), 12);
    for field in ["#", "|P|=", "|S|="] {
        let column = rows[0].find(field).unwrap();
        assert!(
            rows.iter().all(|row| row.find(field) == Some(column)),
            "{field}: {text}"
        );
    }
    for field in ["C=", "I="] {
        assert_eq!(rows[0].find(field), rows[1].find(field));
    }
    assert!(rows[0].starts_with("Worker  1:"));
    assert!(rows[9].starts_with("Worker 10:"));
    assert!(rows[3].contains("idle"));
    assert!(!rows[3].contains("NR="));
    assert!(!rows[2].contains("RP="));
    assert!(!rows[2].contains("FP="));
    assert_eq!(rows[0].find("NR="), rows[2].find("NR="));
    assert!(!rows[1].contains("RP="));
    assert!(!text.contains("Stage"));
    assert!(rows.iter().all(|row| !row.ends_with(' ')));
}

#[test]
fn exhausted_flag_sat_search_does_not_claim_no_triangulation_exists() {
    let settings = Settings {
        flag_sat: true,
        ..Settings::default()
    };
    let mut report = Reporter::new(TerminalOutput::parse("final").unwrap(), false, 1, settings);
    let row = Row {
        index: 1,
        status: Status::Ok,
        triangulations: 0,
        regular: 0,
        flag: 0,
        quadratic: 0,
        seconds: 0.0,
    };
    report.row(&row);
    let mut out = Vec::new();
    report
        .finish(
            &Tally {
                polytopes: 1,
                ..Tally::default()
            },
            &mut out,
        )
        .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(
        frame_value(&text, "No unimodular flag triangulation:"),
        Some("1")
    );
    assert!(!text.contains("Non-Triangulatable:"));
    assert!(text.contains("Flag Triangulatable:"));
}

#[test]
fn eta_places_low_confidence_immediately_after_the_time() {
    let cli = Cli::try_parse_input_from(["unitrisat", "input"]).unwrap();
    let settings = cli.search.settings(Mode::Serial, Solver::Picosat);
    let mut report = Reporter::new(
        TerminalOutput::parse("running").unwrap(),
        false,
        4,
        settings,
    );
    report.set_estimator(Some(crate::eta::Estimator::new(vec![2; 4].into())));
    assert_eq!(report.eta([], 1), "learning (need completed job timings)");
    report.eta_finished(1, Some(10.));
    assert_eq!(report.eta([], 1), "~00:00:30 (low confidence)");
    report.eta_finished(2, Some(10.));
    report.eta_finished(3, Some(10.));
    assert_eq!(report.eta([], 1), "~00:00:10");
    assert_eq!(report.eta([(4, 100.)], 1), "~00:00:25 (low confidence)");
}

#[test]
fn worker_stage_transitions_keep_following_columns_fixed_across_frames() {
    let mut worker = WorkerSnapshot {
        index: Some(1),
        points: Some(3),
        simplices: Some(7),
        ..WorkerSnapshot::default()
    };
    let settings = Settings::default();
    let baseline = worker_rows(std::slice::from_ref(&worker), 1, 12, settings);
    for stage in [
        WorkerStage::Search,
        WorkerStage::Simplices,
        WorkerStage::Search,
        WorkerStage::Completed,
        WorkerStage::Idle,
        WorkerStage::Lattice,
        WorkerStage::Formula,
        WorkerStage::Done,
    ] {
        worker.stage = stage;
        let frame = worker_rows(std::slice::from_ref(&worker), 1, 12, settings);
        for field in ["|P|=", "|S|="] {
            assert_eq!(
                frame.find(field),
                baseline.find(field),
                "{stage:?}: {frame}"
            );
        }
    }
}

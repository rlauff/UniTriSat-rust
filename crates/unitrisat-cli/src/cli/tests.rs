use super::*;
use clap::CommandFactory;
use std::{fs, path::Path};

fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_input_from(std::iter::once("unitrisat").chain(args.iter().copied()))
}

fn run(path: &Path, args: &[&str]) -> (Result<Tally, CliError>, String, String) {
    let cli = parse(
        &std::iter::once(path.to_str().unwrap())
            .chain(args.iter().copied())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let result = execute(&cli, &mut out, &mut err);
    (
        result,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

const TRIANGLE: &str = "[[0,0],[1,0],[0,1]]\n";

#[test]
fn direct_help_lists_flags_and_no_commands_or_dataset_paths() {
    let help = Cli::command().render_long_help().to_string();
    for flag in [
        "--backend",
        "--solver",
        "--mode",
        "--regular",
        "--regular-learn",
        "--flag",
        "--flag-sat",
        "--all",
        "--checkpoint",
        "--terminal-output",
        "--output-format",
        "--parallel",
        "--find-all",
        "--flag-triangulation",
        "--intersection-backend",
    ] {
        assert!(help.contains(flag), "missing {flag}: {help}");
    }
    assert!(!help.contains("Commands:"));
    assert!(!help.contains("Polytopes/"));
    assert!(!help.contains("triangulate FILE"));
    assert_eq!(parse(&["--help"]).unwrap_err().exit_code(), 0);
    assert!(parse(&[]).is_err());
    for former in ["chunk", "tables", "volume", "help", "triangulate"] {
        assert_eq!(
            parse(&[former]).unwrap().input,
            Some(PathBuf::from(former)),
            "a filename must not be a hidden command"
        );
        assert!(parse(&[former, "another-file"]).is_err());
    }
}

#[test]
fn parallel_is_optional_positive_and_independent_of_resumed_search_options() {
    assert_eq!(parse(&["input"]).unwrap().parallel, None);
    for args in [&["--parallel", "input"][..], &["input", "--parallel"]] {
        assert_eq!(parse(args).unwrap().parallel.unwrap(), 4);
    }
    assert_eq!(
        parse(&["input", "--parallel=7"]).unwrap().parallel.unwrap(),
        7
    );
    assert_eq!(
        parse(&["saved", "--parallel=2"]).unwrap().parallel.unwrap(),
        2
    );
    for flag in ["--parallel=-1", "--parallel=", "--parallel=x"] {
        assert!(parse(&["input", flag]).is_err());
    }
}

#[test]
fn parallel_preserves_counts_statuses_and_original_indices() {
    let tree = tempfile::tempdir().unwrap();
    let path = tree.path().join("polytopes.txt");
    fs::write(
        &path,
        format!("{}{}{}", TRIANGLE, "[[0,0],[2,0],[0,2]]\n", TRIANGLE),
    )
    .unwrap();
    for mode in ["serial", "split", "incremental"] {
        for extra in [&[][..], &["--max-solutions", "1"][..]] {
            let mut args = vec![
                "--all",
                "--regular",
                "--flag",
                "--mode",
                mode,
                "--output-format",
                "tsv",
                "--terminal-output",
                "none",
            ];
            args.extend_from_slice(extra);
            let (serial, serial_rows, serial_err) = run(&path, &args);
            args.push("--parallel=2");
            let (parallel, parallel_rows, parallel_err) = run(&path, &args);
            assert!(serial_err.is_empty() && parallel_err.is_empty());
            assert_eq!(serial.unwrap(), parallel.unwrap());
            let rows = |text: &str| {
                let mut rows = text
                    .lines()
                    .filter_map(Row::parse)
                    .map(|row| {
                        (
                            row.index,
                            row.status.as_str(),
                            row.triangulations,
                            row.regular,
                            row.flag,
                            row.quadratic,
                        )
                    })
                    .collect::<Vec<_>>();
                rows.sort_unstable();
                rows
            };
            assert_eq!(rows(&serial_rows), rows(&parallel_rows), "mode {mode}");
        }
    }
}

#[test]
fn extreme_parallel_count_is_bounded_by_actual_input_records() {
    let tree = tempfile::tempdir().unwrap();
    let path = tree.path().join("one.txt");
    fs::write(&path, TRIANGLE).unwrap();
    let flag = format!("--parallel={}", usize::MAX);
    let (result, _, err) = run(&path, &[&flag, "--terminal-output", "none"]);
    assert!(err.is_empty());
    assert_eq!(result.unwrap().polytopes, 1);
}

#[test]
fn parallel_failed_jobs_preserve_successes_and_unknown_tally() {
    let tree = tempfile::tempdir().unwrap();
    let path = tree.path().join("failed.txt");
    fs::write(&path, format!("{TRIANGLE}[[1,1],[1,1]]\n{TRIANGLE}")).unwrap();
    let (result, out, err) = run(&path, &["--parallel", "--terminal-output", "final"]);
    let tally = result.unwrap();
    assert_eq!(
        (tally.polytopes, tally.triangulatable, tally.failed),
        (3, 2, 1)
    );
    assert!(err.contains("polytope 2"));
    assert!(out.lines().any(|line| {
        line.strip_prefix("Unknown / incomplete:")
            .is_some_and(|value| value.trim() == "1")
    }));
}

#[test]
fn direct_flags_work_before_or_after_filename_and_aliases_are_typed() {
    let first = parse(&[
        "--regular",
        "--find-all",
        "--flag-triangulation",
        "--intersection-backend",
        "gpu",
        "--solver",
        "cadical",
        "--mode",
        "incremental",
        "input.txt",
    ])
    .unwrap();
    let s = first.search.settings(Mode::Serial, Solver::Picosat);
    assert_eq!(s.intersection_backend, IntersectionBackend::Gpu);
    assert_eq!(s.mode, Mode::Incremental);
    assert_eq!(s.backend, Backend::Cadical);
    assert_eq!(s.goal, Goal::All);
    assert!(s.regular && s.flag);
    let second = parse(&[
        "input.txt",
        "--regular",
        "--all",
        "--flag",
        "--backend",
        "gpu",
        "--solver",
        "cadical",
        "--mode",
        "incremental",
    ])
    .unwrap();
    assert_eq!(first.search, second.search);
    for args in [
        &["input", "--backend", "x"][..],
        &["input", "--terminal-output", "all,running"],
        &["input", "--output-format", "json"],
        &["input", "--progress", "5"],
    ] {
        assert!(parse(args).is_err(), "{args:?}");
    }
}

#[test]
fn positional_input_accepts_overrides_and_old_resume_flag_is_removed() {
    assert!(
        parse(&[
            "run.resume",
            "--regular=false",
            "--all=false",
            "--flag=false",
            "--parallel=0",
            "--timeout=0"
        ])
        .is_ok()
    );
    assert!(parse(&["--resume", "run.resume"]).is_err());
    assert!(
        !Cli::command()
            .render_long_help()
            .to_string()
            .contains("--resume")
    );
}

#[test]
fn human_default_prints_initial_running_timings_and_final_without_ansi_in_pipes() {
    let tree = tempfile::tempdir().unwrap();
    let path = tree.path().join("polytopes.txt");
    fs::write(&path, TRIANGLE.repeat(2)).unwrap();
    let (result, out, err) = run(&path, &["--all", "--regular", "--flag"]);
    assert_eq!(result.unwrap().polytopes, 2);
    assert!(err.is_empty());
    for text in [
        "Run started at:",
        "Number of polytopes found:",
        "Polytope: 2/2",
        "|P|=3",
        "|S|=1",
        "Elapsed Time:",
        "Estimated Time Left:",
        "Run Summary",
        "Timing Summary",
        "Search and classification",
        "Quadratic Triangulatable:",
    ] {
        assert!(out.contains(text), "{text}: {out}");
    }
    assert!(!out.contains('\x1b'));
    assert!(!out.contains(TSV_HEADER));
}

#[test]
fn sections_are_independent_and_machine_output_is_explicit() {
    let tree = tempfile::tempdir().unwrap();
    let path = tree.path().join("points.txt");
    fs::write(&path, "0 0\n1 0\n0 1\n").unwrap();
    let (result, out, err) = run(&path, &["--terminal-output", "none"]);
    assert_eq!(result.unwrap().triangulatable, 1);
    assert!(out.is_empty() && err.is_empty());
    for (section, present, absent) in [
        ("initial", "Run started at:", "Run Summary"),
        ("running", "Elapsed Time:", "Timing Summary"),
        ("table", "Timing Summary", "Run Summary"),
        ("final", "Run Summary", "Timing Summary"),
    ] {
        let (_, out, _) = run(&path, &["--terminal-output", section]);
        assert!(out.contains(present));
        assert!(!out.contains(absent));
    }
    let (_, out, err) = run(
        &path,
        &["--output-format", "tsv", "--terminal-output", "none"],
    );
    assert!(err.is_empty());
    let mut lines = out.lines();
    assert_eq!(lines.next(), Some(TSV_HEADER));
    let row = Row::parse(lines.next().unwrap()).unwrap();
    assert_eq!(row.triangulations, 1);
    assert!(lines.next().is_none());
    let (_, out, err) = run(
        &path,
        &["--output-format", "tsv", "--terminal-output", "final"],
    );
    assert!(out.starts_with(TSV_HEADER));
    assert!(!out.contains("Run Summary"));
    assert!(err.contains("Run Summary"));
}

#[test]
fn failed_jobs_continue_and_are_unknown_not_nontriangulatable() {
    let tree = tempfile::tempdir().unwrap();
    let path = tree.path().join("batch.txt");
    fs::write(&path, format!("{TRIANGLE}[[1,1],[1,1]]\n{TRIANGLE}")).unwrap();
    let (result, out, err) = run(&path, &["--terminal-output", "final"]);
    let tally = result.unwrap();
    assert_eq!(
        (tally.polytopes, tally.triangulatable, tally.failed),
        (3, 2, 1)
    );
    assert!(err.contains("polytope 2"));
    assert!(
        out.lines().any(|line| line
            .strip_prefix("No unimodular triangulation:")
            .is_some_and(|value| value.trim() == "0")),
        "{out}"
    );
    assert!(
        out.lines().any(|line| line
            .strip_prefix("Unknown / incomplete:")
            .is_some_and(|value| value.trim() == "1")),
        "{out}"
    );
}

#[test]
fn invalid_or_missing_inputs_fail_without_output() {
    let tree = tempfile::tempdir().unwrap();
    let path = tree.path().join("bad.txt");
    let (result, out, _) = run(&path, &[]);
    assert!(result.is_err());
    assert!(out.is_empty());
    fs::write(&path, "[[0,0],[1,\n").unwrap();
    let (result, out, _) = run(&path, &[]);
    assert!(result.is_err());
    assert!(out.is_empty());
}

#[test]
fn terminal_redraws_are_only_enabled_for_terminal_stream() {
    let tree = tempfile::tempdir().unwrap();
    let path = tree.path().join("points.txt");
    fs::write(&path, TRIANGLE).unwrap();
    let cli = parse(&[path.to_str().unwrap()]).unwrap();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    execute_with_terminal(&cli, &mut out, &mut err, true, false).unwrap();
    assert!(out.contains(&0x1b));
    assert!(!err.contains(&0x1b));
}

#[test]
fn tsv_with_both_streams_on_terminal_never_redraws_over_result_rows() {
    let tree = tempfile::tempdir().unwrap();
    let path = tree.path().join("points.txt");
    fs::write(&path, TRIANGLE.repeat(2)).unwrap();
    let cli = parse(&[path.to_str().unwrap(), "--output-format", "tsv"]).unwrap();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    execute_with_terminal(&cli, &mut out, &mut err, true, true).unwrap();
    assert!(!out.contains(&0x1b));
    assert!(!err.contains(&0x1b));
    assert_eq!(String::from_utf8(out).unwrap().lines().count(), 3);
}

#[test]
fn kissat_conflicts_fail_during_parsing_before_input_is_opened() {
    for incompatible in [
        vec!["--mode", "incremental"],
        vec!["--all"],
        vec!["--find-all"],
        vec!["--regular"],
        vec!["--flag"],
        vec!["--flag-triangulation"],
        vec!["--regular", "--flag"],
    ] {
        let mut args = vec!["nonexistent-input", "--solver", "kissat"];
        args.extend(incompatible);
        let error = parse(&args).unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
        assert!(error.to_string().contains("Kissat"));
        assert_eq!(error.exit_code(), 2);
    }
}

#[test]
#[cfg(not(windows))]
fn kissat_valid_options_and_default_solver_are_preserved() {
    for mode in ["serial", "split"] {
        for device in ["cpu", "gpu"] {
            let cli = parse(&[
                "input",
                "--solver",
                "kissat",
                "--mode",
                mode,
                "--backend",
                device,
                "--parallel=2",
                "--timeout",
                "1",
                "--max-solutions",
                "1",
            ])
            .unwrap();
            assert_eq!(
                cli.search.settings(Mode::Serial, Solver::Picosat).backend,
                Backend::Kissat
            );
        }
    }
    assert_eq!(
        parse(&["input"])
            .unwrap()
            .search
            .settings(Mode::Serial, Solver::Picosat)
            .backend,
        Backend::PicoSat
    );
}

#[test]
#[cfg(not(windows))]
fn kissat_invalid_programmatic_cli_is_a_run_error_even_for_empty_input() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("empty.txt");
    fs::write(&input, "").unwrap();
    let mut cli = parse(&[input.to_str().unwrap(), "--solver", "kissat"]).unwrap();
    cli.search.regular = true;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let error = execute(&cli, &mut out, &mut err).unwrap_err();
    assert!(error.to_string().contains("Kissat"));
    assert!(out.is_empty());
    assert!(err.is_empty());
}

#[test]
#[cfg(not(windows))]
fn kissat_outer_parallel_jobs_finish_independently() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("two.txt");
    fs::write(&input, TRIANGLE.repeat(2)).unwrap();
    let (result, out, err) = run(
        &input,
        &[
            "--solver",
            "kissat",
            "--parallel=2",
            "--output-format",
            "tsv",
            "--terminal-output",
            "none",
        ],
    );
    let tally = result.unwrap();
    assert_eq!(tally.polytopes, 2);
    assert_eq!(tally.triangulatable, 2);
    assert_eq!(tally.failed, 0);
    assert_eq!(out.lines().count(), 3);
    assert!(err.is_empty());
}

#[test]
fn snapshot_inherits_settings_and_explicit_defaults_clear_saved_values() {
    let tree = tempfile::tempdir().unwrap();
    let input = tree.path().join("points");
    let checkpoint = tree.path().join("arbitrary.data");
    fs::write(&input, TRIANGLE.repeat(2)).unwrap();
    let initial = parse(&[
        input.to_str().unwrap(),
        "--all",
        "--regular",
        "--regular-learn",
        "--flag",
        "--flag-sat",
        "--solver=cadical",
        "--mode=incremental",
        "--backend=gpu",
        "--simplices=nondegenerate",
        "--max-solutions=9",
        "--max-regularity-tests=4",
        "--timeout=7",
        "--parallel=2",
        "--terminal-output=none",
        "--output-format=tsv",
    ])
    .unwrap();
    let session =
        Session::open_positional(&input, Some(&checkpoint), |s, r| initial.resolve(s, r)).unwrap();
    session.save().unwrap();
    let inherited = parse(&[checkpoint.to_str().unwrap()]).unwrap();
    let restored =
        Session::open_positional(&checkpoint, None, |s, r| inherited.resolve(s, r)).unwrap();
    assert_eq!(restored.settings(), session.settings());
    assert_eq!(restored.runtime().unwrap(), session.runtime().unwrap());
    let cap = parse(&[checkpoint.to_str().unwrap(), "--max-regularity-tests=2"]).unwrap();
    assert_eq!(
        Session::open_positional(&checkpoint, None, |s, r| cap.resolve(s, r))
            .unwrap()
            .settings()
            .max_regularity_tests
            .unwrap()
            .get(),
        2
    );
    let cleared = parse(&[
        checkpoint.to_str().unwrap(),
        "--find-all=false",
        "--regular=false",
        "--regular-learn=false",
        "--flag-triangulation=false",
        "--flag-sat=false",
        "--solver=picosat",
        "--mode=serial",
        "--intersection-backend=cpu",
        "--simplices=unimodular",
        "--max-solutions=0",
        "--max-regularity-tests=0",
        "--timeout=0",
        "--parallel=0",
        "--terminal-output=all",
        "--output-format=human",
    ])
    .unwrap();
    let cleared =
        Session::open_positional(&checkpoint, None, |s, r| cleared.resolve(s, r)).unwrap();
    assert_eq!(cleared.settings(), Settings::default());
    assert_eq!(cleared.runtime().unwrap(), RuntimeOptions::default());
}

#[test]
#[cfg(not(windows))]
fn merged_solver_validation_and_mixed_warning_survive_quiet_resume() {
    let tree = tempfile::tempdir().unwrap();
    let input = tree.path().join("points");
    let checkpoint = tree.path().join("saved.data");
    fs::write(&input, TRIANGLE.repeat(2)).unwrap();
    let initial = parse(&[
        input.to_str().unwrap(),
        "--solver=kissat",
        "--terminal-output=none",
        "--output-format=tsv",
    ])
    .unwrap();
    let session =
        Session::open_positional(&input, Some(&checkpoint), |s, r| initial.resolve(s, r)).unwrap();
    session.begin(1).unwrap();
    let old = Row {
        index: 1,
        status: crate::job::Status::Ok,
        triangulations: 1,
        regular: 0,
        flag: 0,
        quadratic: 0,
        seconds: 12.5,
    };
    session.finish(old).unwrap();
    session.save().unwrap();
    let invalid = parse(&[checkpoint.to_str().unwrap(), "--regular"]).unwrap();
    assert!(Session::open_positional(&checkpoint, None, |s, r| invalid.resolve(s, r)).is_err());
    let changed = parse(&[
        checkpoint.to_str().unwrap(),
        "--regular",
        "--solver=cadical",
    ])
    .unwrap();
    let restored =
        Session::open_positional(&checkpoint, None, |s, r| changed.resolve(s, r)).unwrap();
    assert_eq!(restored.completed(1).unwrap(), Some(old));
    assert!(
        restored
            .warnings()
            .iter()
            .any(|w| w.contains("almost never"))
    );
    restored.save().unwrap();
    let (result, out, err) = run(&checkpoint, &[]);
    assert_eq!(result.unwrap().polytopes, 2);
    assert!(err.contains("mix"), "{err}");
    assert!(
        out.lines()
            .any(|r| r == "1\thistorical\t?\t?\t?\t?\t12.5000")
    );
}

#[test]
fn quiet_filtered_runs_and_table_only_keep_counts_and_stage_samples() {
    let tree = tempfile::tempdir().unwrap();
    let input = tree.path().join("points");
    fs::write(&input, TRIANGLE.repeat(2)).unwrap();
    for parallel in ["--parallel=0", "--parallel=2"] {
        let (quiet, out, err) = run(
            &input,
            &[
                "--all",
                "--regular",
                "--flag",
                parallel,
                "--terminal-output=none",
            ],
        );
        assert!(out.is_empty() && err.is_empty());
        let (timed, out, err) = run(
            &input,
            &[
                "--all",
                "--regular",
                "--flag",
                parallel,
                "--terminal-output=table",
            ],
        );
        assert_eq!(quiet.unwrap(), timed.unwrap());
        assert!(err.is_empty());
        for stage in [
            "Lattice geometry",
            "Formula and intersections",
            "Search and classification",
        ] {
            let row = out.lines().find(|line| line.starts_with(stage)).unwrap();
            assert_eq!(row.split_whitespace().last(), Some("2"));
        }
    }
}

#[test]
fn flag_sat_is_independent_and_redundant_check_warns_even_when_quiet() {
    let tree = tempfile::tempdir().unwrap();
    let input = tree.path().join("points");
    fs::write(&input, TRIANGLE).unwrap();
    let parsed = parse(&[input.to_str().unwrap(), "--flag-sat"]).unwrap();
    assert!(parsed.search.flag_sat);
    assert!(!parsed.search.flag);
    for mode in ["--mode=serial", "--mode=split", "--mode=incremental"] {
        for solver in ["--solver=picosat", "--solver=cadical"] {
            let (result, out, err) = run(
                &input,
                &[
                    "--flag-sat",
                    mode,
                    solver,
                    "--terminal-output=none",
                    "--output-format=tsv",
                ],
            );
            assert!(err.is_empty(), "{err}");
            assert_eq!(result.unwrap().flag, 1);
            let mut lines = out.lines();
            assert_eq!(lines.next(), Some("# idx\tstatus\tn_flag\tseconds"));
            let row: Vec<_> = lines.next().unwrap().split('\t').collect();
            assert_eq!(row.len(), 4);
            assert_eq!(&row[..3], &["1", "ok", "1"]);
        }
    }
    let (result, out, err) = run(&input, &["--flag-sat", "--flag", "--terminal-output=none"]);
    assert_eq!(result.unwrap().flag, 1);
    assert!(out.is_empty());
    assert!(
        err.contains("redundant") && err.contains("independently check"),
        "{err}"
    );
}

#[test]
fn resumed_flag_sat_override_preserves_completed_rows_and_warns() {
    let tree = tempfile::tempdir().unwrap();
    let input = tree.path().join("points");
    let checkpoint = tree.path().join("saved.data");
    fs::write(&input, TRIANGLE.repeat(2)).unwrap();
    let session = Session::open(
        Some(&input),
        None,
        Some(&checkpoint),
        Some(Settings::default()),
    )
    .unwrap();
    let old = Row {
        index: 1,
        status: crate::job::Status::Ok,
        triangulations: 1,
        regular: 0,
        flag: 0,
        quadratic: 0,
        seconds: 0.125,
    };
    session.begin(1).unwrap();
    session.finish(old).unwrap();
    session.save().unwrap();
    let changed = parse(&[checkpoint.to_str().unwrap(), "--flag-sat"]).unwrap();
    let resumed =
        Session::open_positional(&checkpoint, None, |s, r| changed.resolve(s, r)).unwrap();
    assert!(resumed.settings().flag_sat);
    assert!(!resumed.settings().flag);
    assert_eq!(resumed.completed(1).unwrap(), Some(old));
    assert!(
        resumed
            .warnings()
            .iter()
            .any(|w| w.contains("--flag-sat") && w.contains("almost never"))
    );
    assert!(
        resumed
            .warnings()
            .iter()
            .any(|w| w.contains("mixes different requests"))
    );
    resumed.save().unwrap();
    let inherited = parse(&[checkpoint.to_str().unwrap()]).unwrap();
    let restored =
        Session::open_positional(&checkpoint, None, |s, r| inherited.resolve(s, r)).unwrap();
    assert!(restored.settings().flag_sat);
    let disabled = parse(&[checkpoint.to_str().unwrap(), "--flag-sat=false"]).unwrap();
    let restored =
        Session::open_positional(&checkpoint, None, |s, r| disabled.resolve(s, r)).unwrap();
    assert!(!restored.settings().flag_sat);
}

#[test]
fn regular_learning_requires_regular_and_reports_observed_counts() {
    assert!(parse(&["input", "--regular-learn"]).is_err());
    assert!(parse(&["input", "--regular-learn", "--regular=false"]).is_err());
    assert!(parse(&["input", "--regular-learn", "--regular", "--solver=kissat"]).is_err());
    assert!(parse(&["input", "--regular-learn=false"]).is_ok());
    let tree = tempfile::tempdir().unwrap();
    let input = tree.path().join("points");
    fs::write(&input, TRIANGLE).unwrap();
    for mode in ["--mode=serial", "--mode=split", "--mode=incremental"] {
        let (result, out, err) = run(
            &input,
            &[
                "--regular",
                "--regular-learn",
                "--all",
                mode,
                "--terminal-output=none",
                "--output-format=tsv",
            ],
        );
        assert_eq!(result.unwrap().regular, 1);
        let row = out.lines().find_map(Row::parse).unwrap();
        assert_eq!((row.triangulations, row.regular), (1, 1));
        assert!(err.contains("observed lower bounds"), "{err}");
    }
}

#[test]
fn resumed_learning_overrides_inherit_regular_and_preserve_completed_rows() {
    let tree = tempfile::tempdir().unwrap();
    let input = tree.path().join("points");
    let checkpoint = tree.path().join("snapshot");
    fs::write(&input, TRIANGLE.repeat(2)).unwrap();
    let settings = Settings {
        regular: true,
        ..Settings::default()
    };
    let session = Session::open(Some(&input), None, Some(&checkpoint), Some(settings)).unwrap();
    let old = Row {
        index: 1,
        status: crate::job::Status::Ok,
        triangulations: 1,
        regular: 1,
        flag: 0,
        quadratic: 0,
        seconds: 0.125,
    };
    session.begin(1).unwrap();
    session.finish(old).unwrap();
    session.save().unwrap();
    let changed = parse(&[checkpoint.to_str().unwrap(), "--regular-learn"]).unwrap();
    let resumed =
        Session::open_positional(&checkpoint, None, |s, r| changed.resolve(s, r)).unwrap();
    assert!(resumed.settings().regular && resumed.settings().regular_learn);
    assert_eq!(resumed.completed(1).unwrap(), Some(old));
    assert!(
        resumed
            .warnings()
            .iter()
            .any(|w| w.contains("mixes different requests"))
    );
    resumed.save().unwrap();
    let inherited = parse(&[checkpoint.to_str().unwrap()]).unwrap();
    let restored =
        Session::open_positional(&checkpoint, None, |s, r| inherited.resolve(s, r)).unwrap();
    assert!(restored.settings().regular_learn);
    let invalid = parse(&[checkpoint.to_str().unwrap(), "--regular=false"]).unwrap();
    assert!(Session::open_positional(&checkpoint, None, |s, r| invalid.resolve(s, r)).is_err());
    let disabled = parse(&[
        checkpoint.to_str().unwrap(),
        "--regular-learn=false",
        "--regular=false",
    ])
    .unwrap();
    let restored =
        Session::open_positional(&checkpoint, None, |s, r| disabled.resolve(s, r)).unwrap();
    assert!(!restored.settings().regular_learn && !restored.settings().regular);
}

#[test]
fn flag_sat_regularity_omits_regular_count_without_a_warning() {
    let tree = tempfile::tempdir().unwrap();
    let input = tree.path().join("points");
    fs::write(&input, TRIANGLE.repeat(2)).unwrap();
    for parallel in ["--parallel=0", "--parallel=2"] {
        let (result, out, err) = run(
            &input,
            &[
                "--flag-sat",
                "--regular",
                parallel,
                "--terminal-output=running,final",
            ],
        );
        assert_eq!(result.unwrap().quadratic, 2);
        assert!(err.is_empty(), "{err}");
        assert!(!out.contains("Regularly Triangulatable:"));
        assert!(out.contains("Flag Triangulatable:") && out.contains("Quadratic Triangulatable:"));
        let (result, out, err) = run(
            &input,
            &[
                "--flag-sat",
                "--regular",
                parallel,
                "--output-format=tsv",
                "--terminal-output=none",
            ],
        );
        assert_eq!(result.unwrap().quadratic, 2);
        assert!(err.is_empty(), "{err}");
        let mut lines = out.lines();
        assert_eq!(
            lines.next(),
            Some("# idx\tstatus\tn_flag\tn_quadratic\tseconds")
        );
        let rows: Vec<_> = lines
            .map(|line| line.split('\t').collect::<Vec<_>>())
            .collect();
        assert_eq!(rows.len(), 2);
        for row in rows {
            assert_eq!(row.len(), 5);
            assert_eq!(&row[1..4], &["ok", "1", "1"]);
        }
    }
    let (_, out, err) = run(&input, &["--regular", "--flag", "--terminal-output=final"]);
    assert!(err.is_empty());
    assert!(out.contains("Regularly Triangulatable:"));
}

#[test]
fn resumed_quadratic_output_projects_canonical_completed_and_fresh_rows() {
    let tree = tempfile::tempdir().unwrap();
    let input = tree.path().join("points");
    let checkpoint = tree.path().join("saved");
    fs::write(&input, TRIANGLE.repeat(2)).unwrap();
    let settings = Settings {
        regular: true,
        flag_sat: true,
        ..Settings::default()
    };
    let session = Session::open(Some(&input), None, Some(&checkpoint), Some(settings)).unwrap();
    let row = Row {
        index: 1,
        status: Status::Ok,
        triangulations: 1,
        regular: 1,
        flag: 1,
        quadratic: 1,
        seconds: 0.125,
    };
    session.begin(1).unwrap();
    session.finish(row).unwrap();
    session.save().unwrap();
    assert_eq!(Row::parse(&row.to_string()), Some(row));
    for parallel in ["--parallel=0", "--parallel=2"] {
        let (result, out, err) = run(
            &checkpoint,
            &[parallel, "--output-format=tsv", "--terminal-output=none"],
        );
        assert_eq!(result.unwrap().quadratic, 2);
        assert!(err.is_empty(), "{err}");
        assert!(!out.contains("n_regular"));
        let rows: Vec<_> = out.lines().skip(1).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], "1\tok\t1\t1\t0.1250");
        assert!(rows.iter().all(|r| r.split('\t').count() == 5));
    }
}

#[test]
fn projected_skipped_rows_keep_status_counts_and_seconds() {
    let row = Row::skipped(1, 0.25);
    let mut out = Vec::new();
    write_result(
        &row,
        Settings {
            regular: true,
            flag_sat: true,
            ..Settings::default()
        },
        &mut out,
    )
    .unwrap();
    assert_eq!(out, b"1\tskipped\t-1\t-1\t0.2500\n");
    out.clear();
    write_result(&row, Settings::default(), &mut out).unwrap();
    assert_eq!(out, format!("{row}\n").as_bytes());
}

fn summary_value<'a>(text: &'a str, label: &str) -> Option<&'a str> {
    text.lines()
        .rev()
        .find_map(|line| line.strip_prefix(label).map(str::trim))
}

#[test]
fn flag_sat_counts_do_not_claim_general_triangulability() {
    let tree = tempfile::tempdir().unwrap();
    let input = tree.path().join("points");
    fs::write(&input, TRIANGLE.repeat(2)).unwrap();
    for parallel in ["--parallel=0", "--parallel=2"] {
        for regular in ["--regular=false", "--regular"] {
            let (result, out, err) = run(
                &input,
                &[
                    "--flag-sat",
                    regular,
                    parallel,
                    "--terminal-output=running,final",
                ],
            );
            assert_eq!(result.unwrap().flag, 2);
            assert!(err.is_empty(), "{err}");
            assert_eq!(summary_value(&out, "Triangulatable:"), None);
            assert_eq!(summary_value(&out, "Regularly Triangulatable:"), None);
            assert_eq!(summary_value(&out, "Flag Triangulatable:"), Some("2"));
            assert_eq!(
                summary_value(&out, "Quadratic Triangulatable:"),
                (regular == "--regular").then_some("2")
            );
        }
    }
}

#[test]
fn exhausted_unimodular_search_does_not_claim_absence_of_other_triangulations() {
    let tree = tempfile::tempdir().unwrap();
    let input = tree.path().join("reeve");
    // An empty volume-two lattice tetrahedron: its single simplex is not unimodular.
    fs::write(&input, "[[0,0,0],[1,0,0],[0,1,0],[1,1,2]]\n").unwrap();
    for (flag, label) in [
        ("--flag-sat=false", "No unimodular triangulation:"),
        ("--flag-sat", "No unimodular flag triangulation:"),
    ] {
        let (_, out, err) = run(&input, &[flag, "--terminal-output=final"]);
        assert!(err.is_empty(), "{err}");
        assert_eq!(summary_value(&out, label), Some("1"));
        assert_eq!(summary_value(&out, "Non-Triangulatable:"), None);
        let (result, out, err) = run(
            &input,
            &[flag, "--simplices=nondegenerate", "--terminal-output=final"],
        );
        assert!(err.is_empty(), "{err}");
        assert_eq!(result.unwrap().triangulatable, 1);
        let negative = if flag == "--flag-sat" {
            "No flag triangulation:"
        } else {
            "Non-Triangulatable:"
        };
        assert_eq!(summary_value(&out, negative), Some("0"));
    }
}

#[test]
fn different_request_results_are_retained_without_current_query_claims() {
    for parallel in ["--parallel=0", "--parallel=2"] {
        for old_flag_sat in [true, false] {
            let tree = tempfile::tempdir().unwrap();
            let input = tree.path().join("points");
            let checkpoint = tree.path().join("checkpoint");
            fs::write(&input, TRIANGLE).unwrap();
            let settings = Settings {
                flag_sat: old_flag_sat,
                goal: Goal::All,
                ..Settings::default()
            };
            let session =
                Session::open(Some(&input), None, Some(&checkpoint), Some(settings)).unwrap();
            let old = Row {
                index: 1,
                status: Status::Ok,
                triangulations: u64::from(!old_flag_sat),
                regular: 0,
                flag: 0,
                quadratic: 0,
                seconds: 0.125,
            };
            session.begin(1).unwrap();
            session.finish(old).unwrap();
            session.save().unwrap();
            let changed = if old_flag_sat {
                "--flag-sat=false"
            } else {
                "--flag-sat"
            };
            let (result, out, err) =
                run(&checkpoint, &[changed, parallel, "--terminal-output=final"]);
            let tally = result.unwrap();
            assert_eq!(
                (
                    tally.polytopes,
                    tally.historical,
                    tally.triangulatable,
                    tally.flag
                ),
                (1, 1, 0, 0)
            );
            assert!(err.contains("mixes different requests"));
            assert_eq!(summary_value(&out, "Other-request results:"), Some("1"));
            assert_eq!(summary_value(&out, "Unknown / incomplete:"), None);
            let negative = if old_flag_sat {
                "No unimodular triangulation:"
            } else {
                "No unimodular flag triangulation:"
            };
            assert_eq!(summary_value(&out, negative), Some("0"));
            assert_eq!(
                summary_value(&out, "Average targets found/input:"),
                Some("0.00 (observed)")
            );
            let (_, out, _) = run(
                &checkpoint,
                &[
                    changed,
                    parallel,
                    "--output-format=tsv",
                    "--terminal-output=none",
                ],
            );
            let row: Vec<_> = out.lines().nth(1).unwrap().split('\t').collect();
            assert_eq!(row[1], "historical");
            assert_eq!(row.last(), Some(&"0.1250"));
            assert_eq!(row.len(), if old_flag_sat { 7 } else { 4 });
            assert!(row[2..row.len() - 1].iter().all(|v| *v == "?"));
            // A changed solver/mode is still the original query, and reveals the saved result.
            let original = if old_flag_sat {
                "--flag-sat"
            } else {
                "--flag-sat=false"
            };
            let cli = parse(&[
                checkpoint.to_str().unwrap(),
                original,
                "--solver=cadical",
                "--mode=incremental",
            ])
            .unwrap();
            let restored =
                Session::open_positional(&checkpoint, None, |s, r| cli.resolve(s, r)).unwrap();
            assert!(restored.result_matches_request(1).unwrap());
            assert_eq!(restored.completed(1).unwrap(), Some(old));
        }
    }
}

#[test]
fn capped_target_average_is_explicitly_observed() {
    let tree = tempfile::tempdir().unwrap();
    let input = tree.path().join("square");
    fs::write(&input, "[[0,0],[1,0],[1,1],[0,1]]\n").unwrap();
    let (result, out, _) = run(
        &input,
        &["--all", "--max-solutions=1", "--terminal-output=final"],
    );
    assert_eq!(result.unwrap().capped, 1);
    assert_eq!(
        summary_value(&out, "Average targets found/input:"),
        Some("1.00 (observed)")
    );
    assert!(!out.contains("Average Target Solutions/Poly:"));
}

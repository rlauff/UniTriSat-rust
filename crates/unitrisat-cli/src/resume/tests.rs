use super::*;

const INPUT: &str = "[[0,0],[1,0],[0,1]]\n[[0,0],[1,0],[1,1],[0,1]]\n";

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("polytopes.txt");
    fs::write(&input, INPUT).unwrap();
    let checkpoint = dir.path().join("run.resume.json");
    (dir, input, checkpoint)
}

fn row(index: u64, status: Status) -> Row {
    Row {
        index,
        status,
        triangulations: 2,
        regular: 1,
        flag: 1,
        quadratic: 1,
        seconds: 0.125,
    }
}

#[test]
fn completed_rows_restored_interrupted_job_restarts() {
    let (_dir, input, checkpoint) = fixture();
    let settings = Settings {
        mode: Mode::Incremental,
        backend: Backend::Cadical,
        intersection_backend: IntersectionBackend::Gpu,
        restriction: Restriction::Nondegenerate,
        goal: Goal::All,
        regular: true,
        regular_learn: true,
        flag: true,
        flag_sat: true,
        max_solutions: NonZeroU64::new(15),
        max_regularity_tests: NonZeroU64::new(4),
        timeout: Some(Duration::new(60, 123)),
    };
    let session = Session::open(Some(&input), None, Some(&checkpoint), Some(settings)).unwrap();
    assert_eq!(session.total(), 2);
    session.begin(1).unwrap();
    session.finish(row(1, Status::Capped)).unwrap();
    session.begin(2).unwrap();
    session.save().unwrap();
    let state: CurrentCheckpoint = serde_json::from_slice(&fs::read(&checkpoint).unwrap()).unwrap();
    assert_eq!(state.version, 3);
    assert_eq!(state.active, vec![2]);
    assert_eq!(state.attempts.len(), 1);
    let resumed = Session::open(None, Some(&checkpoint), None, None).unwrap();
    assert_eq!(resumed.settings(), settings);
    assert_eq!(resumed.completed(1).unwrap(), Some(row(1, Status::Capped)));
    assert!(resumed.completed(2).unwrap().is_none());
    resumed.begin(2).unwrap();
    resumed.finish(row(2, Status::RegLimit)).unwrap();
    resumed.save().unwrap();
    let final_run = Session::open(None, Some(&checkpoint), None, None).unwrap();
    let completed = final_run.completed(2).unwrap().unwrap();
    assert_eq!(completed.status, Status::RegLimit);
    assert!(completed.seconds >= row(2, Status::RegLimit).seconds);
}

#[test]
fn failed_attempts_retry_without_recomputing_later_completed_rows() {
    let (_dir, input, checkpoint) = fixture();
    let session = Session::open(
        Some(&input),
        None,
        Some(&checkpoint),
        Some(Settings::default()),
    )
    .unwrap();
    session.begin(1).unwrap();
    session.fail(1, "test failure".into()).unwrap();
    session.begin(2).unwrap();
    session.finish(row(2, Status::Ok)).unwrap();
    session.save().unwrap();
    let resumed = Session::open(None, Some(&checkpoint), None, None).unwrap();
    assert!(resumed.completed(1).unwrap().is_none());
    assert!(resumed.completed(2).unwrap().is_some());
    resumed.begin(1).unwrap();
    resumed.save().unwrap();
    let again = Session::open(None, Some(&checkpoint), None, None).unwrap();
    again.begin(1).unwrap();
    again.finish(row(1, Status::Ok)).unwrap();
    assert!(again.begin(2).is_err());
    again.save().unwrap();
}

#[test]
fn rejects_input_changes_settings_changes_and_source_as_destination() {
    let (_dir, input, checkpoint) = fixture();
    assert!(Session::open(Some(&input), None, Some(&input), Some(Settings::default())).is_err());
    let session = Session::open(
        Some(&input),
        None,
        Some(&checkpoint),
        Some(Settings::default()),
    )
    .unwrap();
    session.save().unwrap();
    assert!(
        Session::open(
            None,
            Some(&checkpoint),
            None,
            Some(Settings {
                goal: Goal::All,
                ..Settings::default()
            })
        )
        .is_err()
    );
    fs::write(&input, "[[0,0],[2,0],[0,2]]\n").unwrap();
    assert!(Session::open(None, Some(&checkpoint), None, None).is_err());
    assert_eq!(fs::read_to_string(session.snapshot_path()).unwrap(), INPUT);
}

#[test]
fn rejects_hard_links_and_symbolic_links_to_input() {
    let (_dir, input, checkpoint) = fixture();
    fs::hard_link(&input, &checkpoint).unwrap();
    assert!(
        Session::open(
            Some(&input),
            None,
            Some(&checkpoint),
            Some(Settings::default())
        )
        .is_err()
    );
    assert_eq!(fs::read_to_string(&input).unwrap(), INPUT);
    #[cfg(unix)]
    {
        fs::remove_file(&checkpoint).unwrap();
        std::os::unix::fs::symlink(&input, &checkpoint).unwrap();
        assert!(
            Session::open(
                Some(&input),
                None,
                Some(&checkpoint),
                Some(Settings::default())
            )
            .is_err()
        );
    }
}

#[test]
fn invalid_schema_rows_indices_and_interrupted_markers_rejected() {
    let (_dir, input, checkpoint) = fixture();
    let session = Session::open(
        Some(&input),
        None,
        Some(&checkpoint),
        Some(Settings::default()),
    )
    .unwrap();
    session.begin(1).unwrap();
    session.finish(row(1, Status::Ok)).unwrap();
    session.save().unwrap();
    let original: serde_json::Value =
        serde_json::from_slice(&fs::read(&checkpoint).unwrap()).unwrap();
    for (pointer, replacement) in [
        ("/version", serde_json::json!(4)),
        ("/total", serde_json::json!(3)),
        ("/active", serde_json::json!([1])),
        ("/active", serde_json::json!([3])),
        ("/settings/mode", serde_json::json!("unknown")),
        (
            "/attempts/0/row",
            serde_json::json!(row(3, Status::Ok).to_string()),
        ),
        (
            "/attempts/0/row",
            serde_json::json!(Row::skipped(1, 0.0).to_string()),
        ),
        (
            "/attempts/0/row",
            serde_json::json!("1\tok\t1\t2\t0\t0\t0.0000"),
        ),
    ] {
        let mut mutated = original.clone();
        *mutated.pointer_mut(pointer).unwrap() = replacement;
        fs::write(&checkpoint, serde_json::to_vec(&mutated).unwrap()).unwrap();
        assert!(
            Session::open(None, Some(&checkpoint), None, None).is_err(),
            "accepted {pointer}"
        );
    }
    let mut extra = original;
    extra["unexpected"] = serde_json::json!(true);
    fs::write(&checkpoint, serde_json::to_vec(&extra).unwrap()).unwrap();
    assert!(Session::open(None, Some(&checkpoint), None, None).is_err());
}

#[test]
fn interruption_before_first_job_and_between_jobs_is_valid() {
    let (_dir, input, checkpoint) = fixture();
    let session = Session::open(
        Some(&input),
        None,
        Some(&checkpoint),
        Some(Settings::default()),
    )
    .unwrap();
    session.save().unwrap();
    assert!(
        Session::open(None, Some(&checkpoint), None, None)
            .unwrap()
            .completed(1)
            .unwrap()
            .is_none()
    );
    assert!(session.begin(0).is_err());
    assert!(session.begin(3).is_err());
    session.begin(1).unwrap();
    assert!(session.finish(row(2, Status::Ok)).is_err());
    session.finish(row(1, Status::Ok)).unwrap();
    session.save().unwrap();
    assert!(
        Session::open(None, Some(&checkpoint), None, None)
            .unwrap()
            .completed(1)
            .unwrap()
            .is_some()
    );
}

#[test]
fn checkpoint_save_rechecks_alias_and_preserves_source() {
    let (_dir, input, checkpoint) = fixture();
    let session = Session::open(
        Some(&input),
        None,
        Some(&checkpoint),
        Some(Settings::default()),
    )
    .unwrap();
    fs::hard_link(&input, &checkpoint).unwrap();
    assert!(session.save().is_err());
    assert_eq!(fs::read_to_string(&input).unwrap(), INPUT);
}

#[test]
fn session_exit_waits_for_an_in_progress_checkpoint_handler() {
    let (_dir, input, checkpoint) = fixture();
    let session = Session::open(
        Some(&input),
        None,
        Some(&checkpoint),
        Some(Settings::default()),
    )
    .unwrap();
    let running = Arc::clone(&session.running);
    let handler_gate = running.lock().unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();
    let exiting = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        drop(session);
        finished_tx.send(()).unwrap();
    });
    started_rx.recv().unwrap();
    assert!(matches!(
        finished_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    drop(handler_gate);
    finished_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    exiting.join().unwrap();
    assert!(!*running.lock().unwrap());
}

fn saved_value(session: &Session, checkpoint: &Path) -> serde_json::Value {
    session.save().unwrap();
    serde_json::from_slice(&fs::read(checkpoint).unwrap()).unwrap()
}

fn write_value(checkpoint: &Path, value: &serde_json::Value) {
    fs::write(checkpoint, serde_json::to_vec(value).unwrap()).unwrap();
}

#[test]
fn sparse_checkpoint_keeps_later_completed_work_and_all_active_jobs() {
    let (_dir, input, checkpoint) = fixture();
    fs::write(&input, INPUT.repeat(3)).unwrap();
    let session = Session::open(
        Some(&input),
        None,
        Some(&checkpoint),
        Some(Settings::default()),
    )
    .unwrap();
    for index in [6, 2, 5, 1] {
        session.begin(index).unwrap();
    }
    session.finish(row(5, Status::RegLimit)).unwrap();
    session.finish(row(2, Status::Capped)).unwrap();
    let saved = saved_value(&session, &checkpoint);
    assert_eq!(saved["active"], serde_json::json!([1, 6]));
    assert_eq!(saved["attempts"].as_array().unwrap().len(), 2);
    let resumed = Session::open(None, Some(&checkpoint), None, None).unwrap();
    assert_eq!(
        resumed.completed(5).unwrap(),
        Some(row(5, Status::RegLimit))
    );
    assert_eq!(resumed.completed(2).unwrap(), Some(row(2, Status::Capped)));
    for index in [1, 3, 4, 6] {
        assert_eq!(resumed.completed(index).unwrap(), None);
        resumed.begin(index).unwrap();
    }
    assert!(resumed.begin(2).is_err());
    assert!(resumed.begin(5).is_err());
    let again = saved_value(&resumed, &checkpoint);
    assert_eq!(again["attempts"], saved["attempts"]);
    assert_eq!(again["active"], serde_json::json!([1, 3, 4, 6]));
}

#[test]
fn sparse_retry_replaces_failure_with_active_marker_then_completion() {
    let (_dir, input, checkpoint) = fixture();
    let session = Session::open(
        Some(&input),
        None,
        Some(&checkpoint),
        Some(Settings::default()),
    )
    .unwrap();
    session.begin(2).unwrap();
    session.fail(2, "later job failed".into()).unwrap();
    session.begin(1).unwrap();
    let saved = saved_value(&session, &checkpoint);
    assert_eq!(saved["active"], serde_json::json!([1]));
    assert_eq!(saved["attempts"][0]["index"], 2);
    let resumed = Session::open(None, Some(&checkpoint), None, None).unwrap();
    resumed.begin(2).unwrap();
    let retrying = saved_value(&resumed, &checkpoint);
    assert_eq!(retrying["active"], serde_json::json!([2]));
    assert_eq!(retrying["attempts"], serde_json::json!([]));
    let interrupted_retry = Session::open(None, Some(&checkpoint), None, None).unwrap();
    interrupted_retry.begin(2).unwrap();
    interrupted_retry.finish(row(2, Status::Ok)).unwrap();
    let completed = saved_value(&interrupted_retry, &checkpoint);
    assert_eq!(completed["active"], serde_json::json!([]));
    let result = Row::parse(completed["attempts"][0]["row"].as_str().unwrap()).unwrap();
    assert_eq!(result.status, Status::Ok);
    assert!(result.seconds >= row(2, Status::Ok).seconds);
}

#[test]
fn rejects_sparse_duplicates_collisions_and_malformed_rows() {
    let (_dir, input, checkpoint) = fixture();
    let session = Session::open(
        Some(&input),
        None,
        Some(&checkpoint),
        Some(Settings::default()),
    )
    .unwrap();
    let original = saved_value(&session, &checkpoint);
    let completed = serde_json::json!({"state":"completed", "row":row(2, Status::Ok).to_string()});
    let failed = serde_json::json!({"state":"failed", "index":2, "error":"failure"});
    for (attempts, active) in [
        (
            serde_json::json!([completed.clone(), completed.clone()]),
            serde_json::json!([]),
        ),
        (
            serde_json::json!([failed.clone(), failed.clone()]),
            serde_json::json!([]),
        ),
        (
            serde_json::json!([completed.clone(), failed.clone()]),
            serde_json::json!([]),
        ),
        (serde_json::json!([completed]), serde_json::json!([2])),
        (serde_json::json!([failed]), serde_json::json!([2])),
        (serde_json::json!([]), serde_json::json!([1, 1])),
        (serde_json::json!([]), serde_json::json!([0])),
        (serde_json::json!([]), serde_json::json!([3])),
        (
            serde_json::json!([{"state":"failed", "index":0, "error":"failure"}]),
            serde_json::json!([]),
        ),
        (
            serde_json::json!([{"state":"failed", "index":3, "error":"failure"}]),
            serde_json::json!([]),
        ),
        (
            serde_json::json!([{"state":"failed", "index":1, "error":""}]),
            serde_json::json!([]),
        ),
        (
            serde_json::json!([{"state":"completed", "row":row(0, Status::Ok).to_string()}]),
            serde_json::json!([]),
        ),
    ] {
        let mut mutated = original.clone();
        mutated["attempts"] = attempts;
        mutated["active"] = active;
        write_value(&checkpoint, &mutated);
        assert!(
            Session::open(None, Some(&checkpoint), None, None).is_err(),
            "accepted {mutated}"
        );
    }
    for key in ["interrupted", "unexpected"] {
        let mut mutated = original.clone();
        mutated[key] = serde_json::Value::Null;
        write_value(&checkpoint, &mutated);
        assert!(Session::open(None, Some(&checkpoint), None, None).is_err());
    }
}

#[test]
fn legacy_prefix_and_interrupted_failure_migrate_without_losing_rows() {
    let (_dir, input, checkpoint) = fixture();
    let session = Session::open(
        Some(&input),
        None,
        Some(&checkpoint),
        Some(Settings::default()),
    )
    .unwrap();
    let mut legacy = saved_value(&session, &checkpoint);
    downgrade(&mut legacy, 1);
    legacy.as_object_mut().unwrap().remove("active");
    legacy["version"] = serde_json::json!(1);
    legacy["attempts"] = serde_json::json!([
        {"state":"failed", "index":1, "error":"interrupted retry"},
        {"state":"completed", "row":row(2, Status::Capped).to_string()},
    ]);
    legacy["interrupted"] = serde_json::json!(1);
    write_value(&checkpoint, &legacy);
    let resumed = Session::open(None, Some(&checkpoint), None, None).unwrap();
    assert_eq!(resumed.completed(2).unwrap(), Some(row(2, Status::Capped)));
    resumed.begin(1).unwrap();
    let migrated = saved_value(&resumed, &checkpoint);
    assert_eq!(migrated["version"], 3);
    assert_eq!(migrated["active"], serde_json::json!([1]));
    assert_eq!(
        migrated["attempts"],
        serde_json::json!([legacy["attempts"][1].clone()])
    );
    assert!(migrated.get("interrupted").is_none());
    // Ordinary legacy checkpoints at either side of the prefix boundary work.
    legacy["attempts"] = serde_json::json!([legacy["attempts"][1].clone()]);
    legacy["attempts"][0]["row"] = serde_json::json!(row(1, Status::Capped).to_string());
    for interrupted in [serde_json::Value::Null, serde_json::json!(2)] {
        legacy["interrupted"] = interrupted;
        write_value(&checkpoint, &legacy);
        let resumed = Session::open(None, Some(&checkpoint), None, None).unwrap();
        assert_eq!(resumed.completed(1).unwrap(), Some(row(1, Status::Capped)));
        resumed.begin(2).unwrap();
    }
}

#[test]
fn malformed_legacy_prefixes_and_mixed_schema_fields_are_rejected() {
    let (_dir, input, checkpoint) = fixture();
    let session = Session::open(
        Some(&input),
        None,
        Some(&checkpoint),
        Some(Settings::default()),
    )
    .unwrap();
    let mut legacy = saved_value(&session, &checkpoint);
    downgrade(&mut legacy, 1);
    legacy.as_object_mut().unwrap().remove("active");
    legacy["version"] = serde_json::json!(1);
    legacy["interrupted"] = serde_json::Value::Null;
    for attempts in [
        serde_json::json!([{"state":"completed", "row":row(2, Status::Ok).to_string()}]),
        serde_json::json!([{"state":"failed", "index":2, "error":"failure"}]),
        serde_json::json!([
            {"state":"completed", "row":row(1, Status::Ok).to_string()},
            {"state":"completed", "row":row(1, Status::Ok).to_string()},
        ]),
    ] {
        let mut mutated = legacy.clone();
        mutated["attempts"] = attempts;
        write_value(&checkpoint, &mutated);
        assert!(Session::open(None, Some(&checkpoint), None, None).is_err());
    }
    for (key, value) in [
        ("interrupted", serde_json::json!(0)),
        ("interrupted", serde_json::json!(2)),
        ("interrupted", serde_json::json!(3)),
        ("active", serde_json::json!([])),
        ("active", serde_json::Value::Null),
        ("version", serde_json::json!(2)),
    ] {
        let mut mutated = legacy.clone();
        mutated[key] = value;
        write_value(&checkpoint, &mutated);
        assert!(Session::open(None, Some(&checkpoint), None, None).is_err());
    }
    legacy["attempts"] =
        serde_json::json!([{"state":"completed", "row":row(1, Status::Ok).to_string()}]);
    legacy["interrupted"] = serde_json::json!(1);
    write_value(&checkpoint, &legacy);
    assert!(Session::open(None, Some(&checkpoint), None, None).is_err());
}

#[test]
fn concurrent_workers_publish_checkpoint_results_before_result_notifications() {
    let (_dir, input, checkpoint) = fixture();
    fs::write(&input, INPUT.repeat(16)).unwrap();
    let session = Session::open(
        Some(&input),
        None,
        Some(&checkpoint),
        Some(Settings::default()),
    )
    .unwrap();
    let ready = std::sync::Barrier::new(33);
    let release = std::sync::Barrier::new(33);
    std::thread::scope(|scope| {
        for index in 1..=32 {
            let session = &session;
            let ready = &ready;
            let release = &release;
            scope.spawn(move || {
                session.begin(index).unwrap();
                if index % 3 == 0 {
                    session.fail(index, "retry me".into()).unwrap();
                } else if index % 3 == 1 {
                    session.finish(row(index, Status::Ok)).unwrap();
                }
                // Model a worker blocked on the result queue: state must
                // already be complete when the signal snapshot is saved.
                ready.wait();
                release.wait();
            });
        }
        ready.wait();
        let saved = saved_value(&session, &checkpoint);
        release.wait();
        let resumed = Session::open(None, Some(&checkpoint), None, None).unwrap();
        for index in 1..=32 {
            if index % 3 == 1 {
                assert_eq!(
                    resumed.completed(index).unwrap(),
                    Some(row(index, Status::Ok))
                );
            } else {
                assert_eq!(resumed.completed(index).unwrap(), None);
                resumed.begin(index).unwrap();
            }
        }
        let active: Vec<_> = (1..=32).filter(|index| index % 3 == 2).collect();
        assert_eq!(saved["active"], serde_json::json!(active));
    });
}

#[test]
fn invalid_worker_transitions_leave_active_state_and_rows_unchanged() {
    let (_dir, input, checkpoint) = fixture();
    let session = Session::open(
        Some(&input),
        None,
        Some(&checkpoint),
        Some(Settings::default()),
    )
    .unwrap();
    session.begin(2).unwrap();
    assert!(session.begin(2).is_err());
    assert!(session.finish(row(1, Status::Ok)).is_err());
    assert!(session.finish(Row::skipped(2, 0.0)).is_err());
    assert!(session.fail(2, String::new()).is_err());
    assert!(session.fail(1, "inactive".into()).is_err());
    let active = saved_value(&session, &checkpoint);
    assert_eq!(active["active"], serde_json::json!([2]));
    assert_eq!(active["attempts"], serde_json::json!([]));
    session.finish(row(2, Status::Ok)).unwrap();
    assert!(session.finish(row(2, Status::Capped)).is_err());
    assert!(session.fail(2, "late failure".into()).is_err());
    assert!(session.begin(2).is_err());
    assert_eq!(session.completed(2).unwrap(), Some(row(2, Status::Ok)));
    assert!(session.completed(0).is_err());
    assert!(session.completed(3).unwrap().is_none());
}

#[test]
#[cfg(not(windows))]
fn kissat_checkpoint_versions_round_trip_and_reject_invalid_saved_settings() {
    let (_dir, input, checkpoint) = fixture();
    let settings = Settings {
        backend: Backend::Kissat,
        mode: Mode::Split,
        intersection_backend: IntersectionBackend::Gpu,
        timeout: Some(Duration::from_secs(3)),
        ..Settings::default()
    };
    let session = Session::open(Some(&input), None, Some(&checkpoint), Some(settings)).unwrap();
    session.save().unwrap();
    let original: serde_json::Value =
        serde_json::from_slice(&fs::read(&checkpoint).unwrap()).unwrap();
    assert_eq!(original["settings"]["solver"], "kissat");
    for version in [1, 2, 3] {
        let mut saved = original.clone();
        downgrade(&mut saved, version);
        if version == 1 {
            saved["version"] = serde_json::json!(1);
            saved.as_object_mut().unwrap().remove("active");
            saved["interrupted"] = serde_json::Value::Null;
        }
        fs::write(&checkpoint, serde_json::to_vec(&saved).unwrap()).unwrap();
        assert_eq!(
            Session::open(None, Some(&checkpoint), None, None)
                .unwrap()
                .settings(),
            settings
        );
        for (key, value) in [
            ("mode", serde_json::json!("incremental")),
            ("all", serde_json::json!(true)),
            ("regular", serde_json::json!(true)),
            ("flag", serde_json::json!(true)),
        ] {
            let mut invalid = saved.clone();
            invalid["settings"][key] = value;
            // Settings are rejected even before trying to read the input.
            invalid["input"] = serde_json::json!(input.with_file_name("missing.txt"));
            fs::write(&checkpoint, serde_json::to_vec(&invalid).unwrap()).unwrap();
            let error = Session::open(None, Some(&checkpoint), None, None)
                .err()
                .unwrap();
            assert!(error.to_string().contains("Kissat"), "{error}");
        }
    }
}

fn downgrade(value: &mut serde_json::Value, version: u32) {
    value["version"] = serde_json::json!(version);
    if version < 3 {
        for field in [
            "runtime",
            "elapsed",
            "started_unix",
            "legacy_timing",
            "attempt_elapsed",
            "timings",
            "settings_history",
            "result_settings",
        ] {
            value.as_object_mut().unwrap().remove(field);
        }
    }
}

#[test]
fn elapsed_and_interrupted_cost_accumulate_without_save_double_count_or_offline_gap() {
    let (_dir, input, checkpoint) = fixture();
    let session = Session::open(
        Some(&input),
        None,
        Some(&checkpoint),
        Some(Settings {
            timeout: Some(Duration::from_secs(2)),
            ..Settings::default()
        }),
    )
    .unwrap();
    session.begin(1).unwrap();
    {
        let mut state = session.state().unwrap();
        state.elapsed = Duration::from_secs(37);
        state.running_since = Instant::now() - Duration::from_secs(3);
        state
            .active_started
            .insert(1, Instant::now() - Duration::from_secs(2));
        state.started_unix = Some(Duration::from_secs(1));
    }
    session.save().unwrap();
    session.save().unwrap();
    let resumed = Session::open(None, Some(&checkpoint), None, None).unwrap();
    assert!((40.0..41.0).contains(&resumed.elapsed().unwrap().as_secs_f64()));
    assert_eq!(
        resumed.started_unix().unwrap(),
        Some(Duration::from_secs(1))
    );
    assert_eq!(resumed.settings().timeout, Some(Duration::from_secs(2)));
    resumed.begin(1).unwrap();
    let completed = resumed
        .finish_timed(
            row(1, Status::Ok),
            Some(crate::job::StageTimes {
                lattice: 0.1,
                formula: 0.2,
                search: 0.3,
            }),
        )
        .unwrap();
    assert!((2.125..3.125).contains(&completed.seconds));
    resumed.save().unwrap();
    let again = Session::open(None, Some(&checkpoint), None, None).unwrap();
    assert!(again.state().unwrap().attempt_elapsed.is_empty());
    assert_eq!(again.completed_rows().unwrap()[0].1.unwrap().search, 0.3);
    assert!((40.0..41.0).contains(&again.elapsed().unwrap().as_secs_f64()));
}

#[test]
fn current_schema_rejects_bad_provenance_durations_and_duplicate_indices() {
    let (_dir, input, checkpoint) = fixture();
    let session = Session::open(
        Some(&input),
        None,
        Some(&checkpoint),
        Some(Settings::default()),
    )
    .unwrap();
    session.begin(1).unwrap();
    session.finish(row(1, Status::Ok)).unwrap();
    let saved = saved_value(&session, &checkpoint);
    for (key, value) in [
        ("result_settings", serde_json::json!({})),
        ("result_settings", serde_json::json!({"1":9})),
        (
            "attempt_elapsed",
            serde_json::json!({"1":{"secs":1,"nanos":0}}),
        ),
        (
            "timings",
            serde_json::json!({"1":{"lattice":1e308,"formula":0,"search":0}}),
        ),
        (
            "timings",
            serde_json::json!({"1":{"lattice":-1,"formula":0,"search":0}}),
        ),
        ("elapsed", serde_json::json!({"secs":-1,"nanos":0})),
    ] {
        let mut bad = saved.clone();
        bad[key] = value;
        write_value(&checkpoint, &bad);
        assert!(
            Session::open(None, Some(&checkpoint), None, None).is_err(),
            "{key}"
        );
    }
    let text = serde_json::to_string(&saved).unwrap().replace(
        "\"result_settings\":{\"1\":0}",
        "\"result_settings\":{\"1\":0,\"1\":0}",
    );
    assert_ne!(text, serde_json::to_string(&saved).unwrap());
    fs::write(&checkpoint, text).unwrap();
    assert!(Session::open(None, Some(&checkpoint), None, None).is_err());
    fs::write(&checkpoint, " {\"version\":3,\"broken\":true}").unwrap();
    assert!(is_checkpoint(&checkpoint).unwrap());
    assert!(!is_checkpoint(&input).unwrap());
    assert!(is_checkpoint(input.parent().unwrap()).is_err());
}

#[test]
fn eta_retires_saved_work_but_relearns_unrecorded_topology_and_quiet_skips_descriptors() {
    let (_dir, input, checkpoint) = fixture();
    fs::write(&input, "[[0,0],[1,0],[0,1]]\n".repeat(4)).unwrap();
    let session = Session::open(
        Some(&input),
        None,
        Some(&checkpoint),
        Some(Settings::default()),
    )
    .unwrap();
    for index in 1..=2 {
        session.begin(index).unwrap();
        // Large accumulated row time must not become a calibration sample.
        let mut completed = row(index, Status::Ok);
        completed.seconds = 1000.;
        session
            .finish_timed(
                completed,
                Some(crate::job::StageTimes {
                    lattice: 1.,
                    formula: 1.,
                    search: 1.,
                }),
            )
            .unwrap();
    }
    session.save().unwrap();
    let same = Session::open(None, Some(&checkpoint), None, None).unwrap();
    assert_eq!(
        same.estimator().unwrap().unwrap().estimate([], 1),
        crate::eta::Estimate::Learning
    );
    let changed = Session::open_positional(&checkpoint, None, |saved, runtime| {
        Ok((
            Settings {
                backend: Backend::Cadical,
                ..saved.unwrap()
            },
            runtime,
        ))
    })
    .unwrap();
    assert_eq!(
        changed.estimator().unwrap().unwrap().estimate([], 1),
        crate::eta::Estimate::Learning
    );
    let parallel = Session::open_positional(&checkpoint, None, |saved, mut runtime| {
        runtime.parallel = NonZeroUsize::new(2);
        Ok((saved.unwrap(), runtime))
    })
    .unwrap();
    assert_eq!(
        parallel.estimator().unwrap().unwrap().estimate([], 2),
        crate::eta::Estimate::Learning
    );
    let quiet = Session::open_positional(&checkpoint, None, |saved, mut runtime| {
        runtime.terminal_output.running = false;
        Ok((saved.unwrap(), runtime))
    })
    .unwrap();
    assert!(quiet.eta_ids.is_none());
    assert!(quiet.estimator().unwrap().is_none());
}

#[test]
fn snapshots_without_flag_sat_field_keep_the_original_semantics() {
    let (_dir, input, checkpoint) = fixture();
    let session = Session::open(
        Some(&input),
        None,
        Some(&checkpoint),
        Some(Settings::default()),
    )
    .unwrap();
    session.save().unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&checkpoint).unwrap()).unwrap();
    value["settings"]
        .as_object_mut()
        .unwrap()
        .remove("flag_sat");
    for settings in value["settings_history"].as_array_mut().unwrap() {
        settings.as_object_mut().unwrap().remove("flag_sat");
    }
    fs::write(&checkpoint, serde_json::to_vec(&value).unwrap()).unwrap();
    let restored = Session::open(None, Some(&checkpoint), None, None).unwrap();
    assert_eq!(restored.settings(), Settings::default());
}

#[test]
fn snapshots_without_regular_learning_field_disable_learning() {
    let (_dir, input, checkpoint) = fixture();
    let session = Session::open(
        Some(&input),
        None,
        Some(&checkpoint),
        Some(Settings::default()),
    )
    .unwrap();
    session.save().unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&checkpoint).unwrap()).unwrap();
    value["settings"]
        .as_object_mut()
        .unwrap()
        .remove("regular_learn");
    for settings in value["settings_history"].as_array_mut().unwrap() {
        settings.as_object_mut().unwrap().remove("regular_learn");
    }
    fs::write(&checkpoint, serde_json::to_vec(&value).unwrap()).unwrap();
    let restored = Session::open(None, Some(&checkpoint), None, None).unwrap();
    assert_eq!(restored.settings(), Settings::default());
}

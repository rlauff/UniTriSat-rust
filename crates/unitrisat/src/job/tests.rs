use super::*;
use unitrisat_core::families;

fn settings() -> Settings {
    Settings {
        intersection_backend: IntersectionBackend::Cpu,
        restriction: Restriction::Unimodular,
        mode: Mode::Serial,
        backend: Backend::PicoSat,
        goal: Goal::All,
        regular: false,
        regular_learn: false,
        flag: false,
        flag_sat: false,
        max_solutions: None,
        max_regularity_tests: None,
        timeout: None,
    }
}

/// The row without the seconds.
fn counts_of(row: &Row) -> (Status, u64, u64, u64, u64) {
    (
        row.status,
        row.triangulations,
        row.regular,
        row.flag,
        row.quadratic,
    )
}

fn row_text(row: &Row) -> String {
    let mut out = Vec::new();
    row.write(&mut out).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn rows_are_written_as_chunk_run_writes_them() {
    // `TSV_HEADER` and `write_row` (`@sprintf("%.4f", seconds)`) of
    // scripts/chunk_run.jl.
    assert_eq!(
        TSV_HEADER,
        "# idx\tstatus\tn_triangulations\tn_regular\tn_flag\tn_quadratic\tseconds"
    );
    let row = |status, seconds| Row {
        index: 17,
        status,
        triangulations: 74,
        regular: 72,
        flag: 64,
        quadratic: 0,
        seconds,
    };
    assert_eq!(
        row_text(&row(Status::Ok, 13.443)),
        "17\tok\t74\t72\t64\t0\t13.4430\n"
    );
    assert_eq!(
        row_text(&row(Status::Capped, 0.000_12)),
        "17\tcapped\t74\t72\t64\t0\t0.0001\n"
    );
    assert_eq!(
        row_text(&row(Status::RegLimit, 0.0)),
        "17\treglimit\t74\t72\t64\t0\t0.0000\n"
    );
}

#[test]
fn status_follows_the_outcome() {
    for limited in [false, true] {
        let exact = if limited {
            Status::RegLimit
        } else {
            Status::Ok
        };
        assert_eq!(status(Outcome::Complete, limited), exact);
        assert_eq!(status(Outcome::Found, limited), exact);
        // Lower bounds are never `ok`.
        assert_eq!(status(Outcome::Capped, limited), Status::Capped);
        assert_eq!(status(Outcome::TimedOut, limited), Status::Capped);
    }
}

/// Julia's counts (`triangulate`, find_all, -t 1, serial and split, 2026-10-16;
/// scratch script in the P6.1 claim log): name, vertices, restriction,
/// (regular, flag) and (triangulations, regular, flag, quadratic).
/// A Julia reference case.
type Case = (&'static str, Matrix, Restriction, (bool, bool), [u64; 4]);

fn julia_cases() -> Vec<Case> {
    use Restriction::{Nondegenerate, Unimodular};
    let cube3 = families::hypercube(3).unwrap();
    vec![
        (
            "cube 3",
            cube3.clone(),
            Unimodular,
            (true, true),
            [72, 64, 64, 64],
        ),
        (
            "cube 3",
            cube3.clone(),
            Unimodular,
            (true, false),
            [72, 72, 0, 0],
        ),
        (
            "cube 3",
            cube3.clone(),
            Unimodular,
            (false, true),
            [72, 0, 64, 0],
        ),
        (
            "cube 3",
            cube3.clone(),
            Unimodular,
            (false, false),
            [72, 0, 0, 0],
        ),
        (
            "cube 3",
            cube3.clone(),
            Nondegenerate,
            (false, false),
            [74, 0, 0, 0],
        ),
        (
            "cube 3",
            cube3,
            Nondegenerate,
            (true, false),
            [74, 74, 0, 0],
        ),
        (
            "cross 3",
            families::cross_polytope(3).unwrap(),
            Unimodular,
            (true, true),
            [1, 1, 1, 1],
        ),
        (
            "cross 4",
            families::cross_polytope(4).unwrap(),
            Unimodular,
            (true, true),
            [1, 1, 1, 1],
        ),
        (
            "cyclic 3 4",
            families::cyclic(3, 4).unwrap(),
            Unimodular,
            (true, true),
            [17, 2, 1, 1],
        ),
        (
            "simplex 3 2",
            families::scaled_simplex(3, 2).unwrap(),
            Unimodular,
            (true, true),
            [192, 180, 180, 180],
        ),
    ]
}

#[test]
fn families_match_julia_in_every_mode() {
    for (name, vertices, restriction, (regular, flag), julia) in julia_cases() {
        for mode in [Mode::Serial, Mode::Split, Mode::Incremental] {
            for backend in [Backend::PicoSat, Backend::Cadical] {
                let settings = Settings {
                    restriction,
                    mode,
                    backend,
                    regular,
                    flag,
                    ..settings()
                };
                let row = run(1, &vertices, &settings).unwrap();
                let what = format!("{name} {settings:?}");
                let [n, r, f, q] = julia;
                assert_eq!(row.index, 1);
                assert_eq!(row.status, Status::Ok, "{what}");
                assert_eq!(
                    (row.triangulations, row.flag, row.quadratic),
                    (n, f, q),
                    "{what}"
                );
                if regular && flag {
                    // With both filters the regular count is a lower bound
                    // that depends on the enumeration order (P5.2), but it
                    // still decides "has a regular triangulation".
                    assert!((q..=n).contains(&row.regular), "{what}");
                    assert_eq!(row.regular > 0, r > 0, "{what}");
                } else {
                    assert_eq!(row.regular, r, "{what}");
                }
            }
        }
    }
}

#[test]
fn first_stops_at_a_target() {
    let cube = families::hypercube(3).unwrap();
    for (regular, flag) in [(false, false), (true, false), (false, true), (true, true)] {
        let settings = Settings {
            goal: Goal::First,
            regular,
            flag,
            ..settings()
        };
        let row = run(1, &cube, &settings).unwrap();
        // Every triangulation of the 3-cube is regular, and the first one
        // that is flag is quadratic.
        let target = match (regular, flag) {
            (false, false) | (true, false) => row.triangulations,
            (false, true) => row.flag,
            (true, true) => row.quadratic,
        };
        assert_eq!(row.status, Status::Ok);
        assert_eq!(target, 1, "{settings:?}");
    }
}

#[test]
fn caps_and_timeouts_are_capped() {
    let cube = families::hypercube(3).unwrap();
    // Julia (serial, UNITRISAT_MAX_SOLUTIONS = 5): 5 triangulations, and
    // chunk_run.jl writes "capped".
    for mode in [Mode::Serial, Mode::Split, Mode::Incremental] {
        let capped = Settings {
            mode,
            max_solutions: NonZeroU64::new(5),
            ..settings()
        };
        let row = run(1, &cube, &capped).unwrap();
        assert_eq!(counts_of(&row), (Status::Capped, 5, 0, 0, 0), "{mode:?}");
    }
    // A cap above the count is not reached.
    let above = Settings {
        max_solutions: NonZeroU64::new(73),
        ..settings()
    };
    let row = run(1, &cube, &above).unwrap();
    assert_eq!(counts_of(&row), (Status::Ok, 72, 0, 0, 0));
    // A deadline that has passed before the search starts: a lower bound.
    let timed_out = Settings {
        timeout: Some(Duration::from_nanos(1)),
        ..settings()
    };
    let row = run(1, &cube, &timed_out).unwrap();
    assert_eq!(counts_of(&row), (Status::Capped, 0, 0, 0, 0));
}

#[test]
fn a_polytope_without_simplices_has_no_triangulation() {
    // The Reeve tetrahedron with height 2: four lattice points, normalized
    // volume 2, no unimodular simplex.
    let reeve = Matrix::new(3, vec![0, 0, 0, 1, 0, 0, 0, 1, 0, 1, 1, 2]).unwrap();
    for mode in [Mode::Serial, Mode::Split, Mode::Incremental] {
        let settings = Settings {
            mode,
            regular: true,
            flag: true,
            ..settings()
        };
        let row = run(3, &reeve, &settings).unwrap();
        assert_eq!(row.index, 3);
        assert_eq!(counts_of(&row), (Status::Ok, 0, 0, 0, 0));
    }
    // With every full-dimensional simplex it is its own triangulation.
    let settings = Settings {
        restriction: Restriction::Nondegenerate,
        regular: true,
        ..settings()
    };
    let row = run(1, &reeve, &settings).unwrap();
    assert_eq!(counts_of(&row), (Status::Ok, 1, 1, 0, 0));
}

#[test]
fn lower_dimensional_input_is_projected() {
    // The unit square in the plane z = 5 of Z^3: two triangulations.
    let square = Matrix::new(3, vec![0, 0, 5, 1, 0, 5, 0, 1, 5, 1, 1, 5]).unwrap();
    let row = run(1, &square, &settings()).unwrap();
    assert_eq!(counts_of(&row), (Status::Ok, 2, 0, 0, 0));
}

#[test]
fn bad_polytopes_are_errors() {
    let point = Matrix::new(2, vec![1, 1, 1, 1]).unwrap();
    assert!(matches!(
        run(1, &point, &settings()),
        Err(JobError::Projection(ProjectionError::Point))
    ));
}

#[test]
fn panic_messages_are_kept() {
    let message = |f: fn()| {
        let payload = panic::catch_unwind(f).unwrap_err();
        panic_message(payload.as_ref())
    };
    assert_eq!(message(|| panic!("static text")), "static text");
    assert_eq!(message(|| panic!("formatted {}", 7)), "formatted 7");
    assert_eq!(message(|| std::panic::panic_any(7_u8)), "no message");
}

#[test]
fn gpu_selection_matches_cpu_in_serial_split_and_incremental_modes() {
    let vertices = unitrisat_core::families::hypercube(3).unwrap();
    for mode in [Mode::Serial, Mode::Split, Mode::Incremental] {
        for backend in [Backend::PicoSat, Backend::Cadical] {
            let settings = Settings {
                mode,
                backend,
                goal: Goal::All,
                regular: true,
                flag: false,
                ..settings()
            };
            let cpu = run(1, &vertices, &settings).unwrap();
            let gpu = run(
                1,
                &vertices,
                &Settings {
                    intersection_backend: IntersectionBackend::Gpu,
                    ..settings
                },
            )
            .unwrap();
            assert_eq!(gpu.status, cpu.status, "{mode:?} {backend:?}");
            assert_eq!(
                (gpu.triangulations, gpu.regular, gpu.flag, gpu.quadratic),
                (cpu.triangulations, cpu.regular, cpu.flag, cpu.quadratic),
                "{mode:?} {backend:?}"
            );
        }
    }
}

#[test]
fn detailed_retention_preserves_count_pipeline_in_every_mode() {
    let square = families::hypercube(2).unwrap();
    for mode in [Mode::Serial, Mode::Split, Mode::Incremental] {
        for backend in [Backend::PicoSat, Backend::Cadical] {
            for (regular, flag) in [(false, false), (true, false), (false, true), (true, true)] {
                let options = Settings {
                    mode,
                    backend,
                    regular,
                    flag,
                    ..settings()
                };
                let ordinary = run(7, &square, &options).unwrap();
                for keep in [Keep::None, Keep::First, Keep::All] {
                    let detailed = run_detailed(7, &square, &options, keep).unwrap();
                    assert_eq!(counts_of(&ordinary), counts_of(&detailed.row));
                    assert_eq!(detailed.outcome, Outcome::Complete);
                    let expected = match keep {
                        Keep::None => 0,
                        Keep::First => 1,
                        Keep::All => 2,
                    };
                    assert_eq!(detailed.triangulations.len(), expected);
                    assert!(detailed.triangulations.iter().all(|t| t.len() == 2));
                    assert!(detailed.timings.lattice >= 0.0);
                    assert!(detailed.timings.formula >= 0.0);
                    assert!(detailed.timings.search >= 0.0);
                }
            }
        }
    }
}

#[test]
fn detailed_geometry_uses_projected_coordinates_and_reports_caps() {
    let embedded = Matrix::new(3, vec![0, 0, 4, 1, 0, 4, 0, 1, 4, 1, 1, 4]).unwrap();
    let options = Settings {
        max_solutions: NonZeroU64::new(1),
        ..settings()
    };
    let detailed = run_detailed(1, &embedded, &options, Keep::All).unwrap();
    assert_eq!(detailed.points.columns(), 2);
    assert_eq!(detailed.row.status, Status::Capped);
    assert_eq!(detailed.triangulations.len(), 1);
    assert_eq!(detailed.row.triangulations, 1);
}

#[test]
fn detailed_retains_only_target_triangulations_on_cube_three() {
    let vertices = families::hypercube(3).unwrap();
    let options = Settings {
        regular: true,
        flag: true,
        ..settings()
    };
    let detailed = run_detailed(1, &vertices, &options, Keep::All).unwrap();
    assert_eq!(counts_of(&detailed.row), (Status::Ok, 72, 64, 64, 64));
    assert_eq!(detailed.triangulations.len(), 64);
    let flag = Flag::new(&detailed.simplices, 4, detailed.points.rows()).unwrap();
    for triangulation in &detailed.triangulations {
        assert_eq!(triangulation.len(), 6);
        assert!(flag.is_target(triangulation).unwrap());
    }
}

#[cfg(feature = "cuda")]
#[test]
#[ignore = "requires CUDA and NVRTC; GPU availability is asserted before detailed incremental parity"]
fn detailed_gpu_incremental_matches_retained_cpu_targets() {
    let vertices = families::hypercube(3).unwrap();
    for backend in [Backend::PicoSat, Backend::Cadical] {
        let options = Settings {
            mode: Mode::Incremental,
            backend,
            regular: true,
            flag: true,
            ..settings()
        };
        let cpu = run_detailed(1, &vertices, &options, Keep::All).unwrap();
        let hardware = unitrisat_gpu::search(cpu.points.values(), 3, &cpu.simplices).unwrap();
        assert!(matches!(hardware.backend, unitrisat_gpu::Backend::Cuda(_)));
        let gpu = run_detailed(
            1,
            &vertices,
            &Settings {
                intersection_backend: IntersectionBackend::Gpu,
                ..options
            },
            Keep::All,
        )
        .unwrap();
        for row in [&cpu.row, &gpu.row] {
            assert_eq!(
                (row.status, row.triangulations, row.flag, row.quadratic),
                (Status::Ok, 72, 64, 64)
            );
            assert!((64..=72).contains(&row.regular));
        }
        assert_eq!(gpu.points, cpu.points);
        assert_eq!(gpu.simplices, cpu.simplices);
        assert_eq!(gpu.triangulations, cpu.triangulations);
        let mut last = None;
        let mut counters = Vec::new();
        let observed = run_with_progress(
            1,
            &vertices,
            &Settings {
                intersection_backend: IntersectionBackend::Gpu,
                ..options
            },
            &mut |event| match event {
                Progress::Classification { .. } => last = Some(event),
                Progress::Incremental {
                    clauses_added,
                    interruptions,
                } => counters.push((clauses_added, interruptions)),
                _ => {}
            },
        )
        .unwrap();
        assert_eq!(
            (
                observed.status,
                observed.triangulations,
                observed.flag,
                observed.quadratic
            ),
            (Status::Ok, 72, 64, 64)
        );
        assert!((64..=72).contains(&observed.regular));
        let Some(Progress::Classification {
            triangulations,
            regular,
            non_regular,
            flag,
            non_flag,
            quadratic,
        }) = last
        else {
            panic!("missing GPU classification counts");
        };
        assert_eq!(
            (
                triangulations,
                regular,
                non_regular,
                flag,
                non_flag,
                quadratic
            ),
            (72, observed.regular, 0, 64, 8, 64)
        );
        assert_eq!(counters.first(), Some(&(0, 0)));
        // The reduced solver may finish before CUDA produces its first batch;
        // zero inserted clauses is then a valid final snapshot.
        assert!(counters.len() >= 2);
        assert!(
            counters
                .windows(2)
                .all(|pair| pair[0].0 <= pair[1].0 && pair[0].1 <= pair[1].1)
        );
    }
}

#[test]
fn progress_preserves_results_and_reports_stage_order() {
    let square = families::hypercube(2).unwrap();
    for mode in [Mode::Serial, Mode::Split, Mode::Incremental] {
        for backend in [Backend::PicoSat, Backend::Cadical] {
            for (regular, flag) in [(false, false), (true, false), (false, true), (true, true)] {
                let options = Settings {
                    mode,
                    backend,
                    regular,
                    flag,
                    ..settings()
                };
                let ordinary = run(7, &square, &options).unwrap();
                let mut events = Vec::new();
                let observed = run_with_progress(7, &square, &options, &mut |event| {
                    events.push(event);
                })
                .unwrap();
                assert_eq!(observed.index, ordinary.index);
                assert_eq!(counts_of(&observed), counts_of(&ordinary));
                let counters: Vec<_> = events
                    .iter()
                    .filter_map(|event| match event {
                        Progress::Incremental {
                            clauses_added,
                            interruptions,
                        } => Some((*clauses_added, *interruptions)),
                        _ => None,
                    })
                    .collect();
                if mode == Mode::Incremental {
                    assert_eq!(counters.first(), Some(&(0, 0)));
                    assert!(
                        counters
                            .windows(2)
                            .all(|pair| pair[0].0 <= pair[1].0 && pair[0].1 <= pair[1].1)
                    );
                } else {
                    assert!(counters.is_empty());
                }
                events.retain(|event| {
                    !matches!(
                        event,
                        Progress::Incremental { .. } | Progress::Classification { .. }
                    )
                });
                let [
                    Progress::Points { count: 4 },
                    Progress::Simplices { count: 4 },
                    Progress::Solving,
                    Progress::Finished { timings },
                ] = events.as_slice()
                else {
                    panic!("unexpected progress events: {events:?}");
                };
                for seconds in [timings.lattice, timings.formula, timings.search] {
                    assert!(seconds.is_finite() && seconds >= 0.0);
                }
                assert!(timings.lattice + timings.formula + timings.search <= observed.seconds);
            }
        }
    }
}

#[test]
fn progress_empty_simplex_search_finishes_without_solving() {
    // This empty Reeve tetrahedron has normalized volume two, so its four
    // lattice points cannot form any unimodular simplex.
    let vertices = Matrix::new(3, vec![0, 0, 0, 1, 0, 0, 0, 1, 0, 1, 1, 2]).unwrap();
    let mut events = Vec::new();
    let row = run_with_progress(1, &vertices, &settings(), &mut |event| {
        events.push(event);
    })
    .unwrap();
    assert_eq!(counts_of(&row), (Status::Ok, 0, 0, 0, 0));
    let [
        Progress::Points { count: 4 },
        Progress::Simplices { count: 0 },
        Progress::Finished { timings },
    ] = events.as_slice()
    else {
        panic!("unexpected progress events: {events:?}");
    };
    assert_eq!(timings.formula, 0.0);
    assert_eq!(timings.search, 0.0);
}

#[test]
fn progress_observer_panics_become_job_errors() {
    let square = families::hypercube(2).unwrap();
    let result = run_with_progress(1, &square, &settings(), &mut |_| {
        panic!("observer failed");
    });
    assert!(matches!(result, Err(JobError::Panic(message)) if message == "observer failed"));
}

#[test]
fn progress_disabled_path_never_calls_observer() {
    let square = families::hypercube(2).unwrap();
    let row = run_counts::<false>(1, &square, &settings(), &mut |_| {
        panic!("disabled observer was called");
    })
    .unwrap();
    assert_eq!(counts_of(&row), (Status::Ok, 2, 0, 0, 0));
}

#[test]
fn progress_classification_cube_counts_are_exact_in_every_mode() {
    let cube = families::hypercube(3).unwrap();
    for mode in [Mode::Serial, Mode::Split, Mode::Incremental] {
        for backend in [Backend::PicoSat, Backend::Cadical] {
            for (regular, flag) in [(true, false), (false, true), (true, true)] {
                let options = Settings {
                    mode,
                    backend,
                    regular,
                    flag,
                    ..settings()
                };
                let mut last = None;
                let caller = std::thread::current().id();
                let row = run_with_progress(1, &cube, &options, &mut |event| {
                    assert_eq!(std::thread::current().id(), caller);
                    if let Progress::Classification { .. } = event {
                        last = Some(event);
                    }
                })
                .unwrap();
                let Some(Progress::Classification {
                    triangulations,
                    regular: positives,
                    non_regular,
                    flag: flags,
                    non_flag,
                    quadratic,
                }) = last
                else {
                    panic!("missing final classification snapshot");
                };
                assert_eq!(triangulations, 72);
                assert_eq!(
                    (positives, flags, quadratic),
                    (row.regular, row.flag, row.quadratic)
                );
                assert_eq!(non_regular, 0);
                assert_eq!(non_flag, if flag { 8 } else { 0 });
                if regular && !flag {
                    assert_eq!(positives, 72);
                }
                if flag {
                    assert_eq!(flags, 64);
                }
                if regular && flag {
                    // Non-flag models can skip their regularity tests; they
                    // must not be misreported as non-regular.
                    assert!((64..=72).contains(&positives));
                }
            }
        }
    }
}

#[test]
fn progress_skipped_regularity_tests_never_count_as_negative() {
    use super::classification_progress::{Counted, Tests};
    struct NeverRegular;
    impl Classify for NeverRegular {
        type Error = Infallible;
        fn is_target(&self, _: &[u32]) -> Result<bool, Infallible> {
            Ok(false)
        }
    }
    let cube = families::hypercube(3).unwrap();
    let lattice = Lattice::new::<false>(&cube, Restriction::Unimodular, &mut |_| {}).unwrap();
    for mode in [Mode::Serial, Mode::Split, Mode::Incremental] {
        let formula = Formula::new(&lattice, mode, IntersectionBackend::Cpu, false).unwrap();
        for backend in [Backend::PicoSat, Backend::Cadical] {
            let options = Options {
                backend,
                goal: Goal::All,
                keep: Keep::None,
                max_solutions: None,
                deadline: None,
            };
            let regular = Counted::<_, true>::new(NeverRegular);
            let flag = Counted::<_, true>::new(Flag::new(&lattice.simplices, 4, 8).unwrap());
            let quadratic = Quadratic::new(flag, &regular, NonZeroU64::new(1));
            let mut last = None;
            let found = formula
                .search::<true, _>(
                    &lattice,
                    &options,
                    &quadratic,
                    Tests {
                        regular: Some(&regular.tally),
                        flag: Some(&quadratic.flag_classifier().tally),
                    },
                    &mut |event| {
                        if let Progress::Classification { .. } = event {
                            last = Some(event);
                        }
                    },
                )
                .unwrap();
            let Some(Progress::Classification {
                triangulations,
                regular,
                non_regular,
                flag,
                non_flag,
                quadratic: targets,
            }) = last
            else {
                panic!("missing counts");
            };
            assert_eq!(
                (triangulations, regular, flag, non_flag, targets),
                (72, 0, 64, 8, 0)
            );
            assert_eq!(non_regular, quadratic.counts().tests);
            assert!((64..=65).contains(&non_regular));
            assert!(quadratic.counts().limited);
            assert_eq!(found.outcome, Outcome::Complete);
        }
    }
}

#[test]
fn progress_live_observer_panic_cancels_and_joins_every_mode() {
    use super::classification_progress::{Counted, Tests};
    use std::sync::atomic::{AtomicU64, Ordering};
    struct SlowFalse(AtomicU64);
    impl Classify for SlowFalse {
        type Error = Infallible;
        fn is_target(&self, _: &[u32]) -> Result<bool, Infallible> {
            assert_eq!(
                rayon::current_num_threads(),
                1,
                "caller parallelism must be retained"
            );
            std::thread::sleep(Duration::from_millis(5));
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(false)
        }
    }
    let cube = families::hypercube(3).unwrap();
    let lattice = Lattice::new::<false>(&cube, Restriction::Unimodular, &mut |_| {}).unwrap();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    pool.install(|| {
        for mode in [Mode::Serial, Mode::Split, Mode::Incremental] {
            let formula = Formula::new(&lattice, mode, IntersectionBackend::Cpu, false).unwrap();
            for backend in [Backend::PicoSat, Backend::Cadical] {
                let options = Options {
                    backend,
                    goal: Goal::All,
                    keep: Keep::None,
                    max_solutions: None,
                    deadline: None,
                };
                let classifier = SlowFalse(AtomicU64::new(0));
                let counted = Counted::<_, true>::new(classifier);
                let caller = std::thread::current().id();
                let start = Instant::now();
                let mut live = false;
                let result = panic::catch_unwind(AssertUnwindSafe(|| {
                    formula.search::<true, _>(
                        &lattice,
                        &options,
                        &counted,
                        Tests {
                            regular: Some(&counted.tally),
                            flag: None,
                        },
                        &mut |event| {
                            assert_eq!(std::thread::current().id(), caller);
                            if let Progress::Classification { non_regular, .. } = event {
                                assert!(
                                    (1..72).contains(&non_regular),
                                    "snapshot must arrive before completion"
                                );
                                live = true;
                                panic!("observer stopped live search");
                            }
                        },
                    )
                }));
                assert!(live);
                assert_eq!(
                    panic_message(result.unwrap_err().as_ref()),
                    "observer stopped live search"
                );
                assert!(start.elapsed() < Duration::from_secs(3));
                // Returning from the scoped search means no worker can still
                // refer to this classifier, including Rayon split workers.
            }
        }
    });
}

#[test]
fn progress_capped_snapshot_matches_partial_row() {
    let cube = families::hypercube(3).unwrap();
    for mode in [Mode::Serial, Mode::Split, Mode::Incremental] {
        for backend in [Backend::PicoSat, Backend::Cadical] {
            let options = Settings {
                mode,
                backend,
                regular: true,
                flag: true,
                max_solutions: NonZeroU64::new(3),
                ..settings()
            };
            let mut last = None;
            let row = run_with_progress(1, &cube, &options, &mut |event| {
                if let Progress::Classification { .. } = event {
                    last = Some(event);
                }
            })
            .unwrap();
            assert_eq!(row.status, Status::Capped);
            let Some(Progress::Classification {
                triangulations,
                regular,
                non_regular,
                flag,
                non_flag,
                quadratic,
            }) = last
            else {
                panic!("missing capped counts");
            };
            assert_eq!(triangulations, 3);
            assert_eq!(
                (regular, flag, quadratic),
                (row.regular, row.flag, row.quadratic)
            );
            assert_eq!(non_regular, 0);
            assert_eq!(flag + non_flag, 3);
        }
    }
}

#[test]
fn progress_failed_or_panicking_classifier_joins_without_false_rejections() {
    use super::classification_progress::{Counted, Tests};
    struct Failure(bool);
    impl Classify for Failure {
        type Error = io::Error;
        fn is_target(&self, _: &[u32]) -> Result<bool, io::Error> {
            if self.0 {
                panic!("classifier panic");
            }
            Err(io::Error::other("classifier error"))
        }
    }
    let square = families::hypercube(2).unwrap();
    let lattice = Lattice::new::<false>(&square, Restriction::Unimodular, &mut |_| {}).unwrap();
    for mode in [Mode::Serial, Mode::Split, Mode::Incremental] {
        let formula = Formula::new(&lattice, mode, IntersectionBackend::Cpu, false).unwrap();
        for backend in [Backend::PicoSat, Backend::Cadical] {
            for panics in [false, true] {
                let options = Options {
                    backend,
                    goal: Goal::All,
                    keep: Keep::None,
                    max_solutions: None,
                    deadline: None,
                };
                let classify = Counted::<_, true>::new(Failure(panics));
                let mut seen = false;
                let result = panic::catch_unwind(AssertUnwindSafe(|| {
                    formula.search::<true, _>(
                        &lattice,
                        &options,
                        &classify,
                        Tests {
                            regular: Some(&classify.tally),
                            flag: None,
                        },
                        &mut |event| {
                            if let Progress::Classification {
                                regular,
                                non_regular,
                                ..
                            } = event
                            {
                                assert_eq!((regular, non_regular), (0, 0));
                                seen = true;
                            }
                        },
                    )
                }));
                assert!(
                    seen,
                    "the completed failed worker still emits its final snapshot"
                );
                if panics {
                    assert_eq!(
                        panic_message(result.unwrap_err().as_ref()),
                        "classifier panic"
                    );
                } else {
                    assert!(matches!(result.unwrap(), Err(EnumerateError::Check(_))));
                }
            }
        }
    }
}

#[test]
fn kissat_settings_reject_incompatible_searches_before_geometry() {
    let base = Settings {
        backend: Backend::Kissat,
        ..Settings::default()
    };
    // This point cannot produce a hull; configuration must fail first.
    let vertices = Matrix::new(2, vec![0, 0]).unwrap();
    for settings in [
        Settings {
            mode: Mode::Incremental,
            ..base
        },
        Settings {
            goal: Goal::All,
            ..base
        },
        Settings {
            regular: true,
            ..base
        },
        Settings { flag: true, ..base },
        Settings {
            regular: true,
            flag: true,
            ..base
        },
    ] {
        let error = settings.validate().unwrap_err();
        assert!(error.to_string().contains("Kissat"));
        assert!(matches!(
            run(1, &vertices, &settings),
            Err(JobError::Settings(_))
        ));
        assert!(matches!(
            run_detailed(1, &vertices, &settings, Keep::All),
            Err(JobError::Settings(_))
        ));
        let mut events = 0;
        assert!(matches!(
            run_with_progress(1, &vertices, &settings, &mut |_| events += 1),
            Err(JobError::Settings(_))
        ));
        assert_eq!(events, 0);
    }
}

#[test]
#[cfg(not(windows))]
fn kissat_first_search_settings_allow_split_devices_and_limits() {
    for mode in [Mode::Serial, Mode::Split] {
        for intersection_backend in [IntersectionBackend::Cpu, IntersectionBackend::Gpu] {
            Settings {
                backend: Backend::Kissat,
                mode,
                intersection_backend,
                max_solutions: NonZeroU64::new(1),
                timeout: Some(Duration::from_secs(1)),
                ..Settings::default()
            }
            .validate()
            .unwrap();
        }
    }
    assert_eq!(Settings::default().backend, Backend::PicoSat);
}

#[test]
#[cfg(not(windows))]
fn kissat_first_search_runs_serial_split_detailed_and_progress() {
    let vertices = Matrix::new(2, vec![0, 0, 1, 0, 0, 1, 1, 1]).unwrap();
    for mode in [Mode::Serial, Mode::Split] {
        let settings = Settings {
            backend: Backend::Kissat,
            mode,
            ..Settings::default()
        };
        assert!(run(1, &vertices, &settings).unwrap().triangulations >= 1);
        let detailed = run_detailed(1, &vertices, &settings, Keep::All).unwrap();
        assert!(detailed.row.triangulations >= 1);
        assert_eq!(
            detailed.triangulations.len() as u64,
            detailed.row.triangulations
        );
        let mut finished = false;
        let row = run_with_progress(1, &vertices, &settings, &mut |event| {
            finished |= matches!(event, Progress::Finished { .. });
        })
        .unwrap();
        assert!(row.triangulations >= 1);
        assert!(finished);
    }
}

#[test]
#[cfg(windows)]
fn kissat_unavailable_platform_fails_before_geometry() {
    let settings = Settings {
        backend: Backend::Kissat,
        ..Settings::default()
    };
    assert_eq!(settings.validate(), Err(SettingsError::KissatUnavailable));
    let vertices = Matrix::new(2, vec![0, 0]).unwrap();
    assert!(matches!(
        run(1, &vertices, &settings),
        Err(JobError::Settings(SettingsError::KissatUnavailable))
    ));
}

#[test]
#[cfg(not(windows))]
fn kissat_expired_job_timeout_returns_capped_without_a_solution() {
    let vertices = Matrix::new(2, vec![0, 0, 1, 0, 0, 1]).unwrap();
    for mode in [Mode::Serial, Mode::Split] {
        let settings = Settings {
            backend: Backend::Kissat,
            mode,
            timeout: Some(Duration::ZERO),
            ..Settings::default()
        };
        let row = run(1, &vertices, &settings).unwrap();
        assert_eq!(row.status, Status::Capped);
        assert_eq!(row.triangulations, 0);
    }
}

#[test]
fn flag_sat_pipeline_preserves_geometry_counts_and_classifier_independence() {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap();
    pool.install(|| {
        let vertices = families::hypercube(3).unwrap();
        let expected = run_detailed(
            1,
            &vertices,
            &Settings {
                flag: true,
                ..settings()
            },
            Keep::All,
        )
        .unwrap();
        assert!(expected.row.flag < expected.row.triangulations);
        for mode in [Mode::Serial, Mode::Split, Mode::Incremental] {
            for backend in [Backend::PicoSat, Backend::Cadical] {
                for intersection_backend in [IntersectionBackend::Cpu, IntersectionBackend::Gpu] {
                    for (regular, flag) in
                        [(false, false), (false, true), (true, false), (true, true)]
                    {
                        let options = Settings {
                            mode,
                            backend,
                            intersection_backend,
                            regular,
                            flag,
                            flag_sat: true,
                            ..settings()
                        };
                        let actual = run_detailed(1, &vertices, &options, Keep::All).unwrap();
                        assert_eq!(
                            actual.triangulations, expected.triangulations,
                            "{options:?}"
                        );
                        assert_eq!(actual.row.triangulations, expected.row.flag);
                        assert_eq!(actual.row.flag, expected.row.flag);
                        assert_eq!(
                            actual.row.regular,
                            if regular { expected.row.flag } else { 0 }
                        );
                        assert_eq!(actual.row.quadratic, actual.row.regular);
                        assert_eq!(actual.outcome, Outcome::Complete);
                    }
                }
            }
        }
        for flag in [false, true] {
            let options = Settings {
                flag_sat: true,
                flag,
                ..settings()
            };
            let mut last = None;
            let row = run_with_progress(1, &vertices, &options, &mut |event| {
                if let Progress::Classification { flag, non_flag, .. } = event {
                    last = Some((flag, non_flag));
                }
            })
            .unwrap();
            assert_eq!(row.flag, expected.row.flag);
            if flag {
                assert_eq!(
                    last,
                    Some((expected.row.flag, 0)),
                    "explicit classifier must run"
                );
            } else {
                assert!(
                    last.is_none(),
                    "SAT-only search has no flag tests to report"
                );
            }
        }
    });
}

#[test]
fn flag_sat_caps_and_timeout_count_only_restricted_models() {
    let vertices = families::hypercube(3).unwrap();
    for mode in [Mode::Serial, Mode::Split, Mode::Incremental] {
        let options = Settings {
            mode,
            flag_sat: true,
            max_solutions: NonZeroU64::new(1),
            ..settings()
        };
        let row = run(1, &vertices, &options).unwrap();
        assert_eq!(counts_of(&row), (Status::Capped, 1, 0, 1, 0));
        let row = run(
            1,
            &vertices,
            &Settings {
                timeout: Some(Duration::ZERO),
                ..options
            },
        )
        .unwrap();
        assert_eq!(counts_of(&row), (Status::Capped, 0, 0, 0, 0));
    }
    assert!(!Settings::default().flag_sat);
}

#[cfg(not(windows))]
#[test]
fn flag_sat_allows_kissat_without_enabling_the_checker() {
    let vertices = families::hypercube(3).unwrap();
    for mode in [Mode::Serial, Mode::Split] {
        let options = Settings {
            mode,
            backend: Backend::Kissat,
            flag_sat: true,
            ..Settings::default()
        };
        options.validate().unwrap();
        let found = run_detailed(1, &vertices, &options, Keep::All).unwrap();
        assert_eq!(found.outcome, Outcome::Found);
        assert_eq!(found.row.flag, found.row.triangulations);
        let checker = Flag::new(&found.simplices, 4, found.points.rows()).unwrap();
        assert!(
            found
                .triangulations
                .iter()
                .all(|t| checker.is_flag(t).unwrap())
        );
        assert!(
            Settings {
                flag: true,
                ..options
            }
            .validate()
            .is_err()
        );
    }
}

#[test]
fn regular_learn_requires_regularity_and_keeps_kissat_restriction() {
    assert!(!Settings::default().regular_learn);
    assert_eq!(
        Settings {
            regular_learn: true,
            ..Settings::default()
        }
        .validate(),
        Err(SettingsError::RegularLearnRequiresRegular)
    );
    assert!(matches!(
        Settings {
            regular_learn: true,
            regular: true,
            backend: Backend::Kissat,
            ..Settings::default()
        }
        .validate(),
        Err(SettingsError::KissatUnsupported(_))
    ));
}

#[test]
fn regular_learn_job_modes_devices_and_progress_preserve_targets() {
    let vertices = Matrix::new(3, vec![0, 0, 0, 2, 0, 0, 0, 2, 0, 2, 2, 4]).unwrap();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap();
    pool.install(|| {
        for (flag, flag_sat) in [(false, false), (true, false), (false, true), (true, true)] {
            let baseline = Settings {
                regular: true,
                flag,
                flag_sat,
                ..settings()
            };
            let expected = run_detailed(1, &vertices, &baseline, Keep::All).unwrap();
            for mode in [Mode::Serial, Mode::Split, Mode::Incremental] {
                for intersection_backend in [IntersectionBackend::Cpu, IntersectionBackend::Gpu] {
                    let options = Settings {
                        mode,
                        intersection_backend,
                        regular_learn: true,
                        ..baseline
                    };
                    let actual = run_detailed(1, &vertices, &options, Keep::All).unwrap();
                    assert_eq!(
                        actual.triangulations, expected.triangulations,
                        "{options:?}"
                    );
                    assert_eq!(actual.outcome, Outcome::Complete);
                    assert!(actual.row.triangulations <= expected.row.triangulations);
                    let mut last = None;
                    let observed = run_with_progress(1, &vertices, &options, &mut |event| {
                        if let Progress::Classification {
                            triangulations,
                            regular,
                            non_regular,
                            flag,
                            non_flag,
                            quadratic,
                        } = event
                        {
                            last = Some((
                                triangulations,
                                regular,
                                non_regular,
                                flag,
                                non_flag,
                                quadratic,
                            ));
                        }
                    })
                    .unwrap();
                    let (seen, regular, non_regular, checked_flag, non_flag, quadratic) =
                        last.unwrap();
                    assert_eq!(seen, observed.triangulations);
                    assert_eq!(regular, observed.regular);
                    if !flag {
                        assert_eq!(regular + non_regular, seen);
                    } else {
                        assert_eq!(checked_flag + non_flag, seen);
                        assert_eq!(checked_flag, observed.flag);
                        assert_eq!(quadratic, observed.quadratic);
                    }
                }
            }
        }
    });
}

#[test]
fn regular_learn_caps_timeouts_and_observer_cancellation() {
    let vertices = Matrix::new(3, vec![0, 0, 0, 2, 0, 0, 0, 2, 0, 2, 2, 4]).unwrap();
    for mode in [Mode::Serial, Mode::Split, Mode::Incremental] {
        let options = Settings {
            mode,
            regular: true,
            regular_learn: true,
            max_solutions: NonZeroU64::new(1),
            ..settings()
        };
        let capped = run(1, &vertices, &options).unwrap();
        assert_eq!(capped.status, Status::Capped);
        assert_eq!(capped.triangulations, 1);
        let timed = run(
            1,
            &vertices,
            &Settings {
                timeout: Some(Duration::ZERO),
                ..options
            },
        )
        .unwrap();
        assert_eq!(timed.status, Status::Capped);
        assert_eq!(timed.triangulations, 0);
        let options = Settings {
            max_solutions: None,
            ..options
        };
        let canceled = run_with_progress(1, &vertices, &options, &mut |event| {
            if matches!(event, Progress::Classification { .. }) {
                panic!("cancel learning search")
            }
        });
        assert!(matches!(canceled, Err(JobError::Panic(_))));
    }
}

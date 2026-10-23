use super::*;
use crate::enumerate::tests::{BACKENDS, all, polytopes};
use crate::enumerate::{AnyTriangulation, Goal, Outcome, serial};
use crate::solve::Backend;
use std::num::NonZeroU64;
use std::sync::atomic::AtomicBool;

/// Pauses whenever a clause waits, blocks workers at a few waiting clauses
/// and stops the pair search after `max_injected` clauses.
fn eager(max_injected: usize) -> Tuning {
    Tuning {
        pause_threshold: 1,
        buffer_cap: 4,
        max_injected,
        slice_initial: Duration::ZERO,
        slice_max: Duration::ZERO,
    }
}

/// Every triangulation is a target; the first check waits until the pair
/// search has long had time to offer its rows.
struct WaitFirst {
    waited: AtomicBool,
}

impl Classify for WaitFirst {
    type Error = std::convert::Infallible;

    fn is_target(&self, _: &[u32]) -> Result<bool, Self::Error> {
        if !self.waited.swap(true, Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(40));
        }
        Ok(true)
    }
}

#[test]
fn eager_pausing_and_the_clause_budget_keep_the_enumeration_exact() {
    for (name, p, _) in polytopes() {
        let reference = serial(&p.full, &all(Backend::PicoSat), &AnyTriangulation).unwrap();
        for backend in BACKENDS {
            for threads in [1, 4] {
                for budget in [usize::MAX, 7] {
                    let exchange = Exchange::new(eager(budget));
                    let hook = WaitFirst {
                        waited: AtomicBool::new(false),
                    };
                    let result = rayon::ThreadPoolBuilder::new()
                        .num_threads(threads)
                        .build()
                        .unwrap()
                        .install(|| {
                            run(
                                &p.reduced,
                                &p.points,
                                &p.labels,
                                &all(backend),
                                &hook,
                                &exchange,
                            )
                        })
                        .unwrap();
                    let context = format!("{name} {backend:?} {threads} budget {budget}");
                    assert_eq!(result, reference, "{context}");
                    let state = exchange.lock();
                    assert!(state.closed);
                    assert!(state.injected <= budget.min(p.full.len()), "{context}");
                    if reference.found > 1 {
                        // Two triangulations have overlapping simplices, and
                        // the rows offered during the first check were added.
                        assert!(state.injected > 0, "{context}");
                    }
                }
            }
        }
    }
}

#[test]
fn the_exchange_pauses_blocks_and_closes() {
    let exchange = Exchange::new(eager(6));
    // Not solving: a waiting row does not pause.
    assert!(exchange.offer(&[(0, 1)]));
    assert!(!exchange.pause.load(Ordering::SeqCst));
    // Solving with an elapsed slice: it does.
    exchange.resume().unwrap();
    assert!(exchange.offer(&[(0, 2)]));
    assert!(exchange.pause.load(Ordering::SeqCst));
    exchange.suspend(true);
    assert_eq!(exchange.lock().pauses, 1);
    assert_eq!(exchange.take(), [(0, 1), (0, 2)]);
    exchange.resume().unwrap();
    assert!(!exchange.pause.load(Ordering::SeqCst));
    exchange.suspend(false);
    // Four waiting pairs block the worker until they are taken.
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| exchange.offer(&[(1, 2), (1, 3), (1, 4), (1, 5)]));
        while exchange.lock().pairs.len() < 4 {
            std::thread::yield_now();
        }
        std::thread::sleep(Duration::from_millis(20));
        assert!(!worker.is_finished());
        // The budget of six allows four more pairs and then closes.
        assert_eq!(exchange.take().len(), 4);
        assert!(!worker.join().unwrap());
    });
    let state = exchange.lock();
    assert!(state.closed);
    assert_eq!(state.injected, 6);
    drop(state);
    assert!(!exchange.offer(&[(2, 3)]));
    assert!(exchange.take().is_empty());
}

/// The 3-cube of [`polytopes`] and its points scaled beyond the exact bound
/// of the pair search; the reduced formula itself does not need them.
fn cube_with_huge_points() -> (crate::enumerate::tests::Polytope, Matrix) {
    let (_, p, _) = polytopes().swap_remove(1);
    let huge = Matrix::new(3, p.points.values().iter().map(|&x| x << 40).collect()).unwrap();
    (p, huge)
}

fn is_coordinate_error<T: std::fmt::Debug>(result: &EnumResult<T, AnyTriangulation>) -> bool {
    matches!(
        result,
        Err(EnumerateError::Intersect(
            IntersectError::CoordinateTooLarge { .. }
        ))
    )
}

#[test]
fn a_failed_pair_search_is_reported() {
    let (p, huge) = cube_with_huge_points();
    for backend in BACKENDS {
        let result = incremental(
            &p.reduced,
            &huge,
            &p.labels,
            &all(backend),
            &AnyTriangulation,
        );
        assert!(is_coordinate_error(&result), "{backend:?}: {result:?}");
    }
}

/// P4.5: the pair search fails only after the solver has finished (the
/// interleaving that made the test above flaky under load). Every way the
/// search can end must still report the failure.
#[test]
fn a_pair_search_failing_after_the_solver_finished_is_reported() {
    let (p, huge) = cube_with_huge_points();
    let first = Options {
        goal: Goal::First,
        ..all(Backend::PicoSat)
    };
    let capped = Options {
        max_solutions: NonZeroU64::new(3),
        ..all(Backend::PicoSat)
    };
    let late = Options {
        deadline: Some(std::time::Instant::now()),
        ..all(Backend::PicoSat)
    };
    let endings = [
        (all(Backend::PicoSat), Outcome::Complete),
        (first, Outcome::Found),
        (capped, Outcome::Capped),
        (late, Outcome::TimedOut),
    ];
    for backend in BACKENDS {
        for (options, outcome) in endings {
            let options = Options { backend, ..options };
            let context = format!("{backend:?} {outcome:?}");
            // With valid points, each ending is reached as expected.
            let exchange = Exchange::new(TUNING);
            exchange.lock().hold = true;
            let ok = run(
                &p.reduced,
                &p.points,
                &p.labels,
                &options,
                &AnyTriangulation,
                &exchange,
            );
            assert_eq!(ok.unwrap().outcome, outcome, "{context}");
            // The held pair search starts once the exchange is closed, so
            // it fails after the solver loop has returned.
            let exchange = Exchange::new(TUNING);
            exchange.lock().hold = true;
            let result = run(
                &p.reduced,
                &huge,
                &p.labels,
                &options,
                &AnyTriangulation,
                &exchange,
            );
            assert!(is_coordinate_error(&result), "{context}: {result:?}");
            assert!(exchange.lock().closed, "{context}");
        }
    }
}

/// A failure recorded while the solver loop is between solver calls keeps
/// the solver paused and is returned when it would resume.
#[test]
fn resuming_after_a_failed_pair_search_is_refused() {
    let (p, huge) = cube_with_huge_points();
    let exchange = Exchange::new(eager(usize::MAX));
    assert!(matches!(
        exchange.generate(&huge, &p.labels, 1),
        Err(IntersectError::CoordinateTooLarge { .. })
    ));
    assert!(exchange.pause.load(Ordering::SeqCst));
    assert!(matches!(
        exchange.resume(),
        Err(IntersectError::CoordinateTooLarge { .. })
    ));
    assert!(exchange.pause.load(Ordering::SeqCst));
    assert!(!exchange.lock().solving);
}

/// Force a producer into its indefinite wait between take() and resume().
/// Acquiring the state after observing its row/done state proves the producer
/// released the lock in wait(), so no timing sleep establishes the ordering.
#[test]
fn resuming_wakes_producers_waiting_between_solver_calls() {
    let (p, _) = cube_with_huge_points();
    for final_batch in [false, true] {
        let mut tuning = eager(usize::MAX);
        tuning.slice_initial = Duration::from_millis(10);
        if final_batch {
            tuning.buffer_cap = usize::MAX;
            tuning.pause_threshold = usize::MAX;
        }
        let exchange = Exchange::new(tuning);
        assert!(exchange.take().is_empty());
        let paused = std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                if final_batch {
                    exchange.generate(&p.points, &p.labels, 1)
                } else {
                    exchange.offer(&[(0, 1), (0, 2), (0, 3), (0, 4)]);
                    Ok(())
                }
            });
            let deadline = Instant::now() + Duration::from_secs(5);
            let waiting = loop {
                let pending = exchange.lock();
                if !pending.pairs.is_empty()
                    && (!final_batch || matches!(pending.generation, Generation::Done))
                {
                    assert!(!pending.solving);
                    break true;
                }
                drop(pending);
                if Instant::now() >= deadline {
                    break false;
                }
                std::thread::yield_now();
            };
            exchange.resume().unwrap();
            while !exchange.pause.load(Ordering::SeqCst) && Instant::now() < deadline {
                std::thread::yield_now();
            }
            let paused = waiting && exchange.pause.load(Ordering::SeqCst);
            // Clean up even on failure, so a missing notification fails the
            // assertion instead of hanging the test runner.
            exchange.close();
            worker.join().unwrap().unwrap();
            paused
        });
        assert!(paused, "final batch: {final_batch}");
    }
}

#[test]
fn an_injected_pair_source_is_used_and_its_failure_is_joined() {
    let (p, _) = cube_with_huge_points();
    for backend in BACKENDS {
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let source =
            |points: &[i64], dimension, labels: &[u32], workers, sink: intersect::RowSink<'_>| {
                calls.fetch_add(1, Ordering::SeqCst);
                intersect::stream_pairs(points, dimension, labels, workers, sink)
            };
        let result = incremental_with_pairs(
            &p.reduced,
            &p.points,
            &p.labels,
            &all(backend),
            &AnyTriangulation,
            &source,
        )
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            result,
            serial(&p.full, &all(backend), &AnyTriangulation).unwrap()
        );
        let failure = |_: &[i64], _, _: &[u32], _, _: intersect::RowSink<'_>| {
            Err(IntersectError::ThreadSpawn(std::io::ErrorKind::WouldBlock))
        };
        assert!(matches!(
            incremental_with_pairs(
                &p.reduced,
                &p.points,
                &p.labels,
                &all(backend),
                &AnyTriangulation,
                &failure
            ),
            Err(EnumerateError::Intersect(IntersectError::ThreadSpawn(
                std::io::ErrorKind::WouldBlock
            )))
        ));
    }
}

#[test]
fn progress_counts_actual_interruptions_and_inserted_clauses() {
    // An unsatisfiable pigeonhole formula requires enough work that both native
    // solvers poll their stop control. Tiny triangulation formulas may return SAT
    // before polling an already requested stop, which must not count as a pause.
    let holes: i32 = 13;
    let variable = |p: i32, h: i32| p * holes + h + 1;
    let mut clauses: Vec<Vec<i32>> = (0..=holes)
        .map(|p| (0..holes).map(|h| variable(p, h)).collect())
        .collect();
    for h in 0..holes {
        for p in 0..=holes {
            for q in p + 1..=holes {
                clauses.push(vec![-variable(p, h), -variable(q, h)]);
            }
        }
    }
    let forced = ((holes + 1) * holes + 1) as u32;
    clauses.push(vec![forced as i32]);
    let reduced = Cnf::test_clauses(forced, &clauses);
    let points = Matrix::new(2, vec![0, 0]).unwrap();
    let labels = vec![0; forced as usize * 3];
    // Because the formula is already unsatisfiable, these are redundant. After
    // insertion, they eliminate every hole for pigeon zero, ending immediately.
    let pairs: Vec<_> = (0..holes as u32).map(|h| (h, forced - 1)).collect();
    for backend in BACKENDS {
        let barrier = std::sync::Barrier::new(2);
        let source = |_: &[i64], _, _: &[u32], _, sink: intersect::RowSink<'_>| {
            barrier.wait();
            assert!(sink(&pairs));
            barrier.wait();
            Ok(())
        };
        let mut tuning = eager(usize::MAX);
        tuning.buffer_cap = usize::MAX;
        let mut exchange = Exchange::new(tuning);
        exchange.source = &source;
        exchange.synchronize_first_solve = Some(&barrier);
        let caller = std::thread::current().id();
        let mut events = Vec::new();
        let options = Options {
            deadline: Some(Instant::now() + Duration::from_secs(5)),
            ..all(backend)
        };
        let result = run_observed::<true, _>(
            &reduced,
            &points,
            &labels,
            &options,
            &AnyTriangulation,
            &exchange,
            &mut |progress| {
                assert_eq!(std::thread::current().id(), caller);
                events.push(progress);
            },
        )
        .unwrap();
        assert_eq!(result.outcome, Outcome::Complete);
        assert_eq!(result.found, 0);
        assert_eq!(
            events,
            [
                IncrementalProgress::default(),
                IncrementalProgress {
                    clauses_added: 0,
                    interruptions: 1
                },
                IncrementalProgress {
                    clauses_added: holes as usize,
                    interruptions: 1
                },
            ],
            "{backend:?}"
        );
        assert_eq!(exchange.lock().pauses, 1);
        assert_eq!(exchange.lock().injected, holes as usize);
    }
}

#[test]
fn progress_excludes_budget_discarded_pairs_and_blocking_clauses() {
    let (p, _) = cube_with_huge_points();
    let pairs = intersect::intersecting_pairs(p.points.values(), 3, &p.labels).unwrap();
    for backend in BACKENDS {
        let mut tuning = eager(3);
        tuning.buffer_cap = usize::MAX;
        let exchange = Exchange::new(tuning);
        exchange.lock().hold = true;
        assert!(exchange.offer(&pairs[..7]));
        let mut events = Vec::new();
        let result = run_observed::<true, _>(
            &p.reduced,
            &p.points,
            &p.labels,
            &all(backend),
            &AnyTriangulation,
            &exchange,
            &mut |progress| events.push(progress),
        )
        .unwrap();
        assert_eq!(
            result,
            serial(&p.full, &all(backend), &AnyTriangulation).unwrap()
        );
        assert_eq!(
            events,
            [
                IncrementalProgress::default(),
                IncrementalProgress {
                    clauses_added: 3,
                    interruptions: 0,
                }
            ]
        );
        assert_eq!(exchange.lock().injected, 3);
    }
}

#[test]
fn progress_preserves_late_source_failure_and_reports_no_phantom_work() {
    let (p, huge) = cube_with_huge_points();
    for backend in BACKENDS {
        let exchange = Exchange::new(TUNING);
        exchange.lock().hold = true;
        let mut events = Vec::new();
        let result = run_observed::<true, _>(
            &p.reduced,
            &huge,
            &p.labels,
            &all(backend),
            &AnyTriangulation,
            &exchange,
            &mut |progress| events.push(progress),
        );
        assert!(is_coordinate_error(&result));
        assert_eq!(events, [IncrementalProgress::default()]);
        assert_eq!(exchange.lock().injected, 0);
        assert_eq!(exchange.lock().pauses, 0);
    }
}

#[test]
fn progress_reports_only_successful_insertions_before_invalid_source_pair() {
    let (p, _) = cube_with_huge_points();
    let pairs = intersect::intersecting_pairs(p.points.values(), 3, &p.labels).unwrap();
    for backend in BACKENDS {
        let exchange = Exchange::new(eager(usize::MAX));
        exchange.lock().hold = true;
        assert!(exchange.offer(&[pairs[0], (u32::MAX, 0)]));
        let mut events = Vec::new();
        let result = run_observed::<true, _>(
            &p.reduced,
            &p.points,
            &p.labels,
            &all(backend),
            &AnyTriangulation,
            &exchange,
            &mut |progress| events.push(progress),
        );
        assert!(matches!(result, Err(EnumerateError::Internal)));
        assert_eq!(
            events,
            [
                IncrementalProgress::default(),
                IncrementalProgress {
                    clauses_added: 1,
                    interruptions: 0,
                }
            ]
        );
        assert_eq!(exchange.lock().injected, 2); // Taken is deliberately not reported as inserted.
    }
}

#[test]
fn progress_observer_panic_closes_and_joins_source() {
    let (p, _) = cube_with_huge_points();
    let joined = AtomicBool::new(false);
    let source = |_: &[i64], _, _: &[u32], _, _: intersect::RowSink<'_>| {
        joined.store(true, Ordering::SeqCst);
        Ok(())
    };
    let mut exchange = Exchange::new(TUNING);
    exchange.source = &source;
    exchange.lock().hold = true;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_observed::<true, _>(
            &p.reduced,
            &p.points,
            &p.labels,
            &all(Backend::PicoSat),
            &AnyTriangulation,
            &exchange,
            &mut |_| panic!("observer failed"),
        )
    }));
    assert!(result.is_err());
    assert!(exchange.lock().closed);
    assert!(joined.load(Ordering::SeqCst));
}

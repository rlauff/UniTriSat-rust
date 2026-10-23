use super::*;
use crate::cnf::{formula, reduced};
use crate::families::{cross_polytope, hypercube, scaled_simplex};
use crate::hull::facets;
use crate::intersect::intersecting_pairs;
use crate::lattice::{Central, Restriction, central, faces, lattice_points, simplices};
use crate::matrix::Matrix;
use std::{collections::BTreeSet, sync::atomic::AtomicUsize, time::Duration};

/// A polytope with everything the modes need.
pub(super) struct Polytope {
    pub(super) points: Matrix,
    pub(super) labels: Vec<u32>,
    pub(super) full: Cnf,
    pub(super) reduced: Cnf,
    pub(super) central: Central,
}

pub(super) fn polytope(vertices: &Matrix, restriction: Restriction) -> Polytope {
    let facets = facets(vertices).unwrap();
    let points = lattice_points(vertices, &facets).unwrap();
    let labels = simplices(&points, restriction).unwrap();
    let count = labels.len() / (points.columns() + 1);
    let faces = faces(&points, &facets, &labels).unwrap();
    let pairs = intersecting_pairs(points.values(), points.columns(), &labels).unwrap();
    let central = central(&points, &labels, &faces).unwrap();
    Polytope {
        full: formula(count, &pairs, &faces).unwrap(),
        reduced: reduced(count, &faces, &central).unwrap(),
        points,
        labels,
        central,
    }
}

/// The test polytopes with their triangulation counts. The unimodular counts
/// of the 3-cube (72) and the twice-dilated 3-simplex (192) are Julia's
/// (P0.3 `-solve` jobs).
pub(super) fn polytopes() -> Vec<(&'static str, Polytope, u64)> {
    vec![
        (
            "square",
            polytope(&hypercube(2).unwrap(), Restriction::Unimodular),
            2,
        ),
        (
            "3-cube",
            polytope(&hypercube(3).unwrap(), Restriction::Unimodular),
            72,
        ),
        (
            "3-simplex x2",
            polytope(&scaled_simplex(3, 2).unwrap(), Restriction::Unimodular),
            192,
        ),
        (
            "3-cube nondegenerate",
            polytope(&hypercube(3).unwrap(), Restriction::Nondegenerate),
            74,
        ),
        (
            "octahedron nondegenerate",
            polytope(&cross_polytope(3).unwrap(), Restriction::Nondegenerate),
            4,
        ),
        (
            "2-simplex x3 nondegenerate",
            polytope(&scaled_simplex(2, 3).unwrap(), Restriction::Nondegenerate),
            1166,
        ),
    ]
}

pub(super) const BACKENDS: [Backend; 2] = [Backend::PicoSat, Backend::Cadical];

#[test]
fn kissat_rejects_enumeration_filters_and_incremental_before_work() {
    struct Filter;
    impl Classify for Filter {
        type Error = Infallible;
        fn is_target(&self, _: &[u32]) -> Result<bool, Infallible> {
            panic!("unsupported classifier must not run")
        }
        fn register_stop(&self, _: &Arc<AtomicBool>) {
            panic!("unsupported search must not register cancellation")
        }
    }
    let p = polytope(&hypercube(2).unwrap(), Restriction::Unimodular);
    let all = all(Backend::Kissat);
    assert!(matches!(
        serial(&p.full, &all, &AnyTriangulation),
        Err(EnumerateError::Solve(SolveError::KissatUnsupported(
            "enumerating all models"
        )))
    ));
    let first = Options {
        goal: Goal::First,
        ..all
    };
    for result in [
        serial(&p.full, &first, &Filter),
        parallel_split(&p.full, p.central.simplices(), &first, &Filter),
    ] {
        assert!(matches!(
            result,
            Err(EnumerateError::Solve(SolveError::KissatUnsupported(
                "filtered triangulation search"
            )))
        ));
    }
    let result = incremental_with_pairs(
        &p.reduced,
        &p.points,
        &p.labels,
        &first,
        &Filter,
        &|_, _, _, _, _| panic!("unsupported incremental source must not run"),
    );
    assert!(matches!(
        result,
        Err(EnumerateError::Solve(SolveError::KissatUnsupported(
            "incremental solving"
        )))
    ));
}

#[cfg(not(windows))]
#[test]
fn kissat_serial_and_split_find_a_first_triangulation() {
    let p = polytope(&hypercube(2).unwrap(), Restriction::Unimodular);
    let options = Options {
        goal: Goal::First,
        ..all(Backend::Kissat)
    };
    for result in [
        serial(&p.full, &options, &AnyTriangulation),
        parallel_split(&p.full, p.central.simplices(), &options, &AnyTriangulation),
    ] {
        let result = result.unwrap();
        assert_eq!(result.outcome, Outcome::Found);
        assert!(result.targets >= 1);
        assert!(!result.triangulations.is_empty());
    }
}

#[derive(Clone, Copy, Debug)]
enum Mode {
    Serial,
    ParallelSplit,
    Incremental,
}

const MODES: [Mode; 3] = [Mode::Serial, Mode::ParallelSplit, Mode::Incremental];

pub(super) fn all(backend: Backend) -> Options {
    Options {
        backend,
        goal: Goal::All,
        keep: Keep::All,
        max_solutions: None,
        deadline: None,
    }
}

fn run<C: Classify>(
    p: &Polytope,
    mode: Mode,
    options: &Options,
    classify: &C,
) -> EnumResult<Enumeration, C> {
    match mode {
        Mode::Serial => serial(&p.full, options, classify),
        Mode::ParallelSplit => parallel_split(&p.full, p.central.simplices(), options, classify),
        Mode::Incremental => incremental(&p.reduced, &p.points, &p.labels, options, classify),
    }
}

fn pool(threads: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap()
}

/// Every mode, on every backend and thread count.
fn each_run(check: impl Fn(Mode, Backend, usize) + Sync) {
    for threads in [1, 4] {
        for backend in BACKENDS {
            for mode in MODES {
                pool(threads).install(|| check(mode, backend, threads));
            }
        }
    }
}

#[test]
fn every_mode_backend_and_thread_count_finds_the_same_triangulations() {
    for (name, p, count) in polytopes() {
        let reference = serial(&p.full, &all(Backend::PicoSat), &AnyTriangulation).unwrap();
        assert_eq!(reference.outcome, Outcome::Complete);
        assert_eq!(reference.found, count, "{name}");
        let list = &reference.triangulations;
        assert_eq!(list.len() as u64, count);
        assert!(list.windows(2).all(|w| w[0] < w[1]), "{name}: not sorted");
        for t in list {
            assert!(t.windows(2).all(|w| w[0] < w[1]));
            let central = t
                .iter()
                .filter(|s| p.central.simplices().contains(s))
                .count();
            assert_eq!(central, 1, "{name}: {t:?}");
        }
        each_run(|mode, backend, threads| {
            let result = run(&p, mode, &all(backend), &AnyTriangulation).unwrap();
            assert_eq!(result, reference, "{name}: {mode:?} {backend:?} {threads}");
        });
    }
}

#[test]
fn the_cap_is_global_and_gives_lower_bounds() {
    let (_, p, _) = polytopes().swap_remove(1);
    each_run(|mode, backend, threads| {
        let context = format!("{mode:?} {backend:?} {threads}");
        for (cap, outcome, found) in [
            (1, Outcome::Capped, 1),
            (10, Outcome::Capped, 10),
            // Reaching the cap is a lower bound even if nothing is left.
            (72, Outcome::Capped, 72),
            (73, Outcome::Complete, 72),
        ] {
            let options = Options {
                max_solutions: NonZeroU64::new(cap),
                ..all(backend)
            };
            let result = run(&p, mode, &options, &AnyTriangulation).unwrap();
            assert_eq!(result.outcome, outcome, "{context} cap {cap}");
            assert_eq!(result.found, found, "{context} cap {cap}");
            assert_eq!(result.targets, found);
            let distinct: BTreeSet<_> = result.triangulations.iter().collect();
            assert_eq!(distinct.len() as u64, found);
        }
    });
}

/// Triangulations that use simplex `0` are targets.
struct UsesFirst {
    calls: AtomicUsize,
}

impl Classify for UsesFirst {
    type Error = Infallible;

    fn is_target(&self, simplices: &[u32]) -> Result<bool, Infallible> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(simplices.first() == Some(&0))
    }
}

/// No triangulation is a target.
struct Nothing;

impl Classify for Nothing {
    type Error = Infallible;

    fn is_target(&self, _: &[u32]) -> Result<bool, Infallible> {
        Ok(false)
    }
}

#[test]
fn the_hook_selects_targets_and_the_first_goal_stops_at_one() {
    let (_, p, count) = polytopes().swap_remove(2);
    let reference = serial(&p.full, &all(Backend::PicoSat), &AnyTriangulation).unwrap();
    let expected: Vec<Vec<u32>> = reference
        .triangulations
        .iter()
        .filter(|t| t[0] == 0)
        .cloned()
        .collect();
    assert!(!expected.is_empty() && expected.len() < reference.triangulations.len());
    each_run(|mode, backend, threads| {
        let context = format!("{mode:?} {backend:?} {threads}");
        let hook = UsesFirst {
            calls: AtomicUsize::new(0),
        };
        let result = run(&p, mode, &all(backend), &hook).unwrap();
        assert_eq!(result.outcome, Outcome::Complete, "{context}");
        assert_eq!(result.found, count);
        assert_eq!(hook.calls.load(Ordering::SeqCst) as u64, count);
        assert_eq!(result.targets, expected.len() as u64);
        assert_eq!(result.triangulations, expected, "{context}");

        let first = Options {
            goal: Goal::First,
            keep: Keep::First,
            ..all(backend)
        };
        let result = run(&p, mode, &first, &hook).unwrap();
        assert_eq!(result.outcome, Outcome::Found, "{context}");
        assert!(result.targets >= 1 && result.found >= result.targets);
        if threads == 1 || !matches!(mode, Mode::ParallelSplit) {
            assert_eq!(result.targets, 1, "{context}");
        }
        assert_eq!(result.triangulations.len(), 1);
        assert!(expected.contains(&result.triangulations[0]), "{context}");

        // Without targets the first goal exhausts the search: an exact no.
        let result = run(&p, mode, &first, &Nothing).unwrap();
        assert_eq!(result.outcome, Outcome::Complete, "{context}");
        assert_eq!((result.found, result.targets), (count, 0));
        assert!(result.triangulations.is_empty());

        // Keep::None returns counts only; Keep::First the first target.
        let counts = Options {
            keep: Keep::None,
            ..all(backend)
        };
        let result = run(&p, mode, &counts, &AnyTriangulation).unwrap();
        assert_eq!((result.outcome, result.found), (Outcome::Complete, count));
        assert!(result.triangulations.is_empty());
        let one = Options {
            keep: Keep::First,
            ..all(backend)
        };
        let result = run(&p, mode, &one, &AnyTriangulation).unwrap();
        assert_eq!((result.outcome, result.found), (Outcome::Complete, count));
        assert_eq!(result.triangulations.len(), 1);
        assert!(reference.triangulations.contains(&result.triangulations[0]));
    });
}

#[test]
fn a_target_at_the_cap_is_found_not_capped() {
    let (_, p, _) = polytopes().swap_remove(1);
    for mode in MODES {
        for backend in BACKENDS {
            let options = Options {
                goal: Goal::First,
                max_solutions: NonZeroU64::new(1),
                ..all(backend)
            };
            let result = pool(1)
                .install(|| run(&p, mode, &options, &AnyTriangulation))
                .unwrap();
            assert_eq!(result.outcome, Outcome::Found, "{mode:?} {backend:?}");
            let result = pool(1)
                .install(|| run(&p, mode, &options, &Nothing))
                .unwrap();
            assert_eq!(result.outcome, Outcome::Capped, "{mode:?} {backend:?}");
        }
    }
}

/// Sleeps for every triangulation.
struct Slow;

impl Classify for Slow {
    type Error = Infallible;

    fn is_target(&self, _: &[u32]) -> Result<bool, Infallible> {
        std::thread::sleep(Duration::from_millis(5));
        Ok(true)
    }
}

#[test]
fn a_passed_deadline_gives_lower_bounds() {
    let (_, p, count) = polytopes().swap_remove(2);
    each_run(|mode, backend, threads| {
        let context = format!("{mode:?} {backend:?} {threads}");
        let expired = Options {
            deadline: Some(Instant::now()),
            ..all(backend)
        };
        let result = run(&p, mode, &expired, &AnyTriangulation).unwrap();
        assert_eq!(result.outcome, Outcome::TimedOut, "{context}");
        assert_eq!(result.found, 0);
        // 192 triangulations at 5 ms each take about 1 s per thread.
        let soon = Options {
            deadline: Some(Instant::now() + Duration::from_millis(40)),
            ..all(backend)
        };
        let result = run(&p, mode, &soon, &Slow).unwrap();
        assert_eq!(result.outcome, Outcome::TimedOut, "{context}");
        assert!(result.found < count, "{context}");
        assert_eq!(result.triangulations.len() as u64, result.found);
    });
}

#[derive(Debug, thiserror::Error)]
#[error("refused")]
struct Refused;

/// Fails on the third triangulation.
struct FailsOnThird {
    calls: AtomicUsize,
}

impl Classify for FailsOnThird {
    type Error = Refused;

    fn is_target(&self, _: &[u32]) -> Result<bool, Refused> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 2 {
            return Err(Refused);
        }
        Ok(true)
    }
}

#[test]
fn a_failing_hook_ends_the_search_with_its_error() {
    let (_, p, _) = polytopes().swap_remove(1);
    each_run(|mode, backend, threads| {
        let hook = FailsOnThird {
            calls: AtomicUsize::new(0),
        };
        let result = run(&p, mode, &all(backend), &hook);
        assert!(
            matches!(result, Err(EnumerateError::Check(Refused))),
            "{mode:?} {backend:?} {threads}: {result:?}"
        );
    });
}

#[test]
fn inputs_of_other_simplices_are_rejected() {
    let polytopes = polytopes();
    let (square, cube) = (&polytopes[0].1, &polytopes[1].1);
    let options = all(Backend::PicoSat);
    let result = incremental(
        &cube.reduced,
        &square.points,
        &square.labels,
        &options,
        &AnyTriangulation,
    );
    let variables = cube.reduced.variables();
    assert!(matches!(result, Err(EnumerateError::Mismatch(v)) if v == variables));
    let result = parallel_split(&square.full, &[99], &options, &AnyTriangulation);
    let variables = square.full.variables();
    assert!(matches!(result, Err(EnumerateError::Mismatch(v)) if v == variables));
}

#[test]
fn no_simplices_means_no_triangulation() {
    let square = hypercube(2).unwrap();
    let facets = facets(&square).unwrap();
    let faces = faces(&square, &facets, &[]).unwrap();
    let center = central(&square, &[], &faces).unwrap();
    let full = formula(0, &[], &faces).unwrap();
    let small = reduced(0, &faces, &center).unwrap();
    for backend in BACKENDS {
        let options = all(backend);
        for result in [
            serial(&full, &options, &AnyTriangulation),
            parallel_split(&full, center.simplices(), &options, &AnyTriangulation),
            incremental(&small, &square, &[], &options, &AnyTriangulation),
        ] {
            let result = result.unwrap();
            assert_eq!((result.outcome, result.found), (Outcome::Complete, 0));
        }
    }
}

#[test]
fn flag_sat_all_enumeration_modes_match_independent_classification() {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap();
    pool.install(|| {
        for (_, p, _) in polytopes().into_iter().take(4) {
            let flag =
                crate::flag::Flag::new(&p.labels, p.points.columns() + 1, p.points.rows()).unwrap();
            let encode = |base| {
                crate::cnf::with_flag(base, &p.labels, p.points.columns() + 1, p.points.rows())
                    .unwrap()
            };
            let full = encode(p.full.clone());
            let reduced = encode(p.reduced.clone());
            assert!(full.variables() > full.simplex_variables());
            for backend in BACKENDS {
                let options = all(backend);
                let expected = serial(&p.full, &options, &flag).unwrap();
                for actual in [
                    serial(&full, &options, &AnyTriangulation),
                    parallel_split(&full, p.central.simplices(), &options, &AnyTriangulation),
                    incremental(&reduced, &p.points, &p.labels, &options, &AnyTriangulation),
                    incremental_with_pairs(
                        &reduced,
                        &p.points,
                        &p.labels,
                        &options,
                        &AnyTriangulation,
                        &crate::intersect::stream_pairs,
                    ),
                ] {
                    let actual = actual.unwrap();
                    assert_eq!(actual.outcome, Outcome::Complete);
                    assert_eq!(actual.found, expected.targets);
                    assert_eq!(actual.targets, expected.targets);
                    assert_eq!(actual.triangulations, expected.triangulations);
                    assert!(
                        actual
                            .triangulations
                            .iter()
                            .flatten()
                            .all(|&s| s < full.simplex_variables())
                    );
                }
            }
            // An auxiliary index must never be accepted as a central simplex.
            assert!(matches!(
                parallel_split(
                    &full,
                    &[full.simplex_variables()],
                    &all(Backend::PicoSat),
                    &AnyTriangulation
                ),
                Err(EnumerateError::Mismatch(_))
            ));
        }
    });
}

#[cfg(not(windows))]
#[test]
fn flag_sat_kissat_first_static_search_projects_auxiliary_variables() {
    let p = polytope(&hypercube(3).unwrap(), Restriction::Unimodular);
    let full = crate::cnf::with_flag(p.full, &p.labels, 4, p.points.rows()).unwrap();
    let options = Options {
        goal: Goal::First,
        ..all(Backend::Kissat)
    };
    let checker = crate::flag::Flag::new(&p.labels, 4, p.points.rows()).unwrap();
    for actual in [
        serial(&full, &options, &AnyTriangulation),
        parallel_split(&full, p.central.simplices(), &options, &AnyTriangulation),
    ] {
        let actual = actual.unwrap();
        assert_eq!(actual.outcome, Outcome::Found);
        for triangulation in actual.triangulations {
            assert!(checker.is_flag(&triangulation).unwrap());
            assert!(triangulation.iter().all(|&s| s < full.simplex_variables()));
        }
    }
}

#[test]
fn regular_learn_rejects_invalid_classifier_conflicts() {
    struct Bad(Classification);
    impl Classify for Bad {
        type Error = Infallible;
        fn is_target(&self, _: &[u32]) -> Result<bool, Infallible> {
            Ok(self.0.target)
        }
        fn classify(&self, _: &[u32]) -> Result<Classification, Infallible> {
            Ok(self.0.clone())
        }
    }
    let cnf = Cnf::test_clauses(3, &[vec![1], vec![-2], vec![-3]]);
    for answer in [
        Classification {
            target: false,
            conflict: Some(vec![]),
        },
        Classification {
            target: false,
            conflict: Some(vec![1]),
        },
        Classification {
            target: false,
            conflict: Some(vec![3]),
        },
        Classification {
            target: true,
            conflict: Some(vec![0]),
        },
    ] {
        for backend in BACKENDS {
            for actual in [
                serial(&cnf, &all(backend), &Bad(answer.clone())),
                parallel_split(&cnf, &[0], &all(backend), &Bad(answer.clone())),
            ] {
                assert!(matches!(actual, Err(EnumerateError::InvalidConflict)));
            }
        }
    }
}

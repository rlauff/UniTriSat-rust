use super::*;
use std::{
    collections::BTreeSet,
    sync::{atomic::Ordering, mpsc},
    thread,
    time::Duration,
};

const BACKENDS: [Backend; 2] = [Backend::PicoSat, Backend::Cadical];

#[cfg(not(windows))]
#[test]
fn kissat_one_shot_sat_unsat_and_dont_care() {
    for clauses in [
        vec![],
        vec![vec![1], vec![-2]],
        vec![vec![]],
        vec![vec![1], vec![-1]],
    ] {
        let mut solver = solver_with(Backend::Kissat, 3, &clauses);
        let expected = brute_force(3, &clauses, &[]);
        let result = solver.solve(&[], &Control::default()).unwrap();
        assert_eq!(result == Status::Sat, !expected.is_empty());
        let mut model = vec![99];
        if result == Status::Sat {
            solver.model(&mut model).unwrap();
            assert!(expected.contains(&as_assignment(&model)));
            assert!(!model.contains(&3));
        } else {
            assert_eq!(solver.model(&mut model), Err(SolveError::KissatNoModel));
            assert!(model.is_empty());
        }
        assert!(matches!(
            solver.solve(&[], &Control::default()),
            Err(SolveError::KissatUnsupported(_))
        ));
        assert!(matches!(
            solver.add_clause(&[1]),
            Err(SolveError::KissatUnsupported(_))
        ));
        assert!(matches!(
            solver.block(&model),
            Err(SolveError::KissatUnsupported(_))
        ));
        if result == Status::Sat {
            let old = model.clone();
            solver.model(&mut model).unwrap();
            assert_eq!(old, model);
        }
    }
}

#[cfg(not(windows))]
#[test]
fn kissat_rejects_assumptions_and_invalid_literals_before_native_mutation() {
    let mut solver = Solver::new(Backend::Kissat, 2).unwrap();
    for assumptions in [&[1][..], &[0][..], &[i32::MIN][..]] {
        assert!(matches!(
            solver.solve(assumptions, &Control::default()),
            Err(SolveError::KissatUnsupported("assumptions"))
        ));
    }
    for clause in [[1, 0], [1, 3], [1, i32::MIN]] {
        assert!(matches!(
            solver.add_clause(&clause),
            Err(SolveError::Literal { .. })
        ));
    }
    solver.add_clause(&[-1]).unwrap();
    assert_eq!(solver.solve(&[], &Control::default()), Ok(Status::Sat));
    let mut model = vec![];
    solver.model(&mut model).unwrap();
    assert!(model.is_empty());
    let too_many = usize::try_from(KISSAT_MAX_VARIABLES).unwrap() + 1;
    assert_eq!(
        Solver::new(Backend::Kissat, too_many).unwrap_err(),
        SolveError::KissatTooManyVariables(too_many)
    );
}

#[cfg(not(windows))]
#[test]
fn kissat_stop_and_deadline_are_one_shot() {
    for (stopped, expired, expected) in [
        (true, false, Status::Stopped),
        (false, true, Status::TimedOut),
        (true, true, Status::Stopped),
    ] {
        let mut solver = Solver::new(Backend::Kissat, 1).unwrap();
        let control = Control {
            stop: Arc::new(AtomicBool::new(stopped)),
            deadline: expired.then(Instant::now),
        };
        assert_eq!(solver.solve(&[], &control), Ok(expected));
        assert_eq!(solver.model(&mut vec![]), Err(SolveError::KissatNoModel));
        assert!(matches!(
            solver.solve(&[], &Control::default()),
            Err(SolveError::KissatUnsupported(_))
        ));
        assert!(matches!(
            solver.add_clause(&[]),
            Err(SolveError::KissatUnsupported(_))
        ));
    }
}

/// Deterministic test randomness (64-bit LCG, high bits).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn i32_of(value: u32) -> i32 {
    i32::try_from(value).unwrap()
}

/// A random clause over `1..=n` of length `0..=3`; repeated variables and
/// tautologies are allowed. Empty clauses are rare.
fn random_clause(rng: &mut Lcg, n: u32) -> Vec<i32> {
    let length = match rng.below(40) {
        0 => 0,
        k => 1 + k % 3,
    };
    (0..length)
        .map(|_| {
            let variable = i32_of(1 + u32::try_from(rng.below(u64::from(n))).unwrap());
            if rng.below(2) == 0 {
                variable
            } else {
                -variable
            }
        })
        .collect()
}

fn satisfies(assignment: u32, literal: i32) -> bool {
    let bit = assignment >> (literal.unsigned_abs() - 1) & 1 == 1;
    bit == (literal > 0)
}

/// Every assignment over `n` variables (bit `v - 1` is variable `v`) that
/// satisfies the clauses and assumptions.
fn brute_force(n: u32, clauses: &[Vec<i32>], assumptions: &[i32]) -> BTreeSet<u32> {
    (0..1_u32 << n)
        .filter(|&a| {
            clauses.iter().all(|c| c.iter().any(|&l| satisfies(a, l)))
                && assumptions.iter().all(|&l| satisfies(a, l))
        })
        .collect()
}

fn as_assignment(true_variables: &[u32]) -> u32 {
    true_variables.iter().map(|&v| 1 << (v - 1)).sum()
}

fn solver_with(backend: Backend, n: u32, clauses: &[Vec<i32>]) -> Solver {
    let mut solver = Solver::new(backend, n as usize).unwrap();
    for clause in clauses {
        solver.add_clause(clause).unwrap();
    }
    solver
}

/// All models, each blocked by the negation of its full assignment (as
/// `PicoSAT.itersolve` does), so the enumeration is exact for any CNF.
fn enumerate_full(solver: &mut Solver, n: u32) -> Vec<u32> {
    let control = Control::default();
    let mut found = Vec::new();
    let mut model = Vec::new();
    while solver.solve(&[], &control).unwrap() == Status::Sat {
        solver.model(&mut model).unwrap();
        let assignment = as_assignment(&model);
        found.push(assignment);
        let blocking: Vec<i32> = (1..=n)
            .map(|v| {
                let v = i32_of(v);
                if satisfies(assignment, v) { -v } else { v }
            })
            .collect();
        solver.add_clause(&blocking).unwrap();
    }
    found
}

#[test]
fn both_backends_enumerate_exactly_the_brute_force_models() {
    let mut rng = Lcg(0x5eed_4002);
    for round in 0..300 {
        let n = 1 + u32::try_from(rng.below(8)).unwrap();
        let clauses: Vec<Vec<i32>> = (0..rng.below(u64::from(3 * n)))
            .map(|_| random_clause(&mut rng, n))
            .collect();
        let expected = brute_force(n, &clauses, &[]);
        for backend in BACKENDS {
            let mut solver = solver_with(backend, n, &clauses);
            let found = enumerate_full(&mut solver, n);
            let distinct: BTreeSet<u32> = found.iter().copied().collect();
            assert_eq!(
                distinct.len(),
                found.len(),
                "{backend:?} round {round}: repeated model"
            );
            assert_eq!(distinct, expected, "{backend:?} round {round}: {clauses:?}");
        }
    }
}

#[test]
fn assumptions_match_brute_force_and_last_one_solve() {
    let mut rng = Lcg(0xa55e_0001);
    for round in 0..300 {
        let n = 1 + u32::try_from(rng.below(7)).unwrap();
        let clauses: Vec<Vec<i32>> = (0..rng.below(u64::from(3 * n)))
            .map(|_| random_clause(&mut rng, n))
            .collect();
        let assumptions = random_clause(&mut rng, n);
        let with = brute_force(n, &clauses, &assumptions);
        let without = brute_force(n, &clauses, &[]);
        for backend in BACKENDS {
            let mut solver = solver_with(backend, n, &clauses);
            let control = Control::default();
            let mut model = Vec::new();
            let status = solver.solve(&assumptions, &control).unwrap();
            assert_eq!(
                status == Status::Sat,
                !with.is_empty(),
                "{backend:?} round {round}"
            );
            if status == Status::Sat {
                solver.model(&mut model).unwrap();
                assert!(
                    with.contains(&as_assignment(&model)),
                    "{backend:?} round {round}"
                );
            }
            // The assumptions are gone for the next solve.
            let status = solver.solve(&[], &control).unwrap();
            assert_eq!(
                status == Status::Sat,
                !without.is_empty(),
                "{backend:?} round {round}"
            );
        }
    }
}

/// A random exact-cover instance: `subsets` nonempty subsets of a universe
/// of `elements`, as bit masks, and its CNF (every element covered by a
/// chosen subset, no two chosen subsets overlapping), the shape of the
/// triangulation CNF.
fn exact_cover(rng: &mut Lcg, elements: u32, subsets: u32) -> (Vec<u32>, Vec<Vec<i32>>) {
    let masks: Vec<u32> = (0..subsets)
        .map(|_| 1 + u32::try_from(rng.below((1 << elements) - 1)).unwrap())
        .collect();
    let mut clauses: Vec<Vec<i32>> = (0..elements)
        .map(|e| {
            (1..=subsets)
                .filter(|&s| masks[s as usize - 1] >> e & 1 == 1)
                .map(i32_of)
                .collect()
        })
        .collect();
    for a in 1..=subsets {
        for b in a + 1..=subsets {
            if masks[a as usize - 1] & masks[b as usize - 1] != 0 {
                clauses.push(vec![-i32_of(a), -i32_of(b)]);
            }
        }
    }
    (masks, clauses)
}

#[test]
fn blocking_true_variables_counts_exact_covers() {
    let mut rng = Lcg(0xc0e5_0003);
    let mut total = 0;
    for round in 0..200 {
        let elements = 1 + u32::try_from(rng.below(6)).unwrap();
        let subsets = 1 + u32::try_from(rng.below(12)).unwrap();
        let (masks, clauses) = exact_cover(&mut rng, elements, subsets);
        let full = (1 << elements) - 1;
        let covers: BTreeSet<u32> = (0..1_u32 << subsets)
            .filter(|&choice| {
                let chosen = (0..subsets).filter(|&s| choice >> s & 1 == 1);
                let sizes: u32 = chosen.clone().map(|s| masks[s as usize].count_ones()).sum();
                let union = chosen.fold(0, |u, s| u | masks[s as usize]);
                union == full && sizes == elements
            })
            .collect();
        total += covers.len();
        assert_eq!(
            brute_force(subsets, &clauses, &[]),
            covers,
            "CNF of round {round}"
        );
        for backend in BACKENDS {
            let mut solver = solver_with(backend, subsets, &clauses);
            let control = Control::default();
            let mut found = BTreeSet::new();
            let mut model = Vec::new();
            while solver.solve(&[], &control).unwrap() == Status::Sat {
                solver.model(&mut model).unwrap();
                assert!(
                    found.insert(as_assignment(&model)),
                    "{backend:?} round {round}"
                );
                solver.block(&model).unwrap();
            }
            assert_eq!(found, covers, "{backend:?} round {round}");
        }
    }
    assert!(total > 50, "too few covers ({total}) to exercise blocking");
}

/// Upper bound on any interruptible solve in these tests.
const SAFETY_NET: Duration = Duration::from_secs(30);

/// Pigeonhole formula with `holes + 1` pigeons; hard for both solvers.
/// Variable `p * holes + h + 1` puts pigeon `p` into hole `h`.
fn pigeonhole(holes: u32) -> (u32, Vec<Vec<i32>>) {
    let variable = |p: u32, h: u32| i32_of(p * holes + h + 1);
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
    ((holes + 1) * holes, clauses)
}

#[test]
fn stop_flag_set_from_another_thread_interrupts_the_solve() {
    let delay = Duration::from_millis(150);
    for backend in BACKENDS {
        let (n, clauses) = pigeonhole(13);
        let mut solver = solver_with(backend, n, &clauses);
        // The deadline is only a safety net: a solver that ignored the stop
        // flag would end with `TimedOut` instead of hanging the test.
        let control = Control {
            deadline: Some(Instant::now() + SAFETY_NET),
            ..Control::default()
        };
        let stop = Arc::clone(&control.stop);
        let (ready, started) = mpsc::channel();
        let stopper = thread::spawn(move || {
            started.recv().unwrap();
            thread::sleep(delay);
            stop.store(true, Ordering::Relaxed);
        });
        let begin = Instant::now();
        ready.send(()).unwrap();
        let status = solver.solve(&[], &control).unwrap();
        let elapsed = begin.elapsed();
        stopper.join().unwrap();
        assert_eq!(status, Status::Stopped, "{backend:?}");
        assert!(
            elapsed >= delay,
            "{backend:?} returned before the stop: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "{backend:?} reacted late to the stop: {elapsed:?}"
        );
        assert!(solver.model(&mut Vec::new()).is_err(), "{backend:?}");

        // The solver stays usable: keep pigeon 0 out of every hole.
        for h in 1..=13 {
            solver.add_clause(&[-h]).unwrap();
        }
        assert_eq!(
            solver.solve(&[], &Control::default()).unwrap(),
            Status::Unsat
        );
    }
}

#[test]
fn deadline_times_out_and_stop_takes_precedence() {
    for backend in BACKENDS {
        let (n, clauses) = pigeonhole(13);
        let mut solver = solver_with(backend, n, &clauses);
        let begin = Instant::now();
        let control = Control {
            deadline: Some(begin + Duration::from_millis(100)),
            ..Control::default()
        };
        // Safety net: a solver that ignored the deadline is stopped after
        // `SAFETY_NET` and ends with `Stopped` instead of hanging the test.
        let (done, finished) = mpsc::channel::<()>();
        let stop = Arc::clone(&control.stop);
        let guard = thread::spawn(move || {
            if finished.recv_timeout(SAFETY_NET) == Err(mpsc::RecvTimeoutError::Timeout) {
                stop.store(true, Ordering::Relaxed);
            }
        });
        let status = solver.solve(&[], &control).unwrap();
        let elapsed = begin.elapsed();
        drop(done);
        guard.join().unwrap();
        assert_eq!(status, Status::TimedOut, "{backend:?}");
        assert!(
            elapsed >= Duration::from_millis(100),
            "{backend:?}: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "{backend:?}: {elapsed:?}"
        );

        control.stop.store(true, Ordering::Relaxed);
        assert_eq!(
            solver.solve(&[], &control).unwrap(),
            Status::Stopped,
            "{backend:?}"
        );
    }
}

#[test]
fn literals_must_name_declared_variables() {
    for backend in BACKENDS {
        let mut solver = Solver::new(backend, 3).unwrap();
        assert_eq!(solver.variables(), 3);
        let control = Control::default();
        for bad in [0, 4, -4, i32::MAX, i32::MIN] {
            let expected = SolveError::Literal {
                literal: i64::from(bad),
                variables: 3,
            };
            assert_eq!(solver.add_clause(&[1, bad]), Err(expected), "{backend:?}");
            assert_eq!(solver.solve(&[bad], &control), Err(expected), "{backend:?}");
        }
        for bad in [0, 4, u32::MAX] {
            let expected = SolveError::Literal {
                literal: -i64::from(bad),
                variables: 3,
            };
            assert_eq!(solver.block(&[1, bad]), Err(expected), "{backend:?}");
        }
        // Rejected clauses never reached the solver.
        solver.add_clause(&[-1]).unwrap();
        assert_eq!(solver.solve(&[], &control), Ok(Status::Sat));
        let mut model = vec![99];
        solver.model(&mut model).unwrap();
        assert!(
            !model.contains(&1) && !model.contains(&99),
            "{backend:?}: {model:?}"
        );
    }
}

#[test]
fn variable_count_is_bounded() {
    for backend in BACKENDS {
        let too_many = MAX_VARIABLES as usize + 1;
        assert_eq!(
            Solver::new(backend, too_many).unwrap_err(),
            SolveError::TooManyVariables(too_many)
        );
        // The count is only a declaration: the native solvers allocate for
        // the largest index they actually see.
        let mut solver = Solver::new(backend, MAX_VARIABLES as usize).unwrap();
        assert_eq!(solver.variables(), MAX_VARIABLES);
        solver.add_clause(&[1]).unwrap();
        assert_eq!(solver.solve(&[], &Control::default()), Ok(Status::Sat));
    }
}

#[test]
fn models_need_a_satisfiable_solve() {
    for backend in BACKENDS {
        let mut solver = Solver::new(backend, 2).unwrap();
        let mut model = Vec::new();
        let no_model = |result: Result<(), SolveError>| {
            matches!(
                result,
                Err(SolveError::PicoSat(picosat::Error::NoModel)
                    | SolveError::Cadical(cadical::Error::NoModel))
            )
        };
        assert!(
            no_model(solver.model(&mut model)),
            "{backend:?}: before solving"
        );
        let control = Control::default();
        assert_eq!(solver.solve(&[], &control), Ok(Status::Sat));
        solver.model(&mut model).unwrap();
        solver.add_clause(&[1, 2]).unwrap();
        assert!(
            no_model(solver.model(&mut model)),
            "{backend:?}: after a clause"
        );
        assert_eq!(solver.solve(&[-1, -2], &control), Ok(Status::Unsat));
        assert!(
            no_model(solver.model(&mut model)),
            "{backend:?}: after UNSAT"
        );
        solver.add_clause(&[]).unwrap();
        assert_eq!(solver.solve(&[], &control), Ok(Status::Unsat));
    }
}

#[test]
fn zero_variables() {
    for backend in BACKENDS {
        let mut solver = Solver::new(backend, 0).unwrap();
        let control = Control::default();
        let mut model = vec![1];
        assert!(solver.model(&mut model).is_err(), "{backend:?}: before SAT");
        assert_eq!(solver.solve(&[], &control), Ok(Status::Sat));
        solver.model(&mut model).unwrap();
        assert!(model.is_empty());
        // Rejected inputs leave the previous native SAT state intact.
        assert!(solver.add_clause(&[1]).is_err());
        assert!(solver.solve(&[1], &control).is_err());
        solver.model(&mut model).unwrap();
        solver.block(&[]).unwrap();
        assert!(
            solver.model(&mut model).is_err(),
            "{backend:?}: after clause"
        );
        assert_eq!(solver.solve(&[], &control), Ok(Status::Unsat));
        assert!(
            solver.model(&mut model).is_err(),
            "{backend:?}: after UNSAT"
        );
    }
}

#[test]
fn status_mapping() {
    assert_eq!(status(Some(true), true, true), Ok(Status::Sat));
    assert_eq!(status(Some(false), true, true), Ok(Status::Unsat));
    assert_eq!(status(None, true, true), Ok(Status::Stopped));
    assert_eq!(status(None, false, true), Ok(Status::TimedOut));
    assert_eq!(status(None, false, false), Err(SolveError::Inconclusive));
}

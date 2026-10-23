use super::*;
use std::sync::atomic::Ordering;

fn lit(value: i32) -> Literal {
    Literal::try_from(value).unwrap()
}

fn status(solver: &mut Solver, assumptions: &[i32]) -> Status {
    let assumptions: Vec<Literal> = assumptions.iter().map(|&l| lit(l)).collect();
    solver
        .solve(&assumptions, &Control::default())
        .unwrap()
        .status
}

/// Pigeonhole: `holes + 1` pigeons in `holes` holes; hard for resolution.
fn pigeonhole(solver: &mut Solver, holes: i32) {
    let var = |pigeon: i32, hole: i32| pigeon * holes + hole + 1;
    for pigeon in 0..=holes {
        let clause: Vec<Literal> = (0..holes).map(|hole| lit(var(pigeon, hole))).collect();
        solver.add_clause(&clause).unwrap();
    }
    for hole in 0..holes {
        for a in 0..=holes {
            for b in a + 1..=holes {
                solver
                    .add_clause(&[lit(-var(a, hole)), lit(-var(b, hole))])
                    .unwrap();
            }
        }
    }
}

#[test]
fn literal_domain() {
    assert_eq!(std::mem::size_of::<Literal>(), 4);
    assert_eq!(Literal::try_from(0), Err(LiteralError(0)));
    for raw in [
        i32::MIN,
        i32::MAX,
        -(1 << 30),
        1 << 30,
        -((1 << 30) - 1),
        (1 << 30) - 1,
    ] {
        assert_eq!(Literal::try_from(raw), Err(LiteralError(raw)));
    }
    for raw in [-(1 << 30) + 2, -1, 1, (1 << 30) - 2] {
        assert_eq!(lit(raw).get(), raw);
    }
}

#[test]
fn model_only_after_sat_and_assumptions_are_scoped() {
    let mut solver = Solver::new().unwrap();
    assert_eq!(solver.value(lit(1)), Err(Error::NoModel));
    solver.add_clause(&[lit(1), lit(2)]).unwrap();
    assert_eq!(status(&mut solver, &[-1, -2]), Status::Unsat);
    assert_eq!(solver.value(lit(1)), Err(Error::NoModel));
    // The assumptions applied to that solve only.
    assert_eq!(status(&mut solver, &[]), Status::Sat);
    assert_eq!(solver.variables(), Ok(2));
    let (one, two) = (solver.value(lit(1)).unwrap(), solver.value(lit(2)).unwrap());
    assert!(one == Value::True || two == Value::True);
    assert_eq!(
        solver.value(lit(-1)).unwrap() == Value::True,
        one == Value::False
    );
    // A variable CaDiCaL has never seen is false.
    assert_eq!(solver.value(lit(7)), Ok(Value::False));
    // Adding a clause discards the model.
    solver.add_clause(&[]).unwrap();
    assert_eq!(solver.value(lit(1)), Err(Error::NoModel));
    assert_eq!(status(&mut solver, &[]), Status::Unsat);
}

#[test]
fn incremental_enumeration_with_blocking_clauses() {
    // x1 xor x2 xor x3 = 1 has four models.
    let mut solver = Solver::new().unwrap();
    for clause in [[1, 2, 3], [1, -2, -3], [-1, 2, -3], [-1, -2, 3]] {
        solver.add_clause(&clause.map(lit)).unwrap();
    }
    let mut models = Vec::new();
    while status(&mut solver, &[]) == Status::Sat {
        let model: Vec<i32> = (1..=3)
            .map(|v| match solver.value(lit(v)).unwrap() {
                Value::True => v,
                Value::False => -v,
            })
            .collect();
        let blocking: Vec<Literal> = model.iter().map(|&l| lit(-l)).collect();
        solver.add_clause(&blocking).unwrap();
        models.push(model);
    }
    models.sort();
    assert_eq!(models, [[-1, -2, 3], [-1, 2, -3], [1, -2, -3], [1, 2, 3]]);
}

#[test]
fn new_variables_after_a_solve() {
    let mut solver = Solver::new().unwrap();
    solver.add_clause(&[lit(1)]).unwrap();
    assert_eq!(status(&mut solver, &[]), Status::Sat);
    solver.add_clause(&[lit(-1), lit(50)]).unwrap();
    assert_eq!(status(&mut solver, &[]), Status::Sat);
    assert_eq!(solver.value(lit(50)), Ok(Value::True));
    assert_eq!(status(&mut solver, &[-50]), Status::Unsat);
    assert_eq!(status(&mut solver, &[50]), Status::Sat);
}

#[test]
fn stop_flag_and_deadline_interrupt_a_hard_solve() {
    for control in [
        Control::new(Arc::new(AtomicBool::new(true)), None),
        Control::new(Arc::default(), Some(Instant::now())),
    ] {
        let mut solver = Solver::new().unwrap();
        pigeonhole(&mut solver, 12);
        let outcome = solver.solve(&[], &control).unwrap();
        assert_eq!(outcome.status, Status::Unknown);
        assert!(outcome.interrupts.stopped || outcome.interrupts.deadline);
        assert_eq!(solver.value(lit(1)), Err(Error::NoModel));
        // The solver stays usable after an interrupt.
        let small = solver.solve(&[lit(1)], &control).unwrap();
        assert!(matches!(small.status, Status::Unknown | Status::Unsat));
    }
    let stop = Arc::new(AtomicBool::new(false));
    let mut solver = Solver::new().unwrap();
    pigeonhole(&mut solver, 4);
    let outcome = solver
        .solve(&[], &Control::new(Arc::clone(&stop), None))
        .unwrap();
    assert_eq!(outcome.status, Status::Unsat);
    assert!(!stop.load(Ordering::Relaxed));
}

#[test]
fn status_mapping() {
    let mut solver = Solver::new().unwrap();
    let seen = Interrupts {
        stopped: true,
        deadline: true,
    };
    for (raw, expected) in [(10, Status::Sat), (20, Status::Unsat), (0, Status::Unknown)] {
        assert_eq!(
            solver.accept_status(raw, seen),
            Ok(Outcome {
                status: expected,
                interrupts: seen
            })
        );
    }
}

#[test]
fn failures_inside_cadical_invalidate_the_solver() {
    for (raw, error) in [
        (-1, Error::Native("solve")),
        (19, Error::UnexpectedStatus(19)),
    ] {
        let mut solver = Solver::new().unwrap();
        assert_eq!(solver.accept_status(raw, Interrupts::default()), Err(error));
        assert_eq!(solver.solve(&[], &Control::default()), Err(Error::Invalid));
        assert_eq!(solver.add_clause(&[]), Err(Error::Invalid));
        assert_eq!(solver.variables(), Err(Error::Invalid));
        assert_eq!(solver.value(lit(1)), Err(Error::Invalid));
    }
}

#[test]
fn clause_limit_keeps_the_solver_usable() {
    let mut solver = Solver::new().unwrap();
    // Set the counter directly instead of adding two billion clauses.
    solver.clauses = MAX_CLAUSES;
    assert_eq!(solver.add_clause(&[]), Err(Error::TooManyClauses));
    assert_eq!(status(&mut solver, &[]), Status::Sat);
}

#[test]
fn independent_solvers_on_many_threads() {
    let handles: Vec<_> = (0..8)
        .map(|i| {
            std::thread::spawn(move || {
                let mut solver = Solver::new().unwrap();
                pigeonhole(&mut solver, 4 + i % 3);
                solver.solve(&[], &Control::default()).unwrap().status
            })
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().unwrap(), Status::Unsat);
    }
}

#[test]
fn concurrent_construction_and_option_lookup() {
    let start = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                start.wait();
                for _ in 0..100 {
                    // Construction sets factor through the shared option table.
                    let mut solver = Solver::new().unwrap();
                    solver.add_clause(&[lit(1), lit(2)]).unwrap();
                    assert_eq!(status(&mut solver, &[-1]), Status::Sat);
                    assert_eq!(solver.value(lit(2)), Ok(Value::True));
                    assert_eq!(status(&mut solver, &[-1, -2]), Status::Unsat);
                }
            });
        }
    });
}

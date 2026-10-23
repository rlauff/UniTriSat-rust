//! cddlib used from many threads at once, without a lock (task P1.9).
//!
//! This file is its own test process, so the first cddlib calls below race
//! on the one-time initialization of cddlib's global constants.

#![allow(clippy::disallowed_macros, reason = "tests may assert")]

use std::sync::Barrier;
use unitrisat_cdd::{
    HRepresentation, Lp, LpSolution, LpStatus, Rational, Sense, hull_inequalities, solve_lp,
};

const THREADS: usize = 16;
const ROUNDS: usize = 200;

fn row(values: &[i64]) -> Vec<Rational> {
    values.iter().map(|&v| Rational::from(v)).collect()
}

/// The vertices of the unit 4-cube, row-major.
fn cube4() -> Vec<i64> {
    (0..16_i64)
        .flat_map(|i| [i & 1, (i >> 1) & 1, (i >> 2) & 1, (i >> 3) & 1])
        .collect()
}

/// A bounded 6-variable LP that takes several dual simplex pivots:
/// maximize `sum (k+1) x_k` subject to `x >= 0` and eight packing rows.
fn packing() -> Vec<Vec<Rational>> {
    let mut rows: Vec<Vec<Rational>> = (0..6)
        .map(|k| {
            let mut r = vec![0; 7];
            r[k + 1] = 1;
            row(&r)
        })
        .collect();
    for i in 0..8_i64 {
        let mut r = vec![7 + 3 * i];
        r.extend((0..6_i64).map(|k| -((i * k + i + k) % 5 + 1)));
        rows.push(row(&r));
    }
    rows
}

/// Every call one round makes; the results must not depend on the thread.
fn round(packing: &[Vec<Rational>]) -> (HRepresentation, HRepresentation, LpSolution, LpStatus) {
    let cube = hull_inequalities(&cube4(), 4).unwrap();
    // A triangle in R^3: lower-dimensional, so it has an equation.
    let triangle = hull_inequalities(&[1, 0, 2, 3, 1, 2, 0, 4, 2], 3).unwrap();
    let optimum = solve_lp(&Lp {
        inequalities: packing,
        equations: &[],
        objective: &row(&[0, 1, 2, 3, 4, 5, 6]),
        sense: Sense::Maximize,
    })
    .unwrap();
    // x >= 1 and x <= 0.
    let infeasible = solve_lp(&Lp {
        inequalities: &[row(&[-1, 1]), row(&[0, -1])],
        equations: &[],
        objective: &row(&[0, 1]),
        sense: Sense::Minimize,
    })
    .unwrap()
    .status;
    (cube, triangle, optimum, infeasible)
}

#[test]
fn concurrent_calls_match_sequential_results() {
    let packing = packing();
    let start = Barrier::new(THREADS);
    let results: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                scope.spawn(|| {
                    start.wait();
                    (0..ROUNDS).map(|_| round(&packing)).collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect()
    });
    // Sequential reference, after every thread is done.
    let expected = round(&packing);
    assert_eq!(expected.0.inequalities.len(), 8);
    assert!(expected.0.equations.is_empty());
    assert_eq!(expected.1.inequalities.len(), 3);
    assert_eq!(expected.1.equations.len(), 1);
    assert_eq!(expected.2.status, LpStatus::Optimal);
    assert_ne!(expected.3, LpStatus::Optimal);
    for thread in &results {
        assert_eq!(thread.len(), ROUNDS);
        for result in thread {
            assert_eq!(result, &expected);
        }
    }
}

use super::*;
use rug::Integer;
use std::collections::BTreeSet;

fn q(n: i64, d: i64) -> Rational {
    Rational::from((n, d))
}

fn row(values: &[i64]) -> Vec<Rational> {
    values.iter().map(|&v| Rational::from(v)).collect()
}

/// Rows scaled to primitive integers, so they compare independently of
/// cddlib's scaling.
fn normalized(rows: &[Vec<Rational>]) -> BTreeSet<Vec<Integer>> {
    rows.iter()
        .map(|r| {
            let lcm = r.iter().fold(Integer::from(1), |acc, x| acc.lcm(x.denom()));
            let ints: Vec<Integer> = r
                .iter()
                .map(|x| Integer::from(x.numer() * &lcm) / x.denom())
                .collect();
            let gcd = ints.iter().fold(Integer::new(), |acc, x| acc.gcd(x));
            ints.into_iter().map(|x| x / &gcd).collect()
        })
        .collect()
}

fn ints(rows: &[&[i64]]) -> BTreeSet<Vec<Integer>> {
    rows.iter()
        .map(|r| r.iter().map(|&v| Integer::from(v)).collect())
        .collect()
}

#[test]
fn unit_square_facets() {
    let h = hull_inequalities(&[0, 0, 1, 0, 0, 1, 1, 1], 2).unwrap();
    assert!(h.equations.is_empty());
    assert_eq!(
        normalized(&h.inequalities),
        ints(&[&[0, 1, 0], &[0, 0, 1], &[1, -1, 0], &[1, 0, -1]])
    );
}

#[test]
fn interior_points_are_redundant() {
    // The square [0,2]^2 with its center and edge midpoints.
    let points = [0, 0, 2, 0, 0, 2, 2, 2, 1, 1, 1, 0, 0, 1];
    let h = hull_inequalities(&points, 2).unwrap();
    assert_eq!(
        normalized(&h.inequalities),
        ints(&[&[0, 1, 0], &[0, 0, 1], &[2, -1, 0], &[2, 0, -1]])
    );
}

#[test]
fn cube_and_simplex_facet_counts() {
    let mut cube = Vec::new();
    for i in 0..8_i64 {
        cube.extend([i & 1, (i >> 1) & 1, (i >> 2) & 1]);
    }
    assert_eq!(hull_inequalities(&cube, 3).unwrap().inequalities.len(), 6);
    let simplex = [0, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 2];
    let h = hull_inequalities(&simplex, 4).unwrap();
    assert_eq!(h.inequalities.len(), 5);
    assert!(
        normalized(&h.inequalities).contains(&ints(&[&[2, -1, -1, -1, -1]]).pop_first().unwrap())
    );
}

#[test]
fn lower_dimensional_hulls_have_equations() {
    // A segment on the line y = x in the plane.
    let h = hull_inequalities(&[0, 0, 3, 3], 2).unwrap();
    assert_eq!(h.equations.len(), 1);
    let equation = normalized(&h.equations);
    assert!(equation == ints(&[&[0, 1, -1]]) || equation == ints(&[&[0, -1, 1]]));
    assert_eq!(h.inequalities.len(), 2);
}

#[test]
fn lp_optimum_is_exact() {
    // Maximize x + y subject to 0 <= x, y and 3x + 3y <= 1: value 1/3.
    let inequalities = [row(&[0, 1, 0]), row(&[0, 0, 1]), row(&[1, -3, -3])];
    let solution = solve_lp(&Lp {
        inequalities: &inequalities,
        equations: &[],
        objective: &row(&[0, 1, 1]),
        sense: Sense::Maximize,
    })
    .unwrap();
    assert_eq!(solution.status, LpStatus::Optimal);
    assert_eq!(solution.value, q(1, 3));
    assert_eq!(solution.point.len(), 2);
    let sum = Rational::from(&solution.point[0] + &solution.point[1]);
    assert_eq!(sum, q(1, 3));
}

#[test]
fn lp_with_equations_and_minimization() {
    // Minimize x subject to x + y = 5, 0 <= y <= 2: x = 3.
    let inequalities = [row(&[0, 0, 1]), row(&[2, 0, -1])];
    let equations = [row(&[-5, 1, 1])];
    let solution = solve_lp(&Lp {
        inequalities: &inequalities,
        equations: &equations,
        objective: &row(&[0, 1, 0]),
        sense: Sense::Minimize,
    })
    .unwrap();
    assert_eq!(solution.status, LpStatus::Optimal);
    assert_eq!(solution.value, Rational::from(3));
    assert_eq!(solution.point, [Rational::from(3), Rational::from(2)]);
}

#[test]
fn infeasible_and_unbounded_lps() {
    // x >= 1 and x <= 0.
    let infeasible = [row(&[-1, 1]), row(&[0, -1])];
    let status = solve_lp(&Lp {
        inequalities: &infeasible,
        equations: &[],
        objective: &row(&[0, 0]),
        sense: Sense::Maximize,
    })
    .unwrap()
    .status;
    assert!(matches!(
        status,
        LpStatus::Inconsistent | LpStatus::StructurallyInconsistent
    ));
    // Maximize x subject to x >= 0.
    let open = [row(&[0, 1])];
    let status = solve_lp(&Lp {
        inequalities: &open,
        equations: &[],
        objective: &row(&[0, 1]),
        sense: Sense::Maximize,
    })
    .unwrap()
    .status;
    assert!(matches!(
        status,
        LpStatus::DualInconsistent | LpStatus::StructurallyDualInconsistent | LpStatus::Unbounded
    ));
}

#[test]
fn feasibility_of_a_strict_cone_system() {
    // Julia's regularity test asks whether {w : coeff·w >= 1 for all rows}
    // is nonempty. Rows (1, -1) and (-1, 1) cannot both be positive.
    let feasible = [row(&[-1, 1, 0]), row(&[-1, 0, 1])];
    let infeasible = [row(&[-1, 1, -1]), row(&[-1, -1, 1])];
    let objective = row(&[0, 0, 0]);
    let status = |rows: &[Vec<Rational>]| {
        solve_lp(&Lp {
            inequalities: rows,
            equations: &[],
            objective: &objective,
            sense: Sense::Maximize,
        })
        .unwrap()
        .status
    };
    assert_eq!(status(&feasible), LpStatus::Optimal);
    assert_ne!(status(&infeasible), LpStatus::Optimal);
}

#[test]
fn shape_errors() {
    assert!(matches!(hull_inequalities(&[], 2), Err(CddError::Shape(_))));
    assert!(matches!(
        hull_inequalities(&[1, 2, 3], 2),
        Err(CddError::Shape(_))
    ));
    assert!(matches!(
        hull_inequalities(&[1], 0),
        Err(CddError::Shape(_))
    ));
    let ragged = [row(&[0, 1]), row(&[0, 1, 2])];
    assert!(matches!(
        solve_lp(&Lp {
            inequalities: &ragged,
            equations: &[],
            objective: &row(&[0, 1]),
            sense: Sense::Maximize,
        }),
        Err(CddError::Shape(_))
    ));
    assert!(matches!(
        solve_lp(&Lp {
            inequalities: &[],
            equations: &[],
            objective: &[],
            sense: Sense::Maximize,
        }),
        Err(CddError::Shape(_))
    ));
}

#[test]
fn single_point_hull_has_the_trivial_row() {
    // The case documented on `hull_inequalities`: d equations plus `1 >= 0`.
    let h = hull_inequalities(&[2, -1, 5, 2, -1, 5], 3).unwrap();
    assert_eq!(h.equations.len(), 3);
    assert_eq!(normalized(&h.inequalities), ints(&[&[1, 0, 0, 0]]));
}

#[test]
fn cddlib_and_rug_share_one_gmp() {
    use gmp_mpfr_sys::gmp;
    // The C code calls the very `mpq_set` that rug links, and was compiled
    // against the same gmp.h (not mini-gmp or a system header).
    let (mpq_set, version) = ffi::c_gmp();
    let rug_mpq_set: unsafe extern "C" fn(*mut gmp::mpq_t, *const gmp::mpq_t) = gmp::mpq_set;
    assert_eq!(mpq_set, rug_mpq_set as usize);
    assert_eq!(
        version,
        gmp::VERSION * 10000 + gmp::VERSION_MINOR * 100 + gmp::VERSION_PATCHLEVEL
    );
    // A multi-limb value allocated by rug, copied in by cddlib and written
    // back by cddlib into rug-owned mpq_t values, which rug then frees.
    let q = Rational::from((
        Integer::from(1) << 200,
        Integer::from(Integer::u_pow_u(3, 120)),
    ));
    let inequalities = [
        vec![q.clone(), Rational::from(-1)],
        vec![Rational::new(), Rational::from(1)],
    ];
    let solution = solve_lp(&Lp {
        inequalities: &inequalities,
        equations: &[],
        objective: &row(&[0, 1]),
        sense: Sense::Maximize,
    })
    .unwrap();
    assert_eq!(solution.status, LpStatus::Optimal);
    assert_eq!(solution.value, q);
    assert_eq!(solution.point, [q]);
}

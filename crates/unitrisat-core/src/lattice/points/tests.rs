use super::*;
use crate::hull::facets;

fn matrix(columns: usize, values: &[i64]) -> Matrix {
    Matrix::new(columns, values.to_vec()).unwrap()
}

fn points(vertices: &Matrix) -> Vec<i64> {
    let facets = facets(vertices).unwrap();
    lattice_points(vertices, &facets).unwrap().values().to_vec()
}

/// Julia's scan: every point of the bounding box in `Iterators.product`
/// order, kept if it satisfies every facet (checked in `i128`, enough for the
/// coordinates used here).
fn brute_force(vertices: &Matrix, facets: &[Facet]) -> Vec<i64> {
    let (low, high) = bounding_box(vertices);
    let mut x = low.clone();
    let mut inside = Vec::new();
    loop {
        if facets.iter().all(|f| {
            let dot: i128 = f
                .normal
                .iter()
                .zip(&x)
                .map(|(&a, &b)| i128::from(a) * i128::from(b))
                .sum();
            dot <= i128::from(f.bound)
        }) {
            inside.extend_from_slice(&x);
        }
        let Some(j) = (0..x.len()).find(|&j| x[j] < high[j]) else {
            return inside;
        };
        x[j] += 1;
        x[..j].copy_from_slice(&low[..j]);
    }
}

/// Julia's `points.tsv` for P0.3 baseline cases
/// (`outs/lean-baseline/t1/<job>/<case>/`): vertices and lattice points in
/// Julia's order, both row-major.
#[allow(clippy::type_complexity, reason = "test fixtures")]
const JULIA: &[(&str, usize, &[i64], &[i64])] = &[
    (
        "w-dense-2simplex4/2simplex4-r1-unimodular",
        4,
        &[0, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 2],
        &[
            0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 0, 2, 0, 0, 0, 0, 1, 0, 1,
            0, 1, 0, 0, 1, 1, 0, 0, 0, 2, 0, 0, 0, 0, 1, 1, 0, 0, 1, 0, 1, 0, 1, 0, 0, 1, 1, 0, 0,
            0, 2,
        ],
    ),
    (
        "w-high-6d-v9-r4/v9-r4-unimodular",
        6,
        &[
            0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0,
            0, 0, 2, -2, 1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0, 5, -5, 3, -1, -1,
        ],
        &[
            0, 5, -5, 3, -1, -1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0,
            0, 0, 0, 2, -2, 1, 0, 0, 0, 1, -1, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0,
            0, 0, 0, 1,
        ],
    ),
    (
        "w-high-5d-v13-r9/v13-r9-unimodular",
        5,
        &[
            0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0,
            1, 2, -3, 1, 5, -4, 2, -4, 1, 6, -4,
        ],
        &[
            2, -3, 1, 5, -4, 2, -4, 1, 6, -4, 1, -2, 1, 3, -2, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1,
            0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1,
        ],
    ),
    (
        "w-mid-4d-v11-r5/v11-r5-unimodular",
        4,
        &[0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 7, 7, 10, 11],
        &[
            0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 1, 1, 1, 1, 2, 2, 3, 3, 7, 7, 10, 11,
        ],
    ),
    (
        "chunk-3d-v20/v20-r1-unimodular",
        3,
        &[0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, 20],
        &[
            0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, 1, 0, 1, 2, 0, 1, 3, 0, 1, 4, 0, 1, 5, 0, 1, 6, 0, 1,
            7, 0, 1, 8, 0, 1, 9, 0, 1, 10, 0, 1, 11, 0, 1, 12, 0, 1, 13, 0, 1, 14, 0, 1, 15, 0, 1,
            16, 0, 1, 17, 0, 1, 18, 0, 1, 19, 0, 1, 20,
        ],
    ),
    (
        "chunk-5d-v10/v10-r1-unimodular",
        5,
        &[
            0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 1, -1, 1, 0,
            0, 0, 0, 0, 0, 1, 1, -1, 2, 0, -1, 0, -1, 2, 1, -1, 0, -1, 0, 1, 1,
        ],
        &[
            1, -1, 2, 0, -1, 0, -1, 2, 1, -1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, -1,
            1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, -1, 1, 1, 0, 0, 0, 0, 0, 1, 0, -1, 0, 1, 1,
        ],
    ),
    (
        "w-small-3d-v8/v8-r10-nondegenerate",
        3,
        &[0, 0, 0, 1, 0, 0, 0, 1, 0, 6, 7, 8],
        &[
            0, 0, 0, 1, 0, 0, 0, 1, 0, 1, 1, 1, 2, 2, 2, 3, 4, 4, 6, 7, 8,
        ],
    ),
];

#[test]
fn equal_to_julia_on_baseline_cases() {
    for &(name, columns, vertices, expected) in JULIA {
        assert_eq!(points(&matrix(columns, vertices)), expected, "{name}");
    }
}

#[test]
fn unit_cube_in_julia_order() {
    let cube = crate::families::hypercube(3).unwrap();
    assert_eq!(
        points(&cube),
        [
            0, 0, 0, 1, 0, 0, 0, 1, 0, 1, 1, 0, 0, 0, 1, 1, 0, 1, 0, 1, 1, 1, 1, 1
        ]
    );
}

#[test]
fn one_dimensional_segment() {
    assert_eq!(points(&matrix(1, &[5, -2])), [-2, -1, 0, 1, 2, 3, 4, 5]);
}

#[test]
fn lines_are_cut_by_facets_of_both_signs() {
    // The triangle (0,0), (4,1), (1,3): facets with positive and negative
    // first coefficients, and line bounds at non-integral quotients.
    let triangle = matrix(2, &[0, 0, 4, 1, 1, 3]);
    assert_eq!(
        points(&triangle),
        [0, 0, 1, 1, 2, 1, 3, 1, 4, 1, 1, 2, 2, 2, 1, 3]
    );
}

/// A small deterministic generator (as in the hull tests).
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }

    fn coordinate(&mut self, span: i64) -> i64 {
        i64::try_from(self.below(u64::try_from(2 * span + 1).unwrap())).unwrap() - span
    }
}

#[test]
fn random_polytopes_match_the_box_scan() {
    let mut rng = Lcg(0x5eed_2003);
    let mut checked = 0;
    for trial in 0..600 {
        let dimension = 1 + trial % 5;
        let span = [9, 6, 4, 2, 1][trial % 5];
        let count = dimension + 1 + usize::try_from(rng.below(6)).unwrap();
        let values: Vec<i64> = (0..count * dimension)
            .map(|_| rng.coordinate(span))
            .collect();
        let vertices = matrix(dimension, &values);
        if crate::lattice::affine_rank(&vertices) < dimension {
            continue;
        }
        let facets = facets(&vertices).unwrap();
        let fast = lattice_points(&vertices, &facets).unwrap();
        assert_eq!(fast.values(), brute_force(&vertices, &facets), "{values:?}");
        for vertex in vertices.iter_rows() {
            assert!(fast.iter_rows().any(|p| p == vertex), "{values:?}");
        }
        checked += 1;
    }
    assert!(checked > 400, "{checked}");
}

#[test]
fn shifted_polytopes_have_shifted_points() {
    let shift = [-7_i64, 1_000_000_000_000, -3];
    let moved = |values: &[i64]| -> Vec<i64> {
        values
            .iter()
            .zip(shift.iter().cycle())
            .map(|(x, s)| x + s)
            .collect()
    };
    let simplex = crate::families::scaled_simplex(3, 2).unwrap();
    assert_eq!(
        points(&matrix(3, &moved(simplex.values()))),
        moved(&points(&simplex))
    );
}

#[test]
fn errors() {
    let segment = matrix(1, &[0, 1]);
    let wrong = Facet {
        normal: vec![1, 0],
        bound: 1,
    };
    assert_eq!(
        lattice_points(&segment, &[wrong]),
        Err(PointsError::Dimension {
            expected: 1,
            found: 2
        })
    );
    let outside = Facet {
        normal: vec![1],
        bound: -1,
    };
    assert_eq!(
        lattice_points(&segment, &[outside]),
        Err(PointsError::Empty)
    );
    let long = matrix(1, &[0, i64::try_from(MAX_VALUES).unwrap()]);
    assert_eq!(lattice_points(&long, &[]), Err(PointsError::TooManyPoints));
    // Three outer terms of about 2^126 each overflow i128.
    let far = matrix(4, &[0, i64::MIN, i64::MIN, i64::MIN]);
    let steep = Facet {
        normal: vec![1, i64::MAX, i64::MAX, i64::MAX],
        bound: 0,
    };
    assert_eq!(lattice_points(&far, &[steep]), Err(PointsError::Overflow));
}

#[test]
fn values_near_the_i128_limit_are_exact() {
    // bound - normal · x = i64::MAX + 2^127 - 2^64 - x_0 fits i128 only just.
    let corner = matrix(3, &[-1, i64::MIN, i64::MIN, 1, i64::MIN, i64::MIN]);
    let edge = Facet {
        normal: vec![-1, i64::MAX, i64::MAX],
        bound: i64::MIN,
    };
    let expected = brute_force(&corner, std::slice::from_ref(&edge));
    assert_eq!(lattice_points(&corner, &[edge]).unwrap().values(), expected);
}

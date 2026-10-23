use super::*;
use crate::lattice::project_to_full_dimension;
use std::collections::BTreeSet;

fn matrix(columns: usize, values: &[i64]) -> Matrix {
    Matrix::new(columns, values.to_vec()).unwrap()
}

fn facet_set(facets: &[Facet]) -> BTreeSet<Facet> {
    facets.iter().cloned().collect()
}

fn rows(rows: &[(i64, &[i64])]) -> BTreeSet<Facet> {
    rows.iter()
        .map(|&(bound, normal)| Facet {
            normal: normal.to_vec(),
            bound,
        })
        .collect()
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 { a.abs() } else { gcd(b, a % b) }
}

/// Facets that Julia's `halfspaces(hrep(polyhedron(vrep(V),
/// CDDLib.Library(:exact))))` returns for P0.3 baseline cases
/// (`outs/lean-baseline/t1/<job>/<case>/vertices.tsv`), scaled to coprime
/// integers: `(bound, normal)` for `normal · x <= bound`.
#[allow(clippy::type_complexity, reason = "test fixtures")]
const JULIA: &[(&str, usize, &[i64], &[(i64, &[i64])])] = &[
    (
        "w-dense-2simplex4/2simplex4-r1-unimodular",
        4,
        &[0, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 2],
        &[
            (2, &[1, 1, 1, 1]),
            (0, &[-1, 0, 0, 0]),
            (0, &[0, -1, 0, 0]),
            (0, &[0, 0, -1, 0]),
            (0, &[0, 0, 0, -1]),
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
            (0, &[0, 0, -1, -2, 0, 0]),
            (0, &[0, -1, 0, 0, 0, 0]),
            (0, &[0, 0, 0, -1, 0, 0]),
            (0, &[0, -1, 0, 0, -5, 0]),
            (0, &[0, -1, 0, 0, 0, -5]),
            (0, &[0, 0, 0, -1, -3, 0]),
            (0, &[0, 0, 0, -1, 0, -3]),
            (0, &[0, 0, -1, -2, -1, 0]),
            (0, &[0, 0, -1, -2, 0, -1]),
            (0, &[0, -1, -1, 0, 0, 0]),
            (1, &[1, 1, 1, 1, 1, 1]),
            (0, &[-1, 0, 0, 0, 0, 0]),
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
            (0, &[0, -3, 0, -2, 0]),
            (0, &[0, -1, -4, 0, 0]),
            (0, &[-2, -1, 0, 0, 0]),
            (0, &[-1, 0, 0, 0, 0]),
            (0, &[0, 0, -1, 0, 0]),
            (0, &[0, 0, 0, -1, 0]),
            (0, &[0, 0, 0, -4, -5]),
            (0, &[0, -2, 0, -2, -1]),
            (0, &[-2, 0, 0, 0, -1]),
            (0, &[0, 0, -4, 0, -1]),
            (1, &[1, 1, 1, 1, 1]),
        ],
    ),
    (
        "w-mid-4d-v11-r5/v11-r5-unimodular",
        4,
        &[0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 7, 7, 10, 11],
        &[
            (11, &[11, 11, 11, -23]),
            (0, &[-11, 0, 0, 7]),
            (0, &[0, -11, 0, 7]),
            (0, &[0, 0, -11, 10]),
            (0, &[0, 0, 0, -1]),
        ],
    ),
    (
        "chunk-3d-v20/v20-r1-unimodular",
        3,
        &[0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, 20],
        &[
            (1, &[1, 1, 0]),
            (0, &[-1, 0, 0]),
            (0, &[0, -20, 1]),
            (0, &[0, 0, -1]),
        ],
    ),
];

#[test]
fn equal_to_julia_on_baseline_cases() {
    for &(name, columns, vertices, expected) in JULIA {
        let facets = facets(&matrix(columns, vertices)).unwrap();
        assert_eq!(facet_set(&facets), rows(expected), "{name}");
        assert_eq!(facets.len(), expected.len(), "{name}: duplicate rows");
    }
}

#[test]
fn cube_facets() {
    let mut cube = Vec::new();
    for i in 0..16_i64 {
        cube.extend([i & 1, (i >> 1) & 1, (i >> 2) & 1, (i >> 3) & 1]);
    }
    let mut expected = BTreeSet::new();
    for axis in 0..4 {
        let mut normal = vec![0; 4];
        normal[axis] = 1;
        expected.insert(Facet {
            normal: normal.clone(),
            bound: 1,
        });
        normal[axis] = -1;
        expected.insert(Facet { normal, bound: 0 });
    }
    assert_eq!(facet_set(&facets(&matrix(4, &cube)).unwrap()), expected);
}

#[test]
fn rows_scale_to_coprime_integers() {
    let q = |n: i64, d: u64| Rational::from((n, d));
    // 1/2 - x/3 >= 0 is 2x <= 3.
    let row = [q(1, 2), q(-1, 3), q(0, 1)];
    assert_eq!(
        facet(&row),
        Ok(Some(Facet {
            normal: vec![2, 0],
            bound: 3
        }))
    );
    // 4 - 6x + 2y >= 0 is 3x - y <= 2.
    let row = [q(4, 1), q(-6, 1), q(2, 1)];
    assert_eq!(
        facet(&row),
        Ok(Some(Facet {
            normal: vec![3, -1],
            bound: 2
        }))
    );
    // 2^63 fits only as the negated normal entry i64::MIN.
    let two_63 = || Rational::from(Integer::from(i64::MAX) + 1u8);
    assert_eq!(facet(&[two_63(), q(-1, 1)]), Err(HullError::Overflow));
    assert_eq!(facet(&[q(1, 1), -two_63()]), Err(HullError::Overflow));
    assert_eq!(
        facet(&[q(1, 1), two_63()]),
        Ok(Some(Facet {
            normal: vec![i64::MIN],
            bound: 1
        }))
    );
    let max = [q(i64::MAX, 1), q(-1, 1)];
    assert_eq!(
        facet(&max),
        Ok(Some(Facet {
            normal: vec![1],
            bound: i64::MAX
        }))
    );
}

#[test]
fn trivial_rows_are_dropped() {
    // R28: cddlib can emit [1, 0, ..., 0] (1 >= 0), which Julia keeps as the
    // halfspace 0 · x <= 1 (seen for a single point in Z^2).
    let zero = || Rational::from(0);
    assert_eq!(facet(&[Rational::from(1), zero(), zero()]), Ok(None));
    assert_eq!(facet(&[zero(), zero()]), Ok(None));
    assert_eq!(
        facet(&[Rational::from(-1), zero()]),
        Err(HullError::Malformed("an infeasible inequality"))
    );
    assert_eq!(facet(&[]), Err(HullError::Malformed("empty row")));
}

#[test]
fn lower_dimensional_input_needs_the_projection() {
    // Julia's hrep for these has equations: a segment in Z^3 and a triangle
    // in Z^4 (P0.3 Julia, see the P2.2 claim log).
    let segment = matrix(3, &[0, 0, 0, 1, 2, 3]);
    let triangle = matrix(4, &[0, 0, 0, 0, 1, 0, 0, 1, 0, 1, 0, 1]);
    let point = matrix(2, &[1, 1, 1, 1]);
    for (vertices, rank) in [(&segment, 1), (&triangle, 2)] {
        assert_eq!(facets(vertices), Err(HullError::NotFullDimensional));
        let projected = project_to_full_dimension(vertices).unwrap().unwrap();
        assert_eq!(projected.columns(), rank);
        assert_eq!(facets(&projected).unwrap().len(), rank + 1);
    }
    assert_eq!(facets(&point), Err(HullError::NotFullDimensional));
}

/// Deterministic pseudo-random numbers (64-bit LCG, high bits).
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n
    }

    fn coordinate(&mut self, span: i64) -> i64 {
        self.below(2 * span as u64 + 1) as i64 - span
    }
}

/// Determinant by cofactor expansion (small matrices only).
fn det(m: &[Vec<i128>]) -> i128 {
    if m.is_empty() {
        return 1;
    }
    m[0].iter()
        .enumerate()
        .map(|(j, x)| {
            let minor: Vec<Vec<i128>> = m[1..]
                .iter()
                .map(|r| [&r[..j], &r[j + 1..]].concat())
                .collect();
            let term = x * det(&minor);
            if j % 2 == 0 { term } else { -term }
        })
        .sum()
}

/// Brute-force facets of the hull of `points`: every hyperplane through `d`
/// of the points that has all points on one side and spans a `d - 1` face.
fn brute_force_facets(points: &[Vec<i64>]) -> BTreeSet<Facet> {
    let d = points[0].len();
    let mut found = BTreeSet::new();
    let mut subset: Vec<usize> = (0..d).collect();
    loop {
        let base = &points[subset[0]];
        let diffs: Vec<Vec<i128>> = subset[1..]
            .iter()
            .map(|&i| {
                points[i]
                    .iter()
                    .zip(base)
                    .map(|(&a, &b)| i128::from(a - b))
                    .collect()
            })
            .collect();
        // Generalized cross product of the d - 1 differences.
        let normal: Vec<i128> = (0..d)
            .map(|j| {
                let minor: Vec<Vec<i128>> = diffs
                    .iter()
                    .map(|r| [&r[..j], &r[j + 1..]].concat())
                    .collect();
                if j % 2 == 0 {
                    det(&minor)
                } else {
                    -det(&minor)
                }
            })
            .collect();
        if normal.iter().any(|&x| x != 0) {
            let dot = |p: &Vec<i64>| -> i128 {
                p.iter()
                    .zip(&normal)
                    .map(|(&x, &n)| i128::from(x) * n)
                    .sum()
            };
            let bound = dot(base);
            for sign in [1, -1] {
                if points.iter().all(|p| sign * dot(p) <= sign * bound) {
                    let g = normal
                        .iter()
                        .fold(bound, |g, &x| gcd(g as i64, x as i64) as i128);
                    found.insert(Facet {
                        normal: normal.iter().map(|&x| (sign * x / g) as i64).collect(),
                        bound: (sign * bound / g) as i64,
                    });
                }
            }
        }
        // Next combination of d indices out of points.len().
        let n = points.len();
        let Some(k) = (0..d).rev().find(|&k| subset[k] < n - d + k) else {
            break;
        };
        subset[k] += 1;
        for j in k + 1..d {
            subset[j] = subset[j - 1] + 1;
        }
    }
    found
}

#[test]
fn random_hulls_match_brute_force() {
    let mut rng = Lcg(0x5eed);
    let mut checked = 0;
    for _ in 0..300 {
        let d = 2 + rng.below(3) as usize;
        let n = d + 1 + rng.below(6) as usize;
        let points: Vec<Vec<i64>> = (0..n)
            .map(|_| (0..d).map(|_| rng.coordinate(4)).collect())
            .collect();
        let values: Vec<i64> = points.iter().flatten().copied().collect();
        let vertices = matrix(d, &values);
        if crate::lattice::affine_rank(&vertices) < d {
            assert_eq!(facets(&vertices), Err(HullError::NotFullDimensional));
            continue;
        }
        let facets = facets(&vertices).unwrap();
        assert_eq!(facets.len(), facet_set(&facets).len(), "duplicates");
        for f in &facets {
            let g = f.normal.iter().fold(f.bound, |g, &x| gcd(g, x));
            assert_eq!(g, 1, "{f:?} is not coprime");
        }
        assert_eq!(
            facet_set(&facets),
            brute_force_facets(&points),
            "{points:?}"
        );
        checked += 1;
    }
    assert!(checked > 200, "only {checked} full-dimensional samples");
}

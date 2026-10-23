use super::*;

#[test]
fn triangles_sharing_an_edge_do_not_intersect() {
    let points = [0, 0, 1, 0, 0, 1, 1, 1];
    let simplices = [0, 1, 2, 1, 2, 3];
    assert_eq!(intersecting_pairs(&points, 2, &simplices), Ok(vec![]));
}

#[test]
fn overlapping_triangles_intersect() {
    // Unit square. (0,1,3) and (0,2,3) only share the diagonal; each overlaps
    // the lower-left triangle (0,1,2).
    let points = [0, 0, 1, 0, 0, 1, 1, 1];
    let simplices = [0, 1, 3, 0, 2, 3, 0, 1, 2];
    let pairs = intersecting_pairs(&points, 2, &simplices).unwrap();
    assert_eq!(pairs, vec![(0, 2), (1, 2)]);
}

#[test]
fn rejects_bad_shapes_and_labels() {
    assert_eq!(
        intersecting_pairs(&[0, 0, 1], 2, &[]),
        Err(IntersectError::PointShape {
            len: 3,
            dimension: 2
        })
    );
    assert_eq!(
        intersecting_pairs(&[0, 0], 2, &[0, 1]),
        Err(IntersectError::SimplexShape {
            len: 2,
            vertices: 3
        })
    );
    assert_eq!(
        intersecting_pairs(&[0, 0], 2, &[0, 0, 5]),
        Err(IntersectError::PointOutOfRange {
            simplex: 0,
            label: 5,
            points: 1
        })
    );
    assert_eq!(
        intersecting_pairs(&[], 11, &[]),
        Err(IntersectError::UnsupportedDimension(11))
    );
}

/// All `k`-subsets of `0..n`, computed independently of the generated tables.
fn combos(n: usize, k: usize) -> Vec<Vec<usize>> {
    if k == 0 {
        return vec![vec![]];
    }
    let mut out = Vec::new();
    for first in 0..n {
        for mut rest in combos(n, k - 1) {
            if rest.iter().all(|&x| x > first) {
                rest.insert(0, first);
                out.push(rest);
            }
        }
    }
    out
}

fn separated_full<T: Lane, const D: usize, const V: usize>(
    a: &[[T; D]; V],
    b: &[[T; D]; V],
    axis: &[T; D],
) -> bool {
    let (min_a, max_a) = extent(a, axis);
    let (min_b, max_b) = extent(b, axis);
    max_a <= min_b || max_b <= min_a
}

/// Reference predicate (Julia `_intersect_reference`): every facet normal and
/// every cross axis via full determinants, no prefilter, no witness, no minor
/// reuse, no early exit inside the separation test.
fn reference<T: Lane, const D: usize, const V: usize>(
    a: &Prepared<T, D, V>,
    b: &Prepared<T, D, V>,
) -> bool
where
    Dims: Gcross<D>,
{
    for n in a.normals.iter().chain(&b.normals) {
        if !is_zero(n) && separated_full(&a.verts, &b.verts, n) {
            return false;
        }
    }
    for k in 1..D - 1 {
        let l = D - 1 - k;
        for fa in combos(V, k + 1) {
            for fb in combos(V, l + 1) {
                let mut span = [[T::ZERO; D]; D];
                for j in 0..k {
                    span[j] = sub_vec(&a.verts[fa[j + 1]], &a.verts[fa[0]]);
                }
                for j in 0..l {
                    span[k + j] = sub_vec(&b.verts[fb[j + 1]], &b.verts[fb[0]]);
                }
                let axis = Dims::gcross(&span);
                if !is_zero(&axis) && separated_full(&a.verts, &b.verts, &axis) {
                    return false;
                }
            }
        }
    }
    true
}

/// Deterministic pseudo-random coordinates in `-range..=range`.
struct Lcg(u64);

impl Lcg {
    fn coord(&mut self, range: i64) -> i64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) % (2 * range as u64 + 1)) as i64 - range
    }
}

fn random_simplex<T: Lane, const D: usize, const V: usize>(
    rng: &mut Lcg,
    range: i64,
) -> Prepared<T, D, V>
where
    Dims: Gcross<D>,
{
    let mut verts = [[T::ZERO; D]; V];
    for v in &mut verts {
        for c in v.iter_mut() {
            *c = T::from_i64(rng.coord(range));
        }
    }
    prepare(verts).unwrap()
}

fn check_against_reference<const D: usize, const V: usize>(trials: usize, range: i64)
where
    Dims: Gcross<D> + CrossAxes<D, V>,
{
    let mut rng = Lcg(0x5eed + D as u64);
    let mut intersecting = 0;
    for _ in 0..trials {
        let s1 = random_simplex::<i64, D, V>(&mut rng, range);
        let s2 = random_simplex::<i64, D, V>(&mut rng, range);
        let expected = reference(&s1, &s2);
        assert_eq!(intersects(&s1, &s2), Some(expected), "D={D}");
        assert_eq!(intersects(&s2, &s1), Some(expected), "D={D} swapped");
        let w1 = prepare(s1.verts.map(|v| v.map(i128::from))).unwrap();
        let w2 = prepare(s2.verts.map(|v| v.map(i128::from))).unwrap();
        assert_eq!(intersects(&w1, &w2), Some(expected), "D={D} i128");
        intersecting += usize::from(expected);
    }
    // Both outcomes must actually occur, or the comparison proves little.
    assert!(
        intersecting > 0 && intersecting < trials,
        "D={D}: {intersecting}/{trials}"
    );
}

#[test]
fn kernel_matches_reference_d2() {
    check_against_reference::<2, 3>(3000, 3);
}

#[test]
fn kernel_matches_reference_d3() {
    check_against_reference::<3, 4>(2000, 3);
}

#[test]
fn kernel_matches_reference_d4() {
    check_against_reference::<4, 5>(1000, 3);
}

#[test]
fn kernel_matches_reference_d5() {
    check_against_reference::<5, 6>(400, 2);
}

#[test]
fn kernel_matches_reference_d6() {
    check_against_reference::<6, 7>(150, 2);
}

#[test]
fn coordinate_bounds_select_lane_or_reject() {
    // Within the i64 bound: exact i64 lane.
    let b64 = COORD_BOUND_I64[3];
    let b128 = COORD_BOUND_I128[3];
    assert!(b64 > 0 && b128 > b64);
    let points = |x: i64| vec![0, 0, 0, x, 0, 0, 0, x, 0, 0, 0, x];
    assert!(intersecting_pairs(&points(b64), 3, &[0, 1, 2, 3]).is_ok());
    // Above it: still exact via i128.
    assert!(intersecting_pairs(&points(b64 + 1), 3, &[0, 1, 2, 3]).is_ok());
    // Above the i128 bound: rejected, never wrapped.
    assert!(matches!(
        intersecting_pairs(&points(b128 + 1), 3, &[0, 1, 2, 3]),
        Err(IntersectError::CoordinateTooLarge { .. })
    ));
}

/// Exact determinant by fraction-free Bareiss elimination in `i128`,
/// independent of the generated Laplace code.
fn bareiss(mut m: Vec<Vec<i128>>) -> i128 {
    let n = m.len();
    let mut sign = 1;
    let mut prev = 1;
    for k in 0..n {
        if m[k][k] == 0 {
            match (k + 1..n).find(|&r| m[r][k] != 0) {
                Some(r) => {
                    m.swap(k, r);
                    sign = -sign;
                }
                None => return 0,
            }
        }
        for i in k + 1..n {
            for j in k + 1..n {
                m[i][j] = (m[i][j] * m[k][k] - m[i][k] * m[k][j]) / prev;
            }
        }
        prev = m[k][k];
    }
    sign * m[n - 1][n - 1]
}

/// Component `i` of the generated cross product must equal the cofactor
/// `det([e_i; span rows])`.
fn check_gcross<const D: usize>(rng: &mut Lcg, range: i64)
where
    Dims: Gcross<D>,
{
    for _ in 0..300 {
        let mut span = [[0_i64; D]; D];
        for row in span.iter_mut().take(D - 1) {
            for c in row.iter_mut() {
                *c = rng.coord(range);
            }
        }
        let normal = Dims::gcross(&span);
        for (i, &component) in normal.iter().enumerate() {
            let mut m = vec![vec![0_i128; D]; D];
            m[0][i] = 1;
            for (r, row) in span.iter().take(D - 1).enumerate() {
                for (c, &x) in row.iter().enumerate() {
                    m[r + 1][c] = i128::from(x);
                }
            }
            assert_eq!(i128::from(component), bareiss(m), "D={D} component {i}");
        }
    }
}

#[test]
fn generated_cross_products_match_bareiss() {
    let mut rng = Lcg(7);
    check_gcross::<2>(&mut rng, 5);
    check_gcross::<3>(&mut rng, 5);
    check_gcross::<4>(&mut rng, 5);
    check_gcross::<5>(&mut rng, 5);
    check_gcross::<6>(&mut rng, 5);
    check_gcross::<7>(&mut rng, 4);
    check_gcross::<8>(&mut rng, 3);
    check_gcross::<9>(&mut rng, 3);
    check_gcross::<10>(&mut rng, 2);
}

/// At the generated `i64` coordinate bound the `i64` lane must agree with the
/// `i128` lane (far below its own bound) and with the reference. Even trials
/// use only the corners `+-bound`, odd trials random values in the box.
fn check_lanes_at_bound<const D: usize, const V: usize>(trials: usize)
where
    Dims: Gcross<D> + CrossAxes<D, V>,
{
    let bound = COORD_BOUND_I64[D];
    let mut rng = Lcg(0xb0 + D as u64);
    for t in 0..trials {
        let make = |rng: &mut Lcg| {
            let mut verts = [[0_i64; D]; V];
            for c in verts.iter_mut().flatten() {
                *c = if t % 2 == 0 {
                    rng.coord(2).signum() * bound
                } else {
                    rng.coord(1 << 20) * bound / (1 << 20)
                };
            }
            verts
        };
        let (a, b) = (make(&mut rng), make(&mut rng));
        let (wide_a, wide_b) = (
            prepare(a.map(|v| v.map(i128::from))).unwrap(),
            prepare(b.map(|v| v.map(i128::from))).unwrap(),
        );
        let (a, b) = (prepare(a).unwrap(), prepare(b).unwrap());
        for (n64, n128) in a.normals.iter().zip(&wide_a.normals) {
            assert_eq!(n64.map(i128::from), *n128, "D={D} t={t} normals");
        }
        let expected = intersects(&wide_a, &wide_b).unwrap();
        assert_eq!(
            reference(&wide_a, &wide_b),
            expected,
            "D={D} t={t} reference"
        );
        assert_eq!(intersects(&a, &b), Some(expected), "D={D} t={t}");
    }
}

#[test]
fn i64_lane_is_exact_at_its_bound() {
    check_lanes_at_bound::<2, 3>(2000);
    check_lanes_at_bound::<3, 4>(1000);
    check_lanes_at_bound::<4, 5>(500);
    check_lanes_at_bound::<5, 6>(200);
    check_lanes_at_bound::<6, 7>(40);
    check_lanes_at_bound::<7, 8>(8);
}

/// Generalized cross product by Bareiss cofactors, independent of the
/// generated determinant code: component `i` is `det([e_i; span])`.
fn gcross_bareiss<const D: usize>(span: &[[i128; D]]) -> [i128; D] {
    let mut axis = [0_i128; D];
    for (i, x) in axis.iter_mut().enumerate() {
        let mut m = vec![vec![0_i128; D]; D];
        m[0][i] = 1;
        for (r, row) in span.iter().enumerate() {
            m[r + 1] = row.to_vec();
        }
        *x = bareiss(m);
    }
    axis
}

/// The reference predicate for high dimensions, with Bareiss cross products
/// for the facet normals and the cross axes.
fn reference_bareiss<const D: usize, const V: usize>(
    a: &[[i128; D]; V],
    b: &[[i128; D]; V],
) -> bool {
    let separated = |axis: &[i128; D]| axis.iter().any(|&x| x != 0) && separated_full(a, b, axis);
    for s in [a, b] {
        for omit in 0..V {
            let rest: Vec<usize> = (0..V).filter(|&t| t != omit).collect();
            let span: Vec<[i128; D]> = rest[1..]
                .iter()
                .map(|&t| sub_vec(&s[t], &s[rest[0]]))
                .collect();
            if separated(&gcross_bareiss(&span)) {
                return false;
            }
        }
    }
    for k in 1..D - 1 {
        let l = D - 1 - k;
        for fa in combos(V, k + 1) {
            for fb in combos(V, l + 1) {
                let mut span: Vec<[i128; D]> = Vec::new();
                span.extend(fa[1..].iter().map(|&t| sub_vec(&a[t], &a[fa[0]])));
                span.extend(fb[1..].iter().map(|&t| sub_vec(&b[t], &b[fb[0]])));
                if separated(&gcross_bareiss(&span)) {
                    return false;
                }
            }
        }
    }
    true
}

fn check_against_bareiss<const D: usize, const V: usize>(trials: usize)
where
    Dims: Gcross<D> + CrossAxes<D, V>,
{
    let mut rng = Lcg(0xba + D as u64);
    let mut intersecting = 0;
    for _ in 0..trials {
        let a = random_simplex::<i64, D, V>(&mut rng, 2);
        let b = random_simplex::<i64, D, V>(&mut rng, 2);
        let expected = reference_bareiss(
            &a.verts.map(|v| v.map(i128::from)),
            &b.verts.map(|v| v.map(i128::from)),
        );
        assert_eq!(intersects(&a, &b), Some(expected), "D={D}");
        intersecting += usize::from(expected);
    }
    assert!(
        intersecting > 0 && intersecting < trials,
        "D={D}: {intersecting}/{trials}"
    );
}

#[test]
fn kernel_matches_bareiss_reference_d7() {
    // Dimensions 7 to 10 share the table-driven code; the per-dimension
    // tables are also exercised by the shared-facet cases below.
    check_against_bareiss::<7, 8>(10);
}

/// The generated straight-line cross axes agree with the table-driven loop
/// used above dimension 6.
fn check_generated_cross_axes<const D: usize, const V: usize>(trials: usize, range: i64)
where
    Dims: Gcross<D> + CrossAxes<D, V>,
{
    let mut rng = Lcg(0xc0 + D as u64);
    let mut separated = 0;
    for _ in 0..trials {
        let a = random_simplex::<i64, D, V>(&mut rng, range);
        let b = random_simplex::<i64, D, V>(&mut rng, range);
        let generic = cross_separated_generic(&a.verts, &b.verts).unwrap();
        assert_eq!(
            Dims::cross_separated(&a.verts, &b.verts),
            Some(generic),
            "D={D}"
        );
        separated += usize::from(generic);
    }
    assert!(
        separated > 0 && separated < trials,
        "D={D}: {separated}/{trials}"
    );
}

#[test]
fn generated_cross_axes_match_the_table_driven_loop() {
    check_generated_cross_axes::<3, 4>(3000, 3);
    check_generated_cross_axes::<4, 5>(2000, 3);
    check_generated_cross_axes::<5, 6>(1000, 2);
    check_generated_cross_axes::<6, 7>(500, 2);
}

/// Exact sanity cases for every dimension: a simplex overlaps itself and a
/// small translate, and does not overlap its mirror image across a shared
/// facet.
fn check_shared_facet<const D: usize, const V: usize>()
where
    Dims: Gcross<D> + CrossAxes<D, V>,
{
    // Standard simplex {0, e_1, ..., e_D}, scaled by 2.
    let mut verts = [[0_i64; D]; V];
    for (i, v) in verts.iter_mut().skip(1).enumerate() {
        v[i] = 2;
    }
    // Mirror across the facet x_1 = 0: replace e_1 by -e_1.
    let mut mirror = verts;
    mirror[1][0] = -2;
    // Translate by (1, 0, ..., 0)... shifted inside the overlap region.
    let mut shifted = verts;
    for v in &mut shifted {
        v[0] += 1;
    }
    let p = |v: [[i64; D]; V]| prepare(v).unwrap();
    assert_eq!(intersects(&p(verts), &p(verts)), Some(true), "D={D} self");
    assert_eq!(
        intersects(&p(verts), &p(shifted)),
        Some(true),
        "D={D} shifted"
    );
    assert_eq!(
        intersects(&p(verts), &p(mirror)),
        Some(false),
        "D={D} mirror"
    );
    // Two simplices of the cube's staircase triangulation share a facet.
    let mut stair_a = [[0_i64; D]; V];
    let mut stair_b = [[0_i64; D]; V];
    for i in 1..V {
        stair_a[i][..i].fill(1);
        stair_b[i] = stair_a[i];
    }
    // Swap the order of coordinates 0 and 1 in the path for b.
    stair_b[1] = [0; D];
    stair_b[1][1] = 1;
    assert_eq!(
        intersects(&p(stair_a), &p(stair_b)),
        Some(false),
        "D={D} staircase"
    );
}

#[test]
fn shared_facet_and_overlap_cases_in_every_dimension() {
    check_shared_facet::<2, 3>();
    check_shared_facet::<3, 4>();
    check_shared_facet::<4, 5>();
    check_shared_facet::<5, 6>();
    check_shared_facet::<6, 7>();
    check_shared_facet::<7, 8>();
    check_shared_facet::<8, 9>();
    check_shared_facet::<9, 10>();
    check_shared_facet::<10, 11>();
}

#[test]
fn streamed_rows_are_the_collected_pairs() {
    use crate::{
        families::hypercube,
        hull::facets,
        lattice::{Restriction, lattice_points, simplices},
    };
    let cube = hypercube(3).unwrap();
    let points = lattice_points(&cube, &facets(&cube).unwrap()).unwrap();
    let labels = simplices(&points, Restriction::Nondegenerate).unwrap();
    let expected = intersecting_pairs(points.values(), 3, &labels).unwrap();
    assert!(!expected.is_empty());
    for workers in [0, 1, 3] {
        let rows = Mutex::new(Vec::new());
        let sink = |row: &[(u32, u32)]| {
            assert!(!row.is_empty() && row.iter().all(|&(i, j)| i == row[0].0 && i < j));
            rows.lock().unwrap().extend_from_slice(row);
            true
        };
        stream_pairs(points.values(), 3, &labels, workers, &sink).unwrap();
        let mut streamed = rows.into_inner().unwrap();
        streamed.sort_unstable();
        assert_eq!(streamed, expected, "{workers} workers");
    }
    // A declining sink ends the search: each worker hands over at most one
    // row.
    let calls = AtomicUsize::new(0);
    let decline = |_: &[(u32, u32)]| {
        calls.fetch_add(1, Ordering::Relaxed);
        false
    };
    stream_pairs(points.values(), 3, &labels, 2, &decline).unwrap();
    assert!((1..=2).contains(&calls.load(Ordering::Relaxed)));
    assert_eq!(
        stream_pairs(&[0, 0], 2, &[0, 0, 5], 2, &|_| true),
        Err(IntersectError::PointOutOfRange {
            simplex: 0,
            label: 5,
            points: 1
        })
    );
}

#[test]
fn a_partial_worker_start_failure_aborts_before_sink_calls() {
    let attempts = AtomicUsize::new(0);
    let rows = AtomicUsize::new(0);
    let calls = AtomicUsize::new(0);
    let result = stream_rows(
        4,
        4,
        &|_| {
            calls.fetch_add(1, Ordering::Relaxed);
            true
        },
        |_| {
            rows.fetch_add(1, Ordering::Relaxed);
            Ok(vec![(0, 1)])
        },
        |worker| {
            attempts.fetch_add(1, Ordering::Relaxed);
            if worker == 1 {
                Err(std::io::ErrorKind::WouldBlock.into())
            } else {
                Ok(std::thread::Builder::new())
            }
        },
    );
    assert_eq!(
        result,
        Err(IntersectError::ThreadSpawn(std::io::ErrorKind::WouldBlock))
    );
    assert_eq!(attempts.load(Ordering::Relaxed), 2);
    assert_eq!(rows.load(Ordering::Relaxed), 0);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    // Returning from the scope also proves that the already-created worker
    // exited and joined despite startup being aborted.
}

#[test]
fn a_panicking_stream_worker_returns_an_error() {
    assert_eq!(
        stream_rows(
            2,
            2,
            &|_| panic!("sink failure"),
            |_| Ok(vec![(0, 1)]),
            |_| { Ok(std::thread::Builder::new()) }
        ),
        Err(IntersectError::Internal)
    );
}

proptest::proptest! {
    // Random cube corners often repeat or form flat tetrahedra. Allow enough
    // rejected inputs to complete all 128 valid cases without changing checks.
    #![proptest_config(proptest::test_runner::Config {
        max_global_rejects: 4096,
        ..proptest::test_runner::Config::with_cases(128)
    })]

    /// Shrinking coordinate inputs compare both lanes against independent
    /// Bareiss cofactors, bypassing the generated normals and prefilters.
    #[test]
    fn property_both_lanes_match_independent_oracle(
        coords in proptest::collection::vec(-4_i64..=4, 40)
    ) {
        let a: [[i64; 4]; 5] = std::array::from_fn(|v| std::array::from_fn(|d| coords[4 * v + d]));
        let b: [[i64; 4]; 5] = std::array::from_fn(|v| std::array::from_fn(|d| coords[20 + 4 * v + d]));
        let aw = a.map(|v| v.map(i128::from));
        let bw = b.map(|v| v.map(i128::from));
        let volume = |s: &[[i128; 4]; 5]| bareiss(s[1..].iter().map(|v| v.iter().zip(s[0]).map(|(x, origin)| x - origin).collect()).collect());
        proptest::prop_assume!(volume(&aw) != 0 && volume(&bw) != 0);
        let expected = reference_bareiss(&aw, &bw);
        let (a, b) = (prepare(a).unwrap(), prepare(b).unwrap());
        let (aw, bw) = (prepare(aw).unwrap(), prepare(bw).unwrap());
        proptest::prop_assert_eq!(intersects(&a, &b), Some(expected));
        proptest::prop_assert_eq!(intersects(&aw, &bw), Some(expected));
        proptest::prop_assert_eq!(intersects(&b, &a), Some(expected));
    }

    /// Positive scaling preserves strict intersection. Compare random cube
    /// corners at each exact lane bound with the unscaled Bareiss oracle;
    /// keeping the oracle unscaled avoids overflow in Bareiss intermediates.
    #[test]
    fn property_coordinate_bounds_preserve_exact_answers(
        signs in proptest::collection::vec(proptest::bool::ANY, 24)
    ) {
        let a: [[i128; 3]; 4] = std::array::from_fn(|v| std::array::from_fn(|d| if signs[3 * v + d] { 1 } else { -1 }));
        let b: [[i128; 3]; 4] = std::array::from_fn(|v| std::array::from_fn(|d| if signs[12 + 3 * v + d] { 1 } else { -1 }));
        let volume = |s: &[[i128; 3]; 4]| bareiss(s[1..].iter().map(|v| v.iter().zip(s[0]).map(|(x, origin)| x - origin).collect()).collect());
        proptest::prop_assume!(volume(&a) != 0 && volume(&b) != 0);
        let expected = reference_bareiss(&a, &b);
        for bound in [COORD_BOUND_I64[3], COORD_BOUND_I64[3] + 1, COORD_BOUND_I128[3]] {
            let points: Vec<i64> = a.iter().chain(&b).flatten().map(|&x| i64::try_from(x).unwrap() * bound).collect();
            let pairs = intersecting_pairs(&points, 3, &[0,1,2,3,4,5,6,7]).unwrap();
            proptest::prop_assert_eq!(pairs == vec![(0,1)], expected);
            proptest::prop_assert!(pairs.is_empty() || pairs == vec![(0,1)]);
        }
    }

    #[test]
    fn property_outside_exact_bound_is_rejected(dimension in 2_usize..=10, negative in proptest::bool::ANY) {
        let mut points = vec![0; dimension];
        points[0] = (COORD_BOUND_I128[dimension] + 1) * if negative { -1 } else { 1 };
        let result = intersecting_pairs(&points, dimension, &[]);
        proptest::prop_assert!(matches!(result, Err(IntersectError::CoordinateTooLarge { .. })), "unexpected result: {:?}", result);
    }

    /// These triangles overlap only along the interior of their centroid
    /// segment: neither centroid alone is in the other triangle. Vary scale
    /// and translation while retaining an independent exact oracle check.
    #[test]
    fn property_segment_witness_hits_without_a_centroid_hit(scale in 1_i64..=1000, dx in -1000_i64..=1000, dy in -1000_i64..=1000) {
        let a = [[0,0], [12,0], [0,12]].map(|v| [v[0] * scale + dx, v[1] * scale + dy]);
        let b = [[7,0], [19,0], [7,12]].map(|v| [v[0] * scale + dx, v[1] * scale + dy]);
        let aw = a.map(|v| v.map(i128::from));
        let bw = b.map(|v| v.map(i128::from));
        proptest::prop_assert!(reference_bareiss(&aw, &bw));
        let (a, b) = (prepare(a).unwrap(), prepare(b).unwrap());
        proptest::prop_assert!(!strictly_inside(&a.sum, 1, &b));
        proptest::prop_assert!(!strictly_inside(&b.sum, 1, &a));
        proptest::prop_assert!(witnessed(&a, &b));
        proptest::prop_assert_eq!(intersects(&a, &b), Some(true));
        let (aw, bw) = (prepare(aw).unwrap(), prepare(bw).unwrap());
        proptest::prop_assert!(witnessed(&aw, &bw));
    }
}

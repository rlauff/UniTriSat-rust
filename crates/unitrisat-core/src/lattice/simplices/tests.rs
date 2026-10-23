use super::*;

fn matrix(columns: usize, values: &[i64]) -> Matrix {
    Matrix::new(columns, values.to_vec()).unwrap()
}

/// Determinant by cofactor expansion along the first row, in big integers.
fn reference_determinant(rows: &[Vec<Integer>]) -> Integer {
    let Some((first, rest)) = rows.split_first() else {
        return Integer::from(1);
    };
    let mut sum = Integer::new();
    for (j, a) in first.iter().enumerate() {
        if *a == 0 {
            continue;
        }
        let minor: Vec<Vec<Integer>> = rest
            .iter()
            .map(|r| {
                r.iter()
                    .enumerate()
                    .filter(|&(c, _)| c != j)
                    .map(|(_, x)| x.clone())
                    .collect()
            })
            .collect();
        let term = a * reference_determinant(&minor);
        if j % 2 == 0 {
            sum += term;
        } else {
            sum -= term;
        }
    }
    sum
}

/// All `k`-subsets of `0..n` in lexicographic order.
fn combinations(n: usize, k: usize) -> Vec<Vec<usize>> {
    fn extend(
        start: usize,
        n: usize,
        k: usize,
        prefix: &mut Vec<usize>,
        out: &mut Vec<Vec<usize>>,
    ) {
        if prefix.len() == k {
            out.push(prefix.clone());
            return;
        }
        for x in start..n {
            prefix.push(x);
            extend(x + 1, n, k, prefix, out);
            prefix.pop();
        }
    }
    let mut out = Vec::new();
    extend(0, n, k, &mut Vec::new(), &mut out);
    out
}

/// Julia's `all_simplices` with one thread, with exact determinants.
fn brute_force(points: &Matrix, restriction: Restriction) -> Vec<u32> {
    let d = points.columns();
    let rows: Vec<&[i64]> = points.iter_rows().collect();
    let mut out = Vec::new();
    for labels in combinations(points.rows(), d + 1) {
        let base = rows[labels[0]];
        let differences: Vec<Vec<Integer>> = labels[1..]
            .iter()
            .map(|&l| {
                rows[l]
                    .iter()
                    .zip(base)
                    .map(|(&p, &b)| Integer::from(p) - b)
                    .collect()
            })
            .collect();
        let det = reference_determinant(&differences);
        let keep = match restriction {
            Restriction::Nondegenerate => det != 0,
            Restriction::Unimodular => det == 1 || det == -1,
        };
        if keep {
            out.extend(labels.iter().map(|&l| u32::try_from(l).unwrap()));
        }
    }
    out
}

/// The determinant of `rows` through the production elimination, with
/// the zero vector as base: `None` if an entry does not fit `T`.
fn lane_determinant<T: Exact>(rows: &[Vec<i64>]) -> Option<T> {
    let d = rows.len();
    let mut state = Elimination::<T>::new(d);
    let origin = vec![0; d];
    for (level, row) in rows.iter().enumerate() {
        match state.load(level, row, &origin)? {
            Level::Dependent => return Some(T::difference(0, 0).unwrap()),
            Level::Pivot => assert!(level + 1 < d),
            Level::Last => assert_eq!(level + 1, d),
        }
    }
    state.last().cloned()
}

fn integer_rows(rows: &[Vec<i64>]) -> Vec<Vec<Integer>> {
    rows.iter()
        .map(|r| r.iter().copied().map(Integer::from).collect())
        .collect()
}

/// A small deterministic generator (as in the hull and points tests).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 11
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// Uniform in `-span..=span`, for `span < 2^52`.
    fn coordinate(&mut self, span: i64) -> i64 {
        i64::try_from(self.below(u64::try_from(2 * span + 1).unwrap())).unwrap() - span
    }

    /// Any `i64`.
    fn any(&mut self) -> i64 {
        i64::from_ne_bytes(((self.next() << 11) ^ self.next()).to_ne_bytes())
    }
}

/// Julia's `simplices.tsv` (one thread) for the 3-cube and for record 1 of
/// `3-polytopes/v8.txt`, from `outs/lean-baseline/t1`.
#[test]
fn equal_to_julia_on_baseline_cases() {
    let cube = crate::families::hypercube(3).unwrap();
    let cube_unimodular: [u32; 224] = [
        0, 1, 2, 4, 0, 1, 2, 5, 0, 1, 2, 6, 0, 1, 2, 7, 0, 1, 3, 4, 0, 1, 3, 5, 0, 1, 3, 6, 0, 1,
        3, 7, 0, 1, 4, 6, 0, 1, 4, 7, 0, 1, 5, 6, 0, 1, 5, 7, 0, 2, 3, 4, 0, 2, 3, 5, 0, 2, 3, 6,
        0, 2, 3, 7, 0, 2, 4, 5, 0, 2, 4, 7, 0, 2, 5, 6, 0, 2, 6, 7, 0, 3, 4, 5, 0, 3, 4, 6, 0, 3,
        5, 7, 0, 3, 6, 7, 0, 4, 5, 6, 0, 4, 5, 7, 0, 4, 6, 7, 0, 5, 6, 7, 1, 2, 3, 4, 1, 2, 3, 5,
        1, 2, 3, 6, 1, 2, 3, 7, 1, 2, 4, 5, 1, 2, 4, 6, 1, 2, 5, 7, 1, 2, 6, 7, 1, 3, 4, 5, 1, 3,
        4, 7, 1, 3, 5, 6, 1, 3, 6, 7, 1, 4, 5, 6, 1, 4, 5, 7, 1, 4, 6, 7, 1, 5, 6, 7, 2, 3, 4, 6,
        2, 3, 4, 7, 2, 3, 5, 6, 2, 3, 5, 7, 2, 4, 5, 6, 2, 4, 5, 7, 2, 4, 6, 7, 2, 5, 6, 7, 3, 4,
        5, 6, 3, 4, 5, 7, 3, 4, 6, 7, 3, 5, 6, 7,
    ];
    assert_eq!(
        simplices(&cube, Restriction::Unimodular).unwrap(),
        cube_unimodular
    );
    // The two volume-2 simplices (0,3,5,6) and (1,2,4,7) are added.
    let mut cube_nondegenerate = cube_unimodular.chunks(4).collect::<Vec<_>>();
    cube_nondegenerate.extend([&[0, 3, 5, 6][..], &[1, 2, 4, 7][..]]);
    cube_nondegenerate.sort_unstable();
    assert_eq!(
        simplices(&cube, Restriction::Nondegenerate).unwrap(),
        cube_nondegenerate.concat()
    );

    let mut v8 = vec![0, 0, 0, 1, 0, 0];
    for z in 0..=8 {
        v8.extend([0, 1, z]);
    }
    let v8 = matrix(3, &v8);
    let unimodular: Vec<u32> = (2..10).flat_map(|k| [0, 1, k, k + 1]).collect();
    assert_eq!(simplices(&v8, Restriction::Unimodular).unwrap(), unimodular);
    let nondegenerate: Vec<u32> = combinations(11, 2)
        .into_iter()
        .filter(|c| c[0] >= 2)
        .flat_map(|c| [0, 1, c[0], c[1]].map(|x| u32::try_from(x).unwrap()))
        .collect();
    assert_eq!(
        simplices(&v8, Restriction::Nondegenerate).unwrap(),
        nondegenerate
    );
}

#[test]
fn output_plugs_into_the_intersection_stage() {
    // Julia's meta.tsv for the 3-cube: 58 nondegenerate simplices with 945
    // intersecting pairs, 56 unimodular ones with 840.
    let cube = crate::families::hypercube(3).unwrap();
    for (restriction, count, pairs) in [
        (Restriction::Nondegenerate, 58, 945),
        (Restriction::Unimodular, 56, 840),
    ] {
        let labels = simplices(&cube, restriction).unwrap();
        assert_eq!(labels.len(), 4 * count);
        let found = crate::intersect::intersecting_pairs(cube.values(), 3, &labels).unwrap();
        assert_eq!(found.len(), pairs);
    }
}

#[test]
fn small_inputs() {
    // Fewer than d + 1 points.
    let segment = matrix(3, &[0, 0, 0, 1, 1, 1, 2, 3, 4]);
    assert_eq!(simplices(&segment, Restriction::Nondegenerate), Ok(vec![]));
    // A single point in a huge dimension returns before allocating d × d.
    let wide = matrix(1 << 24, &vec![0; 1 << 24]);
    assert_eq!(simplices(&wide, Restriction::Unimodular), Ok(vec![]));
    // Dimension 1: every pair of distinct points; unimodular ones are adjacent.
    let line = matrix(1, &[0, 1, 2, 2, 5]);
    assert_eq!(
        simplices(&line, Restriction::Nondegenerate).unwrap(),
        [0, 1, 0, 2, 0, 3, 0, 4, 1, 2, 1, 3, 1, 4, 2, 4, 3, 4]
    );
    assert_eq!(
        simplices(&line, Restriction::Unimodular).unwrap(),
        [0, 1, 1, 2, 1, 3]
    );
    // Exactly d + 1 points, flat and full.
    let flat = matrix(2, &[0, 0, 1, 1, 2, 2]);
    assert_eq!(simplices(&flat, Restriction::Nondegenerate), Ok(vec![]));
    let full = matrix(2, &[0, 0, 2, 0, 0, 1]);
    assert_eq!(
        simplices(&full, Restriction::Nondegenerate),
        Ok(vec![0, 1, 2])
    );
    assert_eq!(simplices(&full, Restriction::Unimodular), Ok(vec![]));
}

#[test]
fn random_point_sets_match_the_reference() {
    let mut rng = Lcg(0x5eed_2004);
    let mut kept = [0_usize; 2];
    for trial in 0..400 {
        let d = 1 + trial % 6;
        // Small spans give many dependent prefixes, large ones the fallback.
        let span = [1, 2, 3, 1 << 20, 1 << 40, 1 << 51][(trial / 6) % 6];
        let n = d + 1 + usize::try_from(rng.below([12, 10, 8, 7, 4, 3][d - 1])).unwrap();
        let values: Vec<i64> = (0..n * d).map(|_| rng.coordinate(span)).collect();
        let points = matrix(d, &values);
        for (i, restriction) in [Restriction::Unimodular, Restriction::Nondegenerate]
            .into_iter()
            .enumerate()
        {
            let expected = brute_force(&points, restriction);
            assert_eq!(
                simplices(&points, restriction).unwrap(),
                expected,
                "{d} {values:?}"
            );
            kept[i] += expected.len();
        }
    }
    assert!(kept[0] > 500 && kept[1] > 5000, "{kept:?}");
}

#[test]
fn extreme_coordinates_fall_back_to_big_integers() {
    // Differences of i64::MIN and i64::MAX do not fit i64, and products of
    // the entries do not fit i128.
    let mut rng = Lcg(0x0b16_b175);
    for trial in 0..60 {
        let d = 1 + trial % 4;
        let n = d + 1 + usize::try_from(rng.below(3)).unwrap();
        let values: Vec<i64> = (0..n * d)
            .map(|_| match rng.below(4) {
                0 => i64::MIN,
                1 => i64::MAX,
                2 => rng.coordinate(2),
                _ => rng.any(),
            })
            .collect();
        let points = matrix(d, &values);
        for restriction in [Restriction::Unimodular, Restriction::Nondegenerate] {
            assert_eq!(
                simplices(&points, restriction).unwrap(),
                brute_force(&points, restriction),
                "{d} {values:?}"
            );
        }
    }
    // A unimodular triangle far from the origin: every difference overflows.
    let far = matrix(2, &[i64::MIN, i64::MIN, i64::MAX, i64::MIN, i64::MIN, -1]);
    assert_eq!(
        simplices(&far, Restriction::Nondegenerate),
        Ok(vec![0, 1, 2])
    );
    let near = matrix(
        2,
        &[
            i64::MAX - 1,
            i64::MAX,
            i64::MAX,
            i64::MAX,
            i64::MAX,
            i64::MAX - 1,
        ],
    );
    assert_eq!(simplices(&near, Restriction::Unimodular), Ok(vec![0, 1, 2]));
}

#[test]
fn lane_determinants_match_the_reference() {
    let mut rng = Lcg(0xde7e_2004);
    let mut outcomes = [0_usize; 2];
    for trial in 0..3000 {
        let d = 1 + trial % 7;
        let span = [1, 5, 100, 10_000, 1 << 24, 1 << 40, 1 << 51][(trial / 7) % 7];
        let rows: Vec<Vec<i64>> = (0..d)
            .map(|_| (0..d).map(|_| rng.coordinate(span)).collect())
            .collect();
        let expected = reference_determinant(&integer_rows(&rows));
        let big = lane_determinant::<Integer>(&rows).unwrap();
        assert_eq!(big.clone().abs(), expected.clone().abs(), "{rows:?}");
        match lane_determinant::<i64>(&rows) {
            Some(small) => {
                assert_eq!(Integer::from(small), big, "{rows:?}");
                outcomes[0] += 1;
            }
            None => outcomes[1] += 1,
        }
        if expected.to_i64().is_none() {
            assert!(lane_determinant::<i64>(&rows).is_none(), "{rows:?}");
        }
    }
    // Both the machine lane and its fallback are exercised.
    assert!(outcomes[0] > 1500 && outcomes[1] > 300, "{outcomes:?}");
}

#[test]
fn determinants_at_the_i64_bounds() {
    let max = i64::MAX;
    let min = i64::MIN;
    // Exactly i64::MAX and i64::MIN, through products that overflow i64.
    for rows in [
        vec![vec![max, 1], vec![0, 1]],
        vec![vec![min, 1], vec![0, 1]],
        vec![vec![max, max - 1], vec![max - 1, max - 2]],
        vec![vec![1 << 32, 1], vec![(1 << 31) - 1, 1 << 31]],
        vec![vec![3_037_000_499, -1], vec![1, 3_037_000_499]],
    ] {
        let expected = reference_determinant(&integer_rows(&rows));
        let small = lane_determinant::<i64>(&rows).unwrap();
        assert_eq!(Integer::from(small), expected, "{rows:?}");
    }
    // One past: 2^63 and -2^63 - 1 do not fit, and must not wrap.
    for rows in [
        vec![vec![1 << 32, 0], vec![0, 1 << 31]],
        vec![vec![min, 1], vec![1, 1]],
        vec![vec![min, 0], vec![0, -1]],
        vec![vec![max, 1], vec![-1, 1]],
    ] {
        let expected = reference_determinant(&integer_rows(&rows));
        assert!(expected.to_i64().is_none(), "{rows:?}");
        assert_eq!(lane_determinant::<i64>(&rows), None, "{rows:?}");
        let big = lane_determinant::<Integer>(&rows).unwrap();
        assert_eq!(big.abs(), expected.abs(), "{rows:?}");
    }
}

#[test]
fn elimination_steps_at_the_bounds() {
    let step = |x: i64, p: i64, l: i64, e: i64, q: i64| {
        let mut x = x;
        x.eliminate(&p, &l, &e, &q).map(|()| x)
    };
    // In range only after the exact division.
    assert_eq!(step(i64::MAX, 4, 2, 2, 4), Some(i64::MAX - 1));
    assert_eq!(step(i64::MIN, 2, 0, 0, 2), Some(i64::MIN));
    // The i128 value 2^127 - 2^63 does not fit i64.
    assert_eq!(step(i64::MIN, i64::MIN, i64::MIN, i64::MAX, 1), None);
    assert_eq!(step(i64::MIN, -1, 0, 0, 1), None);
    assert_eq!(step(i64::MIN, 1, 0, 0, -1), None);
    assert_eq!(step(1, 1, 1, 1, 0), None);
    let mut big = Integer::from(5);
    assert_eq!(
        big.eliminate(
            &Integer::from(1),
            &Integer::new(),
            &Integer::new(),
            &Integer::new()
        ),
        None
    );
}

#[test]
fn independent_of_the_thread_count() {
    // 4 x 3 x 3 grid: 36 points, above the parallel threshold.
    let mut values = Vec::new();
    for z in 0..3 {
        for y in 0..3 {
            for x in 0..4 {
                values.extend([x, y, z]);
            }
        }
    }
    let grid = matrix(3, &values);
    for restriction in [Restriction::Unimodular, Restriction::Nondegenerate] {
        let run = |threads| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| simplices(&grid, restriction).unwrap())
        };
        let serial = run(1);
        assert_eq!(serial, brute_force(&grid, restriction));
        assert_eq!(run(4), serial);
        assert_eq!(run(16), serial);
    }
}

#[test]
fn errors_display() {
    assert_eq!(
        SimplicesError::TooManyPoints(1 << 33).to_string(),
        "8589934592 points exceed the supported maximum of 2^32 - 1"
    );
}

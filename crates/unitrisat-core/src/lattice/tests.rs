use super::*;

fn matrix(columns: usize, values: &[i64]) -> Matrix {
    Matrix::new(columns, values.to_vec()).unwrap()
}

fn rows_of(values: &[&[i64]]) -> Rows {
    values
        .iter()
        .map(|r| r.iter().map(|&x| Integer::from(x)).collect())
        .collect()
}

/// Exact determinant by cofactor expansion (small test matrices only).
fn det(m: &[Vec<Integer>]) -> Integer {
    if m.is_empty() {
        return Integer::from(1);
    }
    let mut total = Integer::new();
    for (j, x) in m[0].iter().enumerate() {
        let minor: Vec<Vec<Integer>> = m[1..]
            .iter()
            .map(|r| {
                r.iter()
                    .enumerate()
                    .filter(|&(c, _)| c != j)
                    .map(|(_, v)| v.clone())
                    .collect()
            })
            .collect();
        let term = Integer::from(x * &det(&minor));
        if j % 2 == 0 {
            total += term;
        } else {
            total -= term;
        }
    }
    total
}

fn subsets(n: usize, k: usize) -> Vec<Vec<usize>> {
    if k == 0 {
        return vec![vec![]];
    }
    (0..n)
        .flat_map(|first| {
            subsets(n, k - 1)
                .into_iter()
                .filter(move |rest| rest.iter().all(|&x| x > first))
                .map(move |mut rest| {
                    rest.insert(0, first);
                    rest
                })
        })
        .collect()
}

/// gcd of the `k x k` minors of `m`: invariant under unimodular row
/// operations, so HNF must preserve it.
fn minor_gcd(m: &Rows, k: usize) -> Integer {
    let cols = m.first().map_or(0, Vec::len);
    let mut g = Integer::new();
    for rs in subsets(m.len(), k) {
        for cs in subsets(cols, k) {
            let sub: Vec<Vec<Integer>> = rs
                .iter()
                .map(|&r| cs.iter().map(|&c| m[r][c].clone()).collect())
                .collect();
            g = g.gcd(&det(&sub));
        }
    }
    g
}

fn assert_hermite(h: &Rows, rank: usize) {
    let mut last_pivot = None;
    for (i, row) in h.iter().enumerate() {
        let pivot = row.iter().position(|x| *x != 0);
        if i >= rank {
            assert!(pivot.is_none(), "row {i} below the rank is zero");
            continue;
        }
        let p = pivot.expect("pivot row");
        assert!(last_pivot.is_none_or(|q| q < p), "pivots move right");
        assert!(row[p] > 0, "pivot positive");
        for (k, above) in h.iter().enumerate().take(i) {
            assert!(above[p] >= 0 && above[p] < row[p], "row {k} reduced");
        }
        for below in h.iter().skip(i + 1) {
            assert_eq!(below[p], 0);
        }
        last_pivot = Some(p);
    }
}

#[test]
fn hermite_form_of_known_matrices() {
    let mut m = rows_of(&[&[0, 2, 0], &[0, 2, 3]]);
    assert_eq!(hermite(&mut m, true), 2);
    assert_eq!(m, rows_of(&[&[0, 2, 0], &[0, 0, 3]]));
    let mut m = rows_of(&[&[2, 3, 6, 2], &[5, 6, 1, 6], &[8, 3, 1, 1]]);
    let original = m.clone();
    assert_eq!(hermite(&mut m, true), 3);
    assert_eq!(minor_gcd(&m, 3), minor_gcd(&original, 3));
    assert_hermite(&m, 3);
}

#[test]
fn hermite_invariants_on_random_matrices() {
    let mut state = 0x1234_5678_u64;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((state >> 33) % 13) as i64 - 6
    };
    for trial in 0..300 {
        let rows = 1 + trial % 5;
        let cols = 1 + (trial / 5) % 6;
        let mut m: Rows = (0..rows)
            .map(|_| (0..cols).map(|_| Integer::from(next())).collect())
            .collect();
        // Make some matrices rank-deficient.
        if trial % 3 == 0 && rows > 1 {
            let copy: Vec<Integer> = m[0].iter().map(|x| Integer::from(x * 2)).collect();
            m[rows - 1] = copy;
        }
        let original = m.clone();
        let mut echelon = m.clone();
        let rank = hermite(&mut m, true);
        assert_eq!(hermite(&mut echelon, false), rank);
        assert_hermite(&m, rank);
        for k in 1..=rank {
            assert_eq!(minor_gcd(&m, k), minor_gcd(&original, k), "k={k}");
        }
        if rank < rows.min(cols) {
            assert_eq!(minor_gcd(&original, rank + 1), 0);
        }
    }
}

#[test]
fn archived_examples() {
    // A nonprimitive diagonal edge and a nonprimitive axis edge.
    for vertices in [[0, 0, 2, 2], [0, 0, 2, 0]] {
        let p = project_to_full_dimension(&matrix(2, &vertices))
            .unwrap()
            .unwrap();
        assert_eq!(p, matrix(1, &[0, 2]));
    }
    // Full rank: nothing to do.
    let triangle = matrix(2, &[7, 11, 9, 13, 7, 14]);
    assert_eq!(affine_rank(&triangle), 2);
    assert_eq!(project_to_full_dimension(&triangle), Ok(None));
    // Repeated vertices: a point.
    let point = matrix(2, &[9, -5, 9, -5, 9, -5]);
    assert_eq!(affine_rank(&point), 0);
    assert_eq!(
        project_to_full_dimension(&point),
        Err(ProjectionError::Point)
    );
}

#[test]
fn embedded_simplices_keep_their_lattice_volume() {
    // The triangle conv{0, 2e1, 3e2} (normalized area 6) embedded in Z^4 by
    // x -> A x + c, where A's columns extend to a basis of Z^4.
    let a = [[1, 2], [0, 1], [3, 5], [-2, 7]];
    let c = [5, -1, 4, 0];
    let triangle = [[0, 0], [2, 0], [0, 3]];
    let mut values = Vec::new();
    for p in triangle {
        for (row, shift) in a.iter().zip(c) {
            values.push(row[0] * p[0] + row[1] * p[1] + shift);
        }
    }
    let embedded = matrix(4, &values);
    assert_eq!(affine_rank(&embedded), 2);
    let p = project_to_full_dimension(&embedded).unwrap().unwrap();
    assert_eq!((p.rows(), p.columns()), (3, 2));
    let v: Vec<&[i64]> = p.iter_rows().collect();
    assert_eq!(v[0], [0, 0]);
    let area = (v[1][0] * v[2][1] - v[1][1] * v[2][0]).abs();
    assert_eq!(area, 6);
    // A tetrahedron in a 3-dimensional subspace of Z^5.
    let simplex = matrix(
        5,
        &[0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 1, 1, 1],
    );
    assert_eq!(affine_rank(&simplex), 3);
    let p = project_to_full_dimension(&simplex).unwrap().unwrap();
    assert_eq!(p.columns(), 3);
    assert_eq!(affine_rank(&p), 3);
}

#[test]
fn projected_coordinates_must_fit_i64() {
    // Coordinates near i64::MAX: the HNF entries stay small here.
    let big = i64::MAX - 1;
    let m = matrix(2, &[big, big, big - 2, big - 2]);
    assert_eq!(
        project_to_full_dimension(&m).unwrap().unwrap(),
        matrix(1, &[0, 2])
    );
}

use super::*;

fn rows(m: &Matrix) -> Vec<Vec<i64>> {
    m.iter_rows().map(<[i64]>::to_vec).collect()
}

#[test]
fn hypercube_rows_are_lsb_first() {
    assert_eq!(rows(&hypercube(1).unwrap()), [[0], [1]]);
    assert_eq!(
        rows(&hypercube(2).unwrap()),
        [[0, 0], [1, 0], [0, 1], [1, 1]]
    );
    assert_eq!(
        rows(&hypercube(3).unwrap()),
        [
            [0, 0, 0],
            [1, 0, 0],
            [0, 1, 0],
            [1, 1, 0],
            [0, 0, 1],
            [1, 0, 1],
            [0, 1, 1],
            [1, 1, 1]
        ]
    );
    // Row i read as LSB-first binary equals i.
    for d in 1..=8 {
        let r = rows(&hypercube(d).unwrap());
        assert_eq!(r.len(), 1 << d);
        for (index, row) in r.iter().enumerate() {
            let value: i64 = row.iter().enumerate().map(|(j, b)| b << j).sum();
            assert_eq!(value, index as i64);
        }
    }
}

#[test]
fn cross_rows_interleave_signs() {
    assert_eq!(rows(&cross_polytope(1).unwrap()), [[1], [-1]]);
    assert_eq!(
        rows(&cross_polytope(3).unwrap()),
        [
            [1, 0, 0],
            [-1, 0, 0],
            [0, 1, 0],
            [0, -1, 0],
            [0, 0, 1],
            [0, 0, -1]
        ]
    );
}

#[test]
fn scaled_simplex_has_origin_last() {
    assert_eq!(
        rows(&scaled_simplex(2, 3).unwrap()),
        [[3, 0], [0, 3], [0, 0]]
    );
    assert_eq!(
        rows(&scaled_simplex(3, -2).unwrap()),
        [[-2, 0, 0], [0, -2, 0], [0, 0, -2], [0, 0, 0]]
    );
    assert_eq!(
        rows(&scaled_simplex(2, 0).unwrap()),
        [[0, 0], [0, 0], [0, 0]]
    );
}

#[test]
fn cyclic_rows_are_powers_in_order() {
    assert_eq!(
        rows(&cyclic(2, 4).unwrap()),
        [[0, 0], [1, 1], [2, 4], [3, 9]]
    );
    assert_eq!(
        rows(&cyclic(3, 5).unwrap()),
        [[0, 0, 0], [1, 1, 1], [2, 4, 8], [3, 9, 27], [4, 16, 64]]
    );
    // 2^62 fits, 2^63 does not.
    assert_eq!(cyclic(62, 3).unwrap().values().last(), Some(&(1 << 62)));
    assert_eq!(cyclic(63, 3), Err(FamilyError::Overflow));
    assert_eq!(cyclic(3, 0), Err(FamilyError::NoPoints));
}

#[test]
fn shape_errors() {
    assert_eq!(hypercube(0), Err(FamilyError::ZeroDimension));
    assert_eq!(cross_polytope(0), Err(FamilyError::ZeroDimension));
    assert_eq!(scaled_simplex(0, 2), Err(FamilyError::ZeroDimension));
    assert_eq!(cyclic(0, 2), Err(FamilyError::ZeroDimension));
    // 2^20 rows * 20 columns exceeds MAX_VALUES = 2^24; 2^19 * 19 does not.
    assert_eq!(hypercube(20), Err(FamilyError::TooLarge));
    assert_eq!(hypercube(19).unwrap().rows(), 1 << 19);
    assert_eq!(hypercube(200), Err(FamilyError::TooLarge));
    assert_eq!(cross_polytope(usize::MAX), Err(FamilyError::TooLarge));
    assert_eq!(scaled_simplex(usize::MAX, 1), Err(FamilyError::TooLarge));
}

#[test]
fn families_round_trip_through_the_input_format() {
    for m in [
        hypercube(4).unwrap(),
        cross_polytope(3).unwrap(),
        scaled_simplex(4, 2).unwrap(),
        cyclic(4, 8).unwrap(),
    ] {
        let mut text = Vec::new();
        m.write_nested(&mut text).unwrap();
        let back = crate::input::PointRecords::new(&text[..])
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(back.vertices, m);
    }
}

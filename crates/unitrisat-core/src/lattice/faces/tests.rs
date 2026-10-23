use super::*;
use crate::families::{cross_polytope, hypercube, scaled_simplex};
use crate::hull::facets;
use crate::lattice::{Restriction, lattice_points, simplices};

fn matrix(columns: usize, values: &[i64]) -> Matrix {
    Matrix::new(columns, values.to_vec()).unwrap()
}

/// Points, facets and simplices of the polytope with these vertices.
fn pipeline(vertices: &Matrix, restriction: Restriction) -> (Matrix, Vec<Facet>, Vec<u32>) {
    let facets = facets(vertices).unwrap();
    let points = lattice_points(vertices, &facets).unwrap();
    let simplices = simplices(&points, restriction).unwrap();
    (points, facets, simplices)
}

/// Julia's `internal_faces` test for one point set: no facet has every
/// point on its hyperplane.
fn reference_internal(points: &Matrix, facets: &[Facet], face: &[u32]) -> bool {
    let rows: Vec<&[i64]> = points.iter_rows().collect();
    !facets.iter().any(|f| {
        face.iter().all(|&l| {
            let dot: i128 = f
                .normal
                .iter()
                .zip(rows[l as usize])
                .map(|(&a, &x)| i128::from(a) * i128::from(x))
                .sum();
            dot == i128::from(f.bound)
        })
    })
}

/// Checks `faces` against Julia's quadratic definitions.
fn check_against_reference(points: &Matrix, facets: &[Facet], labels: &[u32]) {
    let d = points.columns();
    let faces = faces(points, facets, labels).unwrap();
    let simplices: Vec<&[u32]> = labels.chunks(d + 1).collect();
    // Faces are distinct, in lexicographic order, each with increasing
    // labels and increasing, nonempty containing simplices.
    let all: Vec<Face<'_>> = faces.iter().collect();
    assert_eq!(all.len(), faces.len());
    assert!(all.windows(2).all(|w| w[0].labels < w[1].labels));
    for face in &all {
        assert_eq!(face.labels.len(), d);
        assert!(face.labels.windows(2).all(|w| w[0] < w[1]));
        assert!(!face.simplices.is_empty());
        assert!(face.simplices.windows(2).all(|w| w[0] < w[1]));
    }
    let incidences: usize = all.iter().map(|f| f.simplices.len()).sum();
    assert_eq!(incidences, labels.len());
    let mut used = vec![false; faces.len()];
    for (s, simplex) in simplices.iter().enumerate() {
        let slots = faces.of_simplex(s as u32).unwrap();
        assert_eq!(slots.len(), d + 1);
        for (k, &f) in slots.iter().enumerate() {
            used[f as usize] = true;
            let face = faces.get(f).unwrap();
            let expected: Vec<u32> = simplex
                .iter()
                .enumerate()
                .filter(|&(i, _)| i != k)
                .map(|(_, &l)| l)
                .collect();
            assert_eq!(face.labels, expected);
            assert_eq!(face.internal, reference_internal(points, facets, &expected));
            // Julia's coverer scan, including the simplex itself.
            let containing: Vec<u32> = simplices
                .iter()
                .enumerate()
                .filter(|(_, t)| expected.iter().all(|l| t.contains(l)))
                .map(|(t, _)| t as u32)
                .collect();
            assert_eq!(face.simplices, containing);
        }
    }
    assert!(used.iter().all(|&u| u));
    assert!(faces.of_simplex(simplices.len() as u32).is_none());
    assert!(faces.get(faces.len() as u32).is_none());
}

#[test]
fn unit_square() {
    let (points, facets, labels) = pipeline(&hypercube(2).unwrap(), Restriction::Unimodular);
    assert_eq!(points.values(), [0, 0, 1, 0, 0, 1, 1, 1]);
    assert_eq!(labels, [0, 1, 2, 0, 1, 3, 0, 2, 3, 1, 2, 3]);
    let faces = faces(&points, &facets, &labels).unwrap();
    let listed: Vec<(Vec<u32>, bool, Vec<u32>)> = faces
        .iter()
        .map(|f| (f.labels.to_vec(), f.internal, f.simplices.to_vec()))
        .collect();
    assert_eq!(
        listed,
        [
            (vec![0, 1], false, vec![0, 1]),
            (vec![0, 2], false, vec![0, 2]),
            (vec![0, 3], true, vec![1, 2]),
            (vec![1, 2], true, vec![0, 3]),
            (vec![1, 3], false, vec![1, 3]),
            (vec![2, 3], false, vec![2, 3]),
        ]
    );
    // Simplex 0 = {0, 1, 2}: opposite vertex 0 is {1, 2} (face 3), then
    // {0, 2} (face 1), then {0, 1} (face 0).
    assert_eq!(faces.of_simplex(0).unwrap(), [3, 1, 0]);
    assert_eq!(faces.of_simplex(3).unwrap(), [5, 4, 3]);
    check_against_reference(&points, &facets, &labels);
}

#[test]
fn families_match_julia_definitions() {
    let polytopes = [
        hypercube(2).unwrap(),
        hypercube(3).unwrap(),
        hypercube(4).unwrap(),
        cross_polytope(3).unwrap(),
        cross_polytope(4).unwrap(),
        scaled_simplex(2, 3).unwrap(),
        scaled_simplex(3, 2).unwrap(),
        scaled_simplex(4, 2).unwrap(),
        // A lattice octahedron-like 3D polytope with non-unit facet normals.
        matrix(3, &[0, 0, 0, 3, 1, 0, 1, 3, 0, 0, 1, 2, 2, 2, 2]),
        // A 3D polytope with an interior point and lattice points inside
        // facets.
        matrix(3, &[0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 2, 2, 2, 1]),
    ];
    for vertices in &polytopes {
        for restriction in [Restriction::Unimodular, Restriction::Nondegenerate] {
            let (points, facets, labels) = pipeline(vertices, restriction);
            check_against_reference(&points, &facets, &labels);
        }
    }
}

#[test]
fn many_facets_use_several_words() {
    let (points, facets, labels) = pipeline(&hypercube(2).unwrap(), Restriction::Nondegenerate);
    let expected = faces(&points, &facets, &labels).unwrap();
    // 70 loose inequalities first, so the real facets sit in the second word.
    let loose = Facet {
        normal: vec![1, 0],
        bound: 5,
    };
    let mut padded = vec![loose; 70];
    padded.extend(facets.iter().cloned());
    assert_eq!(faces(&points, &padded, &labels).unwrap(), expected);
    check_against_reference(&points, &padded, &labels);
}

#[test]
fn no_facets_make_every_face_internal() {
    let (points, _, labels) = pipeline(&hypercube(2).unwrap(), Restriction::Unimodular);
    let faces = faces(&points, &[], &labels).unwrap();
    assert_eq!(faces.len(), 6);
    assert!(faces.iter().all(|f| f.internal));
}

#[test]
fn no_simplices_give_no_faces() {
    let (points, facets, _) = pipeline(&hypercube(2).unwrap(), Restriction::Unimodular);
    let faces = faces(&points, &facets, &[]).unwrap();
    assert!(faces.is_empty());
    assert_eq!(faces.len(), 0);
    assert_eq!(faces.iter().count(), 0);
    assert!(faces.get(0).is_none());
    assert!(faces.of_simplex(0).is_none());
}

#[test]
fn rejects_malformed_input() {
    let (points, facets, labels) = pipeline(&hypercube(2).unwrap(), Restriction::Unimodular);
    let wrong = Facet {
        normal: vec![1, 0, 0],
        bound: 1,
    };
    assert_eq!(
        faces(&points, &[wrong], &labels),
        Err(FacesError::Dimension {
            expected: 2,
            found: 3
        })
    );
    assert_eq!(
        faces(&points, &facets, &labels[..4]),
        Err(FacesError::Shape { len: 4, width: 3 })
    );
    assert_eq!(
        faces(&points, &facets, &[0, 1, 4]),
        Err(FacesError::UnknownPoint(4))
    );
}

#[test]
fn reports_overflow() {
    // (-2^63)^2 + (-2^63)^2 = 2^127 does not fit i128.
    let points = matrix(2, &[i64::MIN, i64::MIN, 0, 0, 1, 0]);
    let huge = Facet {
        normal: vec![i64::MIN, i64::MIN],
        bound: 0,
    };
    assert_eq!(
        faces(&points, &[huge], &[0, 1, 2]),
        Err(FacesError::Overflow)
    );
}

#[test]
fn packed_and_slice_sorts_agree() {
    // Pseudo-random slots of 4 labels below 100, with repeats.
    let mut x = 12345u64;
    let mut labels = Vec::new();
    for _ in 0..500 {
        let mut slot: Vec<u32> = (0..4)
            .map(|_| {
                x = x
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                ((x >> 33) % 7) as u32 * 14 + ((x >> 20) % 3) as u32
            })
            .collect();
        slot.sort_unstable();
        labels.extend(slot);
    }
    let count = 500;
    // 100 points: 7 bits per label, packed. 2^30 points: 120 bits, slices.
    let packed = sorted_slots(&labels, 4, count, 100).unwrap();
    let sliced = sorted_slots(&labels, 4, count, 1 << 30).unwrap();
    assert_eq!(packed, sliced);
    let mut expected: Vec<u32> = (0..count).collect();
    expected.sort_by_key(|&s| (labels[s as usize * 4..s as usize * 4 + 4].to_vec(), s));
    assert_eq!(packed, expected);
}

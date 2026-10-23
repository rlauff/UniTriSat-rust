use super::*;
use crate::families::{cross_polytope, hypercube, scaled_simplex};
use crate::hull::facets;
use crate::intersect::intersecting_pairs;
use crate::lattice::{Restriction, faces, lattice_points, simplices};
use std::collections::BTreeSet;

struct Case {
    points: Matrix,
    labels: Vec<u32>,
    faces: Faces,
    pairs: BTreeSet<(u32, u32)>,
}

fn case(vertices: &Matrix, restriction: Restriction) -> Case {
    let facets = facets(vertices).unwrap();
    let points = lattice_points(vertices, &facets).unwrap();
    let labels = simplices(&points, restriction).unwrap();
    let faces = faces(&points, &facets, &labels).unwrap();
    let pairs = intersecting_pairs(points.values(), points.columns(), &labels)
        .unwrap()
        .into_iter()
        .collect();
    Case {
        points,
        labels,
        faces,
        pairs,
    }
}

fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for vertices in [
        hypercube(2).unwrap(),
        scaled_simplex(2, 3).unwrap(),
        hypercube(3).unwrap(),
        scaled_simplex(3, 2).unwrap(),
        cross_polytope(3).unwrap(),
    ] {
        for restriction in [Restriction::Unimodular, Restriction::Nondegenerate] {
            cases.push(case(&vertices, restriction));
        }
    }
    cases
}

fn slot(faces: &Faces, simplex: u32, face: u32) -> usize {
    faces
        .of_simplex(simplex)
        .unwrap()
        .iter()
        .position(|&f| f == face)
        .unwrap()
}

#[test]
fn vertices_opposite_a_shared_face_are_on_one_side_exactly_when_the_simplices_overlap() {
    for case in cases() {
        let central = central(&case.points, &case.labels, &case.faces).unwrap();
        let mut shared = 0;
        for (f, face) in (0u32..).zip(case.faces.iter()) {
            for (a, &s) in face.simplices.iter().enumerate() {
                for &t in &face.simplices[a + 1..] {
                    let side_s = central.apex_positive(s, slot(&case.faces, s, f)).unwrap();
                    let side_t = central.apex_positive(t, slot(&case.faces, t, f)).unwrap();
                    assert_eq!(side_s == side_t, case.pairs.contains(&(s, t)));
                    shared += 1;
                }
            }
        }
        assert!(shared > 0);
    }
}

#[test]
fn central_simplices_overlap_pairwise_in_every_lane() {
    for case in cases() {
        let result = central(&case.points, &case.labels, &case.faces).unwrap();
        let simplices = result.simplices();
        assert!(!simplices.is_empty());
        assert!(simplices.windows(2).all(|w| w[0] < w[1]));
        for (a, &s) in simplices.iter().enumerate() {
            for &t in &simplices[a + 1..] {
                assert!(case.pairs.contains(&(s, t)), "{s} and {t} are disjoint");
            }
        }
        let input = Input::new(&case.points, &case.labels, &case.faces).unwrap();
        let narrow = compute::<i128>(&input, SEEDED_ATTEMPTS, false)
            .ok()
            .unwrap();
        let wide = compute::<Integer>(&input, SEEDED_ATTEMPTS, true)
            .ok()
            .unwrap();
        assert_eq!(narrow, result);
        assert_eq!(wide, result);
        // The deterministic fallback alone also gives a generic point.
        let fallback = compute::<Integer>(&input, 0, true).ok().unwrap();
        assert_eq!(fallback.apex_positive, result.apex_positive);
        assert!(!fallback.simplices().is_empty());
        for (a, &s) in fallback.simplices().iter().enumerate() {
            for &t in &fallback.simplices()[a + 1..] {
                assert!(case.pairs.contains(&(s, t)));
            }
        }
        assert!(matches!(
            compute::<i128>(&input, 0, false),
            Err(Failure::NotGeneric)
        ));
    }
}

#[test]
fn determinants_match_the_leibniz_formula() {
    let m = [2i128, -1, 0, 3, 0, 4, 1, 5, -2];
    // 2 (0·-2 - 4·5) + 1 (3·-2 - 4·1) + 0 = -40 - 10
    assert_eq!(determinant(&mut m.clone(), 3).ok(), Some(-50));
    let wide: Vec<Integer> = m.iter().map(|&x| Integer::from(x)).collect();
    assert_eq!(
        determinant(&mut wide.clone(), 3).ok(),
        Some(Integer::from(-50))
    );
    // A zero leading entry needs a row swap.
    assert_eq!(determinant(&mut [0i128, 1, 1, 0], 2).ok(), Some(-1));
    assert_eq!(determinant(&mut [1i128, 2, 2, 4], 2).ok(), Some(0));
    assert_eq!(determinant::<i128>(&mut [], 0).ok(), Some(1));
    assert!(matches!(
        determinant(&mut [i128::MAX, 2, 2, i128::MAX], 2),
        Err(Failure::Overflow)
    ));
}

#[test]
fn large_coordinates_fall_back_to_big_integers() {
    // A triangle with edges of length about 2^62: its normals fit i128, but
    // the products with the weighted point do not. The facets only decide
    // which faces are internal, so none are passed.
    let big = 1i64 << 61;
    let points = Matrix::new(2, vec![-big, 0, big, 1, 0, big]).unwrap();
    let labels = vec![0, 1, 2];
    let faces = faces(&points, &[], &labels).unwrap();
    let input = Input::new(&points, &labels, &faces).unwrap();
    assert!(matches!(
        compute::<i128>(&input, SEEDED_ATTEMPTS, false),
        Err(Failure::Overflow)
    ));
    let result = central(&points, &labels, &faces).unwrap();
    assert_eq!(result.simplices(), &[0]);
}

#[test]
fn rejects_mismatched_and_flat_input() {
    let case = case(&hypercube(2).unwrap(), Restriction::Unimodular);
    let shape = Err(CentralError::Shape(2));
    assert_eq!(central(&case.points, &case.labels[..2], &case.faces), shape);
    assert_eq!(central(&case.points, &case.labels[..3], &case.faces), shape);
    let mut unknown = case.labels.clone();
    unknown[1] = 99;
    assert_eq!(
        central(&case.points, &unknown, &case.faces),
        Err(CentralError::UnknownPoint(99))
    );
    // Three collinear points: the face (0, 1) has the vertex 2 on its line.
    let points = Matrix::new(2, vec![0, 0, 1, 0, 2, 0, 0, 1]).unwrap();
    let flat = vec![0, 1, 2];
    let facets = facets(&points).unwrap();
    let flat_faces = faces(&points, &facets, &flat).unwrap();
    assert_eq!(
        central(&points, &flat, &flat_faces),
        Err(CentralError::Flat(0))
    );
    // No simplices: nothing is central.
    let none = faces(&points, &facets, &[]).unwrap();
    assert_eq!(
        central(&points, &[], &none).unwrap().simplices(),
        &[] as &[u32]
    );
}

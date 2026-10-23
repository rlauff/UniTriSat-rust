use super::*;
use crate::families::{cross_polytope, hypercube, scaled_simplex};
use crate::hull::{Facet, facets};
use crate::intersect::intersecting_pairs;
use crate::lattice::{Restriction, central, faces, lattice_points, simplices};
use crate::matrix::Matrix;
use crate::solve::{Backend, Control, Solver, Status};
use std::collections::BTreeSet;

/// Everything Julia's `process_polytope` computes before its formula.
struct Pipeline {
    points: Matrix,
    facets: Vec<Facet>,
    labels: Vec<u32>,
    pairs: Vec<(u32, u32)>,
}

impl Pipeline {
    fn new(vertices: &Matrix, restriction: Restriction) -> Self {
        let facets = facets(vertices).unwrap();
        let points = lattice_points(vertices, &facets).unwrap();
        let labels = simplices(&points, restriction).unwrap();
        let pairs = intersecting_pairs(points.values(), points.columns(), &labels).unwrap();
        Self {
            points,
            facets,
            labels,
            pairs,
        }
    }

    fn simplex_count(&self) -> usize {
        self.labels.len() / (self.points.columns() + 1)
    }

    fn formula(&self) -> Result<Cnf, CnfError> {
        let faces = faces(&self.points, &self.facets, &self.labels).unwrap();
        formula(self.simplex_count(), &self.pairs, &faces)
    }

    fn reduced(&self) -> Result<Cnf, CnfError> {
        let faces = faces(&self.points, &self.facets, &self.labels).unwrap();
        let central = central(&self.points, &self.labels, &faces).unwrap();
        reduced(self.simplex_count(), &faces, &central)
    }
}

fn clauses(cnf: &Cnf) -> Vec<Vec<i32>> {
    cnf.iter().map(<[i32]>::to_vec).collect()
}

fn literal(simplex: usize) -> i32 {
    i32::try_from(simplex + 1).unwrap()
}

/// Julia's `next_combination!` on zero-based indices below `n`.
fn next_combination(indices: &mut [usize], n: usize) -> bool {
    let k = indices.len();
    let Some(j) = (0..k).rev().find(|&j| indices[j] < n - k + j) else {
        return false;
    };
    indices[j] += 1;
    for i in j + 1..k {
        indices[i] = indices[i - 1] + 1;
    }
    true
}

/// Julia's formula, computed as `process_polytope` does with one thread:
/// the non-emptiness clause, the pair clauses, then `compute_face_clauses`
/// literally (every `d`-combination of each simplex, Julia's
/// `internal_faces` test, and a scan of all simplices for the coverers).
fn julia_formula(p: &Pipeline) -> Vec<Vec<i32>> {
    let d = p.points.columns();
    let rows: Vec<&[i64]> = p.points.iter_rows().collect();
    let simplices: Vec<&[u32]> = p.labels.chunks(d + 1).collect();
    let on = |f: &Facet, label: u32| {
        let dot: i128 = f
            .normal
            .iter()
            .zip(rows[label as usize])
            .map(|(&a, &x)| i128::from(a) * i128::from(x))
            .sum();
        dot == i128::from(f.bound)
    };
    let mut cnf = vec![(0..simplices.len()).map(literal).collect::<Vec<_>>()];
    for &(i, j) in &p.pairs {
        cnf.push(vec![-literal(i as usize), -literal(j as usize)]);
    }
    for (i, simplex) in simplices.iter().enumerate() {
        let mut combination: Vec<usize> = (0..d).collect();
        loop {
            let face: Vec<u32> = combination.iter().map(|&k| simplex[k]).collect();
            let internal = !p.facets.iter().any(|f| face.iter().all(|&l| on(f, l)));
            if internal {
                let mut clause = vec![-literal(i)];
                for (j, other) in simplices.iter().enumerate() {
                    if i != j && face.iter().all(|l| other.contains(l)) {
                        clause.push(literal(j));
                    }
                }
                cnf.push(clause);
            }
            if !next_combination(&mut combination, d + 1) {
                break;
            }
        }
    }
    cnf
}

fn matrix(columns: usize, values: &[i64]) -> Matrix {
    Matrix::new(columns, values.to_vec()).unwrap()
}

/// The test polytopes: small families in dimensions 2 and 3.
fn polytopes() -> Vec<(&'static str, Matrix)> {
    vec![
        ("square", hypercube(2).unwrap()),
        ("2-simplex x3", scaled_simplex(2, 3).unwrap()),
        ("3-cube", hypercube(3).unwrap()),
        ("octahedron", cross_polytope(3).unwrap()),
        ("3-simplex x2", scaled_simplex(3, 2).unwrap()),
        (
            "3D v8 record",
            matrix(3, &[0, 0, 0, 1, 0, 0, 0, 1, 0, 6, 7, 8]),
        ),
    ]
}

#[test]
fn equals_julia_clause_for_clause() {
    for (name, vertices) in polytopes() {
        for restriction in [Restriction::Unimodular, Restriction::Nondegenerate] {
            let p = Pipeline::new(&vertices, restriction);
            let cnf = p.formula().unwrap();
            assert_eq!(clauses(&cnf), julia_formula(&p), "{name}, {restriction:?}");
            assert_eq!(cnf.variables() as usize, p.simplex_count());
            assert_eq!(cnf.len(), cnf.iter().count());
            assert!(!cnf.is_empty());
        }
    }
}

#[test]
fn literals_are_dense_variables() {
    for (name, vertices) in polytopes() {
        let p = Pipeline::new(&vertices, Restriction::Nondegenerate);
        let cnf = p.formula().unwrap();
        let n = i32::try_from(cnf.variables()).unwrap();
        for clause in cnf.iter() {
            assert!(
                clause.iter().all(|&l| l != 0 && l.abs() <= n),
                "{name}: {clause:?}"
            );
        }
        // Every variable occurs (the non-emptiness clause).
        assert_eq!(cnf.iter().next().unwrap(), (1..=n).collect::<Vec<_>>());
    }
}

/// The formula Julia wrote for `d3-v2-r1-unimodular` (archive B00
/// `cnf/d3-v2-r1-unimodular/formula.cnf`).
#[test]
fn archived_julia_formula() {
    let vertices = matrix(3, &[0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, 2]);
    let cnf = Pipeline::new(&vertices, Restriction::Unimodular)
        .formula()
        .unwrap();
    assert_eq!(cnf.variables(), 2);
    assert_eq!(clauses(&cnf), [vec![1, 2], vec![-1, 2], vec![-2, 1]]);
}

/// The models, each blocked once found.
fn models(backend: Backend, cnf: &Cnf) -> BTreeSet<Vec<u32>> {
    let mut solver = Solver::new(backend, cnf.variables() as usize).unwrap();
    for clause in cnf.iter() {
        solver.add_clause(clause).unwrap();
    }
    let mut model = Vec::new();
    let mut found = BTreeSet::new();
    while solver.solve(&[], &Control::default()).unwrap() == Status::Sat {
        solver.model(&mut model).unwrap();
        solver.block(&model).unwrap();
        assert!(found.insert(model.clone()));
    }
    found
}

/// Number of models, each blocked once found.
fn count_models(backend: Backend, cnf: &Cnf) -> usize {
    models(backend, cnf).len()
}

/// Unimodular triangulation counts from Julia (P0.3 `-solve` jobs: 72 for
/// the 3-cube, 192 for the twice-dilated 3-simplex) and the square's two.
#[test]
fn models_are_julia_triangulations() {
    let cases = [
        (hypercube(2).unwrap(), 2),
        (hypercube(3).unwrap(), 72),
        (scaled_simplex(3, 2).unwrap(), 192),
    ];
    for (vertices, triangulations) in cases {
        let cnf = Pipeline::new(&vertices, Restriction::Unimodular)
            .formula()
            .unwrap();
        for backend in [Backend::PicoSat, Backend::Cadical] {
            assert_eq!(count_models(backend, &cnf), triangulations, "{backend:?}");
        }
    }
}

#[test]
fn no_simplices_is_the_empty_clause() {
    // The faces of an empty simplex list.
    let p = Pipeline::new(&hypercube(2).unwrap(), Restriction::Unimodular);
    let faces = faces(&p.points, &p.facets, &[]).unwrap();
    let cnf = formula(0, &[], &faces).unwrap();
    assert_eq!(cnf.variables(), 0);
    assert_eq!(clauses(&cnf), [Vec::<i32>::new()]);
    for backend in [Backend::PicoSat, Backend::Cadical] {
        assert_eq!(count_models(backend, &cnf), 0, "{backend:?}");
    }
}

#[test]
fn rejects_bad_pairs() {
    let p = Pipeline::new(&hypercube(2).unwrap(), Restriction::Unimodular);
    let n = p.simplex_count();
    let faces = faces(&p.points, &p.facets, &p.labels).unwrap();
    let n32 = u32::try_from(n).unwrap();
    for (first, second) in [(1, 1), (2, 1), (0, n32), (n32, n32 + 1)] {
        assert_eq!(
            formula(n, &[(first, second)], &faces),
            Err(CnfError::Pair {
                first,
                second,
                simplices: n,
            })
        );
    }
}

#[test]
fn rejects_faces_of_other_simplices() {
    let p = Pipeline::new(&hypercube(3).unwrap(), Restriction::Unimodular);
    let n = p.simplex_count();
    let faces = faces(&p.points, &p.facets, &p.labels).unwrap();
    for wrong in [0, n - 1, n + 1] {
        assert_eq!(formula(wrong, &[], &faces), Err(CnfError::Faces(wrong)));
    }
    assert_eq!(
        formula(usize::MAX, &[], &faces),
        Err(CnfError::TooManySimplices(usize::MAX))
    );
    let limit = MAX_VARIABLES as usize + 1;
    assert_eq!(
        formula(limit, &[], &faces),
        Err(CnfError::TooManySimplices(limit))
    );
}

#[test]
fn reduced_formula_has_the_same_models() {
    for (name, vertices) in polytopes() {
        for restriction in [Restriction::Unimodular, Restriction::Nondegenerate] {
            let p = Pipeline::new(&vertices, restriction);
            let full = p.formula().unwrap();
            let reduced = p.reduced().unwrap();
            let all = clauses(&reduced);
            assert_eq!(reduced.variables(), full.variables());
            // Non-emptiness first, the central clause second.
            assert_eq!(all[0], clauses(&full)[0]);
            let central: Vec<i32> = all[1].clone();
            assert!(!central.is_empty() && central.iter().all(|&l| l > 0));
            // Every binary clause excludes an intersecting pair, each once.
            let pairs: BTreeSet<(u32, u32)> = p.pairs.iter().copied().collect();
            let binary: Vec<(u32, u32)> = all[2..]
                .iter()
                .filter(|c| c.len() == 2 && c[0] < 0 && c[1] < 0)
                .map(|c| ((-c[0] - 1) as u32, (-c[1] - 1) as u32))
                .collect();
            let distinct: BTreeSet<(u32, u32)> = binary.iter().copied().collect();
            assert_eq!(distinct.len(), binary.len(), "{name}: repeated pair");
            assert!(distinct.is_subset(&pairs), "{name}: non-intersecting pair");
            assert!(binary.len() <= pairs.len());
            // The face clauses come last, unchanged.
            let face_clauses = clauses(&full).len() - 1 - p.pairs.len();
            assert_eq!(
                all[all.len() - face_clauses..],
                clauses(&full)[1 + p.pairs.len()..]
            );
            for backend in [Backend::PicoSat, Backend::Cadical] {
                assert_eq!(
                    models(backend, &reduced),
                    models(backend, &full),
                    "{name}, {restriction:?}, {backend:?}"
                );
            }
        }
    }
}

#[test]
fn reduced_formula_rejects_other_faces_and_handles_no_simplices() {
    let p = Pipeline::new(&hypercube(3).unwrap(), Restriction::Unimodular);
    let n = p.simplex_count();
    let faces = faces(&p.points, &p.facets, &p.labels).unwrap();
    let center = central(&p.points, &p.labels, &faces).unwrap();
    for wrong in [0, n - 1, n + 1] {
        assert_eq!(reduced(wrong, &faces, &center), Err(CnfError::Faces(wrong)));
    }
    // No simplices: nothing is central, and the central clause is empty.
    let none = faces_of(&p, &[]);
    let nothing = central(&p.points, &[], &none).unwrap();
    let cnf = reduced(0, &none, &nothing).unwrap();
    assert_eq!(clauses(&cnf), [Vec::<i32>::new(), Vec::new()]);
}

fn faces_of(p: &Pipeline, labels: &[u32]) -> crate::lattice::Faces {
    faces(&p.points, &p.facets, labels).unwrap()
}

/// Exhaust every selected subfamily, including empty complexes and missing
/// cliques of size 3, 4 and 5. A triangle-only encoding fails these cases.
#[test]
fn flag_sat_exhaustively_matches_checker_and_clique_definition() {
    for (points, width) in [(3_usize, 1_usize), (4, 2), (5, 3), (5, 4)] {
        let facets: Vec<Vec<u32>> = (0_u32..1 << points)
            .filter(|mask| mask.count_ones() as usize == width)
            .map(|mask| {
                (0..points as u32)
                    .filter(|v| mask & (1 << v) != 0)
                    .collect()
            })
            .collect();
        let labels: Vec<u32> = facets.iter().flatten().copied().collect();
        let base = Cnf::test_clauses(facets.len() as u32, &[]);
        let cnf = with_flag(base, &labels, width, points).unwrap();
        assert_eq!(cnf.simplex_variables() as usize, facets.len());
        let checker = crate::flag::Flag::new(&labels, width, points).unwrap();
        for backend in [Backend::PicoSat, Backend::Cadical] {
            let mut solver = Solver::new(backend, cnf.variables() as usize).unwrap();
            for clause in cnf.iter() {
                solver.add_clause(clause).unwrap();
            }
            for selected in 0_u32..1 << facets.len() {
                let chosen: Vec<u32> = (0..facets.len() as u32)
                    .filter(|s| selected & (1 << s) != 0)
                    .collect();
                let is_face = |mask: u32| {
                    chosen.iter().any(|&s| {
                        (0..points as u32)
                            .all(|v| mask & (1 << v) == 0 || facets[s as usize].contains(&v))
                    })
                };
                let brute = (0_u32..1 << points)
                    .filter(|mask| mask.count_ones() >= 3)
                    .all(|mask| {
                        let vertices: Vec<u32> = (0..points as u32)
                            .filter(|v| mask & (1 << v) != 0)
                            .collect();
                        let clique = vertices.iter().enumerate().all(|(i, &u)| {
                            vertices
                                .iter()
                                .skip(i + 1)
                                .all(|&v| is_face((1 << u) | (1 << v)))
                        });
                        !clique || is_face(mask)
                    });
                assert_eq!(checker.is_flag(&chosen).unwrap(), brute);
                let assumptions: Vec<i32> = (0..facets.len())
                    .map(|s| {
                        if selected & (1 << s) != 0 {
                            literal(s)
                        } else {
                            -literal(s)
                        }
                    })
                    .collect();
                assert_eq!(
                    solver.solve(&assumptions, &Control::default()).unwrap() == Status::Sat,
                    brute,
                    "{backend:?}, points={points}, width={width}, selected={chosen:?}"
                );
            }
        }
    }
}

#[test]
fn flag_sat_validates_labels_and_handles_sparse_and_unsorted_vertices() {
    let fixture = || Cnf::test_clauses(1, &[]);
    assert!(matches!(
        with_flag(fixture(), &[], 0, 0),
        Err(CnfError::FlagShape { .. })
    ));
    assert!(matches!(
        with_flag(fixture(), &[0], 2, 2),
        Err(CnfError::FlagShape { .. })
    ));
    assert!(matches!(
        with_flag(fixture(), &[0, 2], 2, 2),
        Err(CnfError::FlagPoint { .. })
    ));
    assert_eq!(
        with_flag(fixture(), &[1, 1], 2, 2),
        Err(CnfError::RepeatedPoint(1))
    );
    let empty = with_flag(Cnf::test_clauses(0, &[]), &[], 3, 100).unwrap();
    assert_eq!(empty.variables(), 0);
    let sparse = with_flag(fixture(), &[100, 2, 50], 3, 101).unwrap();
    assert_eq!(
        sparse.variables(),
        1,
        "single-support faces reuse their facet literal"
    );
    assert!(sparse.is_empty(), "one facet always gives a flag complex");
}

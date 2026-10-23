use super::*;
use crate::cnf::formula;
use crate::enumerate::{self, Goal, Keep, Options, Outcome};
use crate::families::{cross_polytope, hypercube, scaled_simplex};
use crate::hull::facets;
use crate::intersect::intersecting_pairs;
use crate::lattice::{Restriction, faces, lattice_points, simplices};
use crate::matrix::Matrix;
use crate::solve::Backend;

/// Flagness of the complex of all `rows` (`width` labels each).
fn verdict(rows: &[u32], width: usize, points: usize) -> bool {
    let all: Vec<u32> = (0..(rows.len() / width) as u32).collect();
    Flag::new(rows, width, points)
        .unwrap()
        .is_flag(&all)
        .unwrap()
}

/// Independent oracle (from the legacy tests): every vertex set of size at
/// least 3 whose pairs lie in common rows must lie in one row.
fn reference(rows: &[u32], width: usize) -> bool {
    let mut vertices = rows.to_vec();
    vertices.sort_unstable();
    vertices.dedup();
    assert!(vertices.len() < 24, "oracle too slow");
    let row = |set: &[u32]| {
        rows.chunks_exact(width)
            .any(|r| set.iter().all(|v| r.contains(v)))
    };
    (0_usize..(1 << vertices.len())).all(|mask| {
        let chosen: Vec<u32> = vertices
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, &v)| v)
            .collect();
        let clique = chosen
            .iter()
            .enumerate()
            .all(|(i, &a)| chosen[i + 1..].iter().all(|&b| row(&[a, b])));
        chosen.len() < 3 || !clique || row(&chosen)
    })
}

#[test]
fn named_cliques_and_faces() {
    // Legacy cases: (rows, width, points, flag).
    let cases: &[(&[u32], usize, usize, bool)] = &[
        (&[0], 1, 1, true),
        (&[0, 4, 9], 1, 10, true),
        // A triangle's boundary: {0, 1, 2} is a clique, not a face.
        (&[0, 1, 0, 2, 1, 2], 2, 3, false),
        (&[0, 1, 0, 3, 1, 2, 2, 3], 2, 4, true),
        // Three triangles around an empty triangle {0, 1, 2}.
        (&[0, 1, 3, 0, 2, 4, 1, 2, 5], 3, 6, false),
        // The boundary of a tetrahedron.
        (&[0, 1, 2, 0, 1, 3, 0, 2, 3, 1, 2, 3], 3, 4, false),
        (&[0, 1, 2, 0, 2, 3], 3, 4, true),
        (&[0, 1, 3, 4, 0, 2, 5, 6, 1, 2, 7, 8], 4, 9, false),
        // A tetrahedron, and two sharing a triangle.
        (&[0, 1, 2, 3], 4, 4, true),
        (&[0, 1, 2, 3, 1, 2, 3, 4], 4, 5, true),
    ];
    for &(rows, width, points, flag) in cases {
        assert_eq!(verdict(rows, width, points), flag, "{rows:?}");
        assert_eq!(reference(rows, width), flag, "{rows:?}");
    }
}

#[test]
fn exhaustive_pure_complexes_through_five_vertices() {
    for n in 1_u32..=5 {
        for width in 1..=n as usize {
            let candidates: Vec<Vec<u32>> = (0_u32..1 << n)
                .filter(|mask| mask.count_ones() as usize == width)
                .map(|mask| (0..n).filter(|v| mask & (1 << v) != 0).collect())
                .collect();
            for selection in 1_usize..(1 << candidates.len()) {
                let rows: Vec<u32> = candidates
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| selection & (1 << i) != 0)
                    .flat_map(|(_, r)| r.iter().copied())
                    .collect();
                assert_eq!(
                    verdict(&rows, width, n as usize),
                    reference(&rows, width),
                    "{rows:?}"
                );
            }
        }
    }
}

/// The edges of the cycle `0, 1, ..., n - 1`.
fn cycle(n: u32) -> Vec<u32> {
    let mut edges: Vec<[u32; 2]> = (0..n).map(|v| [v, (v + 1) % n]).collect();
    for e in &mut edges {
        e.sort_unstable();
    }
    edges.sort_unstable();
    edges.into_iter().flatten().collect()
}

#[test]
fn every_mask_width() {
    for n in [4, 63, 64, 65, 127, 128, 129, 256, 257, 512, 513, 1023, 1024] {
        assert!(verdict(&cycle(n), 2, n as usize), "cycle {n}");
        // A triangle at the highest labels, a path below it.
        let mut edges: Vec<[u32; 2]> = (0..n - 4).map(|i| [i, i + 1]).collect();
        edges.extend([[n - 3, n - 2], [n - 3, n - 1], [n - 2, n - 1]]);
        let rows: Vec<u32> = edges.into_iter().flatten().collect();
        assert!(!verdict(&rows, 2, n as usize), "triangle {n}");
        // The same complexes with a larger point count (wider masks).
        assert!(verdict(&cycle(n), 2, 1024));
        assert!(!verdict(&rows, 2, 1024));
    }
    assert_eq!(
        Flag::new(&cycle(4), 2, 1025),
        Err(FlagError::TooManyPoints(1025))
    );
}

#[test]
fn simplex_boundaries_are_not_flag() {
    for width in 2..=8_u32 {
        let rows: Vec<u32> = (0..=width)
            .flat_map(|omit| (0..=width).filter(move |&v| v != omit))
            .collect();
        assert!(!verdict(&rows, width as usize, width as usize + 1));
        // Shifted to the top of a two-word mask.
        let shifted: Vec<u32> = rows.iter().map(|v| v + 127 - width).collect();
        assert!(!verdict(&shifted, width as usize, 128));
        // One simplex alone is flag.
        let full: Vec<u32> = (0..=width).collect();
        assert!(verdict(&full, width as usize + 1, width as usize + 1));
    }
}

/// Freudenthal's triangulation of the cube `[0, k]^d`: each unit cube is cut
/// into the `d!` simplices `x, x + e_p1, x + e_p1 + e_p2, ...`, points
/// labelled in base `k + 1`. It is flag.
fn freudenthal(d: usize, k: u32) -> Vec<u32> {
    fn permutations(items: Vec<usize>) -> Vec<Vec<usize>> {
        if items.len() <= 1 {
            return vec![items];
        }
        let mut all = Vec::new();
        for i in 0..items.len() {
            let mut rest = items.clone();
            let first = rest.remove(i);
            for mut p in permutations(rest) {
                p.insert(0, first);
                all.push(p);
            }
        }
        all
    }
    let side = k + 1;
    let label = |x: &[u32]| x.iter().rev().fold(0, |acc, &c| acc * side + c);
    let mut rows = Vec::new();
    for base in 0..k.pow(d as u32) {
        let corner: Vec<u32> = (0..d).map(|i| base / k.pow(i as u32) % k).collect();
        for p in permutations((0..d).collect()) {
            let mut x = corner.clone();
            let mut simplex = vec![label(&x)];
            for &axis in &p {
                x[axis] += 1;
                simplex.push(label(&x));
            }
            simplex.sort_unstable();
            rows.extend(simplex);
        }
    }
    rows
}

#[test]
fn freudenthal_triangulations_are_flag() {
    for (d, k) in [
        (2, 1),
        (2, 3),
        (3, 1),
        (3, 3),
        (4, 1),
        (4, 2),
        (3, 5),
        (2, 31),
    ] {
        let rows = freudenthal(d, k);
        let points = (k as usize + 1).pow(d as u32);
        assert!(verdict(&rows, d + 1, points), "d{d} k{k}");
    }
    // Every subcomplex missing one triangle of the 2 x 2 square, against the
    // oracle.
    let rows = freudenthal(2, 2);
    assert!(reference(&rows, 3));
    let all: Vec<u32> = (0..(rows.len() / 3) as u32).collect();
    let flag = Flag::new(&rows, 3, 9).unwrap();
    for skip in 0..all.len() {
        let some: Vec<u32> = all
            .iter()
            .copied()
            .filter(|&s| s as usize != skip)
            .collect();
        let sub: Vec<u32> = some
            .iter()
            .flat_map(|&s| rows[3 * s as usize..3 * s as usize + 3].iter().copied())
            .collect();
        assert_eq!(flag.is_flag(&some).unwrap(), reference(&sub, 3));
    }
}

#[test]
fn errors() {
    assert_eq!(
        Flag::new(&[0, 1, 2], 2, 3),
        Err(FlagError::Shape {
            labels: 3,
            width: 2
        })
    );
    assert_eq!(
        Flag::new(&[], 0, 3),
        Err(FlagError::Shape {
            labels: 0,
            width: 0
        })
    );
    assert_eq!(Flag::new(&[0, 3], 2, 3), Err(FlagError::UnknownPoint(3)));
    assert_eq!(Flag::new(&[0, 1], 2, 0), Err(FlagError::UnknownPoint(0)));
    let flag = Flag::new(&[0, 1, 1, 2], 2, 3).unwrap();
    assert_eq!(flag.is_flag(&[0, 2]), Err(FlagError::UnknownSimplex(2)));
    assert_eq!(
        flag.is_flag(&[u32::MAX]),
        Err(FlagError::UnknownSimplex(u32::MAX))
    );
    assert_eq!(flag.is_flag(&[]), Ok(true));
    assert_eq!(flag.is_flag(&[1, 0]), Ok(true));
}

/// All triangulations of a polytope with the given simplices, and the flag
/// count of the hook.
fn enumerate_with_flag(
    vertices: &Matrix,
    restriction: Restriction,
) -> (Vec<u32>, usize, Vec<Vec<u32>>, u64) {
    let hull = facets(vertices).unwrap();
    let points = lattice_points(vertices, &hull).unwrap();
    let labels = simplices(&points, restriction).unwrap();
    let width = points.columns() + 1;
    let faces = faces(&points, &hull, &labels).unwrap();
    let pairs = intersecting_pairs(points.values(), points.columns(), &labels).unwrap();
    let cnf = formula(labels.len() / width, &pairs, &faces).unwrap();
    let flag = Flag::new(&labels, width, points.rows()).unwrap();
    let options = |keep| Options {
        backend: Backend::PicoSat,
        goal: Goal::All,
        keep,
        max_solutions: None,
        deadline: None,
    };
    let all = enumerate::serial(&cnf, &options(Keep::All), &enumerate::AnyTriangulation).unwrap();
    assert_eq!(all.outcome, Outcome::Complete);
    let flagged = enumerate::serial(&cnf, &options(Keep::All), &flag).unwrap();
    assert_eq!(flagged.outcome, Outcome::Complete);
    assert_eq!(flagged.found, all.found);
    let expected: Vec<Vec<u32>> = all
        .triangulations
        .iter()
        .filter(|t| flag.is_flag(t).unwrap())
        .cloned()
        .collect();
    assert_eq!(flagged.triangulations, expected);
    (labels, width, all.triangulations, flagged.targets)
}

#[test]
fn triangulations_of_small_polytopes_match_the_oracle() {
    let square2 = Matrix::new(2, vec![0, 0, 2, 0, 0, 2, 2, 2]).unwrap();
    let cases = [
        ("square x2", square2, Restriction::Unimodular),
        ("3-cube", hypercube(3).unwrap(), Restriction::Unimodular),
        (
            "3-cube nondegenerate",
            hypercube(3).unwrap(),
            Restriction::Nondegenerate,
        ),
        (
            "3-simplex x2",
            scaled_simplex(3, 2).unwrap(),
            Restriction::Unimodular,
        ),
        (
            "octahedron nondegenerate",
            cross_polytope(3).unwrap(),
            Restriction::Nondegenerate,
        ),
        (
            "2-simplex x3 nondegenerate",
            scaled_simplex(2, 3).unwrap(),
            Restriction::Nondegenerate,
        ),
    ];
    let (mut flag_total, mut non_flag_total) = (0, 0);
    for (name, vertices, restriction) in cases {
        let (labels, width, triangulations, targets) = enumerate_with_flag(&vertices, restriction);
        let mut flags = 0;
        for t in &triangulations {
            let rows: Vec<u32> = t
                .iter()
                .flat_map(|&s| {
                    labels[s as usize * width..(s as usize + 1) * width]
                        .iter()
                        .copied()
                })
                .collect();
            if reference(&rows, width) {
                flags += 1;
            }
        }
        assert_eq!(targets, flags, "{name}");
        println!("{name}: {flags} of {} flag", triangulations.len());
        flag_total += flags;
        non_flag_total += triangulations.len() as u64 - flags;
    }
    // Both verdicts occur.
    assert!(flag_total > 0 && non_flag_total > 0);
}

#[test]
fn parallel_split_counts_the_same() {
    let vertices = scaled_simplex(3, 2).unwrap();
    let hull = facets(&vertices).unwrap();
    let points = lattice_points(&vertices, &hull).unwrap();
    let labels = simplices(&points, Restriction::Unimodular).unwrap();
    let faces = faces(&points, &hull, &labels).unwrap();
    let pairs = intersecting_pairs(points.values(), points.columns(), &labels).unwrap();
    let cnf = formula(labels.len() / 4, &pairs, &faces).unwrap();
    let central = crate::lattice::central(&points, &labels, &faces).unwrap();
    let flag = Flag::new(&labels, 4, points.rows()).unwrap();
    let options = Options {
        backend: Backend::Cadical,
        goal: Goal::All,
        keep: Keep::None,
        max_solutions: None,
        deadline: None,
    };
    let serial = enumerate::serial(&cnf, &options, &flag).unwrap();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    let split = pool
        .install(|| enumerate::parallel_split(&cnf, central.simplices(), &options, &flag))
        .unwrap();
    assert_eq!((split.found, split.targets), (serial.found, serial.targets));
    assert_eq!(serial.found, 192);
    // The first flag triangulation stops a Goal::First search.
    let first = enumerate::serial(
        &cnf,
        &Options {
            goal: Goal::First,
            keep: Keep::First,
            ..options
        },
        &flag,
    )
    .unwrap();
    assert_eq!(first.outcome, Outcome::Found);
    assert!(flag.is_flag(&first.triangulations[0]).unwrap());
}

use super::*;
use crate::cnf::formula;
use crate::enumerate::{AnyTriangulation, Goal, Keep, Options, Outcome, parallel_split, serial};
use crate::families::{hypercube, scaled_simplex};
use crate::flag::Flag;
use crate::hull::facets;
use crate::intersect::intersecting_pairs;
use crate::lattice::{Restriction, central, faces, lattice_points, simplices};
use crate::quadratic::Quadratic;
use crate::solve::Backend;
use std::num::NonZeroU64;

/// A point set with its simplices, faces, formula and central simplices.
struct Setup {
    points: Matrix,
    labels: Vec<u32>,
    faces: Faces,
    cnf: crate::cnf::Cnf,
    central: Vec<u32>,
}

impl Setup {
    /// `points` themselves (no lattice-point scan), so that any point
    /// configuration can be used.
    fn of_points(points: Matrix, restriction: Restriction) -> Self {
        let facets = facets(&points).unwrap();
        let labels = simplices(&points, restriction).unwrap();
        let count = labels.len() / (points.columns() + 1);
        let faces = faces(&points, &facets, &labels).unwrap();
        let pairs = intersecting_pairs(points.values(), points.columns(), &labels).unwrap();
        let central = central(&points, &labels, &faces)
            .unwrap()
            .simplices()
            .to_vec();
        Self {
            cnf: formula(count, &pairs, &faces).unwrap(),
            points,
            labels,
            faces,
            central,
        }
    }

    /// The lattice points of the polytope with these vertices.
    fn of_vertices(vertices: &Matrix, restriction: Restriction) -> Self {
        let points = lattice_points(vertices, &facets(vertices).unwrap()).unwrap();
        Self::of_points(points, restriction)
    }

    fn check(&self) -> Regularity<'_> {
        Regularity::new(&self.points, &self.labels, &self.faces).unwrap()
    }

    /// The index of the simplex with these point labels.
    fn simplex(&self, vertices: &[u32]) -> u32 {
        let mut sorted = vertices.to_vec();
        sorted.sort_unstable();
        let width = self.points.columns() + 1;
        let index = self
            .labels
            .chunks_exact(width)
            .position(|s| s == sorted)
            .unwrap();
        u32::try_from(index).unwrap()
    }

    /// The triangulation with these simplices, as increasing indices.
    fn triangulation(&self, simplices: &[&[u32]]) -> Vec<u32> {
        let mut t: Vec<u32> = simplices.iter().map(|s| self.simplex(s)).collect();
        t.sort_unstable();
        t
    }

    /// Every triangulation, sorted.
    fn all(&self) -> Vec<Vec<u32>> {
        let result = serial(&self.cnf, &options(Keep::All), &AnyTriangulation).unwrap();
        assert_eq!(result.outcome, Outcome::Complete);
        result.triangulations
    }
}

fn options(keep: Keep) -> Options {
    Options {
        backend: Backend::PicoSat,
        goal: Goal::All,
        keep,
        max_solutions: None,
        deadline: None,
    }
}

fn matrix(columns: usize, values: &[i64]) -> Matrix {
    Matrix::new(columns, values.to_vec()).unwrap()
}

/// The "mother of all examples" (De Loera, Rambau and Santos, Ex. 2.2.5):
/// a triangle `A, B, C` and a copy `a, b, c` shrunk by 1/2 towards its
/// centroid (2, 2).
fn mother() -> Setup {
    Setup::of_points(
        matrix(2, &[0, 0, 6, 0, 0, 6, 1, 1, 4, 1, 1, 4]),
        Restriction::Nondegenerate,
    )
}

const A: u32 = 0;
const B: u32 = 1;
const C: u32 = 2;
const AI: u32 = 3;
const BI: u32 = 4;
const CI: u32 = 5;

#[test]
fn mother_of_all_examples() {
    let m = mother();
    let check = m.check();
    // The inner triangle, and each quadrilateral between an outer and the
    // inner edge split by one diagonal. Splitting all three the same way
    // round gives the two famous non-regular triangulations.
    let quads = [(A, B, BI, AI), (B, C, CI, BI), (C, A, AI, CI)];
    let mut regular = 0;
    for choice in 0..8_u32 {
        let mut tris: Vec<Vec<u32>> = vec![vec![AI, BI, CI]];
        for (k, &(p, q, qi, pi)) in quads.iter().enumerate() {
            if choice >> k & 1 == 0 {
                // Diagonal p-qi.
                tris.push(vec![p, q, qi]);
                tris.push(vec![p, qi, pi]);
            } else {
                // Diagonal q-pi.
                tris.push(vec![p, q, pi]);
                tris.push(vec![q, qi, pi]);
            }
        }
        let refs: Vec<&[u32]> = tris.iter().map(Vec::as_slice).collect();
        let t = m.triangulation(&refs);
        let is_regular = check.is_regular(&t).unwrap();
        let twisted = choice == 0 || choice == 7;
        assert_eq!(is_regular, !twisted, "choice {choice}");
        regular += u32::from(is_regular);
    }
    assert_eq!(regular, 6);
}

/// All triangulations of the mother of all examples: 18, of which 16 are
/// regular. Julia's `is_regular` gives the same answer on each of the 18
/// (checked once with `src/subdivision_regularity.jl`, P5.1 log).
#[test]
fn mother_all_triangulations() {
    let m = mother();
    let check = m.check();
    let all = m.all();
    let regular = all.iter().filter(|t| check.is_regular(t).unwrap()).count();
    assert_eq!((all.len(), regular), (18, 16));
}

/// The baseline cases of P0.3 (`outs/lean-baseline/t1/<job>/<case>/`)
/// with Julia's counts from `meta.tsv`: all unimodular triangulations, and
/// the regular ones.
fn julia_cases() -> Vec<(&'static str, Matrix, u64, u64)> {
    vec![
        ("w-dense-cube3-solve", hypercube(3).unwrap(), 72, 72),
        (
            "w-dense-2simplex3-solve",
            scaled_simplex(3, 2).unwrap(),
            192,
            192,
        ),
        (
            "w-mid-3d-v16-solve/v16-r6",
            matrix(3, &[0, 0, 0, 2, 0, 0, 0, 2, 0, 2, 2, 4]),
            256,
            158,
        ),
        (
            "w-mid-3d-v16-solve/v16-r8",
            matrix(3, &[0, 0, 0, 1, 0, 0, 1, 4, 0, 3, 0, 4]),
            226,
            226,
        ),
        (
            "w-mid-3d-v16-solve/v16-r16",
            matrix(3, &[0, 0, 0, 1, 0, 0, 1, 2, 0, 1, 6, 8]),
            998,
            992,
        ),
    ]
}

#[test]
fn julia_regular_counts() {
    for (name, vertices, triangulations, regular) in julia_cases() {
        let setup = Setup::of_vertices(&vertices, Restriction::Unimodular);
        let check = setup.check();
        let serial_result = serial(&setup.cnf, &options(Keep::None), &check).unwrap();
        assert_eq!(serial_result.outcome, Outcome::Complete, "{name}");
        assert_eq!(serial_result.found, triangulations, "{name}");
        assert_eq!(serial_result.targets, regular, "{name}");
        // The hook is shared by the threads of the parallel split.
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap();
        let split = pool
            .install(|| parallel_split(&setup.cnf, &setup.central, &options(Keep::None), &check))
            .unwrap();
        assert_eq!(
            (split.outcome, split.found, split.targets),
            (Outcome::Complete, triangulations, regular),
            "{name}"
        );
    }
}

#[test]
fn first_regular_triangulation_ends_the_search() {
    let (_, vertices, ..) = julia_cases().swap_remove(2);
    let setup = Setup::of_vertices(&vertices, Restriction::Unimodular);
    let check = setup.check();
    let result = serial(
        &setup.cnf,
        &Options {
            goal: Goal::First,
            keep: Keep::First,
            ..options(Keep::None)
        },
        &check,
    )
    .unwrap();
    assert_eq!(result.outcome, Outcome::Found);
    assert_eq!(result.targets, 1);
    assert!(check.is_regular(&result.triangulations[0]).unwrap());
}

/// Regularity does not change under scaling and translation of the points.
/// Coordinates near 2^62 make the `i128` elimination overflow, so the
/// `rug` lane decides; it must agree with the small coordinates on every
/// triangulation.
#[test]
fn huge_coordinates_take_the_exact_fallback() {
    let (_, vertices, ..) = julia_cases().swap_remove(2);
    let setup = Setup::of_vertices(&vertices, Restriction::Unimodular);
    let check = setup.check();
    let scale = 1_i64 << 60;
    let huge = Matrix::new(
        3,
        setup
            .points
            .values()
            .iter()
            .map(|&x| x * scale - (1 << 61) + 7)
            .collect(),
    )
    .unwrap();
    let huge_check = Regularity::new(&huge, &setup.labels, &setup.faces).unwrap();
    let mut differences = 0;
    for t in setup.all() {
        differences +=
            usize::from(check.is_regular(&t).unwrap() != huge_check.is_regular(&t).unwrap());
    }
    assert_eq!(differences, 0);
    // The machine lane really overflows there.
    let point = |l: usize| &huge.values()[3 * l..3 * l + 3];
    let circuit = [point(0), point(1), point(3), point(5), point(10)];
    let mut machine = vec![0_i128; 9];
    assert!(circuit_determinants::<i128>(&circuit, 3, &mut machine).is_none());
    let mut wide = vec![Integer::new(); 9];
    assert!(circuit_determinants::<Integer>(&circuit, 3, &mut wide).is_some());
}

/// Laplace expansion along the first row, for comparison.
fn laplace(m: &[i128], d: usize) -> i128 {
    if d == 1 {
        return m[0];
    }
    let mut total = 0;
    for j in 0..d {
        let minor: Vec<i128> = (1..d)
            .flat_map(|r| (0..d).filter(move |&c| c != j).map(move |c| (r, c)))
            .map(|(r, c)| m[r * d + c])
            .collect();
        let term = m[j] * laplace(&minor, d - 1);
        total += if j % 2 == 0 { term } else { -term };
    }
    total
}

#[test]
fn bareiss_matches_laplace() {
    // Deterministic pseudo-random small matrices, zeros included so that
    // the row pivoting is exercised.
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        i128::from((state >> 33) % 7) - 3
    };
    for d in 1..=6 {
        for _ in 0..300 {
            let m: Vec<i128> = (0..d * d).map(|_| next()).collect();
            let expected = laplace(&m, d);
            let mut narrow = m.clone();
            assert_eq!(determinant(&mut narrow, d), Some(expected));
            let mut wide: Vec<Integer> = m.iter().map(|&x| Integer::from(x)).collect();
            assert_eq!(determinant(&mut wide, d), Some(Integer::from(expected)));
        }
    }
    assert_eq!(determinant::<i128>(&mut [], 0), None);
    assert_eq!(determinant(&mut [1_i128, 2, 3], 2), None);
}

#[test]
fn trivial_triangulations_are_regular() {
    let m = mother();
    let check = m.check();
    assert!(check.is_regular(&[]).unwrap());
    assert!(check.is_regular(&[0]).unwrap());
    // Two triangles with a common edge: one wall, always regular.
    let square = Setup::of_vertices(&hypercube(2).unwrap(), Restriction::Unimodular);
    let check = square.check();
    for t in square.all() {
        assert_eq!(t.len(), 2);
        assert!(check.is_regular(&t).unwrap());
    }
}

#[test]
fn invalid_input_is_an_error() {
    let m = mother();
    assert_eq!(
        Regularity::new(&m.points, &m.labels[1..], &m.faces).unwrap_err(),
        RegularError::Shape {
            len: m.labels.len() - 1,
            width: 3
        }
    );
    let mut far = m.labels.clone();
    far[4] = 6;
    assert_eq!(
        Regularity::new(&m.points, &far, &m.faces).unwrap_err(),
        RegularError::UnknownPoint(6)
    );
    let count = m.labels.len() / 3;
    assert_eq!(
        Regularity::new(&m.points, &m.labels[3..], &m.faces).unwrap_err(),
        RegularError::Faces(count - 1)
    );
    let check = m.check();
    let missing = u32::try_from(count).unwrap();
    assert_eq!(
        check.is_regular(&[0, missing]).unwrap_err(),
        RegularError::UnknownSimplex(missing)
    );
    // A simplex twice, and three triangles on the edge A-B.
    let s = m.simplex(&[A, B, AI]);
    assert!(matches!(
        check.is_regular(&[s, s]),
        Err(RegularError::NotATriangulation(_))
    ));
    let fan = m.triangulation(&[&[A, B, AI], &[A, B, BI], &[A, B, CI]]);
    assert!(matches!(
        check.is_regular(&fan),
        Err(RegularError::NotATriangulation(_))
    ));
}

#[test]
fn degenerate_simplices_are_an_error() {
    // Points 0, 1, 2 on a line; the "simplex" {0, 1, 2} has volume zero
    // and shares the edge {0, 1} with the triangle {0, 1, 3}.
    let points = matrix(2, &[0, 0, 1, 0, 2, 0, 0, 1]);
    let facets = facets(&points).unwrap();
    let labels = vec![0, 1, 2, 0, 1, 3];
    let faces = faces(&points, &facets, &labels).unwrap();
    let check = Regularity::new(&points, &labels, &faces).unwrap();
    assert_eq!(
        check.is_regular(&[0, 1]).unwrap_err(),
        RegularError::Degenerate(0)
    );
}

/// The cap of D1 together with [`Quadratic`] (P5.2): the flag and quadratic
/// counts never depend on it, the regular count is a lower bound, and
/// `limited` only without a regular triangulation.
#[test]
fn regularity_test_cap_in_the_quadratic_search() {
    for (name, vertices, triangulations, regular) in julia_cases() {
        let setup = Setup::of_vertices(&vertices, Restriction::Unimodular);
        let check = setup.check();
        let flag = Flag::new(
            &setup.labels,
            setup.points.columns() + 1,
            setup.points.rows(),
        )
        .unwrap();
        let mut flags = 0;
        let mut quadratic = 0;
        for t in setup.all() {
            if flag.is_flag(&t).unwrap() {
                flags += 1;
                quadratic += u64::from(check.is_regular(&t).unwrap());
            }
        }
        let uncapped = Quadratic::new(flag, &check, None);
        let result = serial(&setup.cnf, &options(Keep::None), &uncapped).unwrap();
        let counts = uncapped.counts();
        assert_eq!(result.found, triangulations, "{name}");
        assert_eq!(
            (result.targets, counts.flag, counts.quadratic),
            (quadratic, flags, quadratic),
            "{name}"
        );
        assert!(counts.regular <= regular && (counts.regular > 0) == (regular > 0));
        assert!(!counts.limited);
        for cap in [1, 2, 5] {
            let capped = Quadratic::new(flag, &check, NonZeroU64::new(cap));
            let result = serial(&setup.cnf, &options(Keep::None), &capped).unwrap();
            let counts = capped.counts();
            assert_eq!(result.found, triangulations, "{name} cap {cap}");
            assert_eq!(
                (result.targets, counts.flag, counts.quadratic),
                (quadratic, flags, quadratic),
                "{name} cap {cap}"
            );
            assert!(counts.regular <= regular);
            assert!(!counts.limited || counts.regular == 0);
            assert!(counts.tests <= cap.max(flags) + flags);
        }
    }
}

#[test]
fn regular_learn_exact_certificate_validation_rejects_bad_witnesses() {
    let rows = vec![vec![(0, Integer::from(1))], vec![(0, Integer::from(-1))]];
    let valid = vec![Rational::from((1, 2)), Rational::from((1, 2))];
    verify_conflict(&rows, 1, &valid).unwrap();
    for invalid in [
        vec![],
        vec![Rational::from(1), Rational::from(0)],
        vec![Rational::from(-1), Rational::from(2)],
        vec![Rational::from(1), Rational::from(1)],
    ] {
        assert_eq!(
            verify_conflict(&rows, 1, &invalid),
            Err(RegularError::ConflictCertificate)
        );
    }
    let conflict = nonregular_conflict(&rows, 1, &[[2, 3], [3, 7]], MAX_CERTIFICATE_CELLS)
        .unwrap()
        .unwrap();
    assert_eq!(conflict, vec![2, 3, 7]);
    assert_eq!(
        nonregular_conflict(&rows, 1, &[[2, 3], [3, 7]], 0).unwrap(),
        None
    );
    assert_eq!(
        nonregular_conflict(&rows, u32::MAX, &[[2, 3], [3, 7]], MAX_CERTIFICATE_CELLS).unwrap(),
        None
    );
}

#[test]
fn regular_learn_conflicts_are_strict_certified_subsets_and_prune_multiple_models() {
    let setup = Setup::of_vertices(
        &matrix(3, &[0, 0, 0, 2, 0, 0, 0, 2, 0, 2, 2, 4]),
        Restriction::Unimodular,
    );
    let ordinary = setup.check();
    let learning = ordinary.with_learning(true);
    let all = setup.all();
    let regular: Vec<bool> = all
        .iter()
        .map(|t| ordinary.is_regular(t).unwrap())
        .collect();
    assert_eq!(all.len(), 256);
    assert_eq!(regular.iter().filter(|&&r| r).count(), 158);
    let mut strict = false;
    let mut multiple = false;
    for (triangulation, &is_regular) in all.iter().zip(&regular) {
        let answer = learning.classify(triangulation).unwrap();
        assert_eq!(answer.target, is_regular);
        if is_regular {
            assert!(answer.conflict.is_none());
            continue;
        }
        let conflict = answer.conflict.unwrap();
        assert!(!conflict.is_empty());
        assert!(conflict.iter().all(|s| triangulation.contains(s)));
        strict |= conflict.len() < triangulation.len();
        let mut excluded = 0;
        for (other, &regular) in all.iter().zip(&regular) {
            if conflict.iter().all(|s| other.contains(s)) {
                excluded += 1;
                assert!(!regular, "learned clause excluded a regular triangulation");
            }
        }
        multiple |= excluded > 1;
    }
    assert!(strict, "certificate must improve on full-model blocking");
    assert!(
        multiple,
        "at least one conflict must prune several nonregular completions"
    );
}

#[test]
fn regular_learn_all_modes_preserve_regular_and_quadratic_target_sets() {
    use crate::enumerate::{incremental, incremental_with_pairs};
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap();
    pool.install(|| {
        for setup in [
            mother(),
            Setup::of_vertices(
                &matrix(3, &[0, 0, 0, 2, 0, 0, 0, 2, 0, 2, 2, 4]),
                Restriction::Unimodular,
            ),
        ] {
            let baseline = setup.check();
            let learning = baseline.with_learning(true);
            let center = central(&setup.points, &setup.labels, &setup.faces).unwrap();
            let reduced = crate::cnf::reduced(
                setup.cnf.simplex_variables() as usize,
                &setup.faces,
                &center,
            )
            .unwrap();
            for flag_sat in [false, true] {
                let encode = |cnf| {
                    if flag_sat {
                        crate::cnf::with_flag(
                            cnf,
                            &setup.labels,
                            setup.points.columns() + 1,
                            setup.points.rows(),
                        )
                        .unwrap()
                    } else {
                        cnf
                    }
                };
                let full = encode(setup.cnf.clone());
                let reduced = encode(reduced.clone());
                for backend in [Backend::PicoSat, Backend::Cadical] {
                    let opts = Options {
                        backend,
                        ..options(Keep::All)
                    };
                    let flag = Flag::new(
                        &setup.labels,
                        setup.points.columns() + 1,
                        setup.points.rows(),
                    )
                    .unwrap();
                    let baseline_quadratic = Quadratic::new(flag, &baseline, None);
                    let expected_regular = serial(&full, &opts, &baseline).unwrap();
                    let expected_quadratic = serial(&full, &opts, &baseline_quadratic).unwrap();
                    for mode in 0..4 {
                        let quadratic = Quadratic::new(flag, &learning, None);
                        macro_rules! run {
                            ($classifier:expr) => {
                                match mode {
                                    0 => serial(&full, &opts, $classifier),
                                    1 => parallel_split(&full, &setup.central, &opts, $classifier),
                                    2 => incremental(
                                        &reduced,
                                        &setup.points,
                                        &setup.labels,
                                        &opts,
                                        $classifier,
                                    ),
                                    _ => incremental_with_pairs(
                                        &reduced,
                                        &setup.points,
                                        &setup.labels,
                                        &opts,
                                        $classifier,
                                        &crate::intersect::stream_pairs,
                                    ),
                                }
                                .unwrap()
                            };
                        }
                        for (actual, expected) in [
                            (run!(&learning), &expected_regular),
                            (run!(&quadratic), &expected_quadratic),
                        ] {
                            assert_eq!(actual.outcome, Outcome::Complete);
                            assert_eq!(actual.targets, expected.targets);
                            assert_eq!(
                                actual.triangulations, expected.triangulations,
                                "{backend:?}, mode={mode}, flag_sat={flag_sat}"
                            );
                            assert!(actual.found <= expected.found);
                        }
                    }
                }
            }
        }
    });
}

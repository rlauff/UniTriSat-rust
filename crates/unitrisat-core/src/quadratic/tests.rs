use super::*;
use crate::cnf::formula;
use crate::enumerate::{self, Goal, Keep, Options, Outcome};
use crate::families::scaled_simplex;
use crate::hull::facets;
use crate::intersect::intersecting_pairs;
use crate::lattice::{Restriction, faces, lattice_points, simplices};
use crate::solve::Backend;
use std::{collections::BTreeSet, sync::Mutex};

#[derive(Debug, thiserror::Error)]
#[error("stub failure")]
struct StubError;

/// A stand-in for the regularity test: the listed triangulations are
/// regular (all of them if `None`). Records every call.
struct Stub {
    regular: Option<BTreeSet<Vec<u32>>>,
    calls: Mutex<Vec<Vec<u32>>>,
    fail: bool,
}

impl Stub {
    fn new(regular: Option<&[&[u32]]>) -> Self {
        Self {
            regular: regular.map(|r| r.iter().map(|t| t.to_vec()).collect()),
            calls: Mutex::default(),
            fail: false,
        }
    }

    fn calls(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

impl Classify for Stub {
    type Error = StubError;

    fn is_target(&self, simplices: &[u32]) -> Result<bool, StubError> {
        self.calls.lock().unwrap().push(simplices.to_vec());
        if self.fail {
            return Err(StubError);
        }
        Ok(self.regular.as_ref().is_none_or(|r| r.contains(simplices)))
    }
}

/// Edges `0: {0, 1}`, `1: {0, 2}`, `2: {1, 2}`, `3: {2, 3}`: the
/// "triangulations" `[0, 1, 2]` and `[0, 1, 2, 3]` contain a triangle's
/// boundary and are not flag; the others are.
const EDGES: [u32; 8] = [0, 1, 0, 2, 1, 2, 2, 3];
const NOT_FLAG_A: &[u32] = &[0, 1, 2];
const NOT_FLAG_B: &[u32] = &[0, 1, 2, 3];
const NOT_FLAG_C: &[u32] = &[0, 1, 2]; // the same set, seen again
const FLAG_A: &[u32] = &[0, 1];
const FLAG_B: &[u32] = &[1, 2, 3];
const FLAG_C: &[u32] = &[3];

fn flag() -> Flag<'static> {
    Flag::new(&EDGES, 2, 4).unwrap()
}

#[test]
fn edge_fixture_flagness() {
    let f = flag();
    for t in [NOT_FLAG_A, NOT_FLAG_B, NOT_FLAG_C] {
        assert!(!f.is_flag(t).unwrap());
    }
    for t in [FLAG_A, FLAG_B, FLAG_C] {
        assert!(f.is_flag(t).unwrap());
    }
}

#[test]
fn regularity_is_tested_only_where_it_matters() {
    let stub = Stub::new(Some(&[NOT_FLAG_B, FLAG_B]));
    let q = Quadratic::new(flag(), &stub, None);
    // Non-flag, not regular: tested, RUT still open.
    assert!(!q.is_target(NOT_FLAG_A).unwrap());
    // Non-flag, regular: tested; RUT decided.
    assert!(!q.is_target(NOT_FLAG_B).unwrap());
    assert_eq!(stub.calls(), 2);
    // Non-flag after RUT is decided: no test.
    assert!(!q.is_target(NOT_FLAG_C).unwrap());
    assert_eq!(stub.calls(), 2);
    // Flag triangulations are always tested; only regular ones are targets.
    assert!(!q.is_target(FLAG_A).unwrap());
    assert!(q.is_target(FLAG_B).unwrap());
    assert_eq!(stub.calls(), 4);
    assert_eq!(
        q.counts(),
        Counts {
            flag: 2,
            regular: 2,
            quadratic: 1,
            tests: 4,
            limited: false
        }
    );
}

#[test]
fn the_cap_skips_only_undecided_non_flag_tests() {
    let stub = Stub::new(Some(&[FLAG_C]));
    let q = Quadratic::new(flag(), &stub, NonZeroU64::new(2));
    // A flag LP counts towards the cap.
    assert!(!q.is_target(FLAG_A).unwrap());
    assert!(!q.is_target(NOT_FLAG_A).unwrap());
    assert_eq!(stub.calls(), 2);
    // Cap reached: the next non-flag test is skipped, RUT unknown.
    assert!(!q.is_target(NOT_FLAG_B).unwrap());
    assert_eq!(stub.calls(), 2);
    assert!(q.counts().limited);
    // Flag triangulations are still tested beyond the cap.
    assert!(!q.is_target(FLAG_B).unwrap());
    assert_eq!(stub.calls(), 3);
    assert_eq!(q.counts().tests, 3);
    // A regular flag triangulation decides RUT after all.
    assert!(q.is_target(FLAG_C).unwrap());
    assert_eq!(
        q.counts(),
        Counts {
            flag: 3,
            regular: 1,
            quadratic: 1,
            tests: 4,
            limited: false
        }
    );
}

#[test]
fn failures_are_reported() {
    let mut stub = Stub::new(None);
    stub.fail = true;
    let q = Quadratic::new(flag(), &stub, None);
    assert!(matches!(
        q.is_target(FLAG_A),
        Err(QuadraticError::Regular(StubError))
    ));
    assert!(matches!(
        q.is_target(&[4]),
        Err(QuadraticError::Flag(FlagError::UnknownSimplex(4)))
    ));
}

#[test]
fn as_an_enumeration_hook() {
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
        backend: Backend::PicoSat,
        goal: Goal::All,
        keep: Keep::All,
        max_solutions: None,
        deadline: None,
    };
    let flags = enumerate::serial(&cnf, &options, &flag).unwrap();
    assert_eq!(flags.found, 192);
    // Every triangulation regular: the quadratic ones are the flag ones.
    let all = Stub::new(None);
    let q = Quadratic::new(flag, &all, None);
    let result = enumerate::serial(&cnf, &options, &q).unwrap();
    assert_eq!(result.outcome, Outcome::Complete);
    assert_eq!(result.triangulations, flags.triangulations);
    let counts = q.counts();
    assert_eq!(
        (counts.flag, counts.quadratic),
        (flags.targets, flags.targets)
    );
    assert!(counts.regular >= counts.quadratic && counts.regular <= counts.quadratic + 1);
    // Besides the flag ones, only a non-flag triangulation seen before the
    // first regular one was tested.
    assert_eq!(
        counts.tests,
        counts.flag + (counts.regular - counts.quadratic)
    );
    // Only the first flag triangulation regular, split over threads.
    let first = flags.triangulations[0].clone();
    let one = Stub::new(Some(&[&first]));
    let q = Quadratic::new(flag, &one, NonZeroU64::new(1));
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    let split = pool
        .install(|| {
            enumerate::parallel_split(
                &cnf,
                central.simplices(),
                &Options {
                    keep: Keep::None,
                    ..options
                },
                &q,
            )
        })
        .unwrap();
    assert_eq!((split.found, split.targets), (192, 1));
    let counts = q.counts();
    assert_eq!(
        (counts.flag, counts.regular, counts.quadratic),
        (flags.targets, 1, 1)
    );
    // Every flag triangulation was tested, and at most the one capped
    // non-flag test besides them.
    assert!(counts.tests >= flags.targets && counts.tests <= flags.targets + 1);
    assert!(!counts.limited);
}

#[test]
fn regular_learn_never_reuses_a_conflict_for_skipped_or_successful_tests() {
    struct Learning(Stub);
    impl Classify for Learning {
        type Error = StubError;
        fn is_target(&self, simplices: &[u32]) -> Result<bool, StubError> {
            self.0.is_target(simplices)
        }
        fn classify(&self, simplices: &[u32]) -> Result<Classification, StubError> {
            let target = self.is_target(simplices)?;
            Ok(Classification {
                target,
                conflict: (!target).then(|| simplices.to_vec()),
            })
        }
    }
    let regular = Learning(Stub::new(Some(&[FLAG_C])));
    let classifier = Quadratic::new(flag(), &regular, NonZeroU64::new(1));
    let first = classifier.classify(NOT_FLAG_A).unwrap();
    assert!(!first.target);
    assert_eq!(first.conflict, Some(NOT_FLAG_A.to_vec()));
    let capped = classifier.classify(NOT_FLAG_B).unwrap();
    assert_eq!(capped, false.into());
    assert_eq!(regular.0.calls(), 1);
    let accepted = classifier.classify(FLAG_C).unwrap();
    assert_eq!(accepted, true.into());
    let skipped = classifier.classify(NOT_FLAG_C).unwrap();
    assert_eq!(skipped, false.into());
    assert_eq!(regular.0.calls(), 2);
}

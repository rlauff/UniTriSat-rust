//! Quadratic triangulations: regular and flag.
//!
//! A unimodular triangulation that is regular and flag gives a quadratic
//! Gröbner basis of the toric ideal; Julia counts such triangulations as
//! *quadratic* when both filters are on (`record_solution!` in `solve.jl`,
//! `--regular --flag` in `chunk_run.jl`). [`Quadratic`] is that
//! combination as an [`enumerate`](crate::enumerate) hook: its targets are
//! the triangulations that are flag ([`Flag`]) and regular (any
//! [`Classify`] hook whose targets are the regular triangulations), and it
//! keeps Julia's four counts, so that one search answers whether the
//! polytope has a unimodular, a regular, a flag and a quadratic
//! triangulation.
//!
//! As in Julia, flagness is tested first, and the regularity test, an
//! exact LP, runs only where its answer can still matter:
//! - on every flag triangulation (it decides quadraticity);
//! - on the others only while no regular triangulation has been seen and
//!   fewer than the regularity-test cap of tests (D1) have run; tests on
//!   flag triangulations count towards the cap.
//!
//! Without learned conflicts, [`Counts::flag`] and [`Counts::quadratic`] are
//! exact for a complete search. With conflicts from the regularity hook, the
//! quadratic count stays exact but the flag count includes only encountered
//! candidates, since nonregular flag triangulations may be pruned. In both cases, [`Counts::regular`] is a lower bound, and `regular > 0` still
//! decides "has a regular triangulation", unless [`Counts::limited`] says
//! that the cap skipped a test before any regular triangulation was seen.
//!
//! With one filter only, the hooks are used directly: [`Flag`] alone counts
//! the flag triangulations as targets, and Julia tests regularity on every
//! triangulation when flagness is off.

use crate::enumerate::{Classification, Classify};
use crate::flag::{Flag, FlagError};
use std::{
    num::NonZeroU64,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

/// Failure of [`Quadratic`]: one of its two checks failed.
#[derive(Debug, thiserror::Error)]
pub enum QuadraticError<E: std::error::Error + 'static> {
    /// The flag check failed.
    #[error(transparent)]
    Flag(#[from] FlagError),
    /// The regularity check failed.
    #[error("regularity check failed: {0}")]
    Regular(#[source] E),
}

/// The counts of a [`Quadratic`] search so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Counts {
    /// Flag triangulations (Julia's `number_of_flag_triangulations_found`).
    pub flag: u64,
    /// Regular triangulations among those tested, a lower bound (Julia's
    /// `number_of_regular_triangulations_found`).
    pub regular: u64,
    /// Regular flag triangulations (Julia's
    /// `number_of_quadratic_triangulations_found`); equal to the search's
    /// targets.
    pub quadratic: u64,
    /// Regularity tests run.
    pub tests: u64,
    /// The cap skipped a regularity test and no regular triangulation has
    /// been seen: "has a regular triangulation" is unknown (table status
    /// `reglimit`). A completed search still has an exact quadratic count;
    /// learned conflicts can omit nonregular flag candidates.
    pub limited: bool,
}

/// Targets the triangulations that are flag and regular, and counts flag,
/// regular and quadratic triangulations as Julia does. It may be used for
/// one search only, possibly from several threads.
#[derive(Debug)]
pub struct Quadratic<'a, R, F = Flag<'a>> {
    flag: F,
    regular: &'a R,
    max_tests: Option<NonZeroU64>,
    flags: AtomicU64,
    regulars: AtomicU64,
    quadratics: AtomicU64,
    tests: AtomicU64,
    skipped: AtomicBool,
}

impl<'a, R: Classify, F: Classify<Error = FlagError>> Quadratic<'a, R, F> {
    /// The combination of `flag` and the regularity check `regular`, with
    /// at most `max_tests` regularity tests (D1's regularity-test cap) while
    /// "has a regular triangulation" is undecided.
    pub fn new(flag: F, regular: &'a R, max_tests: Option<NonZeroU64>) -> Self {
        Self {
            flag,
            regular,
            max_tests,
            flags: AtomicU64::new(0),
            regulars: AtomicU64::new(0),
            quadratics: AtomicU64::new(0),
            tests: AtomicU64::new(0),
            skipped: AtomicBool::new(false),
        }
    }

    /// The flag classifier, which may carry optional observation state.
    pub fn flag_classifier(&self) -> &F {
        &self.flag
    }

    /// The counts so far; final once the search has returned.
    pub fn counts(&self) -> Counts {
        let regular = self.regulars.load(Ordering::SeqCst);
        Counts {
            flag: self.flags.load(Ordering::SeqCst),
            regular,
            quadratic: self.quadratics.load(Ordering::SeqCst),
            tests: self.tests.load(Ordering::SeqCst),
            limited: regular == 0 && self.skipped.load(Ordering::SeqCst),
        }
    }

    /// Whether a non-flag triangulation is tested: only while no regular
    /// triangulation has been seen, and then only within the cap, whose slot
    /// is claimed here.
    fn claim_undecided_test(&self) -> bool {
        if self.regulars.load(Ordering::SeqCst) > 0 {
            return false;
        }
        let cap = self.max_tests.map(NonZeroU64::get);
        let claimed = self
            .tests
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |tests| {
                if cap.is_some_and(|cap| tests >= cap) {
                    None
                } else {
                    Some(tests.saturating_add(1))
                }
            })
            .is_ok();
        if !claimed {
            self.skipped.store(true, Ordering::SeqCst);
        }
        claimed
    }
}

/// Adds one to a counter.
fn bump(counter: &AtomicU64) {
    // Saturating: at most one per triangulation, far below u64::MAX.
    let _ = counter.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_add(1));
}

impl<R: Classify, F: Classify<Error = FlagError>> Classify for Quadratic<'_, R, F> {
    type Error = QuadraticError<R::Error>;

    fn is_target(&self, simplices: &[u32]) -> Result<bool, Self::Error> {
        Ok(self.classify(simplices)?.target)
    }

    fn classify(&self, simplices: &[u32]) -> Result<Classification, Self::Error> {
        let flag = self.flag.is_target(simplices)?;
        if flag {
            bump(&self.flags);
            bump(&self.tests);
        } else if !self.claim_undecided_test() {
            return Ok(false.into());
        }
        let mut regular = self
            .regular
            .classify(simplices)
            .map_err(QuadraticError::Regular)?;
        if regular.target {
            bump(&self.regulars);
            if flag {
                bump(&self.quadratics);
            }
        }
        regular.target &= flag;
        Ok(regular)
    }
}

#[cfg(test)]
mod tests;

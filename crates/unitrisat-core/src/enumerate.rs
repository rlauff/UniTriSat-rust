//! Enumeration of the triangulations: the solving modes of Julia's
//! `solve.jl`.
//!
//! The models of [`crate::cnf::formula`] are the triangulations. Each mode
//! solves, records the model, blocks it ([`Solver::block`]) and solves again
//! until the formula is unsatisfiable or the search ends early:
//! - [`serial`]: one solver on the formula (Julia's `solve_picosat` and
//!   `solve_cadical_standard`);
//! - [`parallel_split`]: the central simplices
//!   ([`crate::lattice::central`]) are dealt round-robin into one group per
//!   thread, at most one group per central simplex, and each group's solver
//!   gets the formula plus the clause "some simplex of this group is used".
//!   Every triangulation uses exactly one central simplex, so the groups
//!   split the triangulations into disjoint classes (Julia's
//!   `solve_parallel`);
//! - [`incremental`]: one solver on the smaller formula
//!   [`crate::cnf::reduced`], while worker threads compute the
//!   intersection clauses and the solver is paused now and then to add them
//!   (Julia's `solve_cadical_incremental`).
//!
//! All modes take the same [`Options`]:
//! - [`Goal`]: stop at the first target triangulation, or enumerate all;
//! - [`Keep`]: which target triangulations to return;
//! - the solution cap and the deadline of D1.
//!
//! A [`Classify`] hook decides which triangulations are targets; this is
//! where per-triangulation checks such as regularity plug in. With
//! [`AnyTriangulation`] every triangulation is a target.
//!
//! **Honest results.** The [`Outcome`] says whether the counts are exact:
//! only [`Outcome::Complete`] means the search was exhausted and the target
//! count is exact. Classifier conflicts may prune non-target triangulations:
//! `found` always counts only encountered candidates, even on completion.
//! A search that
//! stops at the cap reports [`Outcome::Capped`] even if no further
//! triangulation exists, and its counts are lower bounds; one that hits the
//! deadline reports [`Outcome::TimedOut`]. The cap is global in every mode.
//!
//! **Deterministic output.** Triangulations are lists of increasing simplex
//! indices, and [`Keep::All`] returns them sorted, so every mode, backend
//! and thread count returns the same list for a complete search. Which
//! triangulation is found first may differ between modes and runs, as in
//! Julia (REVIEW.md R6).

use crate::cnf::Cnf;
use crate::intersect::IntersectError;
use crate::solve::{Backend, Control, SolveError, Solver, Status};
use rayon::prelude::*;
use std::{
    convert::Infallible,
    num::NonZeroU64,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

mod incremental;

pub use incremental::{
    IncrementalProgress, incremental, incremental_with_pairs, incremental_with_pairs_and_progress,
    incremental_with_progress,
};

/// When a search may stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Goal {
    /// At the first target triangulation (Julia's `find_all = false`).
    First,
    /// Only when every triangulation has been seen (`find_all = true`).
    All,
}

/// Which target triangulations to return (Julia's `return_triangulations`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Keep {
    /// None, only counts.
    None,
    /// The first one found.
    First,
    /// All of them, sorted.
    All,
}

/// Settings shared by all modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// The SAT solver.
    pub backend: Backend,
    /// When to stop.
    pub goal: Goal,
    /// Which target triangulations to return.
    pub keep: Keep,
    /// Stop after this many triangulations (D1's solution cap); the search
    /// is then [`Outcome::Capped`].
    pub max_solutions: Option<NonZeroU64>,
    /// Stop when this instant has passed (D1's per-job timeout); the search
    /// is then [`Outcome::TimedOut`].
    pub deadline: Option<Instant>,
}

/// How a search ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Outcome {
    /// The search was exhausted: the target count is exact. Certified
    /// conflicts may have excluded non-targets, so `found` counts only the
    /// candidates actually encountered, not all unfiltered triangulations.
    Complete,
    /// [`Goal::First`] found a target triangulation and stopped: a target
    /// exists, and the counts are those seen until then.
    Found,
    /// The solution cap was reached before the search ended: the counts are
    /// lower bounds (table status `capped`).
    Capped,
    /// The deadline passed: the counts are lower bounds.
    TimedOut,
}

/// The result of a search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enumeration {
    /// How the search ended, and so what the counts mean.
    pub outcome: Outcome,
    /// Triangulations encountered (Julia's `number_of_triangulations_found`).
    /// With classifier conflicts, omitted non-targets are not counted.
    pub found: u64,
    /// Of those, the targets.
    pub targets: u64,
    /// The target triangulations kept by [`Keep`], each a list of increasing
    /// zero-based simplex indices.
    pub triangulations: Vec<Vec<u32>>,
}

/// One classification result, optionally with a certified rejection conflict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Classification {
    /// Whether this triangulation is a target.
    pub target: bool,
    /// A nonempty subset of this rejected triangulation's zero-based simplex
    /// indices whose joint selection is impossible in any target. Enumerators
    /// may exclude every triangulation containing this subset. The classifier
    /// must prove this stronger condition, not merely reject the current model.
    pub conflict: Option<Vec<u32>>,
}

impl From<bool> for Classification {
    fn from(target: bool) -> Self {
        Self {
            target,
            conflict: None,
        }
    }
}

/// Decides which triangulations are targets. It runs once per triangulation
/// found, possibly from several threads at once ([`parallel_split`]), and
/// may keep its own counts.
pub trait Classify: Sync {
    /// Failure of the check; it ends the search.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Whether the triangulation made of `simplices` (increasing zero-based
    /// simplex indices) is a target.
    fn is_target(&self, simplices: &[u32]) -> Result<bool, Self::Error>;

    /// Classifies one model, optionally returning a certified conflict that
    /// excludes other non-target models. The default preserves ordinary exact
    /// enumeration. With conflicts, `found` counts only encountered candidates.
    fn classify(&self, simplices: &[u32]) -> Result<Classification, Self::Error> {
        self.is_target(simplices).map(Classification::from)
    }

    /// Whether every triangulation is accepted, without filtering. One-shot
    /// backends require this guarantee before solving. Wrappers that preserve
    /// classification may forward it; the conservative default is false.
    fn accepts_all(&self) -> bool {
        false
    }

    /// Registers a solver interruption flag for optional external cancellation.
    /// Setting it aborts a search without a normal termination reason. An
    /// incremental solver can reset its pause flag between solver calls;
    /// cancellation owners must keep signalling until the search exits.
    /// The default hook does nothing.
    fn register_stop(&self, _stop: &Arc<AtomicBool>) {}
}

/// Every triangulation is a target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnyTriangulation;

impl Classify for AnyTriangulation {
    type Error = Infallible;

    fn is_target(&self, _: &[u32]) -> Result<bool, Infallible> {
        Ok(true)
    }

    fn accepts_all(&self) -> bool {
        true
    }
}

/// Failure of a search.
#[derive(Debug, thiserror::Error)]
pub enum EnumerateError<E: std::error::Error + 'static> {
    /// A solver call failed.
    #[error(transparent)]
    Solve(#[from] SolveError),
    /// The intersection clauses of incremental solving could not be
    /// computed.
    #[error(transparent)]
    Intersect(#[from] IntersectError),
    /// The [`Classify`] hook failed.
    #[error("triangulation check failed: {0}")]
    Check(#[source] E),
    /// The simplices or central simplices do not belong to the formula.
    #[error("the simplices do not belong to the formula's {0} variables")]
    Mismatch(u32),
    /// A classifier supplied an empty, accepted, or non-subset conflict.
    #[error("classifier conflict must be a nonempty subset of a rejected triangulation")]
    InvalidConflict,
    /// An internal invariant failed. This indicates a bug.
    #[error("internal invariant of the enumeration violated")]
    Internal,
}

/// The result type of the searches.
type EnumResult<T, C> = Result<T, EnumerateError<<C as Classify>::Error>>;

/// All triangulations of `cnf` (the formula of [`crate::cnf::formula`]) with
/// one solver.
pub fn serial<C: Classify>(
    cnf: &Cnf,
    options: &Options,
    classify: &C,
) -> EnumResult<Enumeration, C> {
    validate_backend(options, classify)?;
    let search = Search::new(options, classify);
    let kept = search.run(cnf, None);
    search.finish(vec![kept])
}

/// All triangulations of `cnf` (the formula of [`crate::cnf::formula`]),
/// split by the `central` simplices ([`crate::lattice::Central::simplices`]
/// of the same simplices) over the threads of the current rayon pool.
///
/// With `G` groups, group `g` gets the central simplices `g, g + G, ...` and
/// one solver. `G` is the smaller of the thread count and the number of
/// central simplices. With no central simplex this is [`serial`].
pub fn parallel_split<C: Classify>(
    cnf: &Cnf,
    central: &[u32],
    options: &Options,
    classify: &C,
) -> EnumResult<Enumeration, C> {
    validate_backend(options, classify)?;
    let count = central.len().min(rayon::current_num_threads()).max(1);
    let mut groups = vec![Vec::new(); count];
    for (group, &simplex) in (0..count).cycle().zip(central) {
        let literal = i32::try_from(simplex)
            .ok()
            .and_then(|s| s.checked_add(1))
            .filter(|&l| l.unsigned_abs() <= cnf.simplex_variables())
            .ok_or(EnumerateError::Mismatch(cnf.simplex_variables()))?;
        groups
            .get_mut(group)
            .ok_or(EnumerateError::Internal)?
            .push(literal);
    }
    let search = Search::new(options, classify);
    let kept: Vec<_> = if central.is_empty() {
        vec![search.run(cnf, None)]
    } else {
        groups
            .par_iter()
            .map(|group| search.run(cnf, Some(group)))
            .collect()
    };
    search.finish(kept)
}

fn validate_backend<C: Classify>(options: &Options, classify: &C) -> Result<(), SolveError> {
    if options.backend == Backend::Kissat {
        if options.goal != Goal::First {
            return Err(SolveError::KissatUnsupported("enumerating all models"));
        }
        if !classify.accepts_all() {
            return Err(SolveError::KissatUnsupported(
                "filtered triangulation search",
            ));
        }
    }
    Ok(())
}

/// The state of one search, shared by its solvers.
struct Search<'a, C: Classify> {
    options: &'a Options,
    classify: &'a C,
    /// Interrupts every solver of this search. It is set only together with
    /// one of `target_found`, `capped` or `failed`.
    stop: Arc<AtomicBool>,
    /// Triangulations counted, at most the cap.
    found: AtomicU64,
    targets: AtomicU64,
    target_found: AtomicBool,
    capped: AtomicBool,
    failed: AtomicBool,
    timed_out: AtomicBool,
    /// Some solver was interrupted by `stop`.
    interrupted: AtomicBool,
}

impl<'a, C: Classify> Search<'a, C> {
    fn new(options: &'a Options, classify: &'a C) -> Self {
        let stop = Arc::default();
        classify.register_stop(&stop);
        Self {
            options,
            classify,
            stop,
            found: AtomicU64::new(0),
            targets: AtomicU64::new(0),
            target_found: AtomicBool::new(false),
            capped: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            timed_out: AtomicBool::new(false),
            interrupted: AtomicBool::new(false),
        }
    }

    /// Sets `reason` and stops every solver of the search.
    fn end(&self, reason: &AtomicBool) {
        reason.store(true, Ordering::SeqCst);
        self.stop.store(true, Ordering::SeqCst);
    }

    /// Whether the deadline has passed; if so, records it.
    fn deadline_passed(&self) -> bool {
        let passed = self
            .options
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline);
        if passed {
            self.timed_out.store(true, Ordering::SeqCst);
        }
        passed
    }

    /// Counts and classifies the triangulation with the true variables
    /// `model` and keeps it as [`Keep`] asks. Returns whether the search
    /// goes on.
    fn record(
        &self,
        model: &[u32],
        simplices: &mut Vec<u32>,
        kept: &mut Vec<Vec<u32>>,
        conflict: &mut Vec<u32>,
    ) -> EnumResult<bool, C> {
        conflict.clear();
        simplices.clear();
        for &variable in model {
            simplices.push(variable.checked_sub(1).ok_or(EnumerateError::Internal)?);
        }
        let cap = self.options.max_solutions.map(NonZeroU64::get);
        // Claim one of the cap's slots; none left means another solver
        // reached the cap first, and this triangulation is not counted.
        let claimed = self
            .found
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |found| {
                if cap.is_some_and(|cap| found >= cap) {
                    None
                } else {
                    found.checked_add(1)
                }
            });
        let Ok(previous) = claimed else {
            self.end(&self.capped);
            return Ok(false);
        };
        let answer = self.classify.classify(simplices).map_err(|error| {
            self.end(&self.failed);
            EnumerateError::Check(error)
        })?;
        if let Some(mut selected) = answer.conflict {
            if answer.target
                || selected.is_empty()
                || selected.iter().any(|s| simplices.binary_search(s).is_err())
            {
                self.end(&self.failed);
                return Err(EnumerateError::InvalidConflict);
            }
            selected.sort_unstable();
            selected.dedup();
            for simplex in selected {
                conflict.push(simplex.checked_add(1).ok_or(EnumerateError::Internal)?);
            }
        }
        if answer.target {
            self.targets.fetch_add(1, Ordering::SeqCst);
            let keep = match self.options.keep {
                Keep::None => false,
                Keep::First => kept.is_empty(),
                Keep::All => true,
            };
            if keep {
                kept.push(simplices.clone());
            }
            if self.options.goal == Goal::First {
                self.end(&self.target_found);
                return Ok(false);
            }
        }
        // After the target test, so that a target found with the last
        // permitted triangulation still counts as found (as in Julia).
        if cap.is_some_and(|cap| previous.saturating_add(1) >= cap) {
            self.end(&self.capped);
            return Ok(false);
        }
        Ok(true)
    }

    /// Enumerates the models of `cnf`, plus the clause `group` if given,
    /// with a new solver. Returns the kept triangulations.
    fn run(&self, cnf: &Cnf, group: Option<&[i32]>) -> EnumResult<Vec<Vec<u32>>, C> {
        let result = self.solve(cnf, group);
        if result.is_err() {
            self.end(&self.failed);
        }
        result
    }

    fn solve(&self, cnf: &Cnf, group: Option<&[i32]>) -> EnumResult<Vec<Vec<u32>>, C> {
        let mut kept = Vec::new();
        if self.stop.load(Ordering::SeqCst) {
            // Another solver already ended the search.
            self.interrupted.store(true, Ordering::SeqCst);
            return Ok(kept);
        }
        let variables = usize::try_from(cnf.variables()).map_err(|_| EnumerateError::Internal)?;
        let mut solver = Solver::new(self.options.backend, variables)?;
        for clause in cnf.iter().chain(group) {
            solver.add_clause(clause)?;
        }
        let control = Control {
            stop: Arc::clone(&self.stop),
            deadline: self.options.deadline,
        };
        let mut model = Vec::new();
        let mut simplices = Vec::new();
        let mut conflict = Vec::new();
        loop {
            if self.deadline_passed() {
                break;
            }
            match solver.solve(&[], &control)? {
                Status::Sat => {
                    solver.model_prefix(cnf.simplex_variables(), &mut model)?;
                    if !self.record(&model, &mut simplices, &mut kept, &mut conflict)? {
                        break;
                    }
                    solver.block(if conflict.is_empty() {
                        &model
                    } else {
                        &conflict
                    })?;
                }
                Status::Unsat => break,
                Status::TimedOut => {
                    self.timed_out.store(true, Ordering::SeqCst);
                    break;
                }
                Status::Stopped => {
                    self.interrupted.store(true, Ordering::SeqCst);
                    break;
                }
            }
        }
        Ok(kept)
    }

    /// The result, from the kept triangulations of each solver (in a fixed
    /// order of the solvers) or the first error.
    fn finish(self, kept: Vec<EnumResult<Vec<Vec<u32>>, C>>) -> EnumResult<Enumeration, C> {
        let kept: Vec<Vec<Vec<u32>>> = kept.into_iter().collect::<EnumResult<_, C>>()?;
        let flag = |flag: &AtomicBool| flag.load(Ordering::SeqCst);
        let outcome = if flag(&self.target_found) {
            Outcome::Found
        } else if flag(&self.capped) {
            Outcome::Capped
        } else if flag(&self.timed_out) {
            Outcome::TimedOut
        } else if flag(&self.interrupted) || flag(&self.failed) {
            // A solver stopped without a reason: the search is incomplete.
            return Err(EnumerateError::Internal);
        } else {
            Outcome::Complete
        };
        let triangulations = match self.options.keep {
            Keep::None => Vec::new(),
            Keep::First => kept.into_iter().flatten().take(1).collect(),
            Keep::All => {
                let mut all: Vec<Vec<u32>> = kept.into_iter().flatten().collect();
                all.sort_unstable();
                all
            }
        };
        Ok(Enumeration {
            outcome,
            found: self.found.load(Ordering::SeqCst),
            targets: self.targets.load(Ordering::SeqCst),
            triangulations,
        })
    }
}

#[cfg(test)]
mod tests;

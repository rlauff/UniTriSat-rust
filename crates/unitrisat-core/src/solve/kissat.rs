//! Safe, deliberately one-shot adapter around RustSAT's maintained binding.
//!
//! RustSAT resets its state to Input after UNKNOWN, but native Kissat forbids
//! another search or clause insertion even then. `searched` guards every such
//! call. The quiet+safe Cargo features suppress native output/global terminal
//! state and subprocess I/O; do not remove them when updating the dependency.

use super::{Control, SolveError, Status, status};
use rustsat::{
    solvers::{ControlSignal, Solve, SolverResult, Terminate},
    types::{Clause, Lit, TernaryVal},
};
use std::{
    marker::PhantomData,
    panic::catch_unwind,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

#[derive(Debug)]
pub(super) struct Solver {
    native: rustsat_kissat::Kissat<'static>,
    clause: Clause,
    searched: bool,
    // RustSAT declares Kissat Send even with non-Send callbacks. Keep the
    // adapter thread-confined, matching the other UniTriSat native solvers.
    thread_confined: PhantomData<Rc<()>>,
}

#[derive(Default)]
struct Observed {
    stopped: AtomicBool,
    deadline: AtomicBool,
    panicked: AtomicBool,
}

impl Observed {
    fn poll(&self, control: &Control) -> bool {
        let stopped = control.stop.load(Ordering::SeqCst);
        let deadline = control
            .deadline
            .is_some_and(|limit| Instant::now() >= limit);
        if stopped {
            self.stopped.store(true, Ordering::SeqCst);
        }
        if deadline {
            self.deadline.store(true, Ordering::SeqCst);
        }
        stopped || deadline
    }
}

impl Solver {
    pub(super) fn new() -> Self {
        Self {
            native: rustsat_kissat::Kissat::default(),
            clause: Clause::new(),
            searched: false,
            thread_confined: PhantomData,
        }
    }

    pub(super) fn check_input(&self) -> Result<(), SolveError> {
        if self.searched {
            Err(SolveError::KissatUnsupported(
                "adding or blocking clauses after a solve",
            ))
        } else {
            Ok(())
        }
    }

    pub(super) fn check_solve(&self, assumptions: bool) -> Result<(), SolveError> {
        if assumptions {
            Err(SolveError::KissatUnsupported("assumptions"))
        } else if self.searched {
            Err(SolveError::KissatUnsupported(
                "repeated solves, including after interruption",
            ))
        } else {
            Ok(())
        }
    }

    pub(super) fn add_clause(&mut self, literals: &[i32]) -> Result<(), SolveError> {
        self.check_input()?;
        drop(self.clause.drain(..));
        for &literal in literals {
            // The outer solver validates the full clause before entering here.
            self.clause.add(
                Lit::from_ipasir(literal)
                    .map_err(|_| SolveError::KissatFailure("converting a validated literal"))?,
            );
        }
        self.native
            .add_clause_ref(&self.clause)
            .map_err(|_| SolveError::KissatFailure("adding a clause"))
    }

    pub(super) fn solve(&mut self, control: &Control) -> Result<Status, SolveError> {
        self.check_solve(false)?;
        // A consumed call stays consumed even if interrupted before native
        // entry. Never rely on RustSAT's post-UNKNOWN state for this guard.
        self.searched = true;
        let observed = Arc::new(Observed::default());
        if observed.poll(control) {
            return status(
                None,
                observed.stopped.load(Ordering::SeqCst),
                observed.deadline.load(Ordering::SeqCst),
            );
        }
        let callback_observed = Arc::clone(&observed);
        let callback_control = control.clone();
        // All captures are owned and 'static. No user callback runs here.
        // Catch an unexpected Rust panic before returning through C's ABI.
        self.native.attach_terminator(move || {
            match catch_unwind(|| callback_observed.poll(&callback_control)) {
                Ok(false) => ControlSignal::Continue,
                Ok(true) => ControlSignal::Terminate,
                Err(_) => {
                    callback_observed.panicked.store(true, Ordering::SeqCst);
                    ControlSignal::Terminate
                }
            }
        });
        let answer = self.native.solve();
        self.native.detach_terminator();
        if observed.panicked.load(Ordering::SeqCst) {
            return Err(SolveError::KissatFailure(
                "polling the interruption callback",
            ));
        }
        let answer = match answer.map_err(|_| SolveError::KissatFailure("solving"))? {
            SolverResult::Sat => Some(true),
            SolverResult::Unsat => Some(false),
            SolverResult::Interrupted => None,
        };
        status(
            answer,
            observed.stopped.load(Ordering::SeqCst),
            observed.deadline.load(Ordering::SeqCst),
        )
    }

    pub(super) fn value(&self, variable: i32) -> Result<bool, SolveError> {
        let literal = Lit::from_ipasir(variable)
            .map_err(|_| SolveError::KissatFailure("converting a validated variable"))?;
        // Native zero means don't-care; choosing false completes the model.
        self.native
            .lit_val(literal)
            .map(|value| value == TernaryVal::True)
            .map_err(|_| SolveError::KissatFailure("reading the model"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_unknown_consumes_the_solver() {
        let mut solver = Solver::new();
        solver.native.set_option("preprocess", 0).unwrap();
        solver.native.set_option("simplify", 0).unwrap();
        solver.native.set_option("lucky", 0).unwrap();
        solver.native.set_limit(rustsat_kissat::Limit::Decisions(0));
        for clause in [[1, 2], [-1, 3], [-2, -3]] {
            solver.add_clause(&clause).unwrap();
        }
        assert_eq!(
            solver.solve(&Control::default()),
            Err(SolveError::Inconclusive)
        );
        assert!(matches!(
            solver.solve(&Control::default()),
            Err(SolveError::KissatUnsupported(_))
        ));
        assert!(matches!(
            solver.add_clause(&[1]),
            Err(SolveError::KissatUnsupported(_))
        ));
    }
}

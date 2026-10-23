//! SAT solving backends.
//!
//! [`Solver`] is the single interface the enumeration modes drive. It runs
//! PicoSAT (Julia's default, `PicoSAT.itersolve`), CaDiCaL (Julia's
//! `solve_cadical_standard` and incremental mode), or one-shot Kissat:
//! - [`Solver::add_clause`] adds a clause, also between solves;
//! - [`Solver::solve`] solves under assumptions that hold for that solve only,
//!   and stops when the [`Control`]'s flag is set or its deadline passes;
//! - [`Solver::model`] reads the true variables of a satisfiable solve;
//! - [`Solver::block`] excludes a model so that the next solve finds another.
//!
//! Variables are dense: a solver is created for a fixed number of variables
//! `n`, and every literal must name a variable in `1..=n`. PicoSAT allocates
//! memory in proportion to the largest index it sees and aborts the process
//! when an allocation fails (REVIEW.md R28), so this bound keeps its memory
//! proportional to the caller's own data (simplex and auxiliary variables).

use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};
use unitrisat_cadical as cadical;
use unitrisat_picosat as picosat;

#[cfg(not(windows))]
mod kissat;

/// Portable variable limit shared by PicoSAT and CaDiCaL.
pub const MAX_VARIABLES: u32 = if picosat::MAX_VARIABLE < cadical::MAX_VARIABLE {
    picosat::MAX_VARIABLE
} else {
    cadical::MAX_VARIABLE
};

/// Kissat 4.0.4's external variable limit (`EXTERNAL_MAX_VAR` in `literal.h`).
pub const KISSAT_MAX_VARIABLES: u32 = (1 << 30) - 1;

/// Which SAT solver runs a job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Backend {
    /// PicoSAT 965, Julia's default solver.
    PicoSat,
    /// CaDiCaL 2.2.1.
    Cadical,
    /// Kissat 4.0.4: one solve, without assumptions or model enumeration.
    /// Unavailable on Windows in the upstream RustSAT binding.
    Kissat,
}

/// When to interrupt a solve: a stop flag that other threads may set, and an
/// optional deadline (the per-job timeout of D1). Both are polled by the
/// solver's own interrupt hook while it runs.
#[derive(Clone, Debug, Default)]
pub struct Control {
    /// Interrupts the solve once it is true.
    pub stop: Arc<AtomicBool>,
    /// Interrupts the solve once this instant has passed.
    pub deadline: Option<Instant>,
}

/// Result of [`Solver::solve`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Satisfiable; [`Solver::model`] reads the model.
    Sat,
    /// Unsatisfiable under the assumptions.
    Unsat,
    /// Interrupted by the stop flag. Takes precedence over [`Status::TimedOut`]
    /// when both were observed, since a stop means the caller already has its
    /// answer.
    Stopped,
    /// Interrupted because the deadline passed.
    TimedOut,
}

/// Failure of a [`Solver`] call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SolveError {
    /// More variables than [`MAX_VARIABLES`].
    #[error("{0} variables exceed the solver limit of {MAX_VARIABLES}")]
    TooManyVariables(usize),
    /// A literal is zero or names a variable outside `1..=variables`.
    #[error("{literal} is not a literal over the variables 1..={variables}")]
    Literal {
        /// The rejected literal.
        literal: i64,
        /// The solver's number of variables.
        variables: u32,
    },
    /// The solver answered "unknown" although neither the stop flag nor the
    /// deadline interrupted it.
    #[error("the solver gave up without being interrupted")]
    Inconclusive,
    /// PicoSAT failed or rejected the call.
    #[error(transparent)]
    PicoSat(#[from] picosat::Error),
    /// CaDiCaL failed or rejected the call.
    #[error(transparent)]
    Cadical(#[from] cadical::Error),
    /// A call requires capabilities that one-shot Kissat does not provide.
    #[error("Kissat does not support {0}; use PicoSAT or CaDiCaL")]
    KissatUnsupported(&'static str),
    /// A Kissat binding call failed.
    #[error("Kissat failed while {0}")]
    KissatFailure(&'static str),
    /// No satisfiable Kissat model is available.
    #[error("Kissat has no satisfiable model")]
    KissatNoModel,
    /// More variables than Kissat's native external index limit.
    #[error("{0} variables exceed the Kissat limit of {KISSAT_MAX_VARIABLES}")]
    KissatTooManyVariables(usize),
}

/// The native solver with a reusable buffer of its literal type.
#[derive(Debug)]
enum Engine {
    PicoSat {
        solver: picosat::Solver,
        literals: Vec<picosat::Literal>,
    },
    Cadical {
        solver: cadical::Solver,
        literals: Vec<cadical::Literal>,
    },
    #[cfg(not(windows))]
    Kissat {
        solver: kissat::Solver,
        literals: Vec<i32>,
    },
}

/// One SAT solver over the variables `1..=variables`. Like the native
/// solvers it is neither `Send` nor `Sync`: create it on the thread that
/// uses it.
#[derive(Debug)]
pub struct Solver {
    engine: Engine,
    /// Within the backend's variable limit, so every index fits `i32`.
    variables: i32,
    /// A SAT result remains readable until the next native mutation.
    has_model: bool,
}

impl Solver {
    /// A new solver with no clauses over the variables `1..=variables`.
    pub fn new(backend: Backend, variables: usize) -> Result<Self, SolveError> {
        let (limit, too_many) = if backend == Backend::Kissat {
            (
                KISSAT_MAX_VARIABLES,
                SolveError::KissatTooManyVariables(variables),
            )
        } else {
            (MAX_VARIABLES, SolveError::TooManyVariables(variables))
        };
        let count = u32::try_from(variables)
            .ok()
            .filter(|&n| n <= limit)
            .and_then(|n| i32::try_from(n).ok())
            .ok_or(too_many)?;
        let engine = match backend {
            Backend::PicoSat => Engine::PicoSat {
                solver: picosat::Solver::new()?,
                literals: Vec::new(),
            },
            Backend::Cadical => Engine::Cadical {
                solver: cadical::Solver::new()?,
                literals: Vec::new(),
            },
            #[cfg(not(windows))]
            Backend::Kissat => Engine::Kissat {
                solver: kissat::Solver::new(),
                literals: Vec::new(),
            },
            #[cfg(windows)]
            Backend::Kissat => return Err(SolveError::KissatUnsupported("Windows")),
        };
        Ok(Self {
            engine,
            variables: count,
            has_model: false,
        })
    }

    /// The number of variables.
    pub fn variables(&self) -> u32 {
        self.variables.unsigned_abs()
    }

    /// Adds `clause`, given as DIMACS literals. An empty clause makes the
    /// formula unsatisfiable. Discards the current model. Kissat rejects
    /// clauses after its first solve, including an interrupted solve.
    pub fn add_clause(&mut self, clause: &[i32]) -> Result<(), SolveError> {
        let literals = clause.iter().map(|&literal| i64::from(literal));
        self.add_literals(literals)
    }

    /// Adds the clause that excludes every model whose true variables include
    /// `true_variables`: the disjunction of their negations. Julia blocks
    /// triangulations the same way in `solve_cadical_standard`.
    ///
    /// Enumeration with this clause is exact when no model's true set
    /// strictly contains another's, as for triangulations: they are exact
    /// covers by full-dimensional simplices, so one cannot contain another.
    pub fn block(&mut self, true_variables: &[u32]) -> Result<(), SolveError> {
        let literals = true_variables
            .iter()
            .map(|&variable| i64::from(variable).wrapping_neg());
        self.add_literals(literals)
    }

    fn add_literals(&mut self, literals: impl Iterator<Item = i64>) -> Result<(), SolveError> {
        let variables = self.variables;
        match &mut self.engine {
            Engine::PicoSat {
                solver,
                literals: buffer,
            } => {
                convert(literals, variables, buffer)?;
                self.has_model = false;
                solver.add_clause(buffer)?;
            }
            Engine::Cadical {
                solver,
                literals: buffer,
            } => {
                convert(literals, variables, buffer)?;
                self.has_model = false;
                solver.add_clause(buffer)?;
            }
            #[cfg(not(windows))]
            Engine::Kissat {
                solver,
                literals: buffer,
            } => {
                solver.check_input()?;
                convert(literals, variables, buffer)?;
                self.has_model = false;
                solver.add_clause(buffer)?;
            }
        }
        Ok(())
    }

    /// Solves under `assumptions` (DIMACS literals), which hold for this
    /// solve only. Kissat accepts only empty assumptions and one solve call.
    pub fn solve(&mut self, assumptions: &[i32], control: &Control) -> Result<Status, SolveError> {
        let variables = self.variables;
        #[cfg(not(windows))]
        let has_assumptions = !assumptions.is_empty();
        let assumptions = assumptions.iter().map(|&literal| i64::from(literal));
        let result = match &mut self.engine {
            Engine::PicoSat {
                solver,
                literals: buffer,
            } => {
                convert(assumptions, variables, buffer)?;
                self.has_model = false;
                let control = picosat::Control::new(Arc::clone(&control.stop), control.deadline);
                let outcome = solver.solve(buffer, &control)?;
                let interrupts = outcome.interrupts;
                status(
                    match outcome.status {
                        picosat::Status::Sat => Some(true),
                        picosat::Status::Unsat => Some(false),
                        picosat::Status::Unknown => None,
                    },
                    interrupts.stopped,
                    interrupts.deadline,
                )
            }
            Engine::Cadical {
                solver,
                literals: buffer,
            } => {
                convert(assumptions, variables, buffer)?;
                self.has_model = false;
                let control = cadical::Control::new(Arc::clone(&control.stop), control.deadline);
                let outcome = solver.solve(buffer, &control)?;
                let interrupts = outcome.interrupts;
                status(
                    match outcome.status {
                        cadical::Status::Sat => Some(true),
                        cadical::Status::Unsat => Some(false),
                        cadical::Status::Unknown => None,
                    },
                    interrupts.stopped,
                    interrupts.deadline,
                )
            }
            #[cfg(not(windows))]
            Engine::Kissat { solver, .. } => {
                solver.check_solve(has_assumptions)?;
                self.has_model = false;
                solver.solve(control)
            }
        };
        self.has_model = result == Ok(Status::Sat);
        result
    }

    /// Replaces the contents of `true_variables` with the variables that are
    /// true in the model of the last solve, in increasing order. That solve
    /// must have returned [`Status::Sat`], with no clause added since.
    ///
    /// A variable that occurs in no clause or assumption is free; it is
    /// reported as false, which is a valid choice for it.
    pub fn model(&mut self, true_variables: &mut Vec<u32>) -> Result<(), SolveError> {
        self.model_prefix(self.variables(), true_variables)
    }

    /// Reads only the true variables in `1..=variables` from the last model.
    /// The prefix must not exceed the solver's variable count. Enumeration
    /// uses this to exclude auxiliary variables from output and blocking.
    pub fn model_prefix(
        &mut self,
        variables: u32,
        true_variables: &mut Vec<u32>,
    ) -> Result<(), SolveError> {
        if variables > self.variables() {
            return Err(literal_error(i64::from(variables), self.variables));
        }
        true_variables.clear();
        if !self.has_model {
            return Err(match &self.engine {
                Engine::PicoSat { .. } => picosat::Error::NoModel.into(),
                Engine::Cadical { .. } => cadical::Error::NoModel.into(),
                #[cfg(not(windows))]
                Engine::Kissat { .. } => SolveError::KissatNoModel,
            });
        }
        let variables = i32::try_from(variables)
            .map_err(|_| literal_error(i64::from(variables), self.variables))?;
        match &mut self.engine {
            Engine::PicoSat { solver, .. } => {
                for variable in 1..=variables {
                    let literal = picosat::Literal::try_from(variable)
                        .map_err(|_| literal_error(variable.into(), variables))?;
                    if solver.value(literal)? == picosat::Value::True {
                        true_variables.push(variable.unsigned_abs());
                    }
                }
            }
            Engine::Cadical { solver, .. } => {
                for variable in 1..=variables {
                    let literal = cadical::Literal::try_from(variable)
                        .map_err(|_| literal_error(variable.into(), variables))?;
                    if solver.value(literal)? == cadical::Value::True {
                        true_variables.push(variable.unsigned_abs());
                    }
                }
            }
            #[cfg(not(windows))]
            Engine::Kissat { solver, .. } => {
                for variable in 1..=variables {
                    if solver.value(variable)? {
                        true_variables.push(variable.unsigned_abs());
                    }
                }
            }
        }
        Ok(())
    }
}

/// Validates `literals` against `1..=variables` and converts them into
/// `buffer`, replacing its contents.
fn convert<L: TryFrom<i32>>(
    literals: impl Iterator<Item = i64>,
    variables: i32,
    buffer: &mut Vec<L>,
) -> Result<(), SolveError> {
    buffer.clear();
    for literal in literals {
        let native = i32::try_from(literal)
            .ok()
            .filter(|value| *value != 0 && value.unsigned_abs() <= variables.unsigned_abs())
            .and_then(|value| L::try_from(value).ok())
            .ok_or_else(|| literal_error(literal, variables))?;
        buffer.push(native);
    }
    Ok(())
}

fn literal_error(literal: i64, variables: i32) -> SolveError {
    SolveError::Literal {
        literal,
        variables: variables.unsigned_abs(),
    }
}

/// Maps a native answer (`None` for unknown) and the observed interrupts to
/// a [`Status`].
fn status(answer: Option<bool>, stopped: bool, timed_out: bool) -> Result<Status, SolveError> {
    match answer {
        Some(true) => Ok(Status::Sat),
        Some(false) => Ok(Status::Unsat),
        None if stopped => Ok(Status::Stopped),
        None if timed_out => Ok(Status::TimedOut),
        None => Err(SolveError::Inconclusive),
    }
}

#[cfg(test)]
mod tests;

//! Safe FFI over the vendored, patched CaDiCaL 2.2.1.
//!
//! [`Solver`] owns one native instance and enforces CaDiCaL's state machine
//! in Rust (REVIEW.md R9): clauses are submitted whole, assumptions only for
//! the next solve, and the model can be read only after a satisfiable solve.
//! An invalid call returns an error instead of reaching C++, where CaDiCaL's
//! `REQUIRE` checks would abort the process. Solving can be interrupted
//! through [`Control`]: a shared stop flag and an optional deadline, polled
//! by CaDiCaL's terminator. The solver supports incremental use: clauses may
//! be added between solves.

#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
        clippy::unreachable,
        clippy::disallowed_macros,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
        clippy::panic_in_result_fn,
        clippy::assertions_on_constants
    )
)]
#![cfg_attr(
    test,
    allow(
        clippy::disallowed_macros,
        reason = "tests may assert; production code may not"
    )
)]
#![deny(unused_crate_dependencies)]

use std::{
    fmt,
    num::NonZeroI32,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

#[allow(unsafe_code, reason = "the only module that calls into C++")]
mod ffi;

/// Largest variable index. CaDiCaL stores literals as `2 * |l| + sign` in a
/// signed `int`, and reserves `2 * |l| + 2` slots with signed arithmetic.
/// Keeping that reservation below `i32::MAX` excludes `2^30 - 1` too.
pub const MAX_VARIABLE: u32 = (1 << 30) - 2;
/// Largest number of clauses one solver accepts (`i32::MAX`).
pub const MAX_CLAUSES: u64 = 2_147_483_647;

/// A nonzero literal with `|literal| <= MAX_VARIABLE`.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Literal(NonZeroI32);

impl Literal {
    /// The literal as a DIMACS integer.
    pub fn get(self) -> i32 {
        self.0.get()
    }
}

/// The value is zero or exceeds [`MAX_VARIABLE`] in magnitude.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0} is not a literal: it must be nonzero with magnitude at most {MAX_VARIABLE}")]
pub struct LiteralError(pub i32);

impl TryFrom<i32> for Literal {
    type Error = LiteralError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        if value.unsigned_abs() > MAX_VARIABLE {
            return Err(LiteralError(value));
        }
        NonZeroI32::new(value).map(Self).ok_or(LiteralError(value))
    }
}

/// When to interrupt a solve: a stop flag shared with other threads and an
/// optional deadline.
#[derive(Clone, Debug, Default)]
pub struct Control {
    stop: Arc<AtomicBool>,
    deadline: Option<Instant>,
}

impl Control {
    /// Interrupts when `stop` becomes true or `deadline` passes.
    pub fn new(stop: Arc<AtomicBool>, deadline: Option<Instant>) -> Self {
        Self { stop, deadline }
    }
}

/// Result of a solve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Satisfiable; the model can be read with [`Solver::value`].
    Sat,
    /// Unsatisfiable under the assumptions.
    Unsat,
    /// Interrupted before an answer was found.
    Unknown,
}

/// What the interrupt hook saw during a solve. Only an `Unknown` status
/// means the solve was actually cut short.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Interrupts {
    /// The stop flag was set.
    pub stopped: bool,
    /// The deadline had passed.
    pub deadline: bool,
}

/// A solve outcome with the interrupts observed during it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The answer.
    pub status: Status,
    /// What the interrupt hook observed.
    pub interrupts: Interrupts,
}

/// Value of a literal in the model. Variables CaDiCaL has never seen are
/// false.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    /// The literal is true.
    True,
    /// The literal is false.
    False,
}

/// Failure of a solver call. After `Native` and `Unexpected*` errors the
/// solver is abandoned and every later call returns [`Error::Invalid`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// CaDiCaL could not be constructed.
    #[error("CaDiCaL could not be initialized")]
    InitFailed,
    /// An earlier call failed inside CaDiCaL (for example, out of memory).
    #[error("the CaDiCaL solver is no longer usable after an earlier failure")]
    Invalid,
    /// No model: the last solve was not satisfiable, or clauses were added
    /// since.
    #[error("no CaDiCaL model is available")]
    NoModel,
    /// More than [`MAX_CLAUSES`] clauses.
    #[error("more than {MAX_CLAUSES} clauses")]
    TooManyClauses,
    /// CaDiCaL threw a C++ exception (for example, out of memory).
    #[error("CaDiCaL failed inside {0}")]
    Native(&'static str),
    /// `solve` returned something other than 0, 10 or 20.
    #[error("CaDiCaL returned the unexpected status {0}")]
    UnexpectedStatus(i32),
    /// `vars` returned a value outside `0..=MAX_VARIABLE`.
    #[error("CaDiCaL reported the unexpected variable count {0}")]
    UnexpectedVariableCount(i32),
    /// `val` returned something other than the literal or its negation.
    #[error("CaDiCaL returned the unexpected literal value {0}")]
    UnexpectedValue(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Clauses may be added and a solve may start; no model.
    Ready,
    /// The last solve was satisfiable and no clause was added since.
    Sat,
}

/// One CaDiCaL instance. It is neither `Send` nor `Sync`: create it on the
/// thread that uses it.
pub struct Solver {
    instance: Option<ffi::Instance>,
    state: State,
    clauses: u64,
}

impl fmt::Debug for Solver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Solver")
            .field("valid", &self.instance.is_some())
            .field("state", &self.state)
            .field("clauses", &self.clauses)
            .finish()
    }
}

impl Solver {
    /// A new, empty solver.
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            instance: Some(ffi::Instance::new().ok_or(Error::InitFailed)?),
            state: State::Ready,
            clauses: 0,
        })
    }

    fn instance(&mut self) -> Result<&mut ffi::Instance, Error> {
        self.instance.as_mut().ok_or(Error::Invalid)
    }

    /// Abandons the native instance after a failure inside CaDiCaL. It is
    /// leaked, not freed: after an exception CaDiCaL's state may be one its
    /// destructor rejects with an abort.
    fn invalidate(&mut self, error: Error) -> Error {
        if let Some(instance) = self.instance.take() {
            std::mem::forget(instance);
        }
        error
    }

    /// Adds one clause. An empty clause makes the formula unsatisfiable.
    /// Discards the current model.
    pub fn add_clause(&mut self, clause: &[Literal]) -> Result<(), Error> {
        let clauses = self
            .clauses
            .checked_add(1)
            .filter(|&n| n <= MAX_CLAUSES)
            .ok_or(Error::TooManyClauses)?;
        if !self.instance()?.add_clause(clause) {
            return Err(self.invalidate(Error::Native("add")));
        }
        self.clauses = clauses;
        self.state = State::Ready;
        Ok(())
    }

    /// Solves under `assumptions`, which hold for this solve only.
    pub fn solve(&mut self, assumptions: &[Literal], control: &Control) -> Result<Outcome, Error> {
        let (raw, interrupts) = self.instance()?.solve(assumptions, control);
        self.accept_status(raw, interrupts)
    }

    fn accept_status(&mut self, raw: i32, interrupts: Interrupts) -> Result<Outcome, Error> {
        let (status, state) = match raw {
            10 => (Status::Sat, State::Sat),
            20 => (Status::Unsat, State::Ready),
            0 => (Status::Unknown, State::Ready),
            -1 => return Err(self.invalidate(Error::Native("solve"))),
            other => return Err(self.invalidate(Error::UnexpectedStatus(other))),
        };
        self.state = state;
        Ok(Outcome { status, interrupts })
    }

    /// The largest variable index CaDiCaL has seen.
    pub fn variables(&mut self) -> Result<u32, Error> {
        let raw = self.instance()?.variables();
        if raw == -1 {
            return Err(self.invalidate(Error::Native("vars")));
        }
        match u32::try_from(raw) {
            Ok(count) if count <= MAX_VARIABLE => Ok(count),
            _ => Err(self.invalidate(Error::UnexpectedVariableCount(raw))),
        }
    }

    /// The value of `literal` in the model of the last satisfiable solve.
    pub fn value(&mut self, literal: Literal) -> Result<Value, Error> {
        let state = self.state;
        let instance = self.instance()?;
        if state != State::Sat {
            return Err(Error::NoModel);
        }
        let raw = instance.value(literal);
        if raw == literal.get() {
            Ok(Value::True)
        } else if Some(raw) == literal.get().checked_neg() {
            Ok(Value::False)
        } else if raw == 0 {
            Err(self.invalidate(Error::Native("val")))
        } else {
            Err(self.invalidate(Error::UnexpectedValue(raw)))
        }
    }
}

#[cfg(test)]
mod tests;

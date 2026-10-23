//! The only module with `unsafe` code. Nothing here is public outside the
//! crate; `Solver` in `lib.rs` guarantees every call is in a valid state.

use crate::{Control, Interrupts, Literal};
use std::{
    ffi::{c_int, c_void},
    marker::PhantomData,
    ptr::NonNull,
    rc::Rc,
    sync::atomic::Ordering,
    time::Instant,
};

#[repr(C)]
struct Ucad {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn ucad_new() -> *mut Ucad;
    fn ucad_free(solver: *mut Ucad);
    fn ucad_add_clause(solver: *mut Ucad, lits: *const c_int, n: usize) -> c_int;
    fn ucad_solve(
        solver: *mut Ucad,
        assumptions: *const c_int,
        n: usize,
        callback: Option<unsafe extern "C" fn(*mut c_void) -> c_int>,
        state: *mut c_void,
    ) -> c_int;
    fn ucad_val(solver: *mut Ucad, lit: c_int) -> c_int;
    fn ucad_vars(solver: *mut Ucad) -> c_int;
}

// `Literal` is a transparent `NonZeroI32`, so a `&[Literal]` is a valid
// `[c_int]` buffer.
const _: [(); size_of::<c_int>()] = [(); size_of::<Literal>()];

/// An owned CaDiCaL instance.
pub(crate) struct Instance {
    handle: NonNull<Ucad>,
    /// CaDiCaL solvers are not documented as movable between threads, so
    /// `Instance` is neither `Send` nor `Sync`.
    _not_send: PhantomData<Rc<()>>,
}

impl Instance {
    pub(crate) fn new() -> Option<Self> {
        // SAFETY: no preconditions; the shim catches exceptions and returns
        // null, which is handled.
        let handle = NonNull::new(unsafe { ucad_new() })?;
        Some(Self {
            handle,
            _not_send: PhantomData,
        })
    }

    /// Adds a whole clause; false if CaDiCaL threw.
    pub(crate) fn add_clause(&mut self, clause: &[Literal]) -> bool {
        // SAFETY: live, exclusively borrowed instance; `clause` is a valid
        // buffer of `clause.len()` nonzero, non-`INT_MIN` ints that the shim
        // only reads during the call; the shim terminates the clause.
        unsafe { ucad_add_clause(self.handle.as_ptr(), clause.as_ptr().cast(), clause.len()) == 0 }
    }

    /// Assumes `assumptions` and solves with the interrupt hook. Returns the
    /// shim's status (10, 20, 0, or -1 if CaDiCaL threw) and the interrupts
    /// the hook observed.
    pub(crate) fn solve(
        &mut self,
        assumptions: &[Literal],
        control: &Control,
    ) -> (i32, Interrupts) {
        let mut context = HookContext {
            control,
            observed: Interrupts::default(),
        };
        // SAFETY: live, exclusively borrowed instance with no open clause;
        // `assumptions` is a valid int buffer read during the call; the hook
        // state points to `context`, which outlives the call, and the shim
        // disconnects the terminator before returning.
        let raw = unsafe {
            ucad_solve(
                self.handle.as_ptr(),
                assumptions.as_ptr().cast(),
                assumptions.len(),
                Some(interrupted),
                std::ptr::from_mut(&mut context).cast(),
            )
        };
        (raw, context.observed)
    }

    pub(crate) fn variables(&mut self) -> i32 {
        // SAFETY: live, exclusively borrowed instance with no open clause.
        unsafe { ucad_vars(self.handle.as_ptr()) }
    }

    /// `literal` if it is true in the model, `-literal` if false.
    pub(crate) fn value(&mut self, literal: Literal) -> i32 {
        // SAFETY: `Solver` calls this only while the last solve returned SAT
        // and no clause was added since (CaDiCaL's SATISFIED state);
        // `literal` is nonzero and not `INT_MIN`.
        unsafe { ucad_val(self.handle.as_ptr(), literal.get()) }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: unique owner of a live instance in a valid state (failed
        // instances are leaked, never dropped); no call is active.
        unsafe { ucad_free(self.handle.as_ptr()) }
    }
}

/// State of the terminator hook during one solve.
struct HookContext<'a> {
    control: &'a Control,
    observed: Interrupts,
}

/// CaDiCaL's terminator: nonzero stops the solve.
unsafe extern "C" fn interrupted(state: *mut c_void) -> c_int {
    // SAFETY: only `Instance::solve` installs this hook, with `state`
    // pointing to a live `HookContext` that CaDiCaL calls back
    // synchronously on the solving thread until the shim disconnects it.
    let Some(context) = (unsafe { state.cast::<HookContext<'_>>().as_mut() }) else {
        return 1;
    };
    let stopped = context.control.stop.load(Ordering::Relaxed);
    let deadline = context
        .control
        .deadline
        .is_some_and(|end| Instant::now() >= end);
    context.observed.stopped |= stopped;
    context.observed.deadline |= deadline;
    c_int::from(stopped || deadline)
}

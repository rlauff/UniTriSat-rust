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
struct PicoSat {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn picosat_init() -> *mut PicoSat;
    fn picosat_reset(solver: *mut PicoSat);
    fn picosat_add(solver: *mut PicoSat, literal: c_int) -> c_int;
    fn picosat_assume(solver: *mut PicoSat, literal: c_int);
    fn picosat_sat(solver: *mut PicoSat, decision_limit: c_int) -> c_int;
    fn picosat_variables(solver: *mut PicoSat) -> c_int;
    fn picosat_deref(solver: *mut PicoSat, literal: c_int) -> c_int;
    fn picosat_set_interrupt(
        solver: *mut PicoSat,
        state: *mut c_void,
        callback: Option<unsafe extern "C" fn(*mut c_void) -> c_int>,
    );
}

/// Test-only record of native lifecycle events.
#[cfg(test)]
#[derive(Default, Debug)]
pub(crate) struct Lifecycle {
    pub(crate) resets: std::cell::Cell<u32>,
    pub(crate) registrations: std::cell::Cell<u32>,
    pub(crate) clears: std::cell::Cell<u32>,
    pub(crate) values: std::cell::Cell<u32>,
    pub(crate) active: std::cell::Cell<bool>,
    pub(crate) reset_while_active: std::cell::Cell<bool>,
    pub(crate) context_freed_while_active: std::cell::Cell<bool>,
}

/// An owned PicoSAT instance.
pub(crate) struct Instance {
    handle: NonNull<PicoSat>,
    /// PicoSAT instances are reentrant but not documented as movable between
    /// threads, so `Instance` is neither `Send` nor `Sync`.
    _not_send: PhantomData<Rc<()>>,
    #[cfg(test)]
    pub(crate) probe: Rc<Lifecycle>,
}

impl Instance {
    pub(crate) fn new() -> Option<Self> {
        // SAFETY: `picosat_init` has no preconditions; null is handled.
        let handle = NonNull::new(unsafe { picosat_init() })?;
        Some(Self {
            handle,
            _not_send: PhantomData,
            #[cfg(test)]
            probe: Rc::default(),
        })
    }

    /// Adds a whole clause, terminated by 0.
    pub(crate) fn add_clause(&mut self, clause: &[Literal]) {
        for literal in clause {
            // SAFETY: live, exclusively borrowed instance; `Literal` is nonzero
            // and within PicoSAT's index range; no context push is ever used.
            unsafe {
                picosat_add(self.handle.as_ptr(), literal.get());
            }
        }
        // SAFETY: as above; 0 terminates the clause before this returns, so
        // no clause is ever left incomplete.
        unsafe {
            picosat_add(self.handle.as_ptr(), 0);
        }
    }

    /// Assumes `assumptions` and solves with the interrupt hook installed.
    /// Returns PicoSAT's raw status and the interrupts the hook observed.
    pub(crate) fn solve(
        &mut self,
        assumptions: &[Literal],
        control: &Control,
        decision_limit: i32,
    ) -> (i32, Interrupts) {
        let mut context = Box::new(HookContext {
            control,
            observed: Interrupts::default(),
            #[cfg(test)]
            probe: Rc::clone(&self.probe),
        });
        let raw;
        {
            // The guard borrows the boxed context, so the context outlives
            // the registration; dropping the guard clears the hook.
            let _guard = HookGuard::register(self.handle, &mut context);
            for literal in assumptions {
                // SAFETY: live, exclusively borrowed instance; every clause is
                // complete; `Literal` is in range. PicoSAT copies the value.
                unsafe {
                    picosat_assume(self.handle.as_ptr(), literal.get());
                }
            }
            // SAFETY: as above; the decision limit is -1 (none) in
            // production and a small nonnegative value in tests.
            raw = unsafe { picosat_sat(self.handle.as_ptr(), decision_limit) };
        }
        (raw, context.observed)
    }

    pub(crate) fn variables(&mut self) -> i32 {
        // SAFETY: live, exclusively borrowed instance with no open clause.
        unsafe { picosat_variables(self.handle.as_ptr()) }
    }

    /// The raw model value of `literal`.
    pub(crate) fn value(&mut self, literal: Literal) -> i32 {
        #[cfg(test)]
        self.probe
            .values
            .set(self.probe.values.get().saturating_add(1));
        // SAFETY: `Solver` calls this only while the last solve returned SAT
        // and no clause was added since (PicoSAT's SAT state, so no empty
        // clause exists); `Literal` is nonzero.
        unsafe { picosat_deref(self.handle.as_ptr(), literal.get()) }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        #[cfg(test)]
        {
            self.probe.reset_while_active.set(self.probe.active.get());
            self.probe
                .resets
                .set(self.probe.resets.get().saturating_add(1));
        }
        // SAFETY: unique owner; no call or hook can be active during drop.
        unsafe {
            picosat_reset(self.handle.as_ptr());
        }
    }
}

/// State of the interrupt hook during one solve.
struct HookContext<'a> {
    control: &'a Control,
    observed: Interrupts,
    #[cfg(test)]
    probe: Rc<Lifecycle>,
}

#[cfg(test)]
impl Drop for HookContext<'_> {
    fn drop(&mut self) {
        self.probe
            .context_freed_while_active
            .set(self.probe.active.get());
    }
}

/// PicoSAT's interrupt hook: nonzero stops the solve.
unsafe extern "C" fn interrupted(state: *mut c_void) -> c_int {
    // SAFETY: only `HookGuard::register` installs this hook, with `state`
    // pointing to a live `HookContext` that PicoSAT calls back synchronously
    // on the owning thread until the guard clears the hook.
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

/// Keeps the interrupt hook installed for its own lifetime.
struct HookGuard<'a, 'control> {
    handle: NonNull<PicoSat>,
    _context: PhantomData<&'a mut HookContext<'control>>,
    #[cfg(test)]
    context: NonNull<HookContext<'control>>,
}

impl<'a, 'control> HookGuard<'a, 'control> {
    fn register(handle: NonNull<PicoSat>, context: &'a mut HookContext<'control>) -> Self {
        #[cfg(test)]
        {
            context
                .probe
                .registrations
                .set(context.probe.registrations.get().saturating_add(1));
            context.probe.active.set(true);
        }
        let pointer = NonNull::from(context);
        // SAFETY: the instance is live for this synchronous scope; the
        // context is mutably borrowed for the guard's lifetime and `Drop`
        // clears the hook before that borrow ends.
        unsafe {
            picosat_set_interrupt(handle.as_ptr(), pointer.as_ptr().cast(), Some(interrupted));
        }
        Self {
            handle,
            _context: PhantomData,
            #[cfg(test)]
            context: pointer,
        }
    }
}

impl Drop for HookGuard<'_, '_> {
    fn drop(&mut self) {
        // SAFETY: same live instance; a null hook disables further callbacks.
        unsafe {
            picosat_set_interrupt(self.handle.as_ptr(), std::ptr::null_mut(), None);
        }
        #[cfg(test)]
        {
            // SAFETY: the hook is cleared and the guard still holds the
            // exclusive borrow of the live context.
            let context = unsafe { self.context.as_ref() };
            context
                .probe
                .clears
                .set(context.probe.clears.get().saturating_add(1));
            context.probe.active.set(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicBool};

    #[test]
    fn hook_records_stops_and_deadlines() {
        let stop = Arc::new(AtomicBool::new(false));
        let running = Control::new(Arc::clone(&stop), None);
        let expired = Control::new(Arc::clone(&stop), Some(Instant::now()));
        let mut context = HookContext {
            control: &running,
            observed: Interrupts::default(),
            probe: Rc::default(),
        };
        let call = |context: &mut HookContext<'_>| {
            // SAFETY: a live context, called directly and synchronously.
            unsafe { interrupted(std::ptr::from_mut(context).cast()) }
        };
        assert_eq!(call(&mut context), 0);
        assert_eq!(context.observed, Interrupts::default());
        stop.store(true, Ordering::Relaxed);
        assert_eq!(call(&mut context), 1);
        assert_eq!(
            context.observed,
            Interrupts {
                stopped: true,
                deadline: false
            }
        );
        context.control = &expired;
        assert_eq!(call(&mut context), 1);
        assert_eq!(
            context.observed,
            Interrupts {
                stopped: true,
                deadline: true
            }
        );
        // SAFETY: a null state pointer is handled by stopping.
        let null = unsafe { interrupted(std::ptr::null_mut()) };
        assert_eq!(null, 1);
    }
}

//! Optional, bounded progress transport. Native solver flags make observer
//! failures interruptible; callbacks themselves always run on the job caller.
use super::{Classify, EnumerateError, Enumeration, Progress};
use std::{
    cell::RefCell,
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc, Condvar, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use unitrisat_core::enumerate::Classification;

const INTERVAL: Duration = Duration::from_millis(100);

thread_local! {
    // cddlib's native scratch survives worker exit. Reuse the same execution
    // threads across observed jobs rather than abandoning scratch per job.
    // Keep only the last width: callers do not accumulate a pool per width.
    static EXECUTION_POOL: RefCell<Option<(usize, Arc<rayon::ThreadPool>)>> = const { RefCell::new(None) };
}

fn execution_pool(width: usize) -> Result<Arc<rayon::ThreadPool>, rayon::ThreadPoolBuildError> {
    EXECUTION_POOL.with(|cached| {
        let mut cached = cached.borrow_mut();
        if let Some((cached_width, pool)) = cached.as_ref()
            && *cached_width == width
        {
            return Ok(Arc::clone(pool));
        }
        let pool = Arc::new(rayon::ThreadPoolBuilder::new().num_threads(width).build()?);
        *cached = Some((width, Arc::clone(&pool)));
        Ok(pool)
    })
}

#[derive(Default)]
pub(super) struct Tally {
    yes: AtomicU64,
    no: AtomicU64,
}

pub(super) struct Counted<C, const ACTIVE: bool> {
    inner: C,
    pub tally: Tally,
}

impl<C, const ACTIVE: bool> Counted<C, ACTIVE> {
    pub(super) fn new(inner: C) -> Self {
        Self {
            inner,
            tally: Tally::default(),
        }
    }
}

impl<C: Classify, const ACTIVE: bool> Classify for Counted<C, ACTIVE> {
    type Error = C::Error;

    fn accepts_all(&self) -> bool {
        self.inner.accepts_all()
    }

    fn is_target(&self, simplices: &[u32]) -> Result<bool, Self::Error> {
        Ok(self.classify(simplices)?.target)
    }

    fn classify(&self, simplices: &[u32]) -> Result<Classification, Self::Error> {
        let answer = self.inner.classify(simplices)?;
        if ACTIVE {
            let counter = if answer.target {
                &self.tally.yes
            } else {
                &self.tally.no
            };
            counter.fetch_add(1, Ordering::Release);
        }
        Ok(answer)
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct Tests<'a> {
    pub regular: Option<&'a Tally>,
    pub flag: Option<&'a Tally>,
}

#[derive(Default)]
struct Cancellation {
    requested: AtomicBool,
    stops: Mutex<Vec<Arc<AtomicBool>>>,
}

impl Cancellation {
    fn interrupt(&self) {
        self.requested.store(true, Ordering::SeqCst);
        for flag in self
            .stops
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
        {
            flag.store(true, Ordering::SeqCst);
        }
    }

    fn check(&self) {
        if self.requested.load(Ordering::SeqCst) {
            panic::resume_unwind(Box::new("progress observer canceled search"));
        }
    }
}

pub(super) struct Tracked<'a, C> {
    inner: &'a C,
    seen: AtomicU64,
    targets: AtomicU64,
    cancellation: Cancellation,
}

impl<C: Classify> Classify for Tracked<'_, C> {
    type Error = C::Error;

    fn accepts_all(&self) -> bool {
        self.inner.accepts_all()
    }

    fn is_target(&self, simplices: &[u32]) -> Result<bool, Self::Error> {
        Ok(self.classify(simplices)?.target)
    }

    fn classify(&self, simplices: &[u32]) -> Result<Classification, Self::Error> {
        self.cancellation.check();
        self.seen.fetch_add(1, Ordering::Relaxed);
        let target = self.inner.classify(simplices)?;
        self.cancellation.check();
        if target.target {
            self.targets.fetch_add(1, Ordering::Release);
        }
        Ok(target)
    }

    fn register_stop(&self, stop: &Arc<AtomicBool>) {
        let mut stops = self
            .cancellation
            .stops
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        stops.push(Arc::clone(stop));
        if self.cancellation.requested.load(Ordering::SeqCst) {
            stop.store(true, Ordering::SeqCst);
        }
    }
}

#[derive(Default)]
struct Transport {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Default)]
struct State {
    incremental: Option<Progress>,
    done: bool,
}

struct Finish<'a>(&'a Transport);
impl Drop for Finish<'_> {
    fn drop(&mut self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .done = true;
        self.0.changed.notify_one();
    }
}

pub(super) fn observe<C: Classify>(
    classify: &C,
    tests: Tests<'_>,
    observer: &mut impl FnMut(Progress),
    search: impl FnOnce(
        &Tracked<'_, C>,
        &mut dyn FnMut(Progress),
    ) -> Result<Enumeration, EnumerateError<C::Error>>
    + Send,
) -> Result<Enumeration, EnumerateError<C::Error>> {
    // Preserve the caller's parallelism, but run native classification on
    // persistent execution threads even when the caller is outside Rayon.
    // The scoped transport worker must never own cddlib's per-thread scratch.
    let pool =
        execution_pool(rayon::current_num_threads()).map_err(|_| EnumerateError::Internal)?;
    let tracked = Tracked {
        inner: classify,
        seen: AtomicU64::new(0),
        targets: AtomicU64::new(0),
        cancellation: Cancellation::default(),
    };
    let transport = Transport::default();
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            let _finish = Finish(&transport);
            let run = || {
                search(&tracked, &mut |event| {
                    tracked.cancellation.check();
                    transport
                        .state
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .incremental = Some(event);
                })
            };
            pool.install(run)
        });
        let mut observer_panic = None;
        loop {
            let mut state = transport
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if !state.done {
                state = transport
                    .changed
                    .wait_timeout(state, INTERVAL)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
            let done = state.done;
            let incremental = state.incremental.take();
            drop(state);
            if observer_panic.is_none() {
                let result = panic::catch_unwind(AssertUnwindSafe(|| {
                    if let Some(event) = incremental {
                        observer(event);
                    }
                    // Read the most restrictive completed count first. Its
                    // release/acquire ordering publishes the inner test counts,
                    // so every snapshot preserves quadratic <= regular, flag.
                    let quadratic = if tests.regular.is_some() && tests.flag.is_some() {
                        tracked.targets.load(Ordering::Acquire)
                    } else {
                        0
                    };
                    let counts = |tally: Option<&Tally>| {
                        tally.map_or((0, 0), |t| {
                            (t.yes.load(Ordering::Acquire), t.no.load(Ordering::Acquire))
                        })
                    };
                    let (regular, non_regular) = counts(tests.regular);
                    let (flag, non_flag) = counts(tests.flag);
                    observer(Progress::Classification {
                        triangulations: tracked.seen.load(Ordering::Relaxed),
                        regular,
                        non_regular,
                        flag,
                        non_flag,
                        quadratic,
                    });
                }));
                observer_panic = result.err();
            }
            if observer_panic.is_some() {
                // Incremental solvers reset their pause flag when resuming.
                // Keep interrupting until their callback sees cancellation.
                tracked.cancellation.interrupt();
            }
            if done {
                break;
            }
        }
        let result = worker.join();
        if let Some(payload) = observer_panic {
            panic::resume_unwind(payload);
        }
        match result {
            Ok(result) => result,
            Err(payload) => panic::resume_unwind(payload),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::{Formula, IntersectionBackend, Lattice, Mode};
    use std::{collections::HashSet, thread::ThreadId};
    use unitrisat_core::{
        enumerate::{Goal, Keep, Options, Outcome},
        families,
        lattice::Restriction,
        regular::{RegularError, Regularity},
        solve::Backend,
    };

    struct NativeRegular<'a> {
        inner: Regularity<'a>,
        threads: Mutex<HashSet<ThreadId>>,
        width: usize,
    }

    impl Classify for NativeRegular<'_> {
        type Error = RegularError;
        fn is_target(&self, simplices: &[u32]) -> Result<bool, Self::Error> {
            assert_eq!(rayon::current_num_threads(), self.width);
            self.threads
                .lock()
                .unwrap()
                .insert(std::thread::current().id());
            self.inner.is_target(simplices)
        }
    }

    fn repeated_native_jobs_reuse_pool() {
        let width = rayon::current_num_threads();
        let pool = execution_pool(width).unwrap();
        let native_threads: HashSet<_> = pool
            .broadcast(|_| std::thread::current().id())
            .into_iter()
            .collect();
        assert_eq!(native_threads.len(), width);
        let caller = std::thread::current().id();
        let vertices = families::hypercube(3).unwrap();
        let lattice =
            Lattice::new::<false>(&vertices, Restriction::Unimodular, &mut |_| {}).unwrap();
        for mode in [Mode::Serial, Mode::Split, Mode::Incremental] {
            let formula = Formula::new(&lattice, mode, IntersectionBackend::Cpu, false).unwrap();
            for backend in [Backend::PicoSat, Backend::Cadical] {
                for _ in 0..3 {
                    let classify = Counted::<_, true>::new(NativeRegular {
                        inner: Regularity::new(&lattice.points, &lattice.simplices, &lattice.faces)
                            .unwrap(),
                        threads: Mutex::new(HashSet::new()),
                        width,
                    });
                    let options = Options {
                        backend,
                        goal: Goal::All,
                        keep: Keep::None,
                        max_solutions: None,
                        deadline: None,
                    };
                    let result = formula
                        .search::<true, _>(
                            &lattice,
                            &options,
                            &classify,
                            Tests {
                                regular: Some(&classify.tally),
                                flag: None,
                            },
                            &mut |_| assert_eq!(std::thread::current().id(), caller),
                        )
                        .unwrap();
                    assert_eq!(
                        (result.outcome, result.found, result.targets),
                        (Outcome::Complete, 72, 72)
                    );
                    let used = classify.inner.threads.lock().unwrap();
                    assert!(!used.is_empty());
                    assert!(
                        used.is_subset(&native_threads),
                        "native LPs must reuse existing execution threads"
                    );
                    assert!(!used.contains(&caller));
                    assert!(Arc::ptr_eq(&pool, &execution_pool(width).unwrap()));
                }
            }
        }
    }

    #[test]
    fn repeated_observed_regular_jobs_reuse_native_threads_from_ordinary_caller() {
        assert!(rayon::current_thread_index().is_none());
        repeated_native_jobs_reuse_pool();
    }

    #[test]
    fn repeated_observed_regular_jobs_reuse_native_threads_in_single_thread_pool() {
        let caller_pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        caller_pool.install(repeated_native_jobs_reuse_pool);
    }

    #[test]
    fn execution_pool_cache_retains_only_last_width() {
        let first = execution_pool(1).unwrap();
        let first_weak = Arc::downgrade(&first);
        assert!(Arc::ptr_eq(&first, &execution_pool(1).unwrap()));
        let second = execution_pool(2).unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        assert!(Arc::ptr_eq(&second, &execution_pool(2).unwrap()));
        drop(first);
        assert!(
            first_weak.upgrade().is_none(),
            "old widths must not accumulate in the cache"
        );
        assert_eq!(second.current_num_threads(), 2);
    }
}

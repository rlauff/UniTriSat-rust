//! Incremental solving: one solver on the reduced formula, while the
//! intersection clauses are computed in the background and added in
//! batches (Julia's `solve_cadical_incremental`).
//!
//! The reduced formula ([`crate::cnf::reduced`]) already has exactly the
//! triangulations as models, but its solver propagates poorly. Worker
//! threads therefore run the pair search ([`intersect::stream_pairs`]) and
//! hand over each row of intersecting pairs. Once enough clauses are
//! waiting and the solver has had its minimum time slice, a worker sets the
//! solver's **pause** flag; the solver returns, the clauses are added, and
//! it resumes, keeping what it has learned. Every added clause (intersection
//! clause or blocking clause of a recorded triangulation) holds for every
//! triangulation not yet recorded, so a model is always a new triangulation
//! and unsatisfiability always ends the enumeration, however few clauses
//! have been added.
//!
//! The pause flag is the only flag the solver polls besides the deadline,
//! and only this module sets it. Pausing never ends the search: the search
//! ends when the solver says unsatisfiable, when the deadline passes, or
//! when recording a triangulation says so (goal reached, cap reached, or the
//! check failed), and then the workers are told to stop through a separate
//! closed state (REVIEW.md R28).
//!
//! **Failures of the pair search.** The result is decided only after the
//! pair search has ended and its thread has been joined, and a failed pair
//! search is an error whatever the solver found (P4.5). Its input errors
//! (dimension, shapes, point labels, coordinate bound, too many simplices)
//! are raised before the first row, and closing the search only stops it between rows, so they surface
//! however the threads are scheduled. Rows the search skips after the
//! solver has finished are not needed: every model of the reduced formula
//! is a triangulation.
//!
//! Flow control follows Julia: at most 1,000,000 clauses wait (workers
//! block beyond that), a pause needs 500,000 waiting clauses or the end of
//! the pair search, the solver's minimum slice starts at 0.5 s and grows by
//! half per pause up to 10 s, and after 20,000,000 added clauses the pair
//! search stops and the solver runs on what it has. Julia's chunking of rows
//! into channel messages is not needed: rows go into one shared buffer.

use super::{Classify, EnumResult, EnumerateError, Enumeration, Options, Search};
use crate::cnf::Cnf;
use crate::intersect::{self, IntersectError};
use crate::matrix::Matrix;
use crate::solve::{Backend, Control, SolveError, Solver, Status};
use std::{
    sync::{
        Arc, Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Flow-control settings of [`Exchange`].
#[derive(Clone, Copy, Debug)]
struct Tuning {
    /// Waiting clauses that justify pausing the solver (Julia's
    /// `SOLVER_UPDATE_THRESHOLD`).
    pause_threshold: usize,
    /// Waiting clauses beyond which workers block (Julia's
    /// `BUFFER_HARD_CAP`).
    buffer_cap: usize,
    /// Clauses added in all, after which the pair search stops (Julia's
    /// `MAX_INJECTED_CLAUSES`).
    max_injected: usize,
    /// First minimum time between two pauses.
    slice_initial: Duration,
    /// Longest minimum time between two pauses.
    slice_max: Duration,
}

/// Julia's settings.
const TUNING: Tuning = Tuning {
    pause_threshold: 500_000,
    buffer_cap: 1_000_000,
    max_injected: 20_000_000,
    slice_initial: Duration::from_millis(500),
    slice_max: Duration::from_secs(10),
};

/// Live counters for an incremental search, delivered on the calling thread.
///
/// Initial formula clauses and model-blocking clauses are excluded. The latest
/// notification contains the final counters; a notification does not imply that
/// the search succeeded, since pair-source failures are joined afterwards.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IncrementalProgress {
    /// Intersection clauses successfully inserted into the solver.
    pub clauses_added: usize,
    /// Solver calls that actually returned [`Status::Stopped`].
    /// Merely requesting a pause does not increment this count.
    pub interruptions: u64,
}

/// All triangulations of `reduced` (the formula of [`crate::cnf::reduced`])
/// with one solver, adding the intersection clauses of the simplices while
/// it runs. `points` and `simplices` are the input of
/// [`crate::lattice::faces`] for the same simplices.
///
/// The pair search uses one thread less than the current rayon pool has,
/// and at least one; the solver runs on the calling thread. Unlike Julia,
/// both backends get the intersection clauses.
pub fn incremental<C: Classify>(
    reduced: &Cnf,
    points: &Matrix,
    simplices: &[u32],
    options: &Options,
    classify: &C,
) -> EnumResult<Enumeration, C> {
    run(
        reduced,
        points,
        simplices,
        options,
        classify,
        &Exchange::new(TUNING),
    )
}

/// Incremental solving with a caller-provided exact pair source, for example
/// a GPU backend. The source has the same contract as [`intersect::stream_pairs`]:
/// it validates inputs before delivering rows, reports every failure, and stops
/// when the sink declines more rows. It is joined before any result is returned.
pub fn incremental_with_pairs<C: Classify>(
    reduced: &Cnf,
    points: &Matrix,
    simplices: &[u32],
    options: &Options,
    classify: &C,
    pair_source: &(
         impl Fn(&[i64], usize, &[u32], usize, intersect::RowSink<'_>) -> Result<(), IntersectError>
         + Sync
     ),
) -> EnumResult<Enumeration, C> {
    let mut exchange = Exchange::new(TUNING);
    exchange.source = pair_source;
    run(reduced, points, simplices, options, classify, &exchange)
}

/// Incremental CPU search with live counters. The observer runs initially at
/// zero, after each nonempty inserted clause batch, and after each actual solver
/// interruption. It never runs per pair or per triangulation. Keep it short:
/// callback time counts towards the search deadline.
pub fn incremental_with_progress<C: Classify>(
    reduced: &Cnf,
    points: &Matrix,
    simplices: &[u32],
    options: &Options,
    classify: &C,
    observer: &mut impl FnMut(IncrementalProgress),
) -> EnumResult<Enumeration, C> {
    run_observed::<true, C>(
        reduced,
        points,
        simplices,
        options,
        classify,
        &Exchange::new(TUNING),
        observer,
    )
}

/// [`incremental_with_progress`] using a caller-provided exact pair source.
/// The source follows the contract of [`incremental_with_pairs`].
pub fn incremental_with_pairs_and_progress<C: Classify>(
    reduced: &Cnf,
    points: &Matrix,
    simplices: &[u32],
    options: &Options,
    classify: &C,
    pair_source: &(
         impl Fn(&[i64], usize, &[u32], usize, intersect::RowSink<'_>) -> Result<(), IntersectError>
         + Sync
     ),
    observer: &mut impl FnMut(IncrementalProgress),
) -> EnumResult<Enumeration, C> {
    let mut exchange = Exchange::new(TUNING);
    exchange.source = pair_source;
    run_observed::<true, C>(
        reduced, points, simplices, options, classify, &exchange, observer,
    )
}

/// One complete streaming search; dynamic dispatch occurs once per search,
/// outside the pair kernel and its row loop.
type PairSource<'a> = dyn Fn(&[i64], usize, &[u32], usize, intersect::RowSink<'_>) -> Result<(), IntersectError>
    + Sync
    + 'a;

/// [`incremental`] with the flow control of `exchange`.
fn run<C: Classify>(
    reduced: &Cnf,
    points: &Matrix,
    simplices: &[u32],
    options: &Options,
    classify: &C,
    exchange: &Exchange,
) -> EnumResult<Enumeration, C> {
    run_observed::<false, C>(
        reduced,
        points,
        simplices,
        options,
        classify,
        exchange,
        &mut |_| {},
    )
}

fn run_observed<const PROGRESS: bool, C: Classify>(
    reduced: &Cnf,
    points: &Matrix,
    simplices: &[u32],
    options: &Options,
    classify: &C,
    exchange: &Exchange,
    observer: &mut impl FnMut(IncrementalProgress),
) -> EnumResult<Enumeration, C> {
    if options.backend == Backend::Kissat {
        return Err(SolveError::KissatUnsupported("incremental solving").into());
    }
    let variables = reduced.simplex_variables();
    let width = points.columns().saturating_add(1);
    if usize::try_from(variables)
        .ok()
        .and_then(|v| v.checked_mul(width))
        != Some(simplices.len())
    {
        return Err(EnumerateError::Mismatch(variables));
    }
    let workers = rayon::current_num_threads().saturating_sub(1).max(1);
    let search = Search::new(options, classify);
    classify.register_stop(&exchange.pause);
    let (kept, generated) = std::thread::scope(|scope| {
        let _close = Close(exchange);
        let generator = std::thread::Builder::new()
            .spawn_scoped(scope, || exchange.generate(points, simplices, workers))
            .map_err(|error| IntersectError::ThreadSpawn(error.kind()))?;
        let kept = {
            // Closed even if the solver loop unwinds, so the join below
            // cannot wait for ever.
            let _close = Close(exchange);
            solve::<PROGRESS, C>(reduced, exchange, &search, observer)
        };
        Ok::<_, IntersectError>((kept, generator.join()))
    })?;
    // The pair search has ended, so its outcome is final, and a failure
    // overrides whatever the solver found.
    match generated {
        Ok(Ok(())) => search.finish(vec![kept]),
        Ok(Err(error)) => Err(error.into()),
        Err(_) => Err(EnumerateError::Internal),
    }
}

/// Closes the exchange when dropped.
struct Close<'a>(&'a Exchange<'a>);

impl Drop for Close<'_> {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// The solver loop on the calling thread.
fn solve<const PROGRESS: bool, C: Classify>(
    reduced: &Cnf,
    exchange: &Exchange,
    search: &Search<'_, C>,
    observer: &mut impl FnMut(IncrementalProgress),
) -> EnumResult<Vec<Vec<u32>>, C> {
    let variables = usize::try_from(reduced.variables()).map_err(|_| EnumerateError::Internal)?;
    let mut solver = Solver::new(search.options.backend, variables)?;
    for clause in reduced.iter() {
        solver.add_clause(clause)?;
    }
    let control = Control {
        stop: Arc::clone(&exchange.pause),
        deadline: search.options.deadline,
    };
    let mut kept = Vec::new();
    let mut model = Vec::new();
    let mut simplices = Vec::new();
    let mut conflict = Vec::new();
    #[cfg(test)]
    let mut synchronize = exchange.synchronize_first_solve;
    let mut progress = IncrementalProgress::default();
    if PROGRESS {
        observer(progress);
    }
    loop {
        if search.deadline_passed() {
            break;
        }
        let previous = progress.clauses_added;
        let inserted = (|| {
            for (i, j) in exchange.take() {
                let clause = [negative(i), negative(j)];
                let [Some(a), Some(b)] = clause else {
                    return Err(EnumerateError::Internal);
                };
                solver.add_clause(&[a, b])?;
                if PROGRESS {
                    progress.clauses_added = progress.clauses_added.saturating_add(1);
                }
            }
            Ok(())
        })();
        if PROGRESS && progress.clauses_added != previous {
            observer(progress);
        }
        inserted?;
        // A failed pair search interrupts the solver and ends the loop here.
        exchange.resume()?;
        #[cfg(test)]
        if let Some(barrier) = synchronize.take() {
            // Tests coordinate an actual producer pause before the first solve.
            barrier.wait();
            barrier.wait();
        }
        let status = solver.solve(&[], &control)?;
        exchange.suspend(status == Status::Stopped);
        if PROGRESS && status == Status::Stopped {
            progress.interruptions = progress.interruptions.saturating_add(1);
            observer(progress);
        }
        match status {
            Status::Sat => {
                solver.model_prefix(reduced.simplex_variables(), &mut model)?;
                if !search.record(&model, &mut simplices, &mut kept, &mut conflict)? {
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
                search.timed_out.store(true, Ordering::SeqCst);
                break;
            }
            // Paused: add the waiting clauses and resume.
            Status::Stopped => {}
        }
    }
    Ok(kept)
}

/// The negative literal of simplex `s`.
fn negative(s: u32) -> Option<i32> {
    i32::try_from(s)
        .ok()
        .and_then(|s| s.checked_add(1))
        .map(i32::wrapping_neg)
}

/// State of the pair search.
enum Generation {
    Running,
    Done,
    Failed(IntersectError),
}

/// What the workers and the solver loop share, under [`Exchange::state`].
struct Pending {
    /// Intersecting pairs waiting to be added.
    pairs: Vec<(u32, u32)>,
    /// No more pairs are wanted: the search is over, or the clause budget
    /// is spent.
    closed: bool,
    generation: Generation,
    /// The solver is running.
    solving: bool,
    /// When the solver last resumed.
    resumed: Instant,
    /// The solver's current minimum time between pauses.
    slice: Duration,
    /// Pairs taken for the solver so far.
    injected: usize,
    /// Pauses so far.
    #[cfg(test)]
    pauses: usize,
    /// Tests: the pair search starts only once the exchange is closed.
    #[cfg(test)]
    hold: bool,
}

struct Exchange<'a> {
    source: &'a PairSource<'a>,
    tuning: Tuning,
    state: Mutex<Pending>,
    /// Signalled when pairs were taken, the search was closed, or the pair
    /// search ended, or the solver resumed.
    changed: Condvar,
    /// The solver's stop flag: set to pause it.
    pause: Arc<AtomicBool>,
    #[cfg(test)]
    synchronize_first_solve: Option<&'a std::sync::Barrier>,
}

impl Exchange<'_> {
    fn new(tuning: Tuning) -> Self {
        Self {
            source: &intersect::stream_pairs,
            tuning,
            state: Mutex::new(Pending {
                pairs: Vec::new(),
                closed: false,
                generation: Generation::Running,
                solving: false,
                resumed: Instant::now(),
                slice: tuning.slice_initial,
                injected: 0,
                #[cfg(test)]
                pauses: 0,
                #[cfg(test)]
                hold: false,
            }),
            changed: Condvar::new(),
            pause: Arc::default(),
            #[cfg(test)]
            synchronize_first_solve: None,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Pending> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Waits for a change, at most `limit` if given.
    fn wait<'a>(
        &self,
        guard: MutexGuard<'a, Pending>,
        limit: Option<Duration>,
    ) -> MutexGuard<'a, Pending> {
        match limit {
            Some(limit) => {
                self.changed
                    .wait_timeout(guard, limit)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            }
            None => self
                .changed
                .wait(guard)
                .unwrap_or_else(PoisonError::into_inner),
        }
    }

    /// Pauses the solver if clauses are due and its slice is over. Returns
    /// how long until the slice is over, if that is what a pause waits for.
    fn request_pause(&self, pending: &Pending) -> Option<Duration> {
        let finished = !matches!(pending.generation, Generation::Running);
        let due = pending.pairs.len() >= self.tuning.pause_threshold
            || (finished && !pending.pairs.is_empty());
        if !due || !pending.solving {
            // The solver loop adds the waiting pairs before it resumes.
            return None;
        }
        let elapsed = pending.resumed.elapsed();
        if elapsed >= pending.slice {
            self.pause.store(true, Ordering::SeqCst);
            None
        } else {
            Some(pending.slice.saturating_sub(elapsed))
        }
    }

    /// Runs the pair search with `workers` threads, then waits until the
    /// last pairs have been taken or the exchange is closed. Returns the
    /// outcome of the pair search.
    fn generate(
        &self,
        points: &Matrix,
        simplices: &[u32],
        workers: usize,
    ) -> Result<(), IntersectError> {
        #[cfg(test)]
        {
            let mut pending = self.lock();
            while pending.hold && !pending.closed {
                pending = self.wait(pending, None);
            }
        }
        let result = (self.source)(
            points.values(),
            points.columns(),
            simplices,
            workers,
            &|row| self.offer(row),
        );
        let mut pending = self.lock();
        pending.generation = match &result {
            Ok(()) => Generation::Done,
            Err(error) => {
                // Interrupt the solver so that the loop reports the error.
                // Under the lock, so `resume` either sees the failure or
                // runs before this.
                self.pause.store(true, Ordering::SeqCst);
                Generation::Failed(error.clone())
            }
        };
        while !pending.closed && !pending.pairs.is_empty() {
            let limit = self.request_pause(&pending);
            pending = self.wait(pending, limit);
        }
        result
    }

    /// Adds one row of pairs; blocks while too many are waiting. Returns
    /// whether more pairs are wanted.
    fn offer(&self, row: &[(u32, u32)]) -> bool {
        let mut pending = self.lock();
        if pending.closed {
            return false;
        }
        pending.pairs.extend_from_slice(row);
        loop {
            let limit = self.request_pause(&pending);
            if pending.closed || pending.pairs.len() < self.tuning.buffer_cap {
                return !pending.closed;
            }
            pending = self.wait(pending, limit);
        }
    }

    /// The waiting pairs, within the clause budget. Spending the budget
    /// closes the pair search.
    fn take(&self) -> Vec<(u32, u32)> {
        let mut pending = self.lock();
        let mut pairs = std::mem::take(&mut pending.pairs);
        let room = self.tuning.max_injected.saturating_sub(pending.injected);
        if pairs.len() >= room {
            pairs.truncate(room);
            pending.closed = true;
        }
        pending.injected = pending.injected.saturating_add(pairs.len());
        self.changed.notify_all();
        pairs
    }

    /// The solver resumes now, unless the pair search failed; then the
    /// pause flag stays set and the error is returned.
    fn resume(&self) -> Result<(), IntersectError> {
        let mut pending = self.lock();
        if let Generation::Failed(error) = &pending.generation {
            return Err(error.clone());
        }
        self.pause.store(false, Ordering::SeqCst);
        pending.solving = true;
        pending.resumed = Instant::now();
        // A producer can start waiting after take() while solving is false.
        // Wake it so it schedules a pause for this new solver slice.
        self.changed.notify_all();
        Ok(())
    }

    /// The solver returned; after a pause its next slice is longer.
    fn suspend(&self, paused: bool) {
        let mut pending = self.lock();
        pending.solving = false;
        if paused {
            let longer = pending.slice.saturating_mul(3).checked_div(2);
            let max = self.tuning.slice_max;
            pending.slice = longer.unwrap_or(max).min(max);
            #[cfg(test)]
            {
                pending.pauses = pending.pauses.saturating_add(1);
            }
        }
        self.changed.notify_all();
    }

    /// Ends the pair search: no more pairs are wanted.
    fn close(&self) {
        let mut pending = self.lock();
        pending.closed = true;
        pending.pairs = Vec::new();
        self.changed.notify_all();
    }
}

#[cfg(test)]
mod tests;

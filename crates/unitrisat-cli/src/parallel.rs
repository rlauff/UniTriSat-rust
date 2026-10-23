//! Bounded, completion-order scheduling. Only the coordinator writes output.

use crate::{
    cli::{Cli, CliError, OutputFormat, Tally},
    display::{ParallelReporter, WorkerSnapshot},
    job::{self, Progress, Row, StageTimes},
    resume::Session,
};
use std::{
    io::Write,
    panic::{self, AssertUnwindSafe},
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError, TrySendError},
    },
    time::{Duration, Instant},
};
use unitrisat::input;

const INTERVAL: Duration = Duration::from_millis(100);

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

enum Event<R> {
    Completed(R),
    Tick,
}

fn pools(workers: usize) -> Result<Vec<rayon::ThreadPool>, CliError> {
    let mut pools = Vec::new();
    for worker in 0..workers {
        pools.push(
            rayon::ThreadPoolBuilder::new()
                .num_threads(1)
                .thread_name(move |_| format!("polytope-{worker}"))
                .build()
                .map_err(|error| CliError::Worker(error.to_string()))?,
        );
    }
    Ok(pools)
}

/// The producer never blocks while dispatching. On failure both coordinator
/// channel ends are dropped before joining, releasing workers blocked in send
/// or receive. Active computations finish before this function returns.
fn schedule<J: Send, R: Send>(
    pools: Vec<rayon::ThreadPool>,
    mut source: impl Iterator<Item = Result<J, CliError>>,
    work: impl Fn(usize, &rayon::ThreadPool, J) -> Result<R, CliError> + Sync,
    mut report: impl FnMut(Event<R>) -> Result<(), CliError>,
) -> Result<(), CliError> {
    let workers = pools.len();
    let (jobs, queue) = mpsc::sync_channel(workers);
    let queue = Mutex::new(queue);
    let (completed, results) = mpsc::sync_channel(workers);
    let stop = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        let mut launch_error = None;
        for (worker, pool) in pools.into_iter().enumerate() {
            let (queue, stop, work) = (&queue, &stop, &work);
            let completed = completed.clone();
            let handle = std::thread::Builder::new()
                .name(format!("polytope-dispatch-{worker}"))
                .spawn_scoped(scope, move || {
                    loop {
                        if stop.load(Ordering::Acquire) {
                            break;
                        }
                        let job = match lock(queue).recv() {
                            Ok(job) => job,
                            Err(_) => break,
                        };
                        if stop.load(Ordering::Acquire) {
                            break;
                        }
                        let result =
                            panic::catch_unwind(AssertUnwindSafe(|| work(worker, &pool, job)))
                                .unwrap_or_else(|_| {
                                    Err(CliError::Worker("worker panicked".into()))
                                });
                        let fatal = result.is_err();
                        if fatal {
                            stop.store(true, Ordering::Release);
                        }
                        if completed.send(result).is_err() || fatal {
                            break;
                        }
                    }
                });
            match handle {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    launch_error = Some(CliError::Worker(error.to_string()));
                    break;
                }
            }
        }
        drop(completed);
        let mut outcome = panic::catch_unwind(AssertUnwindSafe(|| {
            if let Some(error) = launch_error {
                return Err(error);
            }
            let mut pending = None;
            let mut exhausted = false;
            let mut outstanding = 0_usize;
            let mut last_tick = Instant::now();
            loop {
                if last_tick.elapsed() >= INTERVAL {
                    report(Event::Tick)?;
                    last_tick = Instant::now();
                }
                // Limit each dispatch burst as well as the actual queue, so
                // fast workers cannot starve result processing or redraws.
                for _ in 0..workers {
                    if exhausted || stop.load(Ordering::Acquire) {
                        break;
                    }
                    if pending.is_none() {
                        pending = source.next().transpose()?;
                        if pending.is_none() {
                            exhausted = true;
                            break;
                        }
                    }
                    if let Some(job) = pending.take() {
                        match jobs.try_send(job) {
                            Ok(()) => outstanding = outstanding.saturating_add(1),
                            Err(TrySendError::Full(job)) => {
                                pending = Some(job);
                                break;
                            }
                            Err(TrySendError::Disconnected(_)) => {
                                return Err(CliError::Worker("job queue disconnected".into()));
                            }
                        }
                    }
                }
                if exhausted && outstanding == 0 {
                    return Ok(());
                }
                match results.recv_timeout(INTERVAL.saturating_sub(last_tick.elapsed())) {
                    Ok(result) => {
                        outstanding = outstanding.saturating_sub(1);
                        report(Event::Completed(result?))?;
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => {
                        return Err(CliError::Worker("completion queue disconnected".into()));
                    }
                }
            }
        }))
        .unwrap_or_else(|_| Err(CliError::Worker("coordinator panicked".into())));
        stop.store(true, Ordering::Release);
        drop(jobs);
        drop(results);
        for handle in handles {
            if handle.join().is_err() && outcome.is_ok() {
                outcome = Err(CliError::Worker("worker panicked".into()));
            }
        }
        outcome
    })
}

struct Completion {
    index: u64,
    result: Result<Row, job::JobError>,
    timings: Option<StageTimes>,
}

fn snapshots(slots: &[Mutex<WorkerSnapshot>]) -> Vec<WorkerSnapshot> {
    slots.iter().map(|slot| lock(slot).clone()).collect()
}

pub(crate) fn execute<O: Write, E: Write>(
    cli: &Cli,
    session: &Session,
    workers: usize,
    terminal: bool,
    out: &mut O,
    err: &mut E,
) -> Result<Tally, CliError> {
    // More workers than records cannot help. Create their native resources
    // before allocating the queues or telemetry, so extreme requested counts
    // fail through the OS's thread limit instead of an enormous allocation.
    let workers = workers.min(
        usize::try_from(session.total())
            .unwrap_or(usize::MAX)
            .max(1),
    );
    let pools = pools(workers)?;
    let settings = session.settings();
    let mut reporter = ParallelReporter::new(
        cli.terminal_output,
        terminal,
        session.total(),
        settings,
        workers,
    );
    reporter.restore_clock(session.elapsed()?, session.started_unix()?);
    reporter.set_estimator(session.estimator()?);
    let mut tally = Tally::default();
    let mut completed_indices = std::collections::BTreeSet::new();
    for (row, timings) in session.completed_rows()? {
        completed_indices.insert(row.index);
        tally.polytopes = tally.polytopes.saturating_add(1);
        let compatible = session.result_matches_request(row.index)?;
        if compatible {
            tally.add(&row);
        } else {
            tally.historical = tally.historical.saturating_add(1);
        }
        reporter.restored(&row, timings, &tally, compatible);
        if cli.output_format == OutputFormat::Tsv {
            crate::cli::write_restored_result(&row, settings, compatible, out)?;
        }
    }
    let stream: &mut dyn Write = if cli.output_format == OutputFormat::Human {
        out
    } else {
        err
    };
    reporter.initial(stream)?;
    let slots: Vec<_> = (0..workers)
        .map(|_| Mutex::new(WorkerSnapshot::default()))
        .collect();
    let input_error = |source| CliError::Input {
        path: session.input_path().to_owned(),
        source,
    };
    let source = (1_u64..)
        .zip(input::open_points(session.snapshot_path()).map_err(input_error)?)
        .filter(|(index, _)| !completed_indices.contains(index))
        .map(|(index, record)| record.map(|record| (index, record)).map_err(input_error));
    schedule(
        pools,
        source,
        |worker, pool, (index, record)| {
            let slot = slots
                .get(worker)
                .ok_or_else(|| CliError::Worker("invalid worker index".into()))?;
            lock(slot).begin(index);
            let mut timings = None;
            let result = if let Some(row) = session.completed(index)? {
                Ok(row)
            } else {
                session.begin(index)?;
                let result = panic::catch_unwind(AssertUnwindSafe(|| {
                    pool.install(|| {
                        if !cli.terminal_output.running {
                            job::run_detailed(
                                index,
                                &record.vertices,
                                &settings,
                                unitrisat::enumerate::Keep::None,
                            )
                            .map(|detailed| {
                                timings = Some(detailed.timings);
                                detailed.row
                            })
                        } else {
                            job::run_with_progress(
                                index,
                                &record.vertices,
                                &settings,
                                &mut |event| {
                                    if let Progress::Finished { timings: finished } = event {
                                        timings = Some(finished);
                                    }
                                    lock(slot).event(event);
                                },
                            )
                        }
                    })
                }))
                .unwrap_or_else(|_| {
                    Err(job::JobError::Panic("worker computation panicked".into()))
                });
                match result {
                    Ok(row) => Ok(session.finish_timed(row, timings)?),
                    Err(error) => {
                        session.fail(index, error.to_string())?;
                        Err(error)
                    }
                }
            };
            lock(slot).complete();
            Ok(Completion {
                index,
                result,
                timings,
            })
        },
        |event| {
            match event {
                Event::Completed(completion) => {
                    let sample = completion
                        .result
                        .as_ref()
                        .ok()
                        .and_then(|row| crate::eta::sample(row.status, completion.timings));
                    reporter.eta_finished(completion.index, sample);
                    tally.polytopes = tally.polytopes.saturating_add(1);
                    match completion.result {
                        Ok(row) => {
                            tally.add(&row);
                            reporter.completed(&row, completion.timings, &tally);
                            if cli.output_format == OutputFormat::Tsv {
                                // TSV rows share the screen with stderr on many
                                // terminals; terminal redraw is disabled in that case.
                                crate::cli::write_result(&row, settings, out)?;
                                out.flush()?;
                            }
                        }
                        Err(error) => {
                            tally.failed = tally.failed.saturating_add(1);
                            reporter.diagnostic();
                            writeln!(err, "error: polytope {}: {error}", completion.index)?;
                        }
                    }
                }
                Event::Tick => {
                    let stream: &mut dyn Write = if cli.output_format == OutputFormat::Human {
                        out
                    } else {
                        err
                    };
                    reporter.snapshot(&snapshots(&slots), &tally, stream)?;
                }
            }
            Ok(())
        },
    )?;
    for slot in &slots {
        lock(slot).done();
    }
    let stream: &mut dyn Write = if cli.output_format == OutputFormat::Human {
        out
    } else {
        err
    };
    reporter.finish(&snapshots(&slots), &tally, stream)?;
    Ok(tally)
}

#[cfg(test)]
mod tests;

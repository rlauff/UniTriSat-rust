use super::*;
use std::sync::{Arc, Condvar, atomic::AtomicUsize};

#[test]
fn free_workers_take_next_jobs_and_results_are_completion_order() {
    let gate = (Mutex::new(false), Condvar::new());
    let mut received = Vec::new();
    schedule(
        pools(2).unwrap(),
        (0..8).map(Ok),
        |_, pool, job| {
            assert_eq!(pool.install(rayon::current_num_threads), 1);
            if job == 0 {
                let ready = lock(&gate.0);
                let (ready, timeout) = gate
                    .1
                    .wait_timeout_while(ready, Duration::from_secs(5), |ready| !*ready)
                    .unwrap();
                assert!(
                    *ready && !timeout.timed_out(),
                    "another worker must reach job 3 while job 0 runs"
                );
            } else if job == 3 {
                *lock(&gate.0) = true;
                gate.1.notify_one();
            }
            Ok(job)
        },
        |event| {
            if let Event::Completed(job) = event {
                received.push(job);
            }
            Ok(())
        },
    )
    .unwrap();
    assert_ne!(received.first(), Some(&0));
    received.sort_unstable();
    assert_eq!(received, (0..8).collect::<Vec<_>>());
}

#[test]
fn output_failure_drops_bounded_queues_and_joins_all_active_workers() {
    let read = AtomicUsize::new(0);
    let active = AtomicUsize::new(0);
    let finished = AtomicUsize::new(0);
    let source = (0..10_000).map(|job| {
        read.fetch_add(1, Ordering::Relaxed);
        Ok(job)
    });
    let result = schedule(
        pools(2).unwrap(),
        source,
        |_, _, job| {
            active.fetch_add(1, Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(1));
            finished.fetch_add(1, Ordering::Relaxed);
            active.fetch_sub(1, Ordering::Relaxed);
            Ok(job)
        },
        |event| match event {
            Event::Completed(_) => Err(std::io::Error::other("output closed").into()),
            Event::Tick => Ok(()),
        },
    );
    assert!(matches!(result, Err(CliError::Output(_))));
    assert_eq!(active.load(Ordering::Relaxed), 0);
    assert!(finished.load(Ordering::Relaxed) >= 1);
    assert!(
        read.load(Ordering::Relaxed) <= 7,
        "producer must not eagerly read the batch"
    );
}

#[test]
fn input_and_worker_failures_release_waiting_threads() {
    let result = schedule(
        pools(3).unwrap(),
        std::iter::once(Err::<usize, _>(CliError::Worker("input failed".into()))),
        |_, _, job| Ok(job),
        |_| Ok(()),
    );
    assert!(result.is_err());
    let result = schedule(
        pools(3).unwrap(),
        (0..100).map(Ok),
        |_, _, _| Err::<usize, _>(CliError::Worker("persist failed".into())),
        |_| Ok(()),
    );
    assert!(matches!(result, Err(CliError::Worker(message)) if message == "persist failed"));
    let result = schedule(
        pools(3).unwrap(),
        (0..100).map(Ok),
        |_, _, _| -> Result<usize, CliError> { panic!("worker infrastructure panic") },
        |_| Ok(()),
    );
    assert!(matches!(result, Err(CliError::Worker(message)) if message == "worker panicked"));
}

#[test]
fn pools_are_reused_for_successive_jobs() {
    let identities = Arc::new(Mutex::new(Vec::new()));
    schedule(
        pools(1).unwrap(),
        (0..5).map(Ok),
        |_, pool, job| {
            lock(&identities).push(pool.install(|| std::thread::current().id()));
            Ok(job)
        },
        |_| Ok(()),
    )
    .unwrap();
    let identities = lock(&identities);
    assert_eq!(identities.len(), 5);
    assert!(identities.windows(2).all(|ids| ids[0] == ids[1]));
}

#[test]
fn empty_source_shuts_down_idle_workers() {
    let mut count = 0;
    schedule(
        pools(3).unwrap(),
        std::iter::empty::<Result<usize, CliError>>(),
        |_, _, job| Ok(job),
        |event| {
            if matches!(event, Event::Completed(_)) {
                count += 1;
            }
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn coordinator_panics_disconnect_queues_before_joining_workers() {
    let source = std::iter::from_fn(|| -> Option<Result<usize, CliError>> {
        panic!("input iterator panicked")
    });
    let result = schedule(pools(2).unwrap(), source, |_, _, job| Ok(job), |_| Ok(()));
    assert!(matches!(result, Err(CliError::Worker(message)) if message == "coordinator panicked"));
    let result = schedule(
        pools(2).unwrap(),
        (0..100).map(Ok),
        |_, _, job| Ok(job),
        |_| panic!("output writer panicked"),
    );
    assert!(matches!(result, Err(CliError::Worker(message)) if message == "coordinator panicked"));
}

#[test]
fn progress_ticks_continue_while_every_worker_is_busy() {
    let gate = (Mutex::new(false), Condvar::new());
    let mut ticks = 0;
    schedule(
        pools(1).unwrap(),
        std::iter::once(Ok(1)),
        |_, _, job| {
            let ready = lock(&gate.0);
            let (ready, timeout) = gate
                .1
                .wait_timeout_while(ready, Duration::from_secs(5), |ready| !*ready)
                .unwrap();
            assert!(
                *ready && !timeout.timed_out(),
                "coordinator must tick without a completion"
            );
            Ok(job)
        },
        |event| {
            if matches!(event, Event::Tick) {
                ticks += 1;
                *lock(&gate.0) = true;
                gate.1.notify_one();
            }
            Ok(())
        },
    )
    .unwrap();
    assert!(ticks >= 1);
}

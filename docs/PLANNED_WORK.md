# Queued solver and incremental performance work

Requested on 2026-10-21. A bounded Kissat/PicoSAT/CaDiCaL CPU comparison is complete;
see [the measurements](PERFORMANCE.md#kissat-comparison-2026-10-21). Broader solver
evaluation and interruption tuning remain queued. Kissat is available for
compatible first-solution searches; PicoSAT remains the default. Every code
change and final default requires approval by two strong reviewers; an Astra
author may count as one of them. Correctness gates precede performance selection.

## Solver evaluation

Compare the current PicoSAT and CaDiCaL implementations first, then evaluate
CaDiCaL 3.0.1 as a modern candidate. Retain the native thread-safety, exception,
assumption, termination and model-blocking guarantees. Prefer maintained bindings
or existing wrappers over recreating solver infrastructure. Change the default
only after repeatable full-pipeline gains on representative workloads, without
material regressions hidden by an average.

Stock Kissat is not a drop-in incremental candidate: its API lacks assumptions, and it rejects
clause additions after searching. Even serial enumeration needs to add blocking
clauses after a model. The incremental Kissat fork in STP is a separate candidate,
with additional native integration and correctness review requirements. Single
SAT competition rankings do not predict repeated-enumeration performance.
The current option rejects incremental mode, all-solutions enumeration and
regular/flag filtering. Serial and split first-solution searches are compatible:
split groups add their restriction clause before the first solve.

Sources: [Kissat API](https://github.com/arminbiere/kissat/blob/master/src/kissat.h),
[Kissat clause-add restrictions](https://github.com/arminbiere/kissat/blob/master/src/internal.c),
[rustsat-kissat bindings](https://github.com/chrjabs/rustsat/tree/main/kissat),
[STP incremental fork](https://stp.github.io/stp/kissat.html),
[CaDiCaL C API](https://github.com/arminbiere/cadical/blob/master/src/ccadical.h).

Required checks: exact counts and model sets; blocking clauses; assumptions;
serial, split and incremental modes; pause/restart; deadlines and cancellation;
independent concurrent instances. Record memory, solver time, end-to-end time
and timeouts using interleaved repeated measurements with no concurrent builds.

## Incremental interruption thresholds

Generate integer-scaled unit cubes, allowing a different scale on each axis.
Use bounded calibration pilots to find roughly 60–120 seconds per polytope for
each practical dimension. Never allow a calibration run to grow into an
unbounded hours-long job.

After selecting the solver, sweep pending-clause interruption thresholds and
relevant time slices coarsely, then repeat the finalists on held-out scales.
Compare against current defaults and nonincremental baselines. Validate exact
answers independently of timings. Publish the measured best per-dimension
settings with uncertainty and workload scope; preserve a safe fallback for
unmeasured dimensions. Do not claim a universal mathematical optimum.

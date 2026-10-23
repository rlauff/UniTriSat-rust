# Performance

Measured on Linux with Rust 1.97.0, Julia 1.10.11 and an NVIDIA GeForce RTX 5060
Laptop GPU (8GiB, driver 610.57.04, CUDA13.3). Overall CPU runs are roughly **2×
faster** on the measured workloads. This is a descriptive estimate, not a
universal input-by-input guarantee. Compilation, cold startup, warm execution
and Julia subprocess overhead are distinct costs.

## Kissat comparison (2026-10-21)

Kissat improved the SAT search stage on the two largest completed inputs, but
geometry dominated their total runtime. PicoSAT remains the default: Kissat
cannot serve incremental, full-enumeration or regular/flag searches, and smaller
cases showed regressions. No automatic solver-selection policy was added.

The table shows median prepared-formula search times, including solver creation,
clause loading and disposal. Speedup is the median of paired PicoSAT/Kissat
ratios, so it can differ from the ratio of the displayed medians.

| Input | Mode | PicoSAT ms | CaDiCaL ms | Kissat ms | Kissat speedup over PicoSAT |
|---|---|---:|---:|---:|---:|
| Unit 4-cube | Serial | 130.68 | 168.68 | 87.06 | 1.52× |
| Unit 4-cube | Split | 101.22 | 176.20 | 88.95 | 1.13× |
| 2×2×2 cube | Serial | 157.76 | 262.80 | 128.29 | 1.24× |
| 2×2×2 cube | Split | 145.79 | 264.89 | 129.79 | 1.14× |

These formulas contain 2,672 variables/1,593,617 clauses and 3,688 variables/
2,289,881 clauses, respectively. Kissat was roughly twice as fast as CaDiCaL
in these search-stage comparisons. Complete library-call gains over PicoSAT
were only about 0.7–2.1% by median paired ratios. For example, serial unit-4-cube
medians were 3.002 seconds with PicoSAT and 2.962 seconds with Kissat; serial
2×2×2-cube medians were 1.199 and 1.174 seconds. Only three balanced rounds
measured the full pipeline, so these small differences are exploratory.

The smaller unit 3-cube had median serial search times of 0.139/0.060 ms and
full-call times of 0.465/0.358 ms for PicoSAT/Kissat. Conversely, the unit square's
serial search took 3.10/3.59 microseconds, and the immediately unsatisfiable
Reeve tetrahedron took 0.66/1.90 microseconds. There is no universal speedup;
tiny-call rankings can reflect initialization overhead and measurement noise.

Ten inputs spanned 2D–6D, including scaled/unit cubes, retained fixtures and
unsatisfiable cases. First unfiltered searches used one Rayon thread for serial and four for
split, CPU intersections and no outer job parallelism. Each configuration had
a warmup followed by twelve balanced search rounds (tiny calls batched equally
across backends) and three balanced full-call rounds. All 900 measured batches,
containing 29,808 calls, completed without solver timeouts or errors. Every
returned model satisfied the independently prepared full CNF, and successful
SAT/UNSAT outcomes agreed across backends. Validation was outside timed regions;
different valid first triangulations were permitted. Full-call timings exclude
file parsing, process startup, terminal output and Julia transport.

The AMD Ryzen 7 260 laptop retained its `powersave` governor and `low-power`
platform profile. The user's large run was stopped; ordinary desktop services
and a transient package-cache update remained. No concurrent agent builds or
benchmarks ran. CPU frequency and temperature were not controlled. Process RSS
was recorded per whole case, not per solver, and cannot rank solver memory use.
No GPU or incremental solver comparison was performed.

Big3D's first record exceeded the 20-second pilot preparation limit before any
solver ran; it has no solver result in this comparison. `OhsugiHibiExample1`
was excluded because an unresolved merge marker in the input caused a parser
error. The unit-4-cube pilot exceeded its aggregate case limit, then completed
all final rounds under the larger 120-second case limit. Exclusions and partial
pilots were not treated as fast successes. Raw observations, spreads, harnesses,
source identities and reviews are preserved beside the repository in
`UniTriSat-kissat-benchmarks-2026-10-21`.

## CPU stages and known exceptions

Criterion recorded 168 measurements (84 identifiers at 1 and 16 threads,20 samples
each). All 56 matching historical Julia observations were slower than the upper
endpoint of Rust's 95% mean interval. Julia's historical observations were not
jointly sampled confidence bounds, and some stage boundaries differ: Rust's
face stage also constructs incidence information that Julia builds later.
These results do not imply every later input or isolated stage is faster.

Fresh repeated measurements identified five CPU intersection exceptions:

| Input | Julia mean ms | Rust mean ms | Rust slower |
|---|---:|---:|---:|
| 2D simplex, volume 25, row 1 | 0.798 | 0.938 | 17.4% |
| 2D polytope, volume 25, row 1 | 0.189 | 0.245 | 29.3% |
| Known 3D, volume 16, row 2 | 8.856 | 11.088 | 25.2% |
| Known 3D, volume 29, row 69782 | 8.034 | 10.352 | 28.9% |
| 3D polytope, volume 36a, row 1 | 1.207 | 1.552 | 28.5% |

These are complete intersection-stage calls, not complete triangulation searches.
The excess is about 0.055–2.318ms per call. All exact pair sets matched. Both
languages used bounded fixed-width integer arithmetic; these cases fit Rust's
i64 lane. Arbitrary precision does not explain the gap.

Measurements used one thread, identical point/simplex order, five actual-case
warmups and ABBA process order with 15 measured calls per block (30 per side).
Samples share process/cache state; substantial block drift occurred in some
cases. Means are recorded without selecting favorable minima. Actual power
telemetry reported external power, despite the earlier travel/battery expectation.

The applied, independently reviewed D2 witness bypass improves the previous Rust
baseline by about 1.31–1.52× on those two cases. Serial collection reuse,
D3 witness reduction, cached facet projections, facet endpoint shortcuts and GPU
buffer reuse were tested and rejected for regressions or insignificant gains.
The user explicitly accepted documenting remaining stage exceptions after the
focused optimizer found no clean measured solution. Correctness and exactness
were not relaxed. Raw distributions, unsuccessful experiments and review reports
are preserved in the separate migration evidence bundle described in
[VALIDATION.md](VALIDATION.md).

## GPU

All timings below include complete preparation, transfers, search, collection
and sorting. Warm numbers are the minimum of three complete calls; they are
bounded observations, not universal performance guarantees. Cold calls include
NVRTC/module setup and the first case's context initialization.

| Workload | Cold GPU seconds | Warm GPU seconds | CPU 16 seconds | CPU / warm GPU |
|---|---:|---:|---:|---:|
| 4-cube, unimodular | 0.310160 | 0.062395 | 0.293482 | 4.704× |
| 5D volume 10 row 1, nondegenerate | 0.013433 | 0.000966 | 0.004620 | 4.781× |
| 6D volume 11 row 2, nondegenerate | 0.020848 | 0.002402 | 0.020496 | 8.533× |

Each call asserted real CUDA execution and exact CPU/reference pair equality.
Small jobs can lose to CPU because of startup overhead. GPU-requested incremental
search uses GPU intersections when supported and exact CPU fallback otherwise.

## Julia frontend overhead

With five warmup pairs and 15 alternating measured pairs, public Julia calls
averaged 2.561ms for square 2 and 10.231ms for cube 3. Direct Julia bridge transport
averaged 2.535/9.478ms; both include JSON, temporary files and child startup.
Subtracting each public call's own Rust job time leaves mean overhead of
2.302/4.076ms. Package import is excluded, and first-observed calls share JIT
state; these are not independent cold-start measurements.

Use [the development guide](DEVELOPMENT.md) to reproduce current Criterion tests.
Windows/macOS performance has not been measured.

## Live output and checkpoints (2026-10-21)

Before live classification snapshots were added, optional stage callbacks were
compared with the silent library path using ten
warmups and 60 alternating paired samples, one thread, serial PicoSAT and full
enumeration. Square means were 93.832/93.707 microseconds (silent/progress);
3-cube means were 2.575/2.572 milliseconds. No mean regression was measurable
from the callbacks and stage clocks alone. These figures exclude terminal I/O.

A separate full-CLI comparison used three warmups and twelve alternating-order
samples per mode, one thread. The terminal case wrote human progress to a real
pseudo-terminal drained by the harness. The quiet case wrote TSV to `/dev/null`.
Both new modes include input snapshotting, validation and checkpoint bookkeeping.

| Batch | Previous CLI mean | New quiet mean | New live terminal mean |
|---|---:|---:|---:|
| 3,000 squares, enumerate all | 200.04 ms | 211.09 ms | 232.57 ms |
| 200 unit 3-cubes, enumerate all | 262.55 ms | 268.17 ms | 273.92 ms |

Live output added about 21.48 ms per square batch and 5.74 ms per cube batch
compared with the new quiet mode. Median paired relative overheads were 11.0%
and 1.9%, respectively. Very short jobs make display overhead visible; use
`--terminal-output=none` for throughput-sensitive batches. There are no
per-intersection or per-model progress callbacks. A stage update is flushed
before a potentially long solve; other updates are coalesced.

These are local observations, not universal bounds: timings showed process-to-
process drift, and a drained pseudo-terminal excludes the cost of an actual GUI
terminal renderer. Signal checkpoints serialize results only on interruption;
the per-polytope bookkeeping stays in memory. Resuming reads and validates a
snapshot before reusing results. The ordinary Rust library does not perform CLI
printing, file snapshotting, or checkpoint bookkeeping.

The following rejection-counter measurements predate the persistent execution
pool added with parallel file processing. They retain the cost of the earlier
implementation; current measurements appear below.

Live rejection counters add a worker only for observed regular/flag searches,
so snapshots can reach the calling thread even while SAT or classification is
busy. A separate paired library measurement used five warmups, 30 alternating
pairs, one thread, serial PicoSAT and full enumeration. Callbacks performed no
terminal I/O:

| Search | Silent mean | With classification snapshots |
|---|---:|---:|
| Square, flag | 0.047 ms | 0.190 ms |
| Square, regular | 0.088 ms | 0.300 ms |
| Unit 3-cube, flag | 1.230 ms | 2.051 ms |
| Unit 3-cube, regular | 8.671 ms | 9.716 ms |
| Unit 3-cube, regular and flag | 8.326 ms | 9.222 ms |

This overhead is substantial in percentage terms for tiny jobs: starting the
worker and initializing thread-local native state costs more than their short
searches. It is not a 100 ms delay: completion wakes the caller immediately.
Unfiltered searches retain the direct path (unit 3-cube: 0.950 ms with either
API). Quiet CLI output and the ordinary library APIs avoid this telemetry cost.
These measurements exclude terminal rendering and custom Rayon pool creation;
an observed filtered call from inside a Rayon worker preserves the configured
thread count through a private pool, rather than inheriting pool identity or
thread-local state.

A bounded follow-up used a 2×1×1 cube, stopped at 1,000 triangulations, with
two warmups and six alternating pairs per filter. Mean silent/live times were
69.92/70.38 ms (flag), 412.72/395.83 ms (regular), and 284.20/281.49 ms
(both). Median paired differences were +0.97%, +0.49%, and −1.46% respectively.
The negative differences reflect measurement variation, not an optimization.
Worker overhead was much less visible on these longer searches; six pairs do
not establish a universal bound or predict minute-long workloads.

## Parallel file processing (2026-10-21)

The optimizer compared sequential execution with 1, 2, 4 and 8 queued workers
on this 16-logical-CPU laptop. Each polytope used one internal Rayon thread,
PicoSAT, serial solving and full enumeration. Cases ran in shuffled interleaved
order after a warmup of each variant. Five measured samples were taken per
configuration, except the regular-square stress case, which used three.
Quiet runs emitted TSV; live runs also wrote progress to a drained
pseudo-terminal. Input validation, checkpoints' in-memory bookkeeping, process
startup and output were included.

| Batch | Four workers: quiet speedup | Four workers: live speedup |
|---|---:|---:|
| 3,000 squares | 3.63× | 3.86× |
| 200 unit 3-cubes | 2.56× | 3.64× |
| 50 unit 3-cubes, regular | 3.97× | 4.25× |
| 200 unit 3-cubes, flag | 3.67× | 4.41× |
| 50 unit 3-cubes, regular and flag | 3.19× | 3.13× |
| 500 squares, regular | 3.99× | 2.59× |

Ratios compare median times with the current sequential implementation. All
228,000 measured result rows matched after sorting by input index and excluding
elapsed time; no missing or duplicate indices were allowed. Eight workers
continued to improve throughput on this machine, but the short cases were noisy
and CPU power/temperature were not controlled. These observations do not predict
large individual polytopes, GPU workloads or other hardware. The drained PTY
also excludes graphical terminal rendering.

`--parallel=1` adds queue dispatch and worker handoff without concurrent jobs;
omit the flag for sequential throughput. Four workers remain the requested
default. No speculative numerical optimization or solver-default change was
needed. The queue and telemetry are bounded by the worker count; completed
checkpoint rows necessarily consume memory proportional to completed work.

Filtered progress now reuses native execution threads through one cached Rayon
pool per calling thread, replacing it if the requested thread count changes.
This avoids allocating native cddlib scratch on a new thread for every polytope.
Repeated real-LP tests verify stable execution-thread identities across modes
and solvers. Pools retain their workers until replaced or their caller exits;
concurrent library callers can each retain a pool. The CLI's parallel workers
request one thread per pool. Ordinary silent library calls remain unchanged.

## Size-aware ETA overhead (2026-10-21)

The live ETA now uses the remaining distribution of cheap input-size descriptors
and observed successful job timings. Descriptors combine ambient dimension,
vertex count and coordinate spans; they are not volume calculations or proofs
of SAT complexity. They are collected during the existing validation pass and
stored as two-byte bucket IDs per input. No hull, lattice, intersection or SAT
work is performed to estimate remaining time.

The estimator has 512 fixed buckets. Completion updates are constant work;
refreshes inspect those buckets plus active workers, never the remaining input
list. After the 2026-10-23 refinement, even one successful timing permits an
estimate marked `(low confidence)` immediately after the time. Comparable sizes
use observed means; unseen sizes use a usable fitted trend, then the nearest
observed size in the same dimension, then the run's overall mean. Two linear
bucket passes select nearest sizes. Unknown descriptors also contribute valid
timings to that overall mean. Distant smaller tails retain a heuristic floor,
not a guaranteed bound; numerically unusable or highly scattered trends fall
back to observed means.

Variable observations, censored jobs and overdue workers lower confidence
without automatically suppressing the number. Failed or capped jobs are never
treated as completed-duration samples. With no usable timing evidence the
field shows `learning`, or `unpredictable` after failures/censoring; numerical
failures also remain unpredictable. Active residual time grows with overdue age
using a heuristic quarter-age floor, and the largest remaining job prevents the
last parallel worker from being divided by the original worker count. These
confidence labels and residuals are heuristics, not calibrated probabilities
or completion bounds. Resumed jobs need a fresh successful timing; saved stage
tables and elapsed time still restore normally. With no work left, the estimate
is zero. When running output is disabled, descriptor collection and ETA updates
are skipped.

An optimized bookkeeping-only microcheck of the original 2026-10-21 estimator
recorded the following; these measurements predate the confidence refinement:

| Operation | Observed elapsed time |
|---|---:|
| Compute 100,000 four-vertex descriptors | 11.79 ms |
| Initialize estimator and retire 90,000 jobs | 1.30 ms |
| 1,000 refreshes with one populated bucket | 1.97 ms |
| 1,000 refreshes with all 512 buckets populated | 20.50 ms |

The dense refresh is about 20.5 microseconds, or roughly 0.021% of one CPU at
ten refreshes per second for this measured configuration. IDs occupied 200,000
bytes for 100,000 records, plus fixed estimator state. This was one bounded
measurement while another user computation heavily loaded the laptop; it is
not a controlled end-to-end benchmark or a universal overhead guarantee.

Deterministic tests use real scaled-shape descriptors with synthetic runtimes
to check ordering bias. In a wide large-first example, the old completed-job
average predicted 6,144 seconds for 172 seconds of remaining synthetic work;
the new deliberately rough estimate was 320 seconds. In a calibrated small-first
example, the old estimate was 740 seconds versus 4,096 seconds actual/new.
These demonstrate the intended model behavior, not measured solver forecasts.
Tests also cover mixed ordering, one-sample sorted cold buckets, unreliable
samples, unsupported descriptors, resumed runs, parallel stragglers and
numerically unusable extrapolation. Hard SAT instances can still depart widely
from a low-confidence estimate despite similar sizes.

Worker formatting aligns the existing inline fields without adding a header or
new labels. The stage column reserves nine characters across frames, preventing
`solving`, `simplices` and `completed` transitions from moving later columns;
numeric fields use the minimum shared padding needed in each frame.
Raw checks and review receipts are retained beside the repository in
`UniTriSat-eta-validation-2026-10-21`.

## Flag SAT and regularity learning (2026-10-23)

Both features remain opt-in. Flag SAT helps some inputs and costs more on small
controls. Exact regularity conflict learning did **not** provide a meaningful
speedup on these tested inputs: its certificate cost outweighed the few
candidates skipped. This is evidence about these workloads, not a claim that
learning can never help a larger search.

The release binary used CPU intersections, serial PicoSAT, one Rayon thread,
`--all`, and quiet TSV output. Benchmark children were pinned to CPU 10 on an
AMD Ryzen 7 260. Each of 32 fixture/option combinations had a warmup, then three
interleaved repeated runs; identical input batches amortized process startup.
The 96 measured runs plus warmups took about 62 seconds. Timings include input,
geometry, formula construction, SAT and LP work. Every output row completed and
matched its independently expected target count; separate core tests compare
exact accepted sets across modes and solvers.

Median milliseconds per polytope:

| Input | `--flag` | `--flag-sat` | `--regular` | `--regular --regular-learn` |
|---|---:|---:|---:|---:|
| Unit 3-cube | 1.080 | 1.813 | 8.001 | 7.975 |
| 2 × standard 3-simplex | 2.955 | 4.795 | 36.590 | 36.163 |
| v16-r6 | 1.940 | 0.854 | 101.633 | 193.455 |
| v16-r16 | 13.498 | 12.476 | 817.866 | 822.445 |

Flag SAT was **2.27× faster** on v16-r6 and **1.08× faster** on v16-r16, but
**1.68× slower** on the cube and **1.62× slower** on the dilated simplex.
Its visited candidate counts changed respectively from 256 to 9, 998 to 654,
72 to 64, and 192 to 180; accepted flag counts agreed exactly.

Regular learning on v16-r6 was **1.90× slower**, visiting 255 instead of 256
candidates while finding the same 158 regular triangulations. The complete
baseline has 98 nonregular candidates, but the learned clauses happened to
exclude little work still ahead of this solver. v16-r16 retained 998 visits and
992 regular results; its ~0.6% timing change is small. The all-regular controls
had no certificate solves and differed by roughly 1% or less.

For combined regular/flag searches on v16-r6, enabling regular learning changed
5.665 to 10.066 ms with the independent flag checker, and 4.160 to 8.662 ms with
flag SAT: **1.78× and 2.08× slower**, respectively, with no further candidate
reduction. The three regular flag target triangulations remained unchanged.

A supplementary three-repeat paired CaDiCaL check gave the same conclusion:
v16-r6 changed 109.783 to 196.189 ms (**1.79× slower**, 256 to 253 visits, 158
regular results); v16-r16 changed 866.736 to 875.843 ms (~1.1% overhead, unchanged
998 visits and 992 regular results). Thus changing solver order did not produce
a performance win on these fixtures.

These are small complete-enumeration cases, not hard first-solution searches.
Desktop background activity and CPU frequency were not isolated. Small
percentage differences should not be treated as established improvements.
For example, the three PicoSAT v16-r6 regular runs ranged from 101.356–103.251 ms
without learning and 190.672–198.555 ms with it. All raw timings, commands, batch
sizes, CPU affinity, binary hashes, target checks and supplementary results are
retained outside the crate in
`UniTriSat-regular-learn-validation-2026-10-23/benchmark`.

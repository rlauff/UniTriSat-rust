# Validation and retained evidence

The Linux migration, CUDA module and thin Julia frontend passed focused
independent critical-code reviews. Review scope covered exact kernels, bounds,
concurrency, native solver boundaries, resume provenance, GPU streaming/fallback,
protocol validation and returned geometry. Rejected prototype implementations
are not included in production.

- Lattice pipeline:1610 strict differential cases, zero unexplained mismatches.
- Intersections:805 cases at each of 1 and 16 threads, exact reference agreement.
- Classification:10584 variants across configurations/backends/modes. Explicit
  known Julia reference failures were recorded separately with independent
  coverage; they were never counted as successful comparisons.
- Final bounded sample:46 selected cases across dimensions 2–6, volume ranges,
  known results and cap/limit edges.44 completed comparisons passed the bounded
  contract;2 agreed only on timeout status. Capped classification counts remain
  lower bounds. The failed initial run and interrupted follow-up were preserved;
  a separately reviewed artifact adjudication resolved three one-sided timeouts.
- Julia's missing 2D CPU guard was handled by a user-approved, harness-only bound,
  independently checked against 289 BigInt triangle pairs and boundary guards.
  Original reference source was not patched for those comparisons.
- Final production D2 improvement:248 release Rust tests, five separately
  executed actual CUDA hardware tests, full-target/full-feature Clippy and fmt.
  The rebuilt bridge passed 173 Julia assertions; earlier optional-extension and
  real plotting checks bring frontend coverage to 207 assertions. Post-promotion
  root import, bridge discovery, exact geometry, real D3 plotting and local
  package installation were checked separately.
- Native race tests: patched cdd/CaDiCaL runs clean; intentionally defective
  positive controls produced the expected ThreadSanitizer reports.

GPU tests cover all generated dimension/precision variants, exact CPU parity,
incremental solving with both SAT backends, cancellation and late-failure CPU
fallback without duplicated pairs. Headless real D3/D4 rendering was executed;
D5/6 boundary extraction was tested. Browser interaction was not tested.
Windows/macOS hardware testing remains explicitly deferred. The optional
floating-point regularity accelerator was outside this exact migration's scope.

The five documented stage-performance exceptions are accepted under the user's
explicit instruction to finish if focused optimization found no clean solution;
see [PERFORMANCE.md](PERFORMANCE.md). This does not waive correctness gates.

## Direct CLI, checkpoints and live counters (2026-10-21)

The final Linux check passed 248 Rust tests and five separately executed CUDA
hardware tests, plus all-target/all-feature Clippy, formatting and documentation
builds. Two Astra reviewers accepted the implementation and final fixes.
Coverage includes actual solver interruptions, inserted-clause counts, rejected
classification counts, skipped tests, callback panic cleanup, capped searches,
GPU progress and unavailable statistics when replaying saved results. A real
Ctrl+C run saved 20 completed polytopes; resume produced all 3,000 indexed rows
without recomputing saved rows and rejected a modified input. Direct help and
real regular/flag output passed smoke checks in all three solving modes.
Logs and the bounded output-overhead measurements are retained beside the
repository in `UniTriSat-cli-validation-2026-10-21`.

## Parallel file processing (2026-10-21)

The parallel queue, sparse checkpoint format, worker display and persistent
classification pools passed Astra author review and independent Astra
adversarial review. Review found and resolved coordinator-panic shutdown
deadlock and per-job native scratch allocation on short-lived threads.

The final suite passed 276 Rust tests, five real CUDA tests, strict Clippy,
formatting and documentation builds. Independent subprocess checks covered
31 mixed-workload runs across both solvers and all modes, sparse resume with
byte-identical saved rows, failures, caps, timeouts and broken-pipe shutdown.
A real four-worker Ctrl+C run saved 21 completed rows while four other jobs
were active; resuming with two workers produced all 3,001 unique indexed rows
without recomputing saved results. A real pseudo-terminal check verified four
worker rows, intermediate global totals, classification/incremental counters
and resizing. Prior sequential checkpoint files remain readable.

Detailed checks and throughput measurements are retained beside the repository
in `UniTriSat-parallel-validation-2026-10-21`. Windows/macOS hardware testing
remains deferred.

## Kissat option (2026-10-21)

Focused checks passed 17 Kissat tests across the solver, enumeration, public API,
CLI, bridge and checkpoints, plus seven existing CaDiCaL regression tests.
Coverage includes native UNKNOWN handling, one-shot guards, eager rejection of
incompatible options, serial/split first-solution searches, independent parallel
jobs and cancellation/deadline handling. Nine Julia frontend compatibility
checks also passed. These are focused checks, not a rerun of the full migration
suite.

Strict all-target Clippy passed for the affected crates with CUDA enabled,
along with formatting and the offline dependency-policy check. The rebuilt
CUDA-enabled release executable passed tiny file-input checks for serial/split
searches, parallel jobs, displayed solver selection and incompatible-option
errors (including aliases). Astra author and independent adversarial reviews
accepted the changes. Evidence is retained beside the repository in
`UniTriSat-kissat-validation-2026-10-21`.

Native archive inspection confirmed that CaDiCaL's privately prefixed Kitten
symbols do not overlap with Kissat's exports. The vendored CaDiCaL source bytes
are unchanged. At integration time, performance comparisons were deferred at
the user's request; the subsequent comparison is recorded below. PicoSAT
remains the default. Windows/macOS hardware testing remains deferred.

## Positional snapshots and setting overrides (2026-10-21)

The version-3 resume changes passed 66 CLI tests with CUDA enabled, strict
all-target CLI Clippy, formatting and a CUDA-enabled release executable build.
Tests cover strict version-1/2/3 reads, malformed provenance and duplicate map
indices, inherited options versus explicit defaults, compatible final settings,
elapsed-time accumulation without offline gaps or repeated-save double counting,
and quiet/table-only stage timing without live classification telemetry.

External subprocess checks exercised three successive parallel Ctrl+C/resume
cycles plus a separate sequential interruption. Final runs emitted all 8,000
unique input rows and preserved completed rows exactly. The checks verified
arbitrary snapshot filenames, settings/output inheritance, mode/solver/backend
overrides, false/zero overrides, persistent mixed-request warnings and explicit
warnings when adding either regularity or flag filtering. A deliberately seeded
37-second elapsed baseline continued across repeated saves and appeared in the
human summary. Invalid merged Kissat settings and malformed snapshots failed
without modifying the saved state. Original start timestamps were preserved.

Astra author and independent adversarial reviews accepted the implementation,
including the quiet execution path, and the subprocess harness. Evidence is
retained beside the repository in `UniTriSat-resume-validation-2026-10-21`.
No performance benchmark or new GPU hardware test was needed for this CLI-only
change. User computation files were not modified.

## Size-aware ETA and inline alignment (2026-10-21)

The ETA and worker-spacing changes passed 74 CLI tests with CUDA enabled,
strict all-target CLI Clippy, and a CUDA release executable build. Deterministic
tests cover ascending/descending/mixed sizes, wide real scaled-shape descriptors
with synthetic runtimes, unknown sizes, variable/censored observations, active
overruns, parallel stragglers and fresh calibration after resume. Quiet output
skips ETA bookkeeping. Worker tests preserve the original inline labels and
absent optional values while checking shared minimum widths, clipping and resize
behavior. There is no worker header row.

A real 120-column pseudo-terminal run completed 9,000 mixed small inputs,
producing 14 checked frames of four aligned worker rows with the blank separator
preserved. The first emitted frame can already have enough observations for a
numeric ETA; the subprocess harness therefore accepts every valid ETA state.
An explicit optimized bookkeeping microcheck passed separately; its loaded-host
timings and synthetic prediction examples are qualified in
[PERFORMANCE.md](PERFORMANCE.md#size-aware-eta-overhead-2026-10-21).

Astra author and independent adversarial reviews accepted the source, harness,
results and documentation. Evidence is retained beside the repository in
`UniTriSat-eta-validation-2026-10-21`. The user's concurrent computation was not
stopped or modified; no full solver performance benchmark was run for this change.

## Audit archive

The subsequent Kissat/PicoSAT/CaDiCaL CPU benchmark passed 29,808 measured calls
across ten inputs and both compatible solving modes. Returned models were
checked against the full CNF outside timing; no final solver timeouts or errors
occurred. An Astra author and independent Astra reviewer accepted the harness,
balanced measurements and decision to retain PicoSAT as default. See
[PERFORMANCE.md](PERFORMANCE.md#kissat-comparison-2026-10-21) for timings,
excluded pilot inputs and limitations. Raw benchmark evidence is retained in
`UniTriSat-kissat-benchmarks-2026-10-21` beside the repository.

Full migration plans, author/reviewer claims, source identities, Julia reference,
raw benchmark/differential outputs and unsuccessful experiments are retained in
an external maintainer archive named
`UniTriSat-lean-migration-evidence-2026-10-16` beside this repository. This bundle
is not a runtime dependency and is not shipped in a production crate. Its
`ARCHIVE.json` records preserved paths and SHA256 hashes; `README.md` explains
layout/reproduction. The preexisting October 14 migration archive is untouched.
Selected input files and frozen workers are preserved; restoring broad dataset
runs additionally requires the original read-only Polytopes dataset.

The checked-in numerical/CLI fixtures, property tests, native sanitizer harnesses
and Criterion benchmarks remain in the repository for ongoing regression checks.
See [DEVELOPMENT.md](DEVELOPMENT.md) for current commands. No commit, push or
change to another branch was performed by the migration agent.

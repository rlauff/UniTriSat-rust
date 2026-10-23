# Compatibility and exactness

## Arithmetic

The Rust CPU intersection kernel uses `i64`, widening to `i128` when required.
Generated dimension-specific coordinate bounds guarantee that all intermediates
fit the selected type. Inputs exceeding the `i128` bound are rejected with a
`CoordinateTooLarge` error. This is exact, bounded integer arithmetic, not
arbitrary precision. The former Julia CPU kernel likewise used bounded `Int64`.
The five performance comparison inputs fit Rust's `i64` lane; their slowdowns
are not an arbitrary-precision overhead.

CPU intersections support dimensions 2–10. GPU intersections use bounded exact
`i32`/`i64` arithmetic in dimensions 2–7. Unsupported precision/dimensions or CUDA
failures select the exact CPU path. GMP-backed arbitrary-size integers/rationals
are used where required in the lattice, determinant fallback and regularity
pipeline; that does not make every stage accept arbitrarily large inputs.
Touching simplices count as separated: only overlapping interiors intersect.

## Results, caps and ordering

Complete enumeration preserves exact requested target counts. With regularity
learning, visited-triangulation and flag totals can remain lower bounds even
after completion; the combined checker also retains its documented lower-bound
regular count. First solutions and
capped/timed-out subsets can differ with solver order or scheduling. A truncated
count is a lower bound; zero in an incomplete search does not prove absence.
Solution caps are global across split workers; the former Julia split runner
applied its cap per worker. Without flag filtering every encountered solution
is tested for regularity; a regularity limit is reported only when a required
test was actually skipped. Exact regularity witnesses are checked.

The file-based CLI has no dataset, chunk or table commands. Plain TSV rows
remain available with `--output-format=tsv`. Human output includes stage and
aggregate progress; incomplete searches are labeled separately from proven
negative results, which explicitly name the unimodular restriction when used.
With `--flag-sat`, generic triangulation and standalone regular counts are
omitted from human output and CLI TSV; TSV contains only flag counts and,
when requested, quadratic counts. Other CLI requests retain the canonical
count columns. Stage allocation sizes are not measured or fabricated.

Resume snapshots are passed as the positional input, with no `--resume` option.
Their contents identify them; a particular filename extension is not required.
Version 3 stores computation and output settings, parallelism, original start
time, accumulated active-session time and result-setting provenance. Versions 1
and 2 remain readable with a warning about unavailable historical timing.
Explicit CLI options override saved values; omitted options inherit them, and
the merged settings must still form a supported combination.

Ctrl+C saves completed rows and marks active polytopes to restart. Mode, solver
and intersection-backend changes retain completed results. Query-changing
overrides also retain them, with persistent warnings that results mix different
requests; enabling regular/flag filtering warns that this is almost never
intended. Completed results are not retroactively reclassified. Results whose
saved request differs are excluded from current classification totals, negative
proofs and observed target totals, and counted as `Other-request results`. The
observed target average still divides by all input polytopes. CLI TSV
marks those rows `historical`, with `?` count fields; original counts remain in
the snapshot. Selecting their original request makes those counts visible again.
Changing the input invalidates the checkpoint. A prepared input file can contain any selected
subset, regardless of filename or directory structure.

## Kissat compatibility

Kissat 4.0.4 is available through the existing `rustsat-kissat` binding for
first-solution searches without classification filters. Serial and split modes
are supported, as are independent parallel jobs and CPU/GPU intersections.
The static `--flag-sat` encoding is compatible with Kissat first-solution searches.
Incremental mode, full enumeration, regularity filtering and `--flag` postfiltering
are rejected before computation. Saved settings and direct Rust library calls
use the same compatibility checks. No solver is silently substituted.

The low-level adapter also rejects assumptions, repeated solves and clause
additions after a solve, including after interruption. This prevents unsupported
calls from reaching Kissat's native API. Windows builds retain the other
backends and explicitly reject selecting Kissat. Windows/macOS hardware
validation remains deferred. The bounded CPU solver comparison is recorded in
[PERFORMANCE.md](PERFORMANCE.md#kissat-comparison-2026-10-21).

## Julia interface

The optional package in `bindings/julia/` retains `triangulate`, aggregate/result fields, exact
returned simplex matrices, solver/mode options and first/all/none retention.
See [JULIA.md](JULIA.md) for usage and [BRIDGE.md](BRIDGE.md) for the protocol.

- `flag_SAT=true` is retired; use per-triangulation `flag_triangulation=true`.
- `use_normaliz=true` warns and uses Rust's exact lattice implementation.
- Input validation and exact projection always run. The historical unused
  `validate` option does not disable them.
- Incremental Julia calls use CaDiCaL, warning if PicoSAT was requested.
- `return_triangulations="none"` returns aggregate counts with no per-polytope
  retained results. Lower-bound results emit warnings because legacy result
  fields do not include a status field.
- Stage allocation bytes are unavailable (`-1`); timings are real measurements.
  Julia total time includes subprocess transport and optional plotting.

Plotting calls the preserved Python renderer. It displays the first unfiltered
triangulation, even when the regular/flag target filter rejects it, through an
additional Rust request when necessary. D3 displays directly; D4–6 boundary
views need Polyhedra/CDDLib. D2 plotting reports an explicit unsupported error.
Only display copies use floating point; returned geometry stays exact. See
[the plotting helper](../bindings/julia/python/README.md) for dependencies and precision limits.

## Platforms

Linux CPU and NVIDIA CUDA paths were executed. Windows/macOS hardware validation
remains deferred. The optional floating-point regularity optimization was not
part of the completed exact migration.

## Julia keyword and input map

| Legacy surface | Mapping / qualification |
|---|---|
| `triangulate(::Matrix{Int})` | One vertex per row; exact integer conversion and shape validation before Rust. |
| `triangulate(::Vector{Matrix{Int}})` | Ordered batch, one detailed result per input. Empty batch returns zeroed `RunResult`. |
| `triangulate(::String)` | Rust's existing point-record parser; fail explicitly on malformed input, consistent with the migration's strict parsing policy. |
| `triangulate(::Polyhedra.Polyhedron)` and vector | Optional Julia package extension extracting vertices; no geometry algorithm in the adapter. Reject unbounded/rational nonintegral input. |
| Oscar polyhedron and vector | Optional extension as in the old package; exact integral vertex extraction. |
| `unimodular=true` | `Restriction::Unimodular`; false selects nondegenerate simplices. |
| `regular=false`, `flag_triangulation=false` | Existing Rust classification hooks. Preserve the documented lower-bound meaning of regular counts with both filters enabled. |
| `find_all=false` | `Goal::First`; true is `Goal::All`. First/capped subsets can differ by solver order. |
| `return_triangulations="first"` | `Keep::First`, or `All`/`None`; retained indices resolve to simplex coordinate matrices. `none` also empties the final per-polytope result vector but preserves aggregate counters. |
| `solver="picosat"` | PicoSAT/CaDiCaL selection, plus Kissat for compatible first-solution searches. Kissat rejects incompatible options; legacy PicoSAT incremental mode switches to CaDiCaL and disables split mode. |
| `parallel_split_solving=true` | Split unless incremental selected; false gives serial. Preserve Julia defaults independently from CLI defaults. |
| `incremental_solving=false` | Existing incremental pipeline when true, including actual GPU streamed intersection when GPU requested and available. |
| `intersection_backend="cpu"` | Existing CPU/GPU selector and documented exact fallback. |
| `flag_SAT=false` | Reject true: the original Julia implementation already throws "flag_SAT is not implemented". Do not reinterpret as a postfilter. |
| `validate=false` | This is already an unused configuration field in the old public implementation; accepting it does not promise an extra validator. |
| `check_full_dimensionality=false` | Rust always performs exact projection; this accepted migration difference is documented in the adapter. |
| `use_normaliz=false` | Rust uses its exact cdd pipeline. Per explicit user decision, true emits a deprecation warning and uses that same Rust engine; it does not load Normaliz. |
| `plot=false` | True invokes the preserved Python renderer through lightweight Julia presentation glue. The user explicitly requires plotting; supported legacy views are D3 directly and D4–6 boundary views. Original D2 plotting was a no-op; the adapter reports it as unsupported. |
| `terminal_output="final"`, `log_file=""` | Julia presentation layer can print summaries and append logs; exact colors/timestamps/live display are not numerical compatibility. |
| `UNITRISAT_MAX_SOLUTIONS`, `UNITRISAT_MAX_REGULARITY_TESTS` | Preserve positive-integer environment cap behavior through existing Rust settings; do not substitute the CLI's finite default cap. |

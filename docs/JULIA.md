# Julia interface

The optional package in `bindings/julia/` preserves `triangulate` while Rust performs all numerical
computation. Build the bridge once before calling the package.

Build the bridge and instantiate the adapter environment:

```sh
cargo +1.97.0 build --release -p unitrisat-cli --bin unitrisat-julia
julia --project=bindings/julia -e 'using Pkg; Pkg.instantiate()'
```

Use the bridge crate's `cuda` feature for GPU execution. The adapter finds the
local release binary, or a `unitrisat-julia` executable on PATH. Set
`UNITRISAT_EXECUTABLE` to override that lookup. Importing the package never
builds code or downloads a runtime.

```julia
using UniTriSat

vertices = [0 0 0; 1 0 0; 0 1 0; 0 0 1]
result = triangulate(vertices; find_all=true, return_triangulations="all",
                     parallel_split_solving=false)
result.total_number_of_triangulations_found
result.triangulation_results[1].solution_simplices
```

A vector of integer matrices or an input filename can be used in place of one
matrix. Optional `using Polyhedra` or `using Oscar` enables the matching
polyhedron overloads; vertices must be integral and the polytope bounded.
The historical `unimodular`, `regular`, `flag_triangulation`, `find_all`,
`return_triangulations`, `solver`, `parallel_split_solving`,
`incremental_solving`, and `intersection_backend` options retain their roles.

`return_triangulations="none"` retains aggregate counts and returns an empty
per-polytope result vector. Like the old interface, `plot=true` displays the
first triangulation even when regularity or flag filters reject it. The adapter
performs a separate unfiltered first-result Rust call for this optional display;
its extra latency is included in the run time. Primary counts and retained
target solutions are unchanged. Plotting requirements and supported dimensions
are documented in the plotting helper; it invokes the preserved Python Plotly
renderer.

The renderer needs Python packages `numpy` and `plotly`; `UNITRISAT_PYTHON`
selects its interpreter (default `python3`). D3 tetrahedra display directly.
For D4–6 boundary views, load `Polyhedra` and `CDDLib` in Julia as well. D2
plotting was unsupported by the old implementation and now reports that
explicitly instead of silently doing nothing. Display projections do not modify
the exact returned simplex coordinates.

Rust always performs exact full-dimensional projection. `validate` was unused
by the old public implementation; input and arithmetic checks remain active.
`flag_SAT=true` is retired, and `use_normaliz=true` emits a deprecation warning
and uses Rust; the Rust lattice pipeline uses exact cdd arithmetic. Stage allocation
counts are unavailable (`alloc_bytes == -1`), not zero. Run time includes bridge
and plotting overhead; individual stage timings measure the Rust worker.

`UNITRISAT_MAX_SOLUTIONS` and `UNITRISAT_MAX_REGULARITY_TESTS` retain their
positive-integer cap meanings. As in the migrated Rust CLI, a cap is global
across split workers; the old Julia split implementation capped each worker.
Truncated results emit a warning and their counts are lower bounds. Complete
uncapped results preserve the mathematical meaning; first and capped subsets
can differ by solver order.

Validation commands:

```sh
UNITRISAT_ADAPTER_INTEGRATION=1 julia --project=bindings/julia -e 'using Pkg; Pkg.test()'
cargo +1.97.0 test -p unitrisat-cli
```

Set `UNITRISAT_ADAPTER_GPU=1` to include the CUDA-requested incremental geometry
comparison with a CUDA-enabled binary and working runtime. Optional input and
rendering tests are separate scripts under `bindings/julia/test/`; the headless rendering test
uses `PLOTLY_RENDERER=json` and additionally needs IPython/nbformat in Python.
These last two dependencies are for headless verification, not ordinary browser
plotting.

The [compatibility map](COMPATIBILITY.md) records every legacy keyword and the
[bridge contract](BRIDGE.md) defines exact integer transport.
Validation scope and retained evidence are described in [VALIDATION.md](VALIDATION.md).

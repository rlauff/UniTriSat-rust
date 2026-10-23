# Development and verification

The repository root is the Cargo workspace; `bindings/julia/` contains the
optional Julia subprocess frontend. Keep the Rust crates, native provenance/license files, exact
regression fixtures, property tests and Criterion benchmarks together.

Kissat uses the pinned `rustsat-kissat` crate and its bundled native source;
building it requires a C compiler and a libclang shared library for bindgen.
The selected `safe` and `quiet` features disable native shell-pipe support and
solver printing. The bundled version does not need a Git download during the
build. The Kissat dependency is excluded on Windows, where selecting it returns
an explicit unsupported-platform error; other solver backends remain available.
CaDiCaL and Kissat contain different implementations of the internal Kitten
solver. CaDiCaL's build script prefixes its Kitten exports and callers so the
two implementations cannot collide or replace each other at link time. When
upgrading either solver, verify the native archives' exported symbols with
`nm` and update `KITTEN_SYMBOLS` if needed. Vendored source bytes remain unchanged.

From the repository root:

```sh
cargo +1.97.0 fmt --all -- --check
cargo +1.97.0 clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo +1.97.0 test --workspace --all-features --locked
cargo +1.97.0 test --workspace --all-features --release --locked
RUSTDOCFLAGS='-D warnings' cargo +1.97.0 doc --workspace --no-deps --locked
```

From the repository root, run `cargo deny --all-features check` and `cargo machete` for
dependency policy. Add `--offline` to Cargo commands when dependencies are cached.
Normal test runs ignore hardware tests. With a working NVIDIA driver and NVRTC,
run the five actual hardware tests explicitly:

```sh
cargo +1.97.0 test --workspace --all-features --release --locked -- --ignored
```

GPU tests assert real CUDA execution; CPU fallback cannot make them pass.
See [GPU setup](../crates/unitrisat-gpu/README.md). Native race checks and
positive controls are in `crates/unitrisat-{cdd,cadical}/tsan/`; read each
runner before use. Sanitizer runtime support is required.

Build the Julia bridge first, then run frontend integration:

```sh
cargo +1.97.0 build --release -p unitrisat-cli --features cuda
UNITRISAT_ADAPTER_INTEGRATION=1 UNITRISAT_ADAPTER_GPU=1 julia --project=bindings/julia -e 'using Pkg; Pkg.test()'
python3 -m unittest discover -s bindings/julia/python -p test_render.py
```

Omit `UNITRISAT_ADAPTER_GPU=1` on CPU-only machines. Optional Julia input and
plotting checks are in `bindings/julia/test/optional_inputs.jl` and `bindings/julia/test/plot_process.jl`;
install their optional dependencies in a separate development environment.
Headless plotting uses `PLOTLY_RENDERER=json` and additionally needs Python
IPython/nbformat; normal browser plotting needs numpy/plotly.

## Benchmarks

```sh
RAYON_NUM_THREADS=1 cargo +1.97.0 bench -p unitrisat-core --bench stages -- --noplot
RAYON_NUM_THREADS=16 cargo +1.97.0 bench -p unitrisat-core --bench stages -- --noplot
```

Criterion uses local preserved fixtures and checks mathematical answers outside
timed regions. Run benchmarks on a quiet machine, without concurrent builds,
tests or other benchmarks. Record actual power/thread conditions, warmup and
full sample distributions. Historical fixture provenance paths identify archived
origins; benchmark execution does not require the old migration output tree.

Exact arithmetic, checked bounds, deterministic complete result sets and typed
errors are core contracts. New mathematical/FFI/concurrency/GPU changes need
focused independent review and relevant tests. Vendored sources must retain
hash/patch manifests and licenses. Do not discard tests or provenance as clutter.


The file-based executable now has no subcommands. Run `unitrisat --help` for
search, display and checkpoint options. Use `--output-format=tsv
--terminal-output=none` for machine-readable throughput measurements. Ctrl+C
checkpoint tests must run the executable in a subprocess: signal handlers are
process-global. Check both interruption during a job and normal/error shutdown
while a save is active; never hold standard-stream locks for the entire run.

Resume regression checks must cover content-based snapshot detection, strict
v1/v2/v3 decoding, explicit options versus inherited defaults, and validation
after merging settings. Verify cumulative elapsed time across repeated saves
and restarts without counting the offline gap. Completed rows and their query
provenance must survive both execution-only overrides and query changes; mixed
requests require warnings even with quiet output. Exercise real Ctrl+C in both
sequential and parallel subprocesses, including a second interruption after
resuming. Use temporary inputs and snapshots, never a user's computation files.

ETA tests use deterministic synthetic runtimes, including ascending/descending
real size descriptors, rather than unstable wall-clock accuracy thresholds.
The explicit `eta::tests::bookkeeping_overhead_100k_inputs` ignored test measures
only descriptor/update/refresh costs; run it in release mode with `--ignored
--nocapture --test-threads=1`. Keep observed overhead separate from end-to-end
solver performance. Worker formatting tests must preserve the original inline
labels and omit absent optional fields; alignment changes must not add a header.

[Queued work](PLANNED_WORK.md) records the requested solver comparison and
scaled-cube incremental tuning, with two strong reviews required for each code
change and final default.

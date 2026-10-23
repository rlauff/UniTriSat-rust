# UniTriSat

Exact triangulation searches for integer lattice polytopes, with optional
regularity and flagness checks. Use the command-line tool on a text file or
call `unitrisat::triangulate` from Rust.

## Build

Install Rust through rustup, plus a C/C++ compiler, `make`, and libclang
(on Debian/Ubuntu: `sudo apt install build-essential libclang-dev`). The
checkout selects Rust 1.97.0 automatically through rustup.

```sh
git clone https://github.com/rlauff/UniTriSat-rust.git
cd UniTriSat-rust
cargo build --release --locked -p unitrisat-cli --bin unitrisat
./target/release/unitrisat --help
```

The optional dataset submodule is not needed to build or run your own inputs.
Linux is tested; Windows/macOS hardware validation is pending.

## Input and first run

Create `polytopes.txt` with one vertex per line and blank lines between
polytopes. Coordinates are signed 64-bit integers:

```text
0 0
1 0
0 1
1 1

0 0 0
1 0 0
0 1 0
0 0 1
```

Alternatively, write one polytope per line as `[[0,0],[1,0],[0,1],[1,1]]`.
Lines starting with `#` are comments. Prepare subsets or chunks as separate files.

```sh
./target/release/unitrisat polytopes.txt
./target/release/unitrisat polytopes.txt --regular --flag-sat --parallel=4
```

The default searches for the first unimodular triangulation of each polytope,
using CPU intersections, PicoSAT and serial solving. Live output shows worker
progress, counts, elapsed time and an estimated time remaining.

## Main options

| Option | Meaning |
| --- | --- |
| `--all` | Enumerate all triangulations of the requested type. |
| `--regular` | Require regularity, checked with exact linear programming. |
| `--flag` | Check each candidate and reject non-flag triangulations. |
| `--flag-sat` | Encode flagness directly in SAT. With `--regular`, search for quadratic (regular and flag) triangulations. |
| `--regular-learn` | With `--regular`, learn clauses from nonregular candidates; optional and not always faster. |
| `--mode=serial\|split\|incremental` | Solve the full formula, split one polytope's search, or add intersection clauses while solving. |
| `--solver=picosat\|cadical\|kissat` | Choose the SAT solver. |
| `--parallel[=N]` | Process multiple polytopes concurrently; bare flag uses four workers. |
| `--backend=cpu\|gpu` | Choose intersection computation; GPU requires the CUDA build below. |
| `--simplices=nondegenerate` | Allow any full-dimensional simplex instead of only unimodular ones. |
| `--max-solutions N`, `--timeout SECONDS` | Limit work per polytope; incomplete counts are lower bounds. |
| `--output-format=tsv --terminal-output=none` | Print machine-readable result rows without progress reports. |

Kissat supports first-solution serial/split searches, including `--flag-sat`,
but rejects `--all`, `--mode=incremental`, `--regular`,
and `--flag`. Combining `--flag` and `--flag-sat` runs both checks and warns
that the extra check is redundant.

With `--flag-sat`, results show flag counts and, when requested, quadratic
counts; they do not establish general triangulability counts. With
`--regular-learn`, encountered triangulation/flag counts can be lower bounds
even after completion. See `--help` for all options and limits.

## Interrupt and resume

Ctrl+C saves completed results, settings and elapsed time to
`polytopes.txt.resume.json` (choose another path with `--checkpoint FILE`).
Pass that snapshot directly to resume:

```sh
./target/release/unitrisat polytopes.txt.resume.json
./target/release/unitrisat polytopes.txt.resume.json --mode=incremental --solver=cadical
```

Keep the original input file available. Completed polytopes are retained;
interrupted polytopes restart. Omitted flags inherit saved settings, explicit
flags override them; use e.g. `--regular=false` to disable a saved option.
Changing the requested properties keeps previous results but warns about mixed
requests; those earlier results are excluded from the new request's counts.

## Optional GPU and library use

```sh
cargo build --release --locked -p unitrisat-cli --bin unitrisat --features cuda
./target/release/unitrisat polytopes.txt --backend=gpu
```

GPU execution needs an NVIDIA driver and loadable NVRTC runtime; unavailable
CUDA or unsupported inputs fall back to exact CPU computation.
See [GPU setup](crates/unitrisat-gpu/README.md).

For Rust callers, add `unitrisat = { path = "/path/to/UniTriSat-rust/crates/unitrisat" }`
to `[dependencies]` and call `unitrisat::triangulate(&vertices, &settings)`.
Generate API documentation with `cargo doc -p unitrisat --no-deps --open`.

Licensed under [GPL-3.0-only](LICENSE).

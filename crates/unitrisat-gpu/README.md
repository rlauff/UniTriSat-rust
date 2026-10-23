# Exact optional CUDA intersections

`unitrisat-gpu` uses cudarc 0.19.10 and runtime NVRTC compilation. Enable the
`cuda` feature to use CUDA; the default build uses the existing CPU backend.
Neither build needs CUDA headers, `nvcc`, or a toolkit at build time. At runtime,
the driver and NVRTC shared libraries must be discoverable by the platform
loader (for example through `LD_LIBRARY_PATH` on Linux).

`intersecting_pairs` returns sorted exact pairs. `search` additionally reports
`Backend::Cuda(lane)` or `Backend::Cpu(reason)`. `stream_pairs` has the CPU
streaming signature and supports incremental solving: completed tile batches
are delivered while the remaining intersections are still being computed.
Batches can contain several source rows; they are not globally ordered.

Dimensions 2–7 use generated complete separating-axis candidate sets; other
supported CPU dimensions fall back automatically. Runtime failures also fall
back. Collection discards partial device results before retrying. Streaming
retains a completed-tile watermark and excludes those tiles from the CPU retry.
A sink returning false ends the search. No device failure is treated as an
empty or successfully completed pair search.

## Integer safety

For coordinate column `c`, let `w[c]` be its spread and `m[c]` its maximum
absolute coordinate. A determinant of order `k` on columns `S` and **every
partial sum** of its cofactor expansion is bounded by
`k! * product(w[c], c in S)`. The generator uses subset-DP expansions of all
orders up to `D-1`; the host checks every one of those subset bounds, not only
the final normal. This accounts for zero-width columns, where a high-order
minor may be zero while a lower-order intermediate is large.

Normal component `c` omits column `c`. The sum of `m[c]` times these normal
bounds controls every partial projection sum. The host additionally bounds all
stored coordinates and difference-vector entries. Checks use checked `u128`;
an overflowing check rejects CUDA. The narrowest signed i32/i64 lane containing
all bounds is selected. Kernel arithmetic therefore cannot overflow, including
negation. There are no floating-point decisions. These conservative bounds can
select a wider lane than Julia's Hadamard estimates. They can also accept some
translated/anisotropic inputs beyond the CPU's isotropic coordinate bound; such
inputs remain exact on CUDA, but CPU fallback may return its coordinate error.

## Launch safety

Each block loads two 16-simplex tiles into shared memory, with all threads
reaching the barrier before any pair checks. Integer triangular tile decoding
avoids floating-point indexing. The host validates every simplex label and
buffer-size calculation. The kernel guards every vertex read, and each block
writes exactly 256 independent flag bytes. Batches have at most 4096 blocks
(1 MiB output), with no unbounded device output allocation. The full coordinate
buffer is uploaded once per search. Compiled modules and the CUDA context are
cached; streams and input/output buffers are per search.

The launch is explicitly synchronized before copying. Copies target initialized
host storage; the stream is synchronized and cudarc's deferred error state
checked before flags are consumed. Device work cannot become authoritative
pairs until those operations succeed.

## Verification

Run ordinary CPU/fallback tests with `cargo test -p unitrisat-gpu --release`.
Build hardware tests with `cargo test -p unitrisat-gpu --release --features cuda
--no-run`, then run that test executable with `--ignored --test-threads=1` on a
CUDA host. Hardware tests require actual CUDA execution, check every generated
variant compiles, compare D2–7 and i64 outputs with exact CPU results, exercise
post-delivery failure/cancellation, and verify incremental enumeration using
both SAT engines through an asserted CUDA source.

Recorded CPU/GPU measurements and their scope are in
[PERFORMANCE.md](../../docs/PERFORMANCE.md). Historical benchmark workers and
raw evidence are retained in the separate audit archive. Run any new performance
measurements without concurrent builds or tests and record actual power settings.

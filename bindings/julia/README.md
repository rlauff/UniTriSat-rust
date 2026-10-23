# Julia bindings for UniTriSat

This optional compatibility package calls the Rust implementation through
`unitrisat-julia`. It contains no Julia numerical solver. The Cargo workspace
at the repository root builds and runs independently of Julia.

From the repository root:

```sh
cargo build --release -p unitrisat-cli --bin unitrisat-julia
julia --project=bindings/julia -e 'using Pkg; Pkg.instantiate(); using UniTriSat'
```

To add a local checkout to another Julia environment:

```julia
using Pkg
Pkg.develop(path="/path/to/UniTriSat/bindings/julia")
```

See [Julia usage](../../docs/JULIA.md), [compatibility](../../docs/COMPATIBILITY.md),
[bridge protocol](../../docs/BRIDGE.md) and [plotting](python/README.md).

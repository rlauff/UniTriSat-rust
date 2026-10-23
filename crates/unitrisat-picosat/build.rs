//! Compiles the vendored PicoSAT 965 (see `vendor/SOURCE_MANIFEST.json`).

fn main() {
    for path in ["vendor/picosat.c", "vendor/picosat.h"] {
        println!("cargo::rerun-if-changed={path}");
    }
    let result = cc::Build::new()
        .file("vendor/picosat.c")
        .include("vendor")
        // No getrusage-based statistics.
        .define("NGETRUSAGE", None)
        // Internal asserts off, as in Julia's PicoSAT build (1.35-1.56x
        // faster enumeration). The API precondition checks (ABORTIF) do not
        // depend on NDEBUG and stay on.
        .define("NDEBUG", None)
        .warnings(false)
        .try_compile("unitrisat_picosat");
    if let Err(error) = result {
        eprintln!("failed to compile the vendored PicoSAT: {error}");
        std::process::exit(1);
    }
}

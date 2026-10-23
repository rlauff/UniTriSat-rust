//! Compiles the vendored, patched CaDiCaL 2.2.1 (`vendor/PROVENANCE.json`,
//! `vendor/SHA256SUMS`) and the project shim.

/// CaDiCaL's C++ units (upstream `src/`, without the `cadical`, `mobical`,
/// `ccadical` and `ipasir` front ends).
const CPP_UNITS: [&str; 89] = [
    "analyze",
    "arena",
    "assume",
    "averages",
    "backbone",
    "backtrack",
    "backward",
    "bins",
    "block",
    "checker",
    "clause",
    "collect",
    "compact",
    "condition",
    "config",
    "congruence",
    "constrain",
    "contract",
    "cover",
    "decide",
    "decompose",
    "deduplicate",
    "definition",
    "drattracer",
    "elim",
    "elimfast",
    "ema",
    "extend",
    "external",
    "external_propagate",
    "factor",
    "file",
    "flags",
    "flip",
    "format",
    "frattracer",
    "gates",
    "idruptracer",
    "instantiate",
    "internal",
    "lidruptracer",
    "limit",
    "logging",
    "lookahead",
    "lratchecker",
    "lrattracer",
    "lucky",
    "message",
    "minimize",
    "occs",
    "options",
    "parse",
    "phases",
    "probe",
    "profile",
    "proof",
    "propagate",
    "queue",
    "random",
    "reap",
    "reduce",
    "rephase",
    "report",
    "resources",
    "restart",
    "restore",
    "score",
    "shrink",
    "signal",
    "solution",
    "solver",
    "stable",
    "stats",
    "subsume",
    "sweep",
    "terminal",
    "ternary",
    "tier",
    "transred",
    "unstable",
    "util",
    "var",
    "veripbtracer",
    "version",
    "vivify",
    "walk",
    "walk_full_occs",
    "warmup",
    "watch",
];

/// Defines for both compilers: no generated build header, no tracing, no
/// contrib or IPASIR code, and none of CaDiCaL's environment reads
/// (`UNITRISAT_NO_ENVIRONMENT`, from the vendored patch).
const DEFINES: [&str; 9] = [
    // Internal asserts off, as in upstream's default release build and
    // Julia's libcadical (-O3 -DNDEBUG). The API contracts (REQUIRE) depend
    // on NCONTRACTS, not NDEBUG, and stay on.
    "NDEBUG",
    "NBUILD",
    "NTRACING",
    "NCONTRIB",
    "NIPASIR",
    "NFLEXIBLE",
    "NUNLOCKED",
    "NCLOSEFROM",
    "UNITRISAT_NO_ENVIRONMENT",
];

/// Kitten exports C symbols that Kissat also provides, with incompatible
/// implementations. Namespace all exports in both the C definitions and C++
/// callers so either solver can be linked without interposing on the other.
/// This includes the two helper exports absent from upstream's kitten.h.
/// On a CaDiCaL upgrade, refresh this list from all Kitten archive exports
/// (`nm -g --defined-only`) and check it against the bundled Kissat archive.
const KITTEN_SYMBOLS: &[&str] = &[
    "citten_clause_with_id",
    "citten_clause_with_id_and_equivalence",
    "citten_clause_with_id_and_exception",
    "completely_backtrack_to_root_level",
    "kitten_add_prime_implicant",
    "kitten_assume",
    "kitten_assume_signed",
    "kitten_binary",
    "kitten_clause",
    "kitten_clause_with_id_and_exception",
    "kitten_clear",
    "kitten_compute_clausal_core",
    "kitten_compute_prime_implicant",
    "kitten_current_ticks",
    "kitten_failed",
    "kitten_fixed",
    "kitten_fixed_signed",
    "kitten_flip_and_implicant_for_signed_literal",
    "kitten_flip_literal",
    "kitten_flip_phases",
    "kitten_flip_signed_literal",
    "kitten_init",
    "kitten_no_terminator",
    "kitten_no_ticks_limit",
    "kitten_randomize_phases",
    "kitten_release",
    "kitten_set_logging",
    "kitten_set_terminator",
    "kitten_set_ticks_limit",
    "kitten_shrink_to_clausal_core",
    "kitten_shuffle_clauses",
    "kitten_signed_value",
    "kitten_solve",
    "kitten_status",
    "kitten_trace_core",
    "kitten_track_antecedents",
    "kitten_traverse_core_clauses",
    "kitten_traverse_core_clauses_with_id",
    "kitten_traverse_core_ids",
    "kitten_unit",
    "kitten_value",
    "new_learned_klause",
];

fn namespace_kitten(build: &mut cc::Build) {
    for &symbol in KITTEN_SYMBOLS {
        build.define(symbol, Some(format!("unitrisat_cadical_{symbol}").as_str()));
    }
}

fn fail(what: &str, error: &cc::Error) -> ! {
    eprintln!("failed to compile {what}: {error}");
    std::process::exit(1);
}

fn main() {
    println!("cargo::rerun-if-changed=vendor/cadical");
    println!("cargo::rerun-if-changed=shim/ucad.cpp");
    // The C++ archive comes first: it calls into kitten (C).
    let mut cpp = cc::Build::new();
    cpp.cpp(true)
        .std("c++17")
        .include("vendor/cadical/src")
        .warnings(false);
    for define in DEFINES {
        cpp.define(define, None);
    }
    namespace_kitten(&mut cpp);
    for unit in CPP_UNITS {
        cpp.file(format!("vendor/cadical/src/{unit}.cpp"));
    }
    cpp.file("shim/ucad.cpp");
    if let Err(error) = cpp.try_compile("unitrisat_cadical") {
        fail("CaDiCaL", &error);
    }
    let mut c = cc::Build::new();
    c.std("c11")
        .include("vendor/cadical/src")
        .file("vendor/cadical/src/kitten.c")
        .warnings(false);
    for define in DEFINES {
        c.define(define, None);
    }
    namespace_kitten(&mut c);
    if let Err(error) = c.try_compile("unitrisat_kitten") {
        fail("CaDiCaL's kitten", &error);
    }
}

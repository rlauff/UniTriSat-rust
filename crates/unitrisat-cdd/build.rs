//! Compiles the vendored cddlib 0.94n (GMP rational build plus the generated
//! floating-point units it calls) and the project shim, against the GMP that
//! `gmp-mpfr-sys` built. See `vendor/PROVENANCE.json` and `vendor/SHA256SUMS`.

use std::env;

const CDD_UNITS: [&str; 7] = [
    "cddcore", "cddlp", "cddmp", "cddio", "cddlib", "cddproj", "setoper",
];
const FLOAT_UNITS: [&str; 6] = [
    "cddcore_f",
    "cddlp_f",
    "cddmp_f",
    "cddio_f",
    "cddlib_f",
    "cddproj_f",
];

fn main() {
    println!("cargo::rerun-if-changed=vendor/cddlib");
    println!("cargo::rerun-if-changed=shim/ucdd.c");
    // Set by gmp-mpfr-sys (links = "gmp") for crates that compile C against it.
    let Some(gmp_include) = env::var_os("DEP_GMP_INCLUDE_DIR") else {
        eprintln!("DEP_GMP_INCLUDE_DIR is not set; gmp-mpfr-sys must be a dependency");
        std::process::exit(1);
    };
    let mut build = cc::Build::new();
    build
        .std("c11")
        .include(&gmp_include)
        .include("vendor/cddlib/lib-src")
        .include("vendor/cddlib/generated")
        .define("GMPRATIONAL", None)
        .warnings(false);
    for unit in CDD_UNITS {
        build.file(format!("vendor/cddlib/lib-src/{unit}.c"));
    }
    for unit in FLOAT_UNITS {
        build.file(format!("vendor/cddlib/generated/{unit}.c"));
    }
    build.file("shim/ucdd.c");
    if let Err(error) = build.try_compile("unitrisat_cdd") {
        eprintln!("failed to compile the vendored cddlib: {error}");
        std::process::exit(1);
    }
    if env::var("CARGO_CFG_TARGET_FAMILY").is_ok_and(|family| family == "unix") {
        println!("cargo::rustc-link-lib=m");
    }
}

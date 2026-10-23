//! Exact unimodular triangulation search for lattice polytopes.
//!
//! Exact numerical primitives used by the public `unitrisat` library.
//! Usage and guarantees are documented in `docs/` at the repository root.

#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
        clippy::unreachable,
        clippy::disallowed_macros,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
        clippy::panic_in_result_fn,
        clippy::assertions_on_constants
    )
)]
#![cfg_attr(
    test,
    allow(
        clippy::disallowed_macros,
        reason = "tests may assert; production code may not"
    )
)]
#![deny(unused_crate_dependencies)]

#[cfg(test)]
use criterion as _;

pub mod cnf;
pub mod enumerate;
pub mod families;
pub mod flag;
pub mod hull;
pub mod input;
pub mod intersect;
pub mod io;
pub mod lattice;
pub mod matrix;
pub mod output;
pub mod quadratic;
pub mod regular;
pub mod solve;

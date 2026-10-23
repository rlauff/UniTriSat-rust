//! File-based triangulation execution, terminal output and resumable runs.
//! The executable is a thin entry point over [`cli`].

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

pub mod cli;
pub mod display;
pub use unitrisat::job;
mod eta;
pub mod julia_bridge;
mod parallel;

pub mod resume;

//! Exact triangulation search for lattice polytopes.
//!
//! Supply vertex rows as a [`Matrix`] and select a [`Settings`] configuration.
//! The default searches for one unimodular triangulation on the CPU. Set
//! `goal` to [`enumerate::Goal::All`] for exhaustive enumeration; returned
//! counts and [`Detailed::outcome`] distinguish completed, capped and early
//! searches. Retained triangulations index `Detailed::simplices`, whose point
//! indices refer to `Detailed::points` in effective full-dimensional coordinates.
//!
//! ```
//! use unitrisat::{triangulate, Matrix, Settings};
//! let vertices = Matrix::new(2, vec![0, 0, 1, 0, 0, 1, 1, 1])?;
//! let result = triangulate(&vertices, &Settings::default())?;
//! assert_eq!(result.triangulations.len(), 1);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Enable the `cuda` feature and set `intersection_backend` to
//! [`IntersectionBackend::Gpu`] for GPU intersection in every solving mode,
//! including incremental search. Unsupported or unavailable GPU execution
//! falls back to the exact CPU implementation. Low-level numerical modules
//! remain available for applications that need individual pipeline stages.

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

pub use matrix::Matrix;
pub use unitrisat_core::*;
pub mod job;
pub use job::{Detailed, IntersectionBackend, JobError, Mode, Settings};

/// Searches an arbitrary vertex matrix with the requested settings, retaining
/// every target encountered before the selected stopping condition.
/// For count-only work, use [`job::run`] to avoid retaining triangulations.
pub fn triangulate(vertices: &Matrix, settings: &Settings) -> Result<Detailed, JobError> {
    job::run_detailed(1, vertices, settings, enumerate::Keep::All)
}

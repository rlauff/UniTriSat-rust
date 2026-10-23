//! Optional exact CUDA intersection with automatic CPU fallback.
//!
//! Enable `cuda` to load CUDA and NVRTC at runtime. Any unavailable device,
//! unsupported dimension/precision, compilation, transfer or launch failure
//! re-runs the complete job on the exact CPU backend; no partial GPU result
//! escapes. CPU-only builds do not depend on cudarc or a CUDA installation.

#![cfg_attr(not(feature = "cuda"), forbid(unsafe_code))]
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
#![cfg_attr(test, allow(clippy::disallowed_macros))]
#![deny(unused_crate_dependencies)]

#[cfg(feature = "cuda")]
mod cuda;
mod precision;
#[cfg(any(feature = "cuda", test))]
mod source;

pub use precision::{Lane, precision};
use unitrisat::intersect::{self, IntersectError};

/// The implementation that produced a complete pair set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    /// CUDA completed the entire job using this exact integer lane.
    Cuda(Lane),
    /// CPU fallback, with its diagnostic reason.
    Cpu(String),
}

/// Exact pairs and the backend that produced them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
    /// Lexicographically sorted intersecting simplex pairs, zero based.
    pub pairs: Vec<(u32, u32)>,
    /// Execution backend (including the reason for CPU fallback).
    pub backend: Backend,
}

/// Compute exact intersecting pairs, using CUDA if available and safe.
pub fn intersecting_pairs(
    points: &[i64],
    dimension: usize,
    simplices: &[u32],
) -> Result<Vec<(u32, u32)>, IntersectError> {
    search(points, dimension, simplices).map(|result| result.pairs)
}

/// Like [`intersecting_pairs`], additionally reporting CPU fallback reasons.
///
/// Malformed input is always reported by the CPU backend, whose validation
/// defines the shared API. GPU failures never masquerade as empty pair sets.
pub fn search(
    points: &[i64],
    dimension: usize,
    simplices: &[u32],
) -> Result<Search, IntersectError> {
    #[cfg(feature = "cuda")]
    let reason = match cuda::search(points, dimension, simplices) {
        Ok(result) => return Ok(result),
        Err(reason) => reason,
    };
    #[cfg(not(feature = "cuda"))]
    let reason = "CUDA feature disabled".to_owned();
    Ok(Search {
        pairs: intersect::intersecting_pairs(points, dimension, simplices)?,
        backend: Backend::Cpu(reason),
    })
}

/// Stream exact GPU tile batches to the same sink accepted by the CPU path.
///
/// Batches contain pairs from several rows and are not globally ordered.
/// A `false` sink return stops the search. If CUDA fails after some batches
/// succeeded, the CPU computes only the as-yet-undelivered tiles, so no pair
/// is delivered twice. `workers` controls CPU fallback concurrency only.
pub fn stream_pairs(
    points: &[i64],
    dimension: usize,
    simplices: &[u32],
    workers: usize,
    sink: intersect::RowSink<'_>,
) -> Result<(), IntersectError> {
    #[cfg(feature = "cuda")]
    {
        let mut completed = 0;
        if cuda::run(points, dimension, simplices, &mut completed, &mut |batch| {
            sink(batch)
        })
        .is_ok()
        {
            return Ok(());
        }
        if completed > 0 {
            return intersect::stream_pairs(points, dimension, simplices, workers, &|row| {
                let remaining: Vec<_> = row
                    .iter()
                    .copied()
                    .filter(|&(a, b)| cuda::tile_of(a, b).is_none_or(|tile| tile >= completed))
                    .collect();
                remaining.is_empty() || sink(&remaining)
            });
        }
    }
    intersect::stream_pairs(points, dimension, simplices, workers, sink)
}

#[cfg(test)]
mod tests;

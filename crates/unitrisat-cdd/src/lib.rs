//! Safe FFI over the vendored cddlib 0.94n, built with exact GMP rationals.
//!
//! Two operations are exposed, the two that UniTriSat uses (Julia reaches
//! them through `CDDLib.Library(:exact)`):
//!
//! * [`hull_inequalities`]: the H-representation of the convex hull of
//!   integer points;
//! * [`solve_lp`]: an exact linear program.
//!
//! Rows use cddlib's convention: `[b, a_1, ..., a_d]` stands for
//! `b + a_1 x_1 + ... + a_d x_d >= 0` (or `= 0` for equations).
//!
//! Both functions may be called from many threads at once; they take no lock.
//! The first call initializes cddlib's global constants exactly once. Our
//! patch `vendor/patches/0001-thread-local-pivot-counters.patch` makes the
//! last shared objects cddlib writes thread-local (see `src/ffi.rs`).
//! cddlib never frees its small per-thread scratch buffers, so each thread
//! that calls it leaks them when it exits: call it from a fixed pool of
//! threads (rayon), not from a fresh thread per call.

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

use std::ffi::c_long;

/// The exact rational type of every row and LP value (re-exported so that
/// callers name the same `rug` version).
pub use rug::Rational;

#[allow(unsafe_code, reason = "the only module that calls into C")]
mod ffi;

use ffi::{Matrix, Representation};

/// Failure of a cddlib call.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CddError {
    /// The input has no rows, rows of different lengths, or the wrong shape.
    #[error("malformed input: {0}")]
    Shape(&'static str),
    /// The input is too large for cddlib's `long` indices.
    #[error("the input is too large for cddlib")]
    TooLarge,
    /// cddlib could not allocate memory.
    #[error("cddlib could not allocate memory")]
    Allocation,
    /// cddlib reported an error (its `dd_ErrorType` code).
    #[error("cddlib reported error code {0}")]
    Cdd(i32),
    /// cddlib returned no result but reported no error.
    #[error("cddlib returned no result without reporting an error")]
    NoResult,
    /// cddlib returned an LP status this crate does not know.
    #[error("cddlib returned the unexpected LP status {0}")]
    UnexpectedStatus(i32),
}

fn long(value: usize) -> Result<c_long, CddError> {
    c_long::try_from(value).map_err(|_| CddError::TooLarge)
}

/// Copies `rows` (each of length `cols`) into `matrix` from row `start`.
fn fill(
    matrix: &mut Matrix<'_>,
    start: c_long,
    rows: &[Vec<Rational>],
    cols: c_long,
) -> Result<(), CddError> {
    for (index, row) in (start..).zip(rows) {
        if long(row.len())? != cols {
            return Err(CddError::Shape("rows of different lengths"));
        }
        for (col, value) in (0..).zip(row) {
            if !matrix.set(index, col, value) {
                return Err(CddError::Shape("row index out of range"));
            }
        }
    }
    Ok(())
}

/// Reads every row of `matrix`, split into inequalities and equations.
fn read_rows(matrix: &Matrix<'_>) -> Result<HRepresentation, CddError> {
    let (mut inequalities, mut equations) = (Vec::new(), Vec::new());
    for row in 0..matrix.rows() {
        let values = (0..matrix.cols())
            .map(|col| {
                matrix
                    .get(row, col)
                    .ok_or(CddError::Shape("cddlib matrix shape"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if matrix.is_linearity(row) {
            equations.push(values);
        } else {
            inequalities.push(values);
        }
    }
    Ok(HRepresentation {
        inequalities,
        equations,
    })
}

/// An H-representation `[b, a]`: `b + a·x >= 0` for each inequality and
/// `b + a·x = 0` for each equation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HRepresentation {
    /// Inequality rows.
    pub inequalities: Vec<Vec<Rational>>,
    /// Equation rows; empty for a full-dimensional hull.
    pub equations: Vec<Vec<Rational>>,
}

/// The H-representation of the convex hull of `points`, a row-major matrix
/// with `dimension` columns, as cddlib's double description method computes
/// it from the generators `[1, point]`.
///
/// The rows are cddlib's output, unfiltered and not normalized (cddlib may
/// scale them). Lower-dimensional hulls have equations. cddlib may also
/// return the trivial inequality `[c, 0, ..., 0]` with `c > 0` (`c >= 0`),
/// which is not a facet, so callers that want facets must drop it. In a
/// sweep of 20,000 random inputs in dimensions 1 to 4 (task P1.9) it
/// appeared exactly when the hull is a single point: then the result is
/// `dimension` equations plus that one row.
pub fn hull_inequalities(points: &[i64], dimension: usize) -> Result<HRepresentation, CddError> {
    if dimension == 0 || points.is_empty() || points.len().checked_rem(dimension) != Some(0) {
        return Err(CddError::Shape("points must form a nonempty matrix"));
    }
    let rows = long(points.len().checked_div(dimension).unwrap_or(0))?;
    let cols = long(dimension.checked_add(1).ok_or(CddError::TooLarge)?)?;
    let ready = ffi::ready();
    let mut generators =
        Matrix::new(&ready, rows, cols, Representation::Generators).ok_or(CddError::Allocation)?;
    let one = Rational::from(1);
    for (row, point) in (0..rows).zip(points.chunks_exact(dimension)) {
        let mut ok = generators.set(row, 0, &one);
        for (col, &x) in (1..).zip(point) {
            ok &= generators.set(row, col, &Rational::from(x));
        }
        if !ok {
            return Err(CddError::Shape("generator index out of range"));
        }
    }
    let inequalities = generators.inequalities()?;
    read_rows(&inequalities)
}

/// Whether to maximize or minimize the objective.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sense {
    /// Maximize.
    Maximize,
    /// Minimize.
    Minimize,
}

/// An LP over `x` in `R^d`: optimize `c_0 + c·x` subject to `b + a·x >= 0`
/// for every inequality row and `b + a·x = 0` for every equation row.
#[derive(Clone, Copy, Debug)]
pub struct Lp<'a> {
    /// Inequality rows `[b, a]`, each of length `d + 1`.
    pub inequalities: &'a [Vec<Rational>],
    /// Equation rows `[b, a]`, each of length `d + 1`.
    pub equations: &'a [Vec<Rational>],
    /// Objective `[c_0, c]`, of length `d + 1`.
    pub objective: &'a [Rational],
    /// Direction.
    pub sense: Sense,
}

/// cddlib's LP status (`dd_LPStatusType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LpStatus {
    /// An optimal solution was found.
    Optimal,
    /// The constraints are infeasible.
    Inconsistent,
    /// The dual is infeasible.
    DualInconsistent,
    /// The constraints are infeasible for every right-hand side.
    StructurallyInconsistent,
    /// The dual is infeasible for every objective.
    StructurallyDualInconsistent,
    /// The objective is unbounded.
    Unbounded,
    /// The dual objective is unbounded.
    DualUnbounded,
}

/// The result of [`solve_lp`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LpSolution {
    /// The status.
    pub status: LpStatus,
    /// The optimal value (meaningful when `status` is `Optimal`).
    pub value: Rational,
    /// The primal solution `x` (meaningful when `status` is `Optimal`).
    pub point: Vec<Rational>,
}

/// Solves `lp` exactly with cddlib's dual simplex.
pub fn solve_lp(lp: &Lp<'_>) -> Result<LpSolution, CddError> {
    let cols = long(lp.objective.len())?;
    if cols < 1 {
        return Err(CddError::Shape(
            "the objective needs at least the constant term",
        ));
    }
    let inequalities = long(lp.inequalities.len())?;
    let rows = inequalities
        .checked_add(long(lp.equations.len())?)
        .ok_or(CddError::TooLarge)?;
    let ready = ffi::ready();
    let mut matrix = Matrix::new(&ready, rows, cols, Representation::Inequalities)
        .ok_or(CddError::Allocation)?;
    fill(&mut matrix, 0, lp.inequalities, cols)?;
    fill(&mut matrix, inequalities, lp.equations, cols)?;
    for row in inequalities..rows {
        if !matrix.set_linearity(row) {
            return Err(CddError::Shape("equation index out of range"));
        }
    }
    let sense = match lp.sense {
        Sense::Maximize => 1,
        Sense::Minimize => 2,
    };
    if !matrix.set_objective(sense, lp.objective) {
        return Err(CddError::Shape("objective length"));
    }
    if matrix.cols() != cols {
        return Err(CddError::Shape("cddlib matrix shape"));
    }
    let solved = matrix.solve_lp()?;
    let status = match solved.status() {
        1 => LpStatus::Optimal,
        2 => LpStatus::Inconsistent,
        3 => LpStatus::DualInconsistent,
        4 => LpStatus::StructurallyInconsistent,
        5 => LpStatus::StructurallyDualInconsistent,
        6 => LpStatus::Unbounded,
        7 => LpStatus::DualUnbounded,
        other => return Err(CddError::UnexpectedStatus(other)),
    };
    let mut point = solved.solution();
    // Drop the homogenizing coordinate.
    if !point.is_empty() {
        point.remove(0);
    }
    Ok(LpSolution {
        status,
        value: solved.value(),
        point,
    })
}

#[cfg(test)]
mod tests;

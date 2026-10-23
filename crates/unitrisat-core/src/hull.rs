//! Facets of the input polytope.
//!
//! [`facets`] is the Rust counterpart of Julia's
//! `halfspaces(hrep(polyhedron(vrep(V), CDDLib.Library(:exact))))`, which
//! `lattice_points_via_CDDLib` and `internal_faces` use. It runs cddlib's exact
//! double description method through `unitrisat-cdd` and returns each facet as
//! an integer inequality with coprime coefficients.
//!
//! The vertices must be full-dimensional: pass the output of
//! [`crate::lattice::project_to_full_dimension`] when they are not. A
//! full-dimensional polytope has a unique irredundant facet description, so
//! the result describes the same facets as Julia's, whether Julia computes the
//! hull from the vertices or from all lattice points.

use crate::matrix::Matrix;
use rug::{Integer, Rational};
use unitrisat_cdd::{CddError, hull_inequalities};

/// Failure of [`facets`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HullError {
    /// cddlib failed.
    #[error(transparent)]
    Cdd(#[from] CddError),
    /// The vertices lie in a proper affine subspace (cddlib returned
    /// equations); project them first.
    #[error("the vertices are not full-dimensional")]
    NotFullDimensional,
    /// A facet coefficient does not fit `i64`.
    #[error("a facet coefficient does not fit a 64-bit integer")]
    Overflow,
    /// cddlib returned a row that no hull can produce.
    #[error("cddlib returned a malformed H-representation: {0}")]
    Malformed(&'static str),
}

/// A facet inequality `normal · x <= bound`. The coefficients are coprime
/// integers, so every facet has exactly one representation.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Facet {
    /// Outer normal, one entry per coordinate.
    pub normal: Vec<i64>,
    /// Right-hand side.
    pub bound: i64,
}

/// The facets of the convex hull of the full-dimensional `vertices`, in
/// cddlib's order.
pub fn facets(vertices: &Matrix) -> Result<Vec<Facet>, HullError> {
    let hrep = hull_inequalities(vertices.values(), vertices.columns())?;
    if !hrep.equations.is_empty() {
        return Err(HullError::NotFullDimensional);
    }
    let mut facets = Vec::with_capacity(hrep.inequalities.len());
    for row in &hrep.inequalities {
        if row.len() != vertices.columns().saturating_add(1) {
            return Err(HullError::Malformed("row of the wrong length"));
        }
        if let Some(facet) = facet(row)? {
            facets.push(facet);
        }
    }
    Ok(facets)
}

/// The cddlib row `[b, a]`, meaning `b + a · x >= 0`, as the facet
/// `-a · x <= b` scaled to coprime integers. `None` for a row with `a = 0`
/// and `b >= 0`, which holds everywhere: cddlib can emit `[1, 0, ..., 0]`
/// (REVIEW.md R28), and Julia keeps it as the halfspace `0 · x <= 1`.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "rug integers cannot overflow; every divisor is a denominator or a nonzero gcd"
)]
fn facet(row: &[Rational]) -> Result<Option<Facet>, HullError> {
    let lcm = row
        .iter()
        .fold(Integer::from(1), |acc, x| acc.lcm(x.denom()));
    let mut integers: Vec<Integer> = row
        .iter()
        .map(|x| Integer::from(x.numer() * &lcm).div_exact(x.denom()))
        .collect();
    let gcd = integers.iter().fold(Integer::new(), |acc, x| acc.gcd(x));
    let Some((bound, normal)) = integers.split_first_mut() else {
        return Err(HullError::Malformed("empty row"));
    };
    if normal.iter().all(|x| *x == 0) {
        return if *bound >= 0 {
            Ok(None)
        } else {
            Err(HullError::Malformed("an infeasible inequality"))
        };
    }
    let to_i64 = |x: Integer| x.to_i64().ok_or(HullError::Overflow);
    Ok(Some(Facet {
        normal: normal
            .iter_mut()
            .map(|x| to_i64(-std::mem::take(x).div_exact(&gcd)))
            .collect::<Result<_, _>>()?,
        bound: to_i64(std::mem::take(bound).div_exact(&gcd))?,
    }))
}

#[cfg(test)]
mod tests;

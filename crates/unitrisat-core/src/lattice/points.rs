//! Lattice points of a full-dimensional polytope.
//!
//! [`lattice_points`] is the Rust counterpart of Julia's
//! `lattice_points_via_CDDLib`. Julia tests every point of the vertex bounding
//! box against every facet (a float prefilter with an exact rational
//! fallback). Here the box is scanned in lines along the first coordinate:
//! for each choice of the other coordinates, the facets bound the first
//! coordinate to an integer interval, computed exactly with floor division,
//! and every point of that interval is a lattice point of the polytope. The
//! output is Julia's: the same points in the same order, the first coordinate
//! varying fastest (`Iterators.product` over the box ranges).

use crate::hull::Facet;
use crate::matrix::{MAX_VALUES, Matrix};

/// Failure of [`lattice_points`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PointsError {
    /// A facet normal does not have one entry per vertex coordinate.
    #[error("a facet normal has {found} entries for {expected} coordinates")]
    Dimension {
        /// Number of vertex coordinates.
        expected: usize,
        /// Entries of the offending normal.
        found: usize,
    },
    /// A facet value `bound - normal · x` on the bounding box does not fit
    /// `i128`.
    #[error("a facet value on the bounding box does not fit a 128-bit integer")]
    Overflow,
    /// The polytope has more lattice points than a [`Matrix`] holds.
    #[error("more than {MAX_VALUES} lattice point coordinates")]
    TooManyPoints,
    /// No point of the bounding box satisfies every facet, so the facets do
    /// not belong to these vertices.
    #[error("no lattice point of the bounding box satisfies every facet")]
    Empty,
}

/// The facets as columns of `i128` coefficients, one entry per facet in each.
struct Coefficients {
    /// Coefficient of the first coordinate.
    first: Vec<i128>,
    /// Coefficients of coordinate `j + 1`, for each `j`.
    outer: Vec<Vec<i128>>,
    /// `bound - normal · x` for the current outer coordinates and first
    /// coordinate 0.
    rest: Vec<i128>,
}

/// The lattice points of the polytope with these vertices and facets
/// (`normal · x <= bound`, as returned by [`crate::hull::facets`]), in
/// Julia's order: the box from the smallest to the largest vertex
/// coordinates, the first coordinate varying fastest.
pub fn lattice_points(vertices: &Matrix, facets: &[Facet]) -> Result<Matrix, PointsError> {
    let dimension = vertices.columns();
    if let Some(facet) = facets.iter().find(|f| f.normal.len() != dimension) {
        return Err(PointsError::Dimension {
            expected: dimension,
            found: facet.normal.len(),
        });
    }
    let (low, high) = bounding_box(vertices);
    let (Some((&first_low, outer_low)), Some((&first_high, outer_high))) =
        (low.split_first(), high.split_first())
    else {
        return Err(PointsError::Empty);
    };
    let mut coefficients = coefficients(facets, outer_low)?;
    // The amount by which a full sweep of coordinate `j + 1` changes `rest`.
    let sweep: Vec<Vec<i128>> = coefficients
        .outer
        .iter()
        .zip(outer_low.iter().zip(outer_high))
        .map(|(column, (&lo, &hi))| {
            let width = i128::from(hi)
                .checked_sub(i128::from(lo))
                .ok_or(PointsError::Overflow)?;
            column
                .iter()
                .map(|&a| a.checked_mul(width).ok_or(PointsError::Overflow))
                .collect()
        })
        .collect::<Result<_, _>>()?;
    let mut outer = outer_low.to_vec();
    let mut values = Vec::new();
    loop {
        push_line(&coefficients, (first_low, first_high), &outer, &mut values)?;
        // Next outer coordinates in Julia's order: increment the first one
        // below its maximum, and reset the ones before it to their minimum.
        let Some(level) = outer.iter().zip(outer_high).position(|(x, hi)| x < hi) else {
            break;
        };
        let rest = &mut coefficients.rest;
        for ((x, &lo), sweep) in outer.iter_mut().zip(outer_low).zip(&sweep).take(level) {
            *x = lo;
            for (r, &s) in rest.iter_mut().zip(sweep) {
                *r = r.checked_add(s).ok_or(PointsError::Overflow)?;
            }
        }
        let (Some(x), Some(column)) = (outer.get_mut(level), coefficients.outer.get(level)) else {
            return Err(PointsError::Overflow);
        };
        *x = x.checked_add(1).ok_or(PointsError::Overflow)?;
        for (r, &a) in rest.iter_mut().zip(column) {
            *r = r.checked_sub(a).ok_or(PointsError::Overflow)?;
        }
    }
    Matrix::new(dimension, values).map_err(|_| PointsError::Empty)
}

/// Smallest and largest vertex coordinate in each column.
fn bounding_box(vertices: &Matrix) -> (Vec<i64>, Vec<i64>) {
    let mut rows = vertices.iter_rows();
    let first = rows.next().unwrap_or_default();
    let (mut low, mut high) = (first.to_vec(), first.to_vec());
    for row in rows {
        for ((lo, hi), &x) in low.iter_mut().zip(high.iter_mut()).zip(row) {
            *lo = (*lo).min(x);
            *hi = (*hi).max(x);
        }
    }
    (low, high)
}

/// The facet coefficients, with `rest` at the outer coordinates `outer_low`.
fn coefficients(facets: &[Facet], outer_low: &[i64]) -> Result<Coefficients, PointsError> {
    let mut first = Vec::with_capacity(facets.len());
    let mut outer = vec![Vec::with_capacity(facets.len()); outer_low.len()];
    let mut rest = Vec::with_capacity(facets.len());
    for facet in facets {
        let Some((&a, normal)) = facet.normal.split_first() else {
            return Err(PointsError::Dimension {
                expected: outer_low.len().saturating_add(1),
                found: 0,
            });
        };
        first.push(i128::from(a));
        let mut r = i128::from(facet.bound);
        for ((column, &a), &x) in outer.iter_mut().zip(normal).zip(outer_low) {
            column.push(i128::from(a));
            let ax = i128::from(a)
                .checked_mul(i128::from(x))
                .ok_or(PointsError::Overflow)?;
            r = r.checked_sub(ax).ok_or(PointsError::Overflow)?;
        }
        rest.push(r);
    }
    Ok(Coefficients { first, outer, rest })
}

/// Appends the lattice points with these outer coordinates: the first
/// coordinate runs over `first_low..=first_high`, restricted by every facet
/// `a x_0 <= rest`.
fn push_line(
    coefficients: &Coefficients,
    (first_low, first_high): (i64, i64),
    outer: &[i64],
    values: &mut Vec<i64>,
) -> Result<(), PointsError> {
    let (mut lo, mut hi) = (i128::from(first_low), i128::from(first_high));
    for (&a, &r) in coefficients.first.iter().zip(&coefficients.rest) {
        if a > 0 {
            // x_0 <= floor(r / a).
            hi = hi.min(r.checked_div_euclid(a).ok_or(PointsError::Overflow)?);
        } else if a < 0 {
            // -|a| x_0 <= r, so x_0 >= ceil(-r / |a|) = -floor(r / |a|).
            let floor = a
                .checked_neg()
                .and_then(|q| r.checked_div_euclid(q))
                .and_then(i128::checked_neg)
                .ok_or(PointsError::Overflow)?;
            lo = lo.max(floor);
        } else if r < 0 {
            return Ok(());
        }
        if lo > hi {
            return Ok(());
        }
    }
    let (Ok(lo), Ok(hi)) = (i64::try_from(lo), i64::try_from(hi)) else {
        return Err(PointsError::Overflow);
    };
    let count = hi
        .abs_diff(lo)
        .checked_add(1)
        .and_then(|n| usize::try_from(n).ok())
        .and_then(|n| n.checked_mul(outer.len().checked_add(1)?))
        .and_then(|n| n.checked_add(values.len()))
        .filter(|&n| n <= MAX_VALUES)
        .ok_or(PointsError::TooManyPoints)?;
    values.reserve(count.saturating_sub(values.len()));
    for x in lo..=hi {
        values.push(x);
        values.extend_from_slice(outer);
    }
    Ok(())
}

#[cfg(test)]
mod tests;

//! Optional integer vertex generators for standard polytope families.

use crate::matrix::{MAX_VALUES, Matrix};

/// Failure to generate a family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FamilyError {
    /// The dimension must be at least 1.
    #[error("dimension must be at least 1")]
    ZeroDimension,
    /// A cyclic polytope needs at least one point.
    #[error("the cyclic polytope needs at least one point")]
    NoPoints,
    /// The vertex list would have more than [`MAX_VALUES`] coordinates.
    #[error("the vertex list would have more than {MAX_VALUES} coordinates")]
    TooLarge,
    /// A coordinate does not fit `i64`.
    #[error("a coordinate does not fit a 64-bit integer")]
    Overflow,
}

/// Checks `rows * dimension <= MAX_VALUES` and reserves the values.
fn allocate(dimension: usize, rows: Option<usize>) -> Result<Vec<i64>, FamilyError> {
    if dimension == 0 {
        return Err(FamilyError::ZeroDimension);
    }
    let len = rows
        .and_then(|rows| rows.checked_mul(dimension))
        .filter(|&len| len <= MAX_VALUES)
        .ok_or(FamilyError::TooLarge)?;
    Ok(Vec::with_capacity(len))
}

fn finish(dimension: usize, values: Vec<i64>) -> Result<Matrix, FamilyError> {
    // `allocate` admitted the shape and every family has at least one row,
    // so the shape is valid.
    Matrix::new(dimension, values).map_err(|_| FamilyError::TooLarge)
}

/// The 0/1 cube: row `i` for `i = 0..2^d`, coordinate `j` is bit `j` of `i`
/// (Julia's `digits(i, base = 2, pad = d)`).
pub fn hypercube(dimension: usize) -> Result<Matrix, FamilyError> {
    let shift = u32::try_from(dimension).map_err(|_| FamilyError::TooLarge)?;
    let rows = 1_usize.checked_shl(shift);
    let mut values = allocate(dimension, rows)?;
    for row in 0..rows.unwrap_or(0) {
        let mut bits = row;
        for _ in 0..dimension {
            values.push(i64::from(bits & 1 == 1));
            bits >>= 1;
        }
    }
    finish(dimension, values)
}

/// The cross polytope: `+e_i` then `-e_i`, for `i` ascending.
pub fn cross_polytope(dimension: usize) -> Result<Matrix, FamilyError> {
    let mut values = allocate(dimension, dimension.checked_mul(2))?;
    for axis in 0..dimension {
        for sign in [1, -1] {
            values.extend((0..dimension).map(|column| if column == axis { sign } else { 0 }));
        }
    }
    finish(dimension, values)
}

/// The standard simplex scaled by `scale`: `scale * e_i` for `i` ascending,
/// then the origin.
pub fn scaled_simplex(dimension: usize, scale: i64) -> Result<Matrix, FamilyError> {
    let rows = dimension.checked_add(1);
    let mut values = allocate(dimension, rows)?;
    for row in 0..rows.unwrap_or(0) {
        values.extend((0..dimension).map(|column| if column == row { scale } else { 0 }));
    }
    finish(dimension, values)
}

/// The cyclic polytope `C(points, d)` on the moment curve: row `i` is
/// `(i, i^2, ..., i^d)` for `i = 0..points`.
pub fn cyclic(dimension: usize, points: usize) -> Result<Matrix, FamilyError> {
    if points == 0 {
        return Err(FamilyError::NoPoints);
    }
    let mut values = allocate(dimension, Some(points))?;
    for row in 0..points {
        let base = i64::try_from(row).map_err(|_| FamilyError::Overflow)?;
        let mut power: i64 = 1;
        for _ in 0..dimension {
            power = power.checked_mul(base).ok_or(FamilyError::Overflow)?;
            values.push(power);
        }
    }
    finish(dimension, values)
}

#[cfg(test)]
mod tests;

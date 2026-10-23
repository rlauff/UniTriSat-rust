//! Lattice geometry of the input polytope.
//!
//! [`affine_rank`] is the intrinsic dimension of a vertex set (Julia's
//! `Polyhedra.dim`). [`project_to_full_dimension`] is the Rust counterpart of
//! Julia's `full_dimensional_lattice_projection`: it maps a polytope that is
//! not full-dimensional to a lattice-equivalent full-dimensional one.
//! [`lattice_points`] lists the lattice points of a full-dimensional polytope
//! (Julia's `lattice_points_via_CDDLib`). [`simplices`] lists the
//! full-dimensional simplices spanned by those points (Julia's
//! `all_simplices`). [`faces`] lists the codimension-1 faces of those
//! simplices, each marked internal or boundary as in Julia's `internal_faces`,
//! with the simplices containing it. [`central`] finds the simplices that
//! contain one generic point (Julia's `compute_central_indices`).

use crate::matrix::Matrix;
use rug::Integer;

mod central;
mod faces;
mod points;
mod simplices;

pub use central::{Central, CentralError, central};
pub use faces::{Face, Faces, FacesError, faces};
pub use points::{PointsError, lattice_points};
pub use simplices::{Restriction, SimplicesError, simplices};

/// Failure of the projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProjectionError {
    /// All vertices coincide, so the polytope is a point.
    #[error("the polytope is a single point")]
    Point,
    /// A projected coordinate does not fit `i64`.
    #[error("a projected coordinate does not fit a 64-bit integer")]
    Overflow,
}

/// Row-major big-integer matrix used during elimination.
type Rows = Vec<Vec<Integer>>;

/// The shifted, transposed vertex matrix: row `j` holds coordinate `j` of
/// `vertex_i - vertex_0` for every vertex `i`.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "rug integers cannot overflow; every divisor is a nonzero pivot or gcd"
)]
fn shifted_columns(vertices: &Matrix) -> Rows {
    let origin = vertices.iter_rows().next().unwrap_or_default();
    (0..vertices.columns())
        .map(|j| {
            let base = origin.get(j).copied().unwrap_or(0);
            vertices
                .iter_rows()
                .map(|row| Integer::from(row.get(j).copied().unwrap_or(0)) - base)
                .collect()
        })
        .collect()
}

/// Mutable references to rows `a < b`.
fn two_rows(rows: &mut Rows, a: usize, b: usize) -> Option<(&mut Vec<Integer>, &mut Vec<Integer>)> {
    let (top, bottom) = rows.split_at_mut(b);
    Some((top.get_mut(a)?, bottom.first_mut()?))
}

/// Replaces rows `(r, s)` by `(x r + y s, u r + v s)`.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "rug integers cannot overflow; every divisor is a nonzero pivot or gcd"
)]
fn combine(r: &mut [Integer], s: &mut [Integer], [x, y, u, v]: [&Integer; 4]) {
    for (a, b) in r.iter_mut().zip(s.iter_mut()) {
        let new_a = Integer::from(x * &*a) + Integer::from(y * &*b);
        let new_b = Integer::from(u * &*a) + Integer::from(v * &*b);
        *a = new_a;
        *b = new_b;
    }
}

/// Brings `rows` into row Hermite normal form in place: pivots positive and
/// strictly increasing to the right, zeros below each pivot, entries above a
/// pivot in `0..pivot`, zero rows last (AbstractAlgebra's `hnf`). With
/// `reduce` false it stops at an echelon form. Returns the rank.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "rug integers cannot overflow; every divisor is a nonzero pivot or gcd"
)]
fn hermite(rows: &mut Rows, reduce: bool) -> usize {
    let columns = rows.first().map_or(0, Vec::len);
    let mut rank = 0;
    for j in 0..columns {
        let Some(pivot) = (rank..rows.len()).find(|&i| {
            rows.get(i)
                .and_then(|row| row.get(j))
                .is_some_and(|x| *x != 0)
        }) else {
            continue;
        };
        rows.swap(rank, pivot);
        for i in rank.saturating_add(1)..rows.len() {
            let Some((top, row)) = two_rows(rows, rank, i) else {
                continue;
            };
            let (Some(a), Some(b)) = (top.get(j), row.get(j)) else {
                continue;
            };
            if *b == 0 {
                continue;
            }
            // s a + t b = g, so (s, t; -b/g, a/g) is unimodular and clears b.
            let (g, s, t) = a.clone().extended_gcd(b.clone(), Integer::new());
            let a_g = Integer::from(a / &g);
            let minus_b_g = -Integer::from(b / &g);
            combine(top, row, [&s, &t, &minus_b_g, &a_g]);
        }
        let Some(top) = rows.get_mut(rank) else {
            break;
        };
        if top.get(j).is_some_and(|x| *x < 0) {
            top.iter_mut().for_each(|x| *x = -std::mem::take(x));
        }
        if reduce {
            for i in 0..rank {
                let Some((row, top)) = two_rows(rows, i, rank) else {
                    continue;
                };
                let (Some(x), Some(p)) = (row.get(j), top.get(j)) else {
                    continue;
                };
                let (q, _) = x.clone().div_rem_floor(p.clone());
                if q != 0 {
                    for (a, b) in row.iter_mut().zip(top.iter()) {
                        *a -= Integer::from(&q * b);
                    }
                }
            }
        }
        rank = rank.saturating_add(1);
    }
    rank
}

/// The dimension of the affine hull of the vertices.
pub fn affine_rank(vertices: &Matrix) -> usize {
    hermite(&mut shifted_columns(vertices), false)
}

/// For a polytope whose affine hull has dimension `r` below the ambient
/// dimension `d`, a lattice-equivalent polytope in `Z^r`; `None` if it is
/// already full-dimensional.
///
/// As in Julia: shift the first vertex to the origin, put the coordinates in
/// the rows of a `d x n` matrix, take its row Hermite normal form `H = U M`
/// (`U` unimodular), and read the new vertices off the columns of the `r`
/// nonzero rows. `U` maps the lattice points of the affine hull bijectively
/// onto `Z^r`, so lattice points, volumes and triangulations correspond.
pub fn project_to_full_dimension(vertices: &Matrix) -> Result<Option<Matrix>, ProjectionError> {
    let mut rows = shifted_columns(vertices);
    let rank = hermite(&mut rows, true);
    if rank == vertices.columns() {
        return Ok(None);
    }
    if rank == 0 {
        return Err(ProjectionError::Point);
    }
    let mut values = Vec::new();
    for i in 0..vertices.rows() {
        for row in rows.iter().take(rank) {
            let x = row.get(i).ok_or(ProjectionError::Overflow)?;
            values.push(x.to_i64().ok_or(ProjectionError::Overflow)?);
        }
    }
    Matrix::new(rank, values)
        .map(Some)
        .map_err(|_| ProjectionError::Overflow)
}

#[cfg(test)]
mod tests;

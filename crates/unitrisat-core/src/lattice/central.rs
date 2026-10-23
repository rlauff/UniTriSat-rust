//! Central simplices: the simplices whose interior contains one fixed generic
//! point of the polytope.
//!
//! Julia's parallel split and incremental modes rest on such a point
//! (`find_generic_point`, `compute_central_indices`). The point lies in the
//! interior of the polytope and on no hyperplane spanned by a face of a
//! simplex, so every triangulation by these simplices has exactly one simplex
//! containing it: the central simplices split the triangulations into
//! disjoint classes.
//!
//! Extracted from the archived `central/` module (P0.4), on the faces of
//! [`crate::lattice::faces`] instead of the legacy incidence index:
//! - every face gets the integer normal `n` of its hyperplane (the cofactors
//!   of its edge matrix, so `n · (x - q) = det[edges; x - q]` for its first
//!   point `q`), and every simplex slot records on which side of its face's
//!   hyperplane the opposite vertex lies;
//! - the point is a weighted average `Σ w_i p_i / Σ w_i` of all lattice
//!   points with positive weights. The weights are first drawn from a seeded
//!   generator in `1..=10000` (Julia draws them unseeded), so the result is
//!   reproducible. If no draw avoids every hyperplane, the weights `t^i` for
//!   `t = 1, 2, ...` are tried: for each face, `n · (Σ t^i p_i - q Σ t^i)` is
//!   a polynomial in `t` of degree below the number of points `m` that is
//!   not identically zero (not all points lie on the hyperplane), so among
//!   `g (m - 1) + 1` values of `t` for `g` faces one avoids all hyperplanes.
//!   Julia gives up with an error after 1000 random draws instead;
//! - a simplex contains the point in its interior exactly when the point lies
//!   on the side of the opposite vertex for each of its faces.
//!
//! Julia tests the point against the hyperplanes of all `d`-subsets of
//! lattice points not on one facet; the faces of the simplices are the only
//! ones that matter for the central set, so only those are tested here.
//!
//! Everything is exact. It runs on checked `i128` and is redone on `rug`
//! integers if a value does not fit, or if the fallback weights are needed.

use super::Faces;
use crate::matrix::Matrix;
use rug::Integer;
use std::cmp::Ordering;

/// Failure of [`central`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CentralError {
    /// The simplices are not `d + 1` labels each, or the faces do not belong
    /// to them.
    #[error("the simplices and faces do not match the {0}-dimensional points")]
    Shape(usize),
    /// A simplex or face label is not a row of the point matrix.
    #[error("label {0} is not a point")]
    UnknownPoint(u32),
    /// A simplex has a vertex on the hyperplane of its opposite face, so it
    /// is not full-dimensional.
    #[error("simplex {0} is flat")]
    Flat(u32),
    /// An internal invariant failed. This indicates a bug.
    #[error("internal invariant of the central-set computation violated")]
    Internal,
}

/// The central simplices, and the side of each simplex vertex relative to
/// the hyperplane of the opposite face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Central {
    /// Vertices per simplex, `d + 1`.
    width: usize,
    /// Slot `k` of simplex `s` at `s * width + k`: whether vertex `k` lies
    /// on the positive side of the normal of the face opposite it.
    apex_positive: Vec<bool>,
    /// The central simplices, increasing.
    simplices: Vec<u32>,
}

impl Central {
    /// The simplices whose interior contains the generic point, increasing.
    /// Every triangulation by the simplices uses exactly one of them; if
    /// there are none, the simplices do not cover the polytope and there is
    /// no triangulation.
    pub fn simplices(&self) -> &[u32] {
        &self.simplices
    }

    /// Whether vertex `slot` of `simplex` lies on the positive side of the
    /// hyperplane of the opposite face (face slot `slot` of
    /// [`Faces::of_simplex`]), for an orientation of that hyperplane fixed
    /// per face. Two simplices sharing a face overlap exactly when their
    /// vertices opposite it lie on the same side. `None` if there is no such
    /// slot.
    pub fn apex_positive(&self, simplex: u32, slot: usize) -> Option<bool> {
        if slot >= self.width {
            return None;
        }
        let index = usize::try_from(simplex)
            .ok()?
            .checked_mul(self.width)?
            .checked_add(slot)?;
        self.apex_positive.get(index).copied()
    }
}

/// The central simplices of `simplices` (`d + 1` point labels each, as
/// returned by [`crate::lattice::simplices`]) with their `faces` (as returned
/// by [`crate::lattice::faces`] for the same simplices), for the lattice
/// `points` of the polytope.
pub fn central(points: &Matrix, simplices: &[u32], faces: &Faces) -> Result<Central, CentralError> {
    let input = Input::new(points, simplices, faces)?;
    match compute::<i128>(&input, SEEDED_ATTEMPTS, false) {
        Ok(result) => Ok(result),
        Err(Failure::Error(error)) => Err(error),
        Err(Failure::Overflow | Failure::NotGeneric) => {
            compute::<Integer>(&input, SEEDED_ATTEMPTS, true).map_err(Failure::into_error)
        }
    }
}

/// Seed of the weight generator.
const SEED: u64 = 0x5eed_0000_c3e7_7a15;
/// Seeded weight draws before the deterministic fallback.
const SEEDED_ATTEMPTS: usize = 16;
/// Seeded weights are drawn from `1..=MAX_WEIGHT`, as in Julia.
const MAX_WEIGHT: u64 = 10_000;

/// Checked inputs.
struct Input<'a> {
    points: &'a Matrix,
    simplices: &'a [u32],
    faces: &'a Faces,
    /// Dimension.
    d: usize,
    /// Vertices per simplex, `d + 1`.
    width: usize,
}

impl<'a> Input<'a> {
    fn new(
        points: &'a Matrix,
        simplices: &'a [u32],
        faces: &'a Faces,
    ) -> Result<Self, CentralError> {
        let d = points.columns();
        let shape = CentralError::Shape(d);
        let width = d.checked_add(1).ok_or(shape)?;
        if simplices.len().checked_rem(width) != Some(0) {
            return Err(shape);
        }
        let count =
            u32::try_from(simplices.len().checked_div(width).ok_or(shape)?).map_err(|_| shape)?;
        // The faces must have exactly `count` simplices of `width` slots.
        let last = count.checked_sub(1);
        if last.is_some_and(|s| faces.of_simplex(s).is_none_or(|slots| slots.len() != width))
            || faces.of_simplex(count).is_some()
            || faces.iter().any(|face| face.labels.len() != d)
        {
            return Err(shape);
        }
        let rows = points.rows();
        let known = |label: &u32| usize::try_from(*label).is_ok_and(|l| l < rows);
        if let Some(&label) = simplices
            .iter()
            .chain(faces.iter().flat_map(|face| face.labels))
            .find(|label| !known(label))
        {
            return Err(CentralError::UnknownPoint(label));
        }
        Ok(Self {
            points,
            simplices,
            faces,
            d,
            width,
        })
    }

    fn point(&self, label: u32) -> Result<&'a [i64], CentralError> {
        let start = usize::try_from(label)
            .ok()
            .and_then(|l| l.checked_mul(self.d))
            .ok_or(CentralError::UnknownPoint(label))?;
        let end = start
            .checked_add(self.d)
            .ok_or(CentralError::UnknownPoint(label))?;
        self.points
            .values()
            .get(start..end)
            .ok_or(CentralError::UnknownPoint(label))
    }
}

/// Why a lane gave up.
enum Failure {
    /// A value does not fit the lane's integers.
    Overflow,
    /// No weights tried avoid every hyperplane.
    NotGeneric,
    /// The input is invalid.
    Error(CentralError),
}

impl Failure {
    /// The error of the final lane, which cannot overflow and always finds
    /// a generic point.
    fn into_error(self) -> CentralError {
        match self {
            Self::Error(error) => error,
            Self::Overflow | Self::NotGeneric => CentralError::Internal,
        }
    }
}

impl From<CentralError> for Failure {
    fn from(error: CentralError) -> Self {
        Self::Error(error)
    }
}

/// A value does not fit the lane's integers.
struct Overflow;

impl From<Overflow> for Failure {
    fn from(_: Overflow) -> Self {
        Self::Overflow
    }
}

/// Exact integers: checked `i128`, or `rug` integers, which do not overflow.
trait Ring: Clone {
    fn from_i64(value: i64) -> Self;
    fn from_u64(value: u64) -> Self;
    fn sign(&self) -> Ordering;
    fn add(&self, other: &Self) -> Result<Self, Overflow>;
    fn sub(&self, other: &Self) -> Result<Self, Overflow>;
    fn mul(&self, other: &Self) -> Result<Self, Overflow>;
    /// Division that is known to be exact; `Overflow` for a zero divisor.
    fn div_exact(&self, other: &Self) -> Result<Self, Overflow>;
    fn neg(&self) -> Result<Self, Overflow>;
}

impl Ring for i128 {
    fn from_i64(value: i64) -> Self {
        Self::from(value)
    }
    fn from_u64(value: u64) -> Self {
        Self::from(value)
    }
    fn sign(&self) -> Ordering {
        self.cmp(&0)
    }
    fn add(&self, other: &Self) -> Result<Self, Overflow> {
        self.checked_add(*other).ok_or(Overflow)
    }
    fn sub(&self, other: &Self) -> Result<Self, Overflow> {
        self.checked_sub(*other).ok_or(Overflow)
    }
    fn mul(&self, other: &Self) -> Result<Self, Overflow> {
        self.checked_mul(*other).ok_or(Overflow)
    }
    fn div_exact(&self, other: &Self) -> Result<Self, Overflow> {
        self.checked_div(*other).ok_or(Overflow)
    }
    fn neg(&self) -> Result<Self, Overflow> {
        self.checked_neg().ok_or(Overflow)
    }
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "rug integers cannot overflow; the divisor is checked to be nonzero"
)]
impl Ring for Integer {
    fn from_i64(value: i64) -> Self {
        Self::from(value)
    }
    fn from_u64(value: u64) -> Self {
        Self::from(value)
    }
    fn sign(&self) -> Ordering {
        self.cmp0()
    }
    fn add(&self, other: &Self) -> Result<Self, Overflow> {
        Ok(Self::from(self + other))
    }
    fn sub(&self, other: &Self) -> Result<Self, Overflow> {
        Ok(Self::from(self - other))
    }
    fn mul(&self, other: &Self) -> Result<Self, Overflow> {
        Ok(Self::from(self * other))
    }
    fn div_exact(&self, other: &Self) -> Result<Self, Overflow> {
        if other.cmp0() == Ordering::Equal {
            return Err(Overflow);
        }
        Ok(Self::from(self.div_exact_ref(other)))
    }
    fn neg(&self) -> Result<Self, Overflow> {
        Ok(Self::from(-self))
    }
}

/// Entry `(row, column)` of a row-major matrix with `columns` columns.
fn at(row: usize, column: usize, columns: usize) -> Result<usize, CentralError> {
    row.checked_mul(columns)
        .and_then(|i| i.checked_add(column))
        .ok_or(CentralError::Internal)
}

fn entry<R: Ring>(m: &[R], row: usize, column: usize, size: usize) -> Result<&R, CentralError> {
    m.get(at(row, column, size)?).ok_or(CentralError::Internal)
}

/// The determinant of the `size x size` row-major matrix `m`, by fraction-free
/// (Bareiss) elimination in place.
fn determinant<R: Ring>(m: &mut [R], size: usize) -> Result<R, Failure> {
    let mut negate = false;
    let mut previous = R::from_i64(1);
    for i in 0..size {
        let Some(pivot) =
            (i..size).find(|&r| entry(m, r, i, size).is_ok_and(|x| x.sign() != Ordering::Equal))
        else {
            return Ok(R::from_i64(0));
        };
        if pivot != i {
            for c in 0..size {
                m.swap(at(i, c, size)?, at(pivot, c, size)?);
            }
            negate = !negate;
        }
        let p = entry(m, i, i, size)?.clone();
        for r in i.saturating_add(1)..size {
            let lead = entry(m, r, i, size)?.clone();
            for c in i.saturating_add(1)..size {
                let index = at(r, c, size)?;
                let x = m.get(index).ok_or(CentralError::Internal)?;
                let value = x
                    .mul(&p)?
                    .sub(&lead.mul(entry(m, i, c, size)?)?)?
                    .div_exact(&previous)?;
                *m.get_mut(index).ok_or(CentralError::Internal)? = value;
            }
        }
        previous = p;
    }
    if negate {
        Ok(previous.neg()?)
    } else {
        Ok(previous)
    }
}

/// The normals of all faces, `d` entries per face: entry `k` is the
/// cofactor of column `k` in the last row of `[edges; x]`, where the edges
/// are the face's points minus its first point.
fn normals<R: Ring>(input: &Input<'_>) -> Result<Vec<R>, Failure> {
    let d = input.d;
    let rows = d.checked_sub(1).ok_or(CentralError::Internal)?;
    let capacity = input
        .faces
        .len()
        .checked_mul(d)
        .ok_or(CentralError::Internal)?;
    let mut normals = Vec::with_capacity(capacity);
    let mut edges = Vec::with_capacity(rows.checked_mul(d).ok_or(CentralError::Internal)?);
    let mut minor = Vec::with_capacity(rows.checked_mul(rows).ok_or(CentralError::Internal)?);
    for face in input.faces.iter() {
        let (&first, others) = face.labels.split_first().ok_or(CentralError::Internal)?;
        let anchor = input.point(first)?;
        edges.clear();
        for &label in others {
            for (&x, &q) in input.point(label)?.iter().zip(anchor) {
                edges.push(R::from_i64(x).sub(&R::from_i64(q))?);
            }
        }
        for k in 0..d {
            minor.clear();
            for row in edges.chunks_exact(d) {
                minor.extend(
                    row.iter()
                        .enumerate()
                        .filter(|&(c, _)| c != k)
                        .map(|(_, x)| x.clone()),
                );
            }
            let cofactor = determinant(&mut minor, rows)?;
            // The sign of position (d - 1, k), zero-based.
            let odd = !rows
                .checked_add(k)
                .ok_or(CentralError::Internal)?
                .is_multiple_of(2);
            normals.push(if odd { cofactor.neg()? } else { cofactor });
        }
    }
    Ok(normals)
}

/// `normal · (x - anchor)`.
fn side<R: Ring>(normal: &[R], x: impl Iterator<Item = R>, anchor: &[i64]) -> Result<R, Failure> {
    let mut sum = R::from_i64(0);
    for ((n, x), &q) in normal.iter().zip(x).zip(anchor) {
        sum = sum.add(&n.mul(&x.sub(&R::from_i64(q))?)?)?;
    }
    Ok(sum)
}

/// The normal of face `face` in `normals`.
fn normal_of<R: Ring>(normals: &[R], face: u32, d: usize) -> Result<&[R], CentralError> {
    let start = usize::try_from(face)
        .ok()
        .and_then(|f| f.checked_mul(d))
        .ok_or(CentralError::Internal)?;
    normals
        .get(start..start.checked_add(d).ok_or(CentralError::Internal)?)
        .ok_or(CentralError::Internal)
}

/// For every simplex slot, whether its vertex lies on the positive side of
/// the opposite face.
fn apex_sides<R: Ring>(input: &Input<'_>, normals: &[R]) -> Result<Vec<bool>, Failure> {
    let mut sides = Vec::with_capacity(input.simplices.len());
    for (s, labels) in (0u32..).zip(input.simplices.chunks_exact(input.width)) {
        let slots = input.faces.of_simplex(s).ok_or(CentralError::Internal)?;
        for (&apex, &face) in labels.iter().zip(slots) {
            let first = *input
                .faces
                .get(face)
                .and_then(|f| f.labels.first())
                .ok_or(CentralError::Internal)?;
            let apex = input.point(apex)?.iter().map(|&x| R::from_i64(x));
            let value = side(
                normal_of(normals, face, input.d)?,
                apex,
                input.point(first)?,
            )?;
            match value.sign() {
                Ordering::Greater => sides.push(true),
                Ordering::Less => sides.push(false),
                Ordering::Equal => return Err(CentralError::Flat(s).into()),
            }
        }
    }
    Ok(sides)
}

/// The side of the point `Σ w_i p_i / Σ w_i` for every face, or `None` if
/// it lies on a face hyperplane.
fn point_sides<R: Ring>(
    input: &Input<'_>,
    normals: &[R],
    weights: impl Iterator<Item = Result<R, Overflow>>,
) -> Result<Option<Vec<bool>>, Failure> {
    let mut numerator = vec![R::from_i64(0); input.d];
    let mut denominator = R::from_i64(0);
    let mut count = 0usize;
    for (weight, point) in weights.zip(input.points.iter_rows()) {
        let weight = weight?;
        for (sum, &x) in numerator.iter_mut().zip(point) {
            *sum = sum.add(&weight.mul(&R::from_i64(x))?)?;
        }
        denominator = denominator.add(&weight)?;
        count = count.checked_add(1).ok_or(CentralError::Internal)?;
    }
    if count != input.points.rows() {
        return Err(CentralError::Internal.into());
    }
    // n · (N / W - q) has the sign of n · (N - W q), as W > 0.
    let mut sides = Vec::with_capacity(input.faces.len());
    for (f, face) in (0u32..).zip(input.faces.iter()) {
        let first = *face.labels.first().ok_or(CentralError::Internal)?;
        let anchor: Vec<R> = input
            .point(first)?
            .iter()
            .map(|&q| denominator.mul(&R::from_i64(q)))
            .collect::<Result<_, _>>()?;
        let mut value = R::from_i64(0);
        for ((n, x), q) in normal_of(normals, f, input.d)?
            .iter()
            .zip(&numerator)
            .zip(&anchor)
        {
            value = value.add(&n.mul(&x.sub(q)?)?)?;
        }
        match value.sign() {
            Ordering::Greater => sides.push(true),
            Ordering::Less => sides.push(false),
            Ordering::Equal => return Ok(None),
        }
    }
    Ok(Some(sides))
}

/// SplitMix64, the weight generator.
struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ z.wrapping_shr(30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ z.wrapping_shr(27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ z.wrapping_shr(31)
    }

    /// A weight in `1..=MAX_WEIGHT`.
    fn weight(&mut self) -> u64 {
        self.next()
            .checked_rem(MAX_WEIGHT)
            .unwrap_or(0)
            .saturating_add(1)
    }
}

/// The central set in lane `R`: `attempts` seeded weight draws, then the
/// fallback weights `t^i` if `fallback`.
fn compute<R: Ring>(
    input: &Input<'_>,
    attempts: usize,
    fallback: bool,
) -> Result<Central, Failure> {
    let normals = normals::<R>(input)?;
    let apex_positive = apex_sides(input, &normals)?;
    let mut generator = SplitMix(SEED);
    let mut point = None;
    for _ in 0..attempts {
        let weights = (0..input.points.rows()).map(|_| Ok(R::from_u64(generator.weight())));
        point = point_sides(input, &normals, weights)?;
        if point.is_some() {
            break;
        }
    }
    if point.is_none() && fallback {
        let faces = u64::try_from(input.faces.len()).unwrap_or(u64::MAX);
        let degree = u64::try_from(input.points.rows().saturating_sub(1)).unwrap_or(u64::MAX);
        let bound = faces.saturating_mul(degree).saturating_add(1);
        for t in 1..=bound {
            let base = R::from_u64(t);
            let mut power = R::from_i64(1);
            let weights = (0..input.points.rows()).map(|_| {
                let weight = power.clone();
                power = power.mul(&base)?;
                Ok(weight)
            });
            point = point_sides(input, &normals, weights)?;
            if point.is_some() {
                break;
            }
        }
    }
    let point = point.ok_or(Failure::NotGeneric)?;

    let mut simplices = Vec::new();
    for (s, sides) in (0u32..).zip(apex_positive.chunks_exact(input.width)) {
        let slots = input.faces.of_simplex(s).ok_or(CentralError::Internal)?;
        let mut inside = true;
        for (&apex, &face) in sides.iter().zip(slots) {
            let face = usize::try_from(face).map_err(|_| CentralError::Internal)?;
            inside &= *point.get(face).ok_or(CentralError::Internal)? == apex;
        }
        if inside {
            simplices.push(s);
        }
    }
    Ok(Central {
        width: input.width,
        apex_positive,
        simplices,
    })
}

#[cfg(test)]
mod tests;

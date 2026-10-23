//! Regularity of triangulations by an exact LP.
//!
//! Port of Julia's `is_regular` (`subdivision_regularity.jl`), which
//! `record_solution!` (`solve.jl`) runs on the enumerated triangulations.
//!
//! **The test.** A triangulation is regular if and only if its secondary
//! cone has a nonempty interior, and that cone is cut out by the local
//! folding conditions at the interior walls (De Loera, Rambau and Santos,
//! *Triangulations*, Thm. 5.2.11). A wall `F` shared by the simplices
//! `s1 = F + {p}` and `s2 = F + {q}` gives the affine dependence `λ` of the
//! circuit on `s1 + {q}`, and the condition is `λ · w > 0` on the heights
//! `w` of the vertices, with `λ` oriented the same way at every wall. As in
//! Julia, `λ_i = (-1)^(i+1) det(circuit without point i)` (zero-based `i`,
//! the points of `s1` in order, then `q`), multiplied by the sign of
//! `det(s1)`, so that `λ_q` has the sign `(-1)^d` at every wall. The rows are
//! divided by the gcd of their entries and deduplicated, and the
//! triangulation is regular if and only if the cone `{w : λ · w >= 1 for
//! every wall}` is nonempty. That is decided by an exact LP with a zero
//! objective through [`unitrisat_cdd::solve_lp`]: cddlib's dual simplex
//! over GMP rationals, the LP that Julia's `isempty` on a
//! `CDDLib.Library(:exact)` polyhedron solves (`dd_Matrix2Feasibility`).
//! One variable per vertex of the triangulation and one row per distinct
//! wall: Julia's LP up to the order of rows and columns.
//!
//! The determinants are exact: fraction-free (Bareiss) elimination of the
//! `d x d` difference matrices in checked `i128`, redone with `rug` integers
//! if an entry overflows. A feasible answer is verified against every row.
//!
//! **Hook.** As a [`Classify`] hook, [`Regularity`] makes the regular
//! triangulations the targets: Julia's `regular = true` without the flag
//! filter, where every triangulation is tested.
//!
//! **The regularity-test cap (D1)** only skips LPs together with the flag
//! filter, so it lives in [`crate::quadratic::Quadratic`], which uses this
//! test. Without the flag filter Julia runs every LP as well, and only
//! `chunk_run.jl` labels the polytope `reglimit` afterwards when it has no
//! regular triangulation among at least `max_regularity_tests`
//! triangulations; here such a search is exact (DIFFERENCES.md, P5.1).

use crate::enumerate::{Classification, Classify};
use crate::lattice::Faces;
use crate::matrix::Matrix;
use rug::{Integer, ops::NegAssign};
use unitrisat_cdd::{CddError, Lp, LpStatus, Rational, Sense, solve_lp};

/// Failure of the regularity test.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RegularError {
    /// The simplex list is not a whole number of simplices with `d + 1`
    /// labels each.
    #[error("{len} simplex labels are not a multiple of {width}")]
    Shape {
        /// Number of labels given.
        len: usize,
        /// Labels per simplex.
        width: usize,
    },
    /// A simplex label is not a row of the point matrix.
    #[error("simplex label {0} is not a point")]
    UnknownPoint(u32),
    /// The faces were not computed from these simplices.
    #[error("the faces do not belong to the {0} simplices")]
    Faces(usize),
    /// A triangulation names a simplex that does not exist.
    #[error("simplex {0} is not one of the simplices")]
    UnknownSimplex(u32),
    /// The simplices do not form a triangulation: a codimension-1 face lies
    /// in more than two of them, or a simplex is listed twice.
    #[error("face {0} is not shared by at most two distinct simplices of the triangulation")]
    NotATriangulation(u32),
    /// A simplex of the triangulation has volume zero.
    #[error("simplex {0} has volume zero")]
    Degenerate(u32),
    /// cddlib failed.
    #[error(transparent)]
    Cdd(#[from] CddError),
    /// cddlib returned an LP status that a feasibility LP cannot have.
    #[error("cddlib returned the LP status {0:?} for a feasibility problem")]
    Status(LpStatus),
    /// cddlib's solution violates a wall inequality.
    #[error("cddlib's height vector violates a wall inequality")]
    Certificate,
    /// The exact dual certificate did not prove infeasibility.
    #[error("invalid exact certificate of nonregularity")]
    ConflictCertificate,
    /// An internal invariant failed. This indicates a bug.
    #[error("internal invariant of the regularity test violated")]
    Internal,
}

/// The regularity test for the triangulations of one polytope.
#[derive(Clone, Copy, Debug)]
pub struct Regularity<'a> {
    points: &'a Matrix,
    simplices: &'a [u32],
    faces: &'a Faces,
    /// Dimension.
    d: usize,
    learn: bool,
}

/// A wall inequality `Σ coefficient · w_column >= 1`: nonzero coprime
/// coefficients, increasing columns.
type Row = Vec<(u32, Integer)>;

impl<'a> Regularity<'a> {
    /// The test for triangulations made of `simplices` (`d + 1` increasing
    /// zero-based labels of `points` per simplex, as returned by
    /// [`crate::lattice::simplices`]), with their `faces` (as returned by
    /// [`crate::lattice::faces`] for the same simplices).
    pub fn new(
        points: &'a Matrix,
        simplices: &'a [u32],
        faces: &'a Faces,
    ) -> Result<Self, RegularError> {
        let d = points.columns();
        let width = d.checked_add(1).ok_or(RegularError::Internal)?;
        let count = simplices.len().checked_div(width).unwrap_or(0);
        if simplices.len().checked_rem(width) != Some(0) {
            return Err(RegularError::Shape {
                len: simplices.len(),
                width,
            });
        }
        if let Some(&label) = simplices
            .iter()
            .find(|&&l| !usize::try_from(l).is_ok_and(|l| l < points.rows()))
        {
            return Err(RegularError::UnknownPoint(label));
        }
        let mismatch = RegularError::Faces(count);
        let per_simplex = |s: usize| {
            u32::try_from(s)
                .ok()
                .and_then(|s| faces.of_simplex(s))
                .map(<[u32]>::len)
        };
        let last_fits = count
            .checked_sub(1)
            .is_none_or(|last| per_simplex(last) == Some(width));
        if !last_fits || per_simplex(count).is_some() {
            return Err(mismatch);
        }
        Ok(Self {
            points,
            simplices,
            faces,
            d,
            learn: false,
        })
    }

    /// Enables exact conflict extraction after a nonregularity result.
    /// Regular candidates use the same single feasibility LP as before.
    /// [`Classify::classify`] returns a per-call certificate-derived conflict;
    /// direct [`Self::is_regular`] calls remain ordinary boolean tests. Dual
    /// systems exceeding one million dense rational cells use ordinary model
    /// blocking instead of learning, bounding this optional LP's input storage.
    pub fn with_learning(mut self, enabled: bool) -> Self {
        self.learn = enabled;
        self
    }

    /// The labels of simplex `simplex`.
    fn simplex(&self, simplex: u32) -> Result<&'a [u32], RegularError> {
        let unknown = RegularError::UnknownSimplex(simplex);
        let width = self.d.checked_add(1).ok_or(RegularError::Internal)?;
        let start = usize::try_from(simplex)
            .ok()
            .and_then(|s| s.checked_mul(width))
            .ok_or(unknown.clone())?;
        let end = start.checked_add(width).ok_or(unknown.clone())?;
        self.simplices.get(start..end).ok_or(unknown)
    }

    /// The coordinates of point `label`.
    fn point(&self, label: u32) -> Result<&'a [i64], RegularError> {
        let start = usize::try_from(label)
            .ok()
            .and_then(|l| l.checked_mul(self.d))
            .ok_or(RegularError::UnknownPoint(label))?;
        let end = start
            .checked_add(self.d)
            .ok_or(RegularError::UnknownPoint(label))?;
        self.points
            .values()
            .get(start..end)
            .ok_or(RegularError::UnknownPoint(label))
    }

    /// Whether the triangulation made of `triangulation` (indices into the
    /// simplices) is regular. One simplex or none is regular, as in Julia.
    pub fn is_regular(&self, triangulation: &[u32]) -> Result<bool, RegularError> {
        Ok(self.evaluate(triangulation, false)?.target)
    }

    fn evaluate(&self, triangulation: &[u32], learn: bool) -> Result<Classification, RegularError> {
        if triangulation.len() <= 1 {
            for &s in triangulation {
                self.simplex(s)?;
            }
            return Ok(true.into());
        }
        // Columns: the vertices, numbered by first appearance (as Julia's
        // `standardize_input` does).
        let mut column = vec![u32::MAX; self.points.rows()];
        let mut columns: u32 = 0;
        // Face slots `(face, simplex, slot)`, grouped by face below.
        let mut slots = Vec::new();
        for &s in triangulation {
            for &label in self.simplex(s)? {
                let entry = usize::try_from(label)
                    .ok()
                    .and_then(|l| column.get_mut(l))
                    .ok_or(RegularError::UnknownPoint(label))?;
                if *entry == u32::MAX {
                    *entry = columns;
                    columns = columns.checked_add(1).ok_or(RegularError::Internal)?;
                }
            }
            let faces = self
                .faces
                .of_simplex(s)
                .ok_or(RegularError::UnknownSimplex(s))?;
            for (slot, &face) in faces.iter().enumerate() {
                slots.push((face, s, slot));
            }
        }
        slots.sort_unstable();
        let mut walls: Vec<(Row, [u32; 2])> = Vec::new();
        let mut machine = vec![0_i128; self.d.saturating_mul(self.d)];
        for group in slots.chunk_by(|a, b| a.0 == b.0) {
            match *group {
                [_] => {} // boundary face
                [(_, s1, _), (_, s2, k2)] if s1 != s2 => {
                    walls.push((self.wall(s1, s2, k2, &column, &mut machine)?, [s1, s2]));
                }
                [(face, ..), ..] => return Err(RegularError::NotATriangulation(face)),
                [] => return Err(RegularError::Internal),
            }
        }
        if walls.is_empty() {
            return Ok(true.into());
        }
        // Equal inequalities need only one activating pair. Sorting the pair
        // as a tie-breaker makes its choice deterministic.
        walls.sort_unstable();
        walls.dedup_by(|a, b| a.0 == b.0);
        let (rows, sources): (Vec<Row>, Vec<[u32; 2]>) = walls.into_iter().unzip();
        let target = cone_is_nonempty(&rows, columns)?;
        let conflict = if !target && learn {
            nonregular_conflict(&rows, columns, &sources, MAX_CERTIFICATE_CELLS)?
        } else {
            None
        };
        Ok(Classification { target, conflict })
    }

    /// The normalized wall inequality between `s1 = F + {p}` and
    /// `s2 = F + {q}` (`q` in slot `k2` of `s2`), over `column`.
    fn wall(
        &self,
        s1: u32,
        s2: u32,
        k2: usize,
        column: &[u32],
        machine: &mut [i128],
    ) -> Result<Row, RegularError> {
        let q = *self
            .simplex(s2)?
            .get(k2)
            .ok_or(RegularError::UnknownSimplex(s2))?;
        let mut circuit: Vec<&[i64]> = Vec::with_capacity(self.d.saturating_add(2));
        let mut labels: Vec<u32> = Vec::with_capacity(self.d.saturating_add(2));
        for &label in self.simplex(s1)?.iter().chain([&q]) {
            circuit.push(self.point(label)?);
            labels.push(label);
        }
        let determinants = match circuit_determinants::<i128>(&circuit, self.d, machine) {
            Some(narrow) => narrow.into_iter().map(Integer::from).collect(),
            None => {
                let mut wide = vec![Integer::new(); machine.len()];
                circuit_determinants::<Integer>(&circuit, self.d, &mut wide)
                    .ok_or(RegularError::Internal)?
            }
        };
        // `det(s1)` is the determinant without `q`, the last one.
        let negative = match determinants.last().map(Integer::cmp0) {
            Some(std::cmp::Ordering::Less) => true,
            Some(std::cmp::Ordering::Greater) => false,
            Some(std::cmp::Ordering::Equal) => return Err(RegularError::Degenerate(s1)),
            None => return Err(RegularError::Internal),
        };
        let mut row: Row = Vec::with_capacity(determinants.len());
        for (i, (determinant, &label)) in determinants.into_iter().zip(&labels).enumerate() {
            if determinant.is_zero() {
                continue;
            }
            // (-1)^(i+1) sign(det s1) det_i.
            let mut value = determinant;
            if i.is_multiple_of(2) != negative {
                value.neg_assign();
            }
            let col = usize::try_from(label)
                .ok()
                .and_then(|l| column.get(l))
                .copied()
                .filter(|&c| c != u32::MAX)
                .ok_or(RegularError::Internal)?;
            row.push((col, value));
        }
        let gcd = row
            .iter()
            .fold(Integer::new(), |gcd, (_, value)| gcd.gcd(value));
        if gcd.is_zero() {
            return Err(RegularError::Internal);
        }
        for (_, value) in &mut row {
            value.div_exact_mut(&gcd);
        }
        row.sort_unstable();
        Ok(row)
    }
}

/// Whether `{w : row · w >= 1 for every row}` is nonempty, by cddlib's
/// exact LP with a zero objective. A feasible point is checked against
/// every row.
fn cone_is_nonempty(rows: &[Row], columns: u32) -> Result<bool, RegularError> {
    let width = usize::try_from(columns)
        .ok()
        .and_then(|c| c.checked_add(1))
        .ok_or(RegularError::Internal)?;
    let mut inequalities = Vec::with_capacity(rows.len());
    for row in rows {
        // cddlib's `[b, a]` means `b + a · w >= 0`: here `-1 + row · w >= 0`.
        let mut dense = vec![Rational::new(); width];
        *dense.first_mut().ok_or(RegularError::Internal)? = Rational::from(-1);
        for (col, value) in row {
            let entry = usize::try_from(*col)
                .ok()
                .and_then(|c| c.checked_add(1))
                .and_then(|c| dense.get_mut(c))
                .ok_or(RegularError::Internal)?;
            *entry = Rational::from(value);
        }
        inequalities.push(dense);
    }
    let objective = vec![Rational::new(); width];
    let solution = solve_lp(&Lp {
        inequalities: &inequalities,
        equations: &[],
        objective: &objective,
        sense: Sense::Maximize,
    })?;
    match solution.status {
        LpStatus::Optimal => {
            for row in rows {
                if !satisfies(row, &solution.point).ok_or(RegularError::Certificate)? {
                    return Err(RegularError::Certificate);
                }
            }
            Ok(true)
        }
        LpStatus::Inconsistent | LpStatus::StructurallyInconsistent => Ok(false),
        status => Err(RegularError::Status(status)),
    }
}

/// Limit the optional dense dual LP to one million input rational cells.
/// Larger systems retain ordinary full-model blocking after the primal test.
const MAX_CERTIFICATE_CELLS: usize = 1_000_000;

/// Farkas' alternative: Aw >= 1 is impossible exactly when there is
/// y >= 0 with A^T y = 0 and sum(y) = 1. Solve this certificate LP once,
/// then verify every equality and sign in exact rationals. Each positive
/// row is activated by its two source simplices; any triangulation containing
/// their union inherits this infeasible subsystem and is nonregular.
fn nonregular_conflict(
    rows: &[Row],
    columns: u32,
    sources: &[[u32; 2]],
    cell_budget: usize,
) -> Result<Option<Vec<u32>>, RegularError> {
    let count = usize::try_from(columns).map_err(|_| RegularError::Internal)?;
    let Some(width) = rows.len().checked_add(1) else {
        return Ok(None);
    };
    // m nonnegativity rows, n+1 equality rows, and one objective row.
    let cells = rows
        .len()
        .checked_add(count)
        .and_then(|n| n.checked_add(2))
        .and_then(|n| n.checked_mul(width));
    if cells.is_none_or(|n| n > cell_budget) {
        return Ok(None);
    }
    let mut inequalities = Vec::with_capacity(rows.len());
    let mut equations = vec![vec![Rational::new(); width]; count.saturating_add(1)];
    let normalization = equations.last_mut().ok_or(RegularError::Internal)?;
    *normalization.first_mut().ok_or(RegularError::Internal)? = Rational::from(-1);
    for value in normalization.iter_mut().skip(1) {
        *value = Rational::from(1);
    }
    for (index, row) in (1_usize..).zip(rows) {
        let mut nonnegative = vec![Rational::new(); width];
        *nonnegative.get_mut(index).ok_or(RegularError::Internal)? = Rational::from(1);
        inequalities.push(nonnegative);
        for (column, value) in row {
            let entry = usize::try_from(*column)
                .ok()
                .and_then(|c| equations.get_mut(c))
                .and_then(|equation| equation.get_mut(index))
                .ok_or(RegularError::Internal)?;
            *entry = Rational::from(value);
        }
    }
    let solution = solve_lp(&Lp {
        inequalities: &inequalities,
        equations: &equations,
        objective: &vec![Rational::new(); width],
        sense: Sense::Maximize,
    })?;
    if solution.status != LpStatus::Optimal {
        return Err(RegularError::Status(solution.status));
    }
    verify_conflict(rows, columns, &solution.point)?;
    if sources.len() != rows.len() {
        return Err(RegularError::Internal);
    }
    let mut conflict: Vec<u32> = solution
        .point
        .iter()
        .zip(sources)
        .filter(|(weight, _)| **weight > 0)
        .flat_map(|(_, pair)| pair.iter().copied())
        .collect();
    conflict.sort_unstable();
    conflict.dedup();
    if conflict.is_empty() {
        return Err(RegularError::ConflictCertificate);
    }
    Ok(Some(conflict))
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "rug rationals cannot overflow"
)]
fn verify_conflict(rows: &[Row], columns: u32, weights: &[Rational]) -> Result<(), RegularError> {
    if weights.len() != rows.len() || weights.iter().any(|weight| *weight < 0) {
        return Err(RegularError::ConflictCertificate);
    }
    let mut sum = Rational::new();
    let mut residual =
        vec![Rational::new(); usize::try_from(columns).map_err(|_| RegularError::Internal)?];
    for (row, weight) in rows.iter().zip(weights) {
        sum += weight;
        for (column, value) in row {
            let entry = usize::try_from(*column)
                .ok()
                .and_then(|c| residual.get_mut(c))
                .ok_or(RegularError::ConflictCertificate)?;
            *entry += Rational::from(weight * value);
        }
    }
    if sum != 1 || residual.iter().any(|value| !value.is_zero()) {
        return Err(RegularError::ConflictCertificate);
    }
    Ok(())
}

/// Whether `row · heights >= 1`; `None` if a column has no height.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "rug rationals cannot overflow"
)]
fn satisfies(row: &Row, heights: &[Rational]) -> Option<bool> {
    let mut value = Rational::new();
    for (col, coefficient) in row {
        let height = heights.get(usize::try_from(*col).ok()?)?;
        value += Rational::from(height * coefficient);
    }
    Some(value >= 1)
}

/// Exact integers for the determinants.
trait Exact: Clone {
    /// The number 0.
    fn zero() -> Self;
    /// The number 1.
    fn one() -> Self;
    /// `a - b`, or `None` if it does not fit.
    fn difference(a: i64, b: i64) -> Option<Self>;
    /// Whether this is 0.
    fn is_zero(&self) -> bool;
    /// `-self`, or `None` if it does not fit.
    fn negated(self) -> Option<Self>;
    /// One Bareiss update, `self = (self * pivot - lead * entry) / previous`,
    /// where the division is exact. `None` if the result does not fit (or
    /// `previous` is 0).
    fn eliminate(&mut self, pivot: &Self, lead: &Self, entry: &Self, previous: &Self)
    -> Option<()>;
}

impl Exact for i128 {
    fn zero() -> Self {
        0
    }

    fn one() -> Self {
        1
    }

    fn difference(a: i64, b: i64) -> Option<Self> {
        i128::from(a).checked_sub(i128::from(b))
    }

    fn is_zero(&self) -> bool {
        *self == 0
    }

    fn negated(self) -> Option<Self> {
        self.checked_neg()
    }

    #[inline]
    fn eliminate(
        &mut self,
        pivot: &Self,
        lead: &Self,
        entry: &Self,
        previous: &Self,
    ) -> Option<()> {
        *self = self
            .checked_mul(*pivot)?
            .checked_sub(lead.checked_mul(*entry)?)?
            .checked_div(*previous)?;
        Some(())
    }
}

impl Exact for Integer {
    fn zero() -> Self {
        Integer::new()
    }

    fn one() -> Self {
        Integer::from(1)
    }

    fn difference(a: i64, b: i64) -> Option<Self> {
        Some(Integer::from(i128::from(a).checked_sub(i128::from(b))?))
    }

    fn is_zero(&self) -> bool {
        self.is_zero()
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "rug integers cannot overflow"
    )]
    fn negated(self) -> Option<Self> {
        Some(-self)
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "rug integers cannot overflow; the divisor is checked to be nonzero"
    )]
    fn eliminate(
        &mut self,
        pivot: &Self,
        lead: &Self,
        entry: &Self,
        previous: &Self,
    ) -> Option<()> {
        if previous.is_zero() {
            return None;
        }
        *self *= pivot;
        *self -= lead * entry;
        self.div_exact_mut(previous);
        Some(())
    }
}

/// For the `d + 2` points of `circuit` (each with `d` coordinates), the
/// `d + 2` determinants `det(circuit without point i)` of the difference
/// matrices (rows: the other points minus the first remaining one).
/// `scratch` holds `d * d` entries. `None` if an entry does not fit `T`.
///
/// The homogeneous determinant of `d + 1` points (rows `[x, 1]`) is
/// `(-1)^d` times this one, the same factor for every `i`; it cancels in the
/// wall inequality, which multiplies by the sign of one of them.
fn circuit_determinants<T: Exact>(
    circuit: &[&[i64]],
    d: usize,
    scratch: &mut [T],
) -> Option<Vec<T>> {
    let mut determinants = Vec::with_capacity(circuit.len());
    for skip in 0..circuit.len() {
        let mut rest = circuit
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != skip)
            .map(|(_, &point)| point);
        let base = rest.next()?;
        for (row, point) in scratch.chunks_exact_mut(d).zip(rest) {
            for ((x, &p), &b) in row.iter_mut().zip(point).zip(base) {
                *x = T::difference(p, b)?;
            }
        }
        determinants.push(determinant(scratch, d)?);
    }
    Some(determinants)
}

/// The determinant of the row-major `d x d` matrix `m` (overwritten), by
/// fraction-free elimination with row pivoting. `None` if an entry does not
/// fit `T`.
fn determinant<T: Exact>(m: &mut [T], d: usize) -> Option<T> {
    if d == 0 || m.len() != d.checked_mul(d)? {
        return None;
    }
    let mut negative = false;
    let mut previous = T::one();
    for k in 0..d {
        // A row from `k` on with a nonzero entry in column `k`.
        let Some(pivot) = (k..d).find(|&r| {
            r.checked_mul(d)
                .and_then(|i| i.checked_add(k))
                .and_then(|i| m.get(i))
                .is_some_and(|x| !x.is_zero())
        }) else {
            return Some(T::zero());
        };
        if pivot != k {
            let (top, bottom) = m.split_at_mut_checked(pivot.checked_mul(d)?)?;
            let start = k.checked_mul(d)?;
            top.get_mut(start..start.checked_add(d)?)?
                .swap_with_slice(bottom.get_mut(..d)?);
            negative = !negative;
        }
        let (upper, lower) = m.split_at_mut_checked(k.checked_add(1)?.checked_mul(d)?)?;
        let pivot_row = upper.get(k.checked_mul(d)?..)?;
        let pivot_value = pivot_row.get(k)?;
        for row in lower.chunks_exact_mut(d) {
            let lead = row.get(k)?.clone();
            for (x, entry) in row.iter_mut().zip(pivot_row).skip(k.checked_add(1)?) {
                x.eliminate(pivot_value, &lead, entry, &previous)?;
            }
        }
        previous = pivot_value.clone();
    }
    let last = m.last()?.clone();
    if negative { last.negated() } else { Some(last) }
}

impl Classify for Regularity<'_> {
    type Error = RegularError;

    /// Whether the triangulation is regular ([`Regularity::is_regular`]).
    fn is_target(&self, simplices: &[u32]) -> Result<bool, RegularError> {
        self.is_regular(simplices)
    }

    fn classify(&self, simplices: &[u32]) -> Result<Classification, RegularError> {
        self.evaluate(simplices, self.learn)
    }
}

#[cfg(test)]
mod tests;

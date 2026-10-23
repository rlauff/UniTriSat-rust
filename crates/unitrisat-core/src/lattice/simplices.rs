//! Full-dimensional lattice simplices of a point set.
//!
//! [`simplices`] is the Rust counterpart of Julia's `all_simplices`: every
//! set of `d + 1` points (`d` the dimension) whose difference matrix has a
//! nonzero determinant, or determinant `±1` for unimodular simplices, in
//! Julia's single-thread order (lexicographic in the point labels).
//!
//! Julia computes a fresh Bareiss determinant of the `d x d` difference
//! matrix for every combination. Here the combinations of a base point are
//! walked depth first, one point per level, and each level keeps its row of
//! the fraction-free (Bareiss) elimination: row `m` holds point `m` minus the
//! base, eliminated by the pivot rows of the levels below it. Moving to the
//! next combination recomputes only the levels that changed, and a level
//! whose row is zero in every remaining column proves that its points are
//! affinely dependent, so every combination with that prefix is skipped.
//! The last level's remaining entry is the determinant up to sign.
//!
//! The arithmetic is exact. By Sylvester's identity every entry of the
//! elimination is a minor of the difference matrix and every division is
//! exact. The machine lane keeps entries in `i64`, computes each update
//! `(x p - l e) / q` in `i64` and, if that overflows, in `i128`, and gives up
//! as soon as an entry does not fit `i64`; the base point is then redone with
//! `rug` integers, which cannot overflow. No overflow can change a result.

use crate::matrix::Matrix;
use rayon::prelude::*;
use rug::Integer;

/// Which nondegenerate simplices to keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Restriction {
    /// Normalized volume 1: determinant `±1` (Julia's `unimodular = true`).
    Unimodular,
    /// Any nonzero volume (Julia's `unimodular = false`).
    Nondegenerate,
}

/// Failure of [`simplices`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SimplicesError {
    /// A point label does not fit `u32`. A [`Matrix`] holds at most 2^24
    /// values, so this needs a larger point set than the pipeline produces.
    #[error("{0} points exceed the supported maximum of 2^32 - 1")]
    TooManyPoints(usize),
    /// An internal invariant of the enumeration failed. This indicates a bug.
    #[error("internal invariant of the simplex enumeration violated")]
    Internal,
}

/// From this many points on, base points are distributed over threads.
const PARALLEL_THRESHOLD: usize = 32;

/// The full-dimensional simplices spanned by the rows of `points`, as a flat
/// list of `d + 1` zero-based, increasing point labels per simplex (the input
/// of [`crate::intersect::intersecting_pairs`]). Simplices are listed in
/// lexicographic order of their labels, independently of the thread count.
/// Fewer than `d + 1` points give an empty list.
pub fn simplices(points: &Matrix, restriction: Restriction) -> Result<Vec<u32>, SimplicesError> {
    let n = points.rows();
    let d = points.columns();
    if d == 0 {
        // A Matrix has at least one column; the elimination needs it.
        return Err(SimplicesError::Internal);
    }
    if u32::try_from(n).is_err() {
        return Err(SimplicesError::TooManyPoints(n));
    }
    // Base points with at least `d` later points.
    let bases = n.saturating_sub(d);
    if bases == 0 {
        // No simplex; return before allocating the d × d elimination.
        return Ok(Vec::new());
    }
    let walk = Walk {
        points: points.values(),
        d,
        last: bases,
        restriction,
    };
    let per_base: Vec<Vec<u32>> = if n < PARALLEL_THRESHOLD || rayon::current_num_threads() <= 1 {
        let mut machine = Elimination::new(d);
        (0..bases)
            .map(|base| walk.base(base, &mut machine))
            .collect::<Result<_, _>>()?
    } else {
        (0..bases)
            .into_par_iter()
            .map_init(
                || Elimination::new(d),
                |machine, base| walk.base(base, machine),
            )
            .collect::<Result<_, _>>()?
    };
    Ok(per_base.concat())
}

/// Exact integers for the elimination.
trait Exact: Clone {
    /// The number 1.
    fn one() -> Self;
    /// `a - b`, or `None` if it does not fit.
    fn difference(a: i64, b: i64) -> Option<Self>;
    /// Whether this is 0.
    fn is_zero(&self) -> bool;
    /// Whether this is 1 or -1.
    fn is_unit(&self) -> bool;
    /// One Bareiss update, `self = (self * pivot - lead * entry) / previous`,
    /// where the division is exact and `previous` nonzero. `None` if the
    /// result does not fit (or `previous` is 0).
    fn eliminate(&mut self, pivot: &Self, lead: &Self, entry: &Self, previous: &Self)
    -> Option<()>;
}

impl Exact for i64 {
    fn one() -> Self {
        1
    }

    fn difference(a: i64, b: i64) -> Option<Self> {
        a.checked_sub(b)
    }

    fn is_zero(&self) -> bool {
        *self == 0
    }

    fn is_unit(&self) -> bool {
        self.unsigned_abs() == 1
    }

    #[inline]
    fn eliminate(
        &mut self,
        pivot: &Self,
        lead: &Self,
        entry: &Self,
        previous: &Self,
    ) -> Option<()> {
        let narrow = self
            .checked_mul(*pivot)
            .zip(lead.checked_mul(*entry))
            .and_then(|(a, b)| a.checked_sub(b))
            .and_then(|v| v.checked_div(*previous));
        *self = match narrow {
            Some(v) => v,
            None => {
                let wide = i128::from(*self)
                    .checked_mul(i128::from(*pivot))?
                    .checked_sub(i128::from(*lead).checked_mul(i128::from(*entry))?)?
                    .checked_div(i128::from(*previous))?;
                i64::try_from(wide).ok()?
            }
        };
        Some(())
    }
}

impl Exact for Integer {
    fn one() -> Self {
        Integer::from(1)
    }

    fn difference(a: i64, b: i64) -> Option<Self> {
        Some(Integer::from(i128::from(a).checked_sub(i128::from(b))?))
    }

    fn is_zero(&self) -> bool {
        self.is_zero()
    }

    fn is_unit(&self) -> bool {
        *self == 1 || *self == -1
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

/// Outcome of loading one level.
enum Level {
    /// The level's points are affinely dependent: skip every extension.
    Dependent,
    /// The level has a pivot: descend.
    Pivot,
    /// The last level: [`Elimination::last`] is the determinant up to sign.
    Last,
}

/// Fraction-free elimination state of one base point, reused across bases.
struct Elimination<T> {
    /// Dimension.
    d: usize,
    /// Row `m` (`d` entries, physical columns): point `m` minus the base,
    /// eliminated by the pivot rows of levels `0..m`.
    rows: Vec<T>,
    /// `previous[m]` divides the updates by pivot row `m`: 1 for `m = 0`,
    /// otherwise the pivot of level `m - 1`.
    previous: Vec<T>,
    /// Column permutation: `order[m]` is the pivot column of level `m`, and
    /// `order[m + 1..]` the columns not yet eliminated after level `m`.
    order: Vec<usize>,
    /// Point labels of the current combination, one per level.
    labels: Vec<usize>,
}

impl<T: Exact> Elimination<T> {
    fn new(d: usize) -> Self {
        Self {
            d,
            rows: vec![T::one(); d.saturating_mul(d)],
            previous: vec![T::one(); d],
            order: (0..d).collect(),
            labels: vec![0; d],
        }
    }

    /// Loads `point - base` into row `level` and eliminates it by the pivot
    /// rows below; at a level below the last one, also picks its pivot.
    /// `None` if an entry does not fit `T`.
    fn load(&mut self, level: usize, point: &[i64], base: &[i64]) -> Option<Level> {
        let d = self.d;
        let (below, rest) = self.rows.split_at_mut(level.checked_mul(d)?);
        let row = rest.get_mut(..d)?;
        for ((x, &p), &b) in row.iter_mut().zip(point).zip(base) {
            *x = T::difference(p, b)?;
        }
        for (k, (pivot_row, previous)) in below.chunks_exact(d).zip(&self.previous).enumerate() {
            let column = *self.order.get(k)?;
            let pivot = pivot_row.get(column)?;
            let lead = row.get(column)?.clone();
            for &c in self.order.get(k.checked_add(1)?..)? {
                row.get_mut(c)?
                    .eliminate(pivot, &lead, pivot_row.get(c)?, previous)?;
            }
        }
        let remaining = self.order.get_mut(level..)?;
        let next = level.checked_add(1)?;
        if next == d {
            return Some(Level::Last);
        }
        let Some(position) = remaining
            .iter()
            .position(|&c| row.get(c).is_some_and(|x| !x.is_zero()))
        else {
            return Some(Level::Dependent);
        };
        remaining.swap(0, position);
        *self.previous.get_mut(next)? = row.get(*remaining.first()?)?.clone();
        Some(Level::Pivot)
    }

    /// The last level's remaining entry: the determinant up to sign, once
    /// every level is loaded.
    fn last(&self) -> Option<&T> {
        let column = *self.order.last()?;
        let start = self.d.checked_sub(1)?.checked_mul(self.d)?;
        self.rows.get(start.checked_add(column)?)
    }
}

/// The point set and the filter, shared by all base points.
struct Walk<'a> {
    /// Row-major points.
    points: &'a [i64],
    /// Dimension.
    d: usize,
    /// Number of base points: `n - d`, so that level `m` runs up to label
    /// `last + m`.
    last: usize,
    /// Which simplices to keep.
    restriction: Restriction,
}

impl Walk<'_> {
    /// Point `label` as a row.
    fn point(&self, label: usize) -> Option<&[i64]> {
        let start = label.checked_mul(self.d)?;
        self.points.get(start..start.checked_add(self.d)?)
    }

    /// The simplices with smallest label `base`, on the machine lane and,
    /// if an entry overflows it, on `rug` integers.
    fn base(
        &self,
        base: usize,
        machine: &mut Elimination<i64>,
    ) -> Result<Vec<u32>, SimplicesError> {
        let mut out = Vec::new();
        if self.walk(base, machine, &mut out).is_some() {
            return Ok(out);
        }
        out.clear();
        self.walk(base, &mut Elimination::<Integer>::new(self.d), &mut out)
            .ok_or(SimplicesError::Internal)?;
        Ok(out)
    }

    /// Appends the simplices `base < labels[0] < ... < labels[d - 1]` in
    /// lexicographic order. `None` if an entry does not fit `T`.
    fn walk<T: Exact>(
        &self,
        base: usize,
        state: &mut Elimination<T>,
        out: &mut Vec<u32>,
    ) -> Option<()> {
        let origin = self.point(base)?;
        let mut level = 0;
        *state.labels.first_mut()? = base.checked_add(1)?;
        loop {
            let label = *state.labels.get(level)?;
            match state.load(level, self.point(label)?, origin)? {
                Level::Pivot => {
                    level = level.checked_add(1)?;
                    *state.labels.get_mut(level)? = label.checked_add(1)?;
                    continue;
                }
                Level::Last => {
                    let determinant = state.last()?;
                    let keep = match self.restriction {
                        Restriction::Nondegenerate => !determinant.is_zero(),
                        Restriction::Unimodular => determinant.is_unit(),
                    };
                    if keep {
                        out.push(u32::try_from(base).ok()?);
                        for &l in &state.labels {
                            out.push(u32::try_from(l).ok()?);
                        }
                    }
                }
                Level::Dependent => {}
            }
            // Next combination: advance the deepest level that can move.
            loop {
                let label = state.labels.get_mut(level)?;
                *label = label.checked_add(1)?;
                if *label <= self.last.checked_add(level)? {
                    break;
                }
                if level == 0 {
                    return Some(());
                }
                level = level.checked_sub(1)?;
            }
        }
    }
}

#[cfg(test)]
mod tests;

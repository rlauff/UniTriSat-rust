//! Flagness of a triangulation.
//!
//! A simplicial complex is *flag* if every clique of its 1-skeleton (a set
//! of vertices that are pairwise joined by edges) is a face; equivalently,
//! its minimal non-faces are edges. Julia's `is_flag_triangulation`
//! (`subdivision_regularity.jl`) tests the equivalent condition
//!
//! > for every face `σ` with at least two vertices, every vertex `v`
//! > adjacent to all of `σ` gives a face `σ ∪ {v}`,
//!
//! which implies flagness by induction on the clique size. It lists every
//! face as a bitmask in a hash set (`u64` or `u128` masks up to 128 points,
//! `BitSet`s and vectors beyond).
//!
//! This is the same test on the same condition (P0.4: rewrite of Julia's
//! `_is_flag_mask`; the legacy checker was 2–23× slower), with every face
//! visited once and no face set:
//! - `σ ∪ {v}` is a face exactly when `v` lies in the union `U(σ)` of the
//!   facets containing `σ`, so the condition reads "the common neighbours
//!   of `σ` all lie in `U(σ)`";
//! - the faces are walked depth first, each from its smallest vertex `u`
//!   upwards: the facets containing `{u}` are the star of `u`, and the
//!   facets containing `σ ∪ {w}` are those of `σ` that contain `w`. The
//!   children of `σ` are `σ ∪ {w}` for the vertices `w` of `U(σ)` above
//!   the largest vertex of `σ`, so every face is reached exactly once;
//! - common neighbours only shrink along the walk, so a face without common
//!   neighbours ends its branch: none of its extensions can fail.
//!
//! Vertex sets are bitmasks of `W` words, with `W` the smallest of 1, 2, 4,
//! 8 or 16 that holds the points. Everything is exact.

use crate::enumerate::Classify;

/// The largest number of points [`Flag`] supports: 16 words of 64 bits.
pub const MAX_POINTS: usize = 1024;

/// Failure of [`Flag::new`] or [`Flag::is_flag`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FlagError {
    /// The labels are not whole simplices of the given width, or the width
    /// is 0.
    #[error("{labels} labels are not whole simplices of {width} vertices")]
    Shape {
        /// Number of labels.
        labels: usize,
        /// Vertices per simplex.
        width: usize,
    },
    /// More points than [`MAX_POINTS`].
    #[error("{0} points exceed the supported maximum of {MAX_POINTS}")]
    TooManyPoints(usize),
    /// A simplex label is not one of the points.
    #[error("label {0} is not a point")]
    UnknownPoint(u32),
    /// A triangulation names a simplex that does not exist.
    #[error("simplex {0} does not exist")]
    UnknownSimplex(u32),
    /// An internal invariant failed. This indicates a bug.
    #[error("internal invariant of the flag check violated")]
    Internal,
}

/// Mask width in 64-bit words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Words {
    One,
    Two,
    Four,
    Eight,
    Sixteen,
}

/// The flag test for triangulations made of a fixed list of simplices. As a
/// [`Classify`] hook, the flag triangulations are the targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flag<'a> {
    /// `width` labels per simplex.
    labels: &'a [u32],
    width: usize,
    /// Labels are below this.
    points: usize,
    words: Words,
}

impl<'a> Flag<'a> {
    /// The flag test for the simplices `labels` (`width` point labels each,
    /// all below `points`), such as those of [`crate::lattice::simplices`]
    /// with `width = d + 1` and the number of lattice points. The labels of
    /// one simplex must be distinct.
    pub fn new(labels: &'a [u32], width: usize, points: usize) -> Result<Self, FlagError> {
        if width == 0 || !labels.len().is_multiple_of(width) {
            return Err(FlagError::Shape {
                labels: labels.len(),
                width,
            });
        }
        let words = match points.div_ceil(64) {
            0..=1 => Words::One,
            2 => Words::Two,
            3..=4 => Words::Four,
            5..=8 => Words::Eight,
            9..=16 => Words::Sixteen,
            _ => return Err(FlagError::TooManyPoints(points)),
        };
        if let Some(&label) = labels
            .iter()
            .find(|&&l| usize::try_from(l).map_or(true, |l| l >= points))
        {
            return Err(FlagError::UnknownPoint(label));
        }
        Ok(Self {
            labels,
            width,
            points,
            words,
        })
    }

    /// Whether the complex made of the simplices with the indices
    /// `simplices` is flag. No simplex gives the empty complex, which is
    /// flag.
    pub fn is_flag(&self, simplices: &[u32]) -> Result<bool, FlagError> {
        match self.words {
            Words::One => self.check::<1>(simplices),
            Words::Two => self.check::<2>(simplices),
            Words::Four => self.check::<4>(simplices),
            Words::Eight => self.check::<8>(simplices),
            Words::Sixteen => self.check::<16>(simplices),
        }
    }

    /// The labels of simplex `s`.
    fn simplex(&self, s: u32) -> Result<&'a [u32], FlagError> {
        let start = usize::try_from(s)
            .ok()
            .and_then(|s| s.checked_mul(self.width))
            .ok_or(FlagError::UnknownSimplex(s))?;
        let end = start
            .checked_add(self.width)
            .ok_or(FlagError::UnknownSimplex(s))?;
        self.labels
            .get(start..end)
            .ok_or(FlagError::UnknownSimplex(s))
    }

    fn check<const W: usize>(&self, simplices: &[u32]) -> Result<bool, FlagError> {
        // The facet masks, the neighbours of each vertex, and the number of
        // facets containing each vertex.
        let mut facets: Vec<Mask<W>> = Vec::with_capacity(simplices.len());
        let mut neighbours = vec![[0_u64; W]; self.points];
        let mut start = vec![0_usize; self.points.saturating_add(1)];
        for &s in simplices {
            let labels = self.simplex(s)?;
            let mut facet = [0_u64; W];
            for &v in labels {
                insert(&mut facet, v)?;
            }
            for &v in labels {
                let mut others = facet;
                remove(&mut others, v)?;
                let slot = index(v);
                union_into(
                    neighbours.get_mut(slot).ok_or(FlagError::Internal)?,
                    &others,
                );
                let count = start.get_mut(slot).ok_or(FlagError::Internal)?;
                *count = count.checked_add(1).ok_or(FlagError::Internal)?;
            }
            facets.push(facet);
        }
        // Stars: the facets containing vertex `v` are
        // `star[start[v]..start[v + 1]]` (counting sort).
        let mut total = 0_usize;
        for slot in &mut start {
            let count = *slot;
            *slot = total;
            total = total.checked_add(count).ok_or(FlagError::Internal)?;
        }
        let mut fill = start.clone();
        let mut star = vec![[0_u64; W]; total];
        for (&s, facet) in simplices.iter().zip(&facets) {
            for &v in self.simplex(s)? {
                let next = fill.get_mut(index(v)).ok_or(FlagError::Internal)?;
                *star.get_mut(*next).ok_or(FlagError::Internal)? = *facet;
                *next = next.checked_add(1).ok_or(FlagError::Internal)?;
            }
        }
        let mut walk = Walk {
            neighbours: &neighbours,
            stack: Vec::with_capacity(total),
        };
        for (u, bounds) in (0_u32..).zip(start.windows(2)) {
            let (&from, &to) = (
                bounds.first().ok_or(FlagError::Internal)?,
                bounds.get(1).ok_or(FlagError::Internal)?,
            );
            let common = *neighbours.get(index(u)).ok_or(FlagError::Internal)?;
            if is_empty(&common) {
                // Unused vertex, or one that is a facet by itself.
                continue;
            }
            walk.stack.clear();
            walk.stack
                .extend_from_slice(star.get(from..to).ok_or(FlagError::Internal)?);
            if !walk.face(0, false, u, &common)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

impl Classify for Flag<'_> {
    type Error = FlagError;

    fn is_target(&self, simplices: &[u32]) -> Result<bool, FlagError> {
        self.is_flag(simplices)
    }
}

/// A vertex set of up to `64 W` vertices.
type Mask<const W: usize> = [u64; W];

/// The depth-first walk over the faces.
struct Walk<'s, const W: usize> {
    neighbours: &'s [Mask<W>],
    /// The facet lists of the faces on the current path, one segment each.
    stack: Vec<Mask<W>>,
}

impl<const W: usize> Walk<'_, W> {
    /// Checks the face `σ` whose facets are `stack[from..]`, whose largest
    /// vertex is `top`, and whose common neighbours are `common`, and then
    /// every face reached from it. `checked` is false for single vertices,
    /// which always pass. Returns false at the first violation.
    fn face(
        &mut self,
        from: usize,
        checked: bool,
        top: u32,
        common: &Mask<W>,
    ) -> Result<bool, FlagError> {
        let end = self.stack.len();
        let mut union = [0_u64; W];
        for facet in self.stack.get(from..).ok_or(FlagError::Internal)? {
            union_into(&mut union, facet);
        }
        if checked && !is_subset(common, &union) {
            // A common neighbour v with σ ∪ {v} not a face: a clique of the
            // 1-skeleton that is not a face.
            return Ok(false);
        }
        let mut children = above(&union, top);
        while let Some(w) = pop_lowest(&mut children) {
            let mut next = *common;
            intersect_into(
                &mut next,
                self.neighbours.get(index(w)).ok_or(FlagError::Internal)?,
            );
            if is_empty(&next) {
                // No common neighbour, here or in any extension.
                continue;
            }
            for i in from..end {
                let facet = *self.stack.get(i).ok_or(FlagError::Internal)?;
                if contains(&facet, w) {
                    self.stack.push(facet);
                }
            }
            let ok = self.face(end, true, w, &next)?;
            self.stack.truncate(end);
            if !ok {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// `v` as an index. Labels are checked against the point count, which fits
/// `usize`.
fn index(v: u32) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

/// The word of `v` and its bit in that word.
fn position(v: u32) -> (usize, u64) {
    (index(v.wrapping_shr(6)), 1_u64.wrapping_shl(v & 63))
}

fn insert<const W: usize>(mask: &mut Mask<W>, v: u32) -> Result<(), FlagError> {
    let (word, bit) = position(v);
    *mask.get_mut(word).ok_or(FlagError::UnknownPoint(v))? |= bit;
    Ok(())
}

fn remove<const W: usize>(mask: &mut Mask<W>, v: u32) -> Result<(), FlagError> {
    let (word, bit) = position(v);
    *mask.get_mut(word).ok_or(FlagError::UnknownPoint(v))? &= !bit;
    Ok(())
}

fn contains<const W: usize>(mask: &Mask<W>, v: u32) -> bool {
    let (word, bit) = position(v);
    mask.get(word).is_some_and(|w| w & bit != 0)
}

fn union_into<const W: usize>(a: &mut Mask<W>, b: &Mask<W>) {
    for (x, y) in a.iter_mut().zip(b) {
        *x |= y;
    }
}

fn intersect_into<const W: usize>(a: &mut Mask<W>, b: &Mask<W>) {
    for (x, y) in a.iter_mut().zip(b) {
        *x &= y;
    }
}

fn is_empty<const W: usize>(a: &Mask<W>) -> bool {
    a.iter().all(|&x| x == 0)
}

fn is_subset<const W: usize>(a: &Mask<W>, b: &Mask<W>) -> bool {
    a.iter().zip(b).all(|(x, y)| x & !y == 0)
}

/// The vertices of `mask` above `top`.
fn above<const W: usize>(mask: &Mask<W>, top: u32) -> Mask<W> {
    let (word, bit) = position(top);
    // The bits above `bit` in its word: all bits not at or below it.
    let higher = !(bit | bit.wrapping_sub(1));
    let mut result = *mask;
    for (i, x) in result.iter_mut().enumerate() {
        if i < word {
            *x = 0;
        } else if i == word {
            *x &= higher;
        }
    }
    result
}

/// Removes and returns the smallest vertex of `mask`.
fn pop_lowest<const W: usize>(mask: &mut Mask<W>) -> Option<u32> {
    for (i, x) in (0_u32..).zip(mask.iter_mut()) {
        if *x != 0 {
            let v = i.wrapping_shl(6) | x.trailing_zeros();
            *x &= x.wrapping_sub(1);
            return Some(v);
        }
    }
    None
}

#[cfg(test)]
mod tests;

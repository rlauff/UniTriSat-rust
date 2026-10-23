//! Codimension-1 faces of the simplices, and which of them are internal.
//!
//! Julia's `internal_faces` lists every set of `d` lattice points (`d` the
//! dimension) that does not lie on one facet hyperplane of the polytope, by
//! testing all `C(n, d)` point sets against all facets. Its only consumer of
//! the face covering clauses, `compute_face_clauses`, then scans every
//! simplex for every internal face of every simplex (quadratic, REVIEW.md
//! R26).
//!
//! [`faces`] instead lists each distinct face of the given simplices once,
//! with the sorted simplices containing it and whether it is internal. The
//! test is Julia's: a face is on the boundary if and only if all of its
//! points satisfy `normal · x = bound` for one common facet. It is decided
//! by precomputing, for every point, the bit set of facets it lies on; a
//! face is internal if and only if the bit sets of its points have an empty
//! intersection. The faces are grouped by one sort of the `(d + 1) m` face
//! slots of the `m` simplices, so everything is `O(d m log(d m))` plus the
//! point-facet incidence.

use crate::hull::Facet;
use crate::matrix::Matrix;

/// Failure of [`faces`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FacesError {
    /// A facet normal does not have one entry per point coordinate.
    #[error("a facet normal has {found} entries for {expected} coordinates")]
    Dimension {
        /// Number of point coordinates.
        expected: usize,
        /// Entries of the offending normal.
        found: usize,
    },
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
    /// A facet value `normal · x` does not fit `i128`.
    #[error("a facet value does not fit a 128-bit integer")]
    Overflow,
    /// The simplices have more face slots than a `u32` index supports.
    #[error("{0} face slots exceed the supported maximum of 2^32 - 1")]
    TooManyFaces(usize),
    /// An internal invariant failed. This indicates a bug.
    #[error("internal invariant of the face computation violated")]
    Internal,
}

/// The distinct codimension-1 faces of a list of simplices.
///
/// Faces are numbered `0..len()` in lexicographic order of their point
/// labels. Each simplex has `d + 1` face slots: slot `k` is the face
/// opposite its `k`-th vertex (the simplex without its `k`-th label).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Faces {
    /// Points per face, `d`.
    width: usize,
    /// `width` increasing point labels per face.
    labels: Vec<u32>,
    /// Whether each face is internal.
    internal: Vec<bool>,
    /// The simplices containing face `f` are
    /// `simplices[offsets[f]..offsets[f + 1]]`.
    offsets: Vec<usize>,
    /// Containing simplices, increasing within each face.
    simplices: Vec<u32>,
    /// `width + 1` face numbers per simplex, one per slot.
    slots: Vec<u32>,
}

/// One face of [`Faces`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Face<'a> {
    /// The `d` point labels of the face, increasing.
    pub labels: &'a [u32],
    /// Whether the face is internal: its points do not all lie on one facet
    /// of the polytope (Julia's `internal_faces` membership).
    pub internal: bool,
    /// The simplices containing the face, increasing. An internal face of
    /// simplex `s` gives Julia's face clause `-s ∨ t_1 ∨ ...` over the other
    /// simplices `t_i` listed here.
    pub simplices: &'a [u32],
}

impl Faces {
    /// Number of distinct faces.
    pub fn len(&self) -> usize {
        self.internal.len()
    }

    /// Whether there are no faces (no simplices).
    pub fn is_empty(&self) -> bool {
        self.internal.is_empty()
    }

    /// Face number `face`, if it exists.
    pub fn get(&self, face: u32) -> Option<Face<'_>> {
        let f = usize::try_from(face).ok()?;
        let start = f.checked_mul(self.width)?;
        let labels = self.labels.get(start..start.checked_add(self.width)?)?;
        let internal = *self.internal.get(f)?;
        let from = *self.offsets.get(f)?;
        let to = *self.offsets.get(f.checked_add(1)?)?;
        let simplices = self.simplices.get(from..to)?;
        Some(Face {
            labels,
            internal,
            simplices,
        })
    }

    /// All faces in order.
    pub fn iter(&self) -> impl Iterator<Item = Face<'_>> {
        (0..u32::try_from(self.len()).unwrap_or(u32::MAX)).filter_map(|f| self.get(f))
    }

    /// The face numbers of simplex `simplex`, one per slot (slot `k` is the
    /// face opposite vertex `k`), if the simplex exists.
    pub fn of_simplex(&self, simplex: u32) -> Option<&[u32]> {
        let per_simplex = self.width.checked_add(1)?;
        let start = usize::try_from(simplex).ok()?.checked_mul(per_simplex)?;
        self.slots.get(start..start.checked_add(per_simplex)?)
    }
}

/// The distinct codimension-1 faces of `simplices` (`d + 1` zero-based point
/// labels per simplex, as returned by [`crate::lattice::simplices`]), each
/// classified as internal or boundary against the polytope's `facets` (as
/// returned by [`crate::hull::facets`]) and listed with the simplices
/// containing it.
pub fn faces(points: &Matrix, facets: &[Facet], simplices: &[u32]) -> Result<Faces, FacesError> {
    let d = points.columns();
    if let Some(facet) = facets.iter().find(|f| f.normal.len() != d) {
        return Err(FacesError::Dimension {
            expected: d,
            found: facet.normal.len(),
        });
    }
    let per_simplex = d.checked_add(1).ok_or(FacesError::Internal)?;
    if simplices.len().checked_rem(per_simplex) != Some(0) {
        return Err(FacesError::Shape {
            len: simplices.len(),
            width: per_simplex,
        });
    }
    if let Some(&label) = simplices
        .iter()
        .find(|&&l| usize::try_from(l).map_or(true, |l| l >= points.rows()))
    {
        return Err(FacesError::UnknownPoint(label));
    }
    let slot_count = simplices.len();
    let slot_count_u32 =
        u32::try_from(slot_count).map_err(|_| FacesError::TooManyFaces(slot_count))?;
    let words = facets.len().div_ceil(64).max(1);
    let on_facets = incidence(points, facets, words)?;

    // The labels of every face slot: slot `k` of a simplex drops label `k`.
    let mut slot_labels =
        Vec::with_capacity(slot_count.checked_mul(d).ok_or(FacesError::Internal)?);
    for simplex in simplices.chunks_exact(per_simplex) {
        for k in 0..per_simplex {
            slot_labels.extend(
                simplex
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &l)| (i != k).then_some(l)),
            );
        }
    }
    // Slots by face, and by slot number (so by simplex) within a face.
    let order =
        sorted_slots(&slot_labels, d, slot_count_u32, points.rows()).ok_or(FacesError::Internal)?;

    let per_simplex_u32 = u32::try_from(per_simplex).map_err(|_| FacesError::Internal)?;
    let mut faces = Faces {
        width: d,
        labels: Vec::new(),
        internal: Vec::new(),
        offsets: vec![0],
        simplices: Vec::with_capacity(slot_count),
        slots: vec![0; slot_count],
    };
    let mut previous: Option<&[u32]> = None;
    let mut face = 0u32;
    for &s in &order {
        let labels = row(&slot_labels, d, s).ok_or(FacesError::Internal)?;
        if previous != Some(labels) {
            if previous.is_some() {
                faces.offsets.push(faces.simplices.len());
                face = face.checked_add(1).ok_or(FacesError::Internal)?;
            }
            faces.labels.extend_from_slice(labels);
            faces
                .internal
                .push(!on_common_facet(&on_facets, words, labels).ok_or(FacesError::Internal)?);
            previous = Some(labels);
        }
        faces
            .simplices
            .push(s.checked_div(per_simplex_u32).ok_or(FacesError::Internal)?);
        *faces
            .slots
            .get_mut(usize::try_from(s).map_err(|_| FacesError::Internal)?)
            .ok_or(FacesError::Internal)? = face;
    }
    if previous.is_some() {
        faces.offsets.push(faces.simplices.len());
    }
    Ok(faces)
}

/// The slot numbers `0..count` sorted by their labels (`width` per slot in
/// `slot_labels`, each below `points`), ties by slot number. When the labels
/// of a slot fit 96 bits, they are packed with the slot number into one
/// `u128` key, so the sort compares plain integers; otherwise it compares
/// label slices.
fn sorted_slots(slot_labels: &[u32], width: usize, count: u32, points: usize) -> Option<Vec<u32>> {
    let bits = usize::BITS.checked_sub(points.saturating_sub(1).leading_zeros())?;
    let packed_bits = u32::try_from(width).ok()?.checked_mul(bits)?;
    if packed_bits > 96 {
        let slot = |s: u32| -> &[u32] { row(slot_labels, width, s).unwrap_or_default() };
        let mut order: Vec<u32> = (0..count).collect();
        order.sort_unstable_by(|&a, &b| slot(a).cmp(slot(b)).then(a.cmp(&b)));
        return Some(order);
    }
    let mut keys = Vec::with_capacity(usize::try_from(count).ok()?);
    for (s, labels) in (0..count).zip(slot_labels.chunks_exact(width)) {
        let mut key = 0u128;
        for &l in labels {
            key = key.checked_shl(bits)? | u128::from(l);
        }
        keys.push(key.checked_shl(32)? | u128::from(s));
    }
    keys.sort_unstable();
    // The low 32 bits are the slot number.
    keys.iter()
        .map(|&k| u32::try_from(k & u128::from(u32::MAX)).ok())
        .collect()
}

/// Row `index` of a row-major list with `width` entries per row.
fn row(values: &[u32], width: usize, index: u32) -> Option<&[u32]> {
    let start = usize::try_from(index).ok()?.checked_mul(width)?;
    values.get(start..start.checked_add(width)?)
}

/// For every point, `words` 64-bit words whose bit `j` is set if the point
/// lies on facet `j` (`normal · x = bound`).
fn incidence(points: &Matrix, facets: &[Facet], words: usize) -> Result<Vec<u64>, FacesError> {
    let size = points
        .rows()
        .checked_mul(words)
        .ok_or(FacesError::Internal)?;
    let mut masks = vec![0u64; size];
    for (mask, point) in masks.chunks_exact_mut(words).zip(points.iter_rows()) {
        for (word, group) in mask.iter_mut().zip(facets.chunks(64)) {
            for (bit, facet) in (0u32..).zip(group) {
                let mut value: i128 = 0;
                for (&a, &x) in facet.normal.iter().zip(point) {
                    let term = i128::from(a)
                        .checked_mul(i128::from(x))
                        .ok_or(FacesError::Overflow)?;
                    value = value.checked_add(term).ok_or(FacesError::Overflow)?;
                }
                if value == i128::from(facet.bound) {
                    *word |= 1u64.checked_shl(bit).ok_or(FacesError::Internal)?;
                }
            }
        }
    }
    Ok(masks)
}

/// Whether all points of `face` lie on one common facet. `None` if a label
/// is not a point of `masks`.
fn on_common_facet(masks: &[u64], words: usize, face: &[u32]) -> Option<bool> {
    for w in 0..words {
        let mut common = u64::MAX;
        for &label in face {
            let index = usize::try_from(label)
                .ok()?
                .checked_mul(words)?
                .checked_add(w)?;
            common &= *masks.get(index)?;
        }
        if common != 0 {
            return Some(true);
        }
    }
    Some(false)
}

#[cfg(test)]
mod tests;

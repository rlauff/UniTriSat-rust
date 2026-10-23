//! The SAT formula whose models are the triangulations.
//!
//! Port of the formula that Julia's `process_polytope` hands to the solver
//! when incremental solving is off. Variable `s + 1` stands for simplex `s`
//! (zero-based, in the order of [`crate::lattice::simplices`]), so the
//! variables are the dense range `1..=n` that [`crate::solve::Solver`]
//! requires. The clauses are, in Julia's order:
//! 1. **non-emptiness:** `1 ∨ 2 ∨ ... ∨ n`, some simplex is used;
//! 2. **intersection:** `-(i + 1) ∨ -(j + 1)` for every pair `(i, j)` from
//!    [`crate::intersect::intersecting_pairs`], in its lexicographic order
//!    (`compute_intersections_standard`);
//! 3. **face covering:** for each simplex `s` and each internal face of `s`,
//!    `-(s + 1) ∨ (t_1 + 1) ∨ ...` over the other simplices `t_i` containing
//!    that face, increasing (`compute_face_clauses`): a used simplex needs a
//!    neighbour across each of its internal faces. Per simplex, the faces
//!    come in Julia's `combinations` order, which drops the last vertex
//!    first.
//!
//! This is Julia's formula clause for clause, in the order Julia produces
//! with one thread. With more threads Julia's order depends on task
//! scheduling; here it does not. The face clauses come from the
//! face-to-simplices lookup of [`crate::lattice::faces`] instead of Julia's
//! scan of all simplices per face (REVIEW.md R26).
//!
//! [`with_flag`] optionally restricts these models to flag complexes using
//! shared face-presence variables and ordered clique-extension clauses.
//! Simplex variables remain the prefix `1..=simplex_variables()`; auxiliary
//! variables follow them and are excluded from enumeration output.
//!
//! [`reduced`] is the smaller formula of Julia's incremental mode
//! (`compute_intersections_incremental`), which replaces the intersection
//! clauses by clauses that only need the central simplices and the faces.
//! The central-simplex split of the parallel mode adds one clause to
//! [`formula`] per subproblem (see [`crate::enumerate`]).

use crate::lattice::{Central, Faces};
use crate::solve::MAX_VARIABLES;

mod flag_sat;
pub use flag_sat::with_flag;

/// Failure of [`formula`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CnfError {
    /// More simplices than the solvers accept variables.
    #[error("{0} simplices exceed the solver limit of {MAX_VARIABLES} variables")]
    TooManySimplices(usize),
    /// Simplex and auxiliary variables exceed the solver index limit.
    #[error("{0} total variables exceed the solver limit of {MAX_VARIABLES}")]
    TooManyVariables(usize),
    /// The flag encoding needs one whole simplex per primary variable.
    #[error("{labels} labels of width {width} do not describe {simplices} simplices")]
    FlagShape {
        /// Supplied labels.
        labels: usize,
        /// Vertices per simplex.
        width: usize,
        /// Required simplex count.
        simplices: u32,
    },
    /// A vertex label exceeds the supplied point count.
    #[error("label {label} is not one of {points} points")]
    FlagPoint {
        /// Invalid vertex label.
        label: u32,
        /// Number of points.
        points: usize,
    },
    /// Each simplex must contain distinct vertices.
    #[error("simplex contains repeated vertex {0}")]
    RepeatedPoint(u32),
    /// An intersecting pair is not `(i, j)` with `i < j < n`.
    #[error("({first}, {second}) is not a pair i < j of the {simplices} simplices")]
    Pair {
        /// First simplex of the pair.
        first: u32,
        /// Second simplex of the pair.
        second: u32,
        /// Number of simplices.
        simplices: usize,
    },
    /// The faces were not computed from `n` simplices.
    #[error("the faces do not belong to {0} simplices")]
    Faces(usize),
}

/// A formula in conjunctive normal form over the variables
/// `1..=variables()`, with DIMACS literals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cnf {
    /// Number of variables.
    variables: u32,
    /// Primary simplex variables; auxiliary variables follow this prefix.
    simplex_variables: u32,
    /// The literals of all clauses, one clause after the other.
    literals: Vec<i32>,
    /// Clause `c` ends at `ends[c]` (exclusive) in `literals`.
    ends: Vec<usize>,
}

impl Cnf {
    /// Number of variables: every literal names a variable in
    /// `1..=variables()`.
    pub fn variables(&self) -> u32 {
        self.variables
    }

    /// Number of primary simplex variables. Only this prefix belongs in
    /// triangulations and model-blocking clauses.
    pub fn simplex_variables(&self) -> u32 {
        self.simplex_variables
    }

    /// Number of clauses.
    pub fn len(&self) -> usize {
        self.ends.len()
    }

    /// Whether there are no clauses.
    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    /// The clauses in order.
    pub fn iter(&self) -> impl Iterator<Item = &[i32]> {
        let starts = std::iter::once(0).chain(self.ends.iter().copied());
        starts
            .zip(&self.ends)
            .map(|(start, &end)| self.literals.get(start..end).unwrap_or_default())
    }

    /// Builds a synthetic solver-control fixture without geometric preprocessing.
    #[cfg(test)]
    pub(crate) fn test_clauses(variables: u32, clauses: &[Vec<i32>]) -> Self {
        let mut result = Self {
            variables,
            simplex_variables: variables,
            literals: Vec::new(),
            ends: Vec::new(),
        };
        for clause in clauses {
            result.literals.extend_from_slice(clause);
            result.end_clause();
        }
        result
    }

    /// Ends the clause made of the literals pushed since the previous one.
    fn end_clause(&mut self) {
        self.ends.push(self.literals.len());
    }
}

/// Julia's formula for `simplices` simplices with the intersecting `pairs`
/// (as returned by [`crate::intersect::intersecting_pairs`]) and the `faces`
/// of the same simplices (as returned by [`crate::lattice::faces`]).
///
/// With no simplices the formula is the empty clause, which is
/// unsatisfiable: no triangulation. Julia returns that answer before
/// solving.
pub fn formula(simplices: usize, pairs: &[(u32, u32)], faces: &Faces) -> Result<Cnf, CnfError> {
    let (face_clauses, face_literals) = face_clause_size(faces);
    let clauses = pairs.len().saturating_add(face_clauses).saturating_add(1);
    let literals = simplices
        .saturating_add(pairs.len().saturating_mul(2))
        .saturating_add(face_literals);
    let mut builder = Builder::new(simplices, faces, clauses, literals)?;
    builder.non_emptiness()?;
    builder.pairs(pairs.iter().copied())?;
    builder.face_clauses(faces)?;
    Ok(builder.cnf)
}

/// Julia's reduced formula of incremental solving, for the same simplices
/// and faces as [`formula`] and their [`Central`] set
/// ([`crate::lattice::central`]). Its clauses are, in this order:
/// 1. **non-emptiness**, as in [`formula`];
/// 2. **central:** `c_1 ∨ c_2 ∨ ...` over the central simplices, and
///    `-c_i ∨ -c_j` for every two of them: exactly one central simplex is
///    used. With no central simplex this is the empty clause: the simplices
///    do not cover the polytope (Julia raises an error instead);
/// 3. **separation:** `-(s + 1) ∨ -(t + 1)` for every two simplices
///    `s < t` that share a face with their opposite vertices on the same
///    side of it (they overlap near the face), sorted, each pair once and
///    none that the central clauses already contain;
/// 4. **face covering**, as in [`formula`].
///
/// Its models are the same triangulations as those of [`formula`]: the
/// face clauses and separation clauses make the simplices meet face to face
/// without overlapping across a face, and exactly one simplex covers the
/// generic point, so every point is covered exactly once. The intersection
/// clauses of [`formula`] are all satisfied by every model, so incremental
/// solving adds them while it solves, to speed up propagation.
///
/// Julia collects the separation clauses per thread and then removes
/// duplicates, so its order depends on scheduling; here they are sorted.
pub fn reduced(simplices: usize, faces: &Faces, central: &Central) -> Result<Cnf, CnfError> {
    let mismatch = CnfError::Faces(simplices);
    let center = central.simplices();
    let mut separation = Vec::new();
    let mut sides = [Vec::new(), Vec::new()];
    for (f, face) in (0u32..).zip(faces.iter()) {
        sides.iter_mut().for_each(Vec::clear);
        for &s in face.simplices {
            let slot = faces
                .of_simplex(s)
                .and_then(|slots| slots.iter().position(|&g| g == f))
                .ok_or(mismatch)?;
            let positive = central.apex_positive(s, slot).ok_or(mismatch)?;
            sides
                .get_mut(usize::from(positive))
                .ok_or(mismatch)?
                .push(s);
        }
        for side in &sides {
            for (a, &s) in side.iter().enumerate() {
                for &t in side.iter().skip(a.saturating_add(1)) {
                    let both_central =
                        center.binary_search(&s).is_ok() && center.binary_search(&t).is_ok();
                    if !both_central {
                        separation.push((s, t));
                    }
                }
            }
        }
    }
    separation.sort_unstable();
    separation.dedup();

    let central_pairs = center
        .len()
        .saturating_mul(center.len().saturating_sub(1))
        .checked_div(2)
        .unwrap_or(0);
    let (face_clauses, face_literals) = face_clause_size(faces);
    let clauses = central_pairs
        .saturating_add(separation.len())
        .saturating_add(face_clauses)
        .saturating_add(2);
    let literals = simplices
        .saturating_add(center.len())
        .saturating_add(central_pairs.saturating_mul(2))
        .saturating_add(separation.len().saturating_mul(2))
        .saturating_add(face_literals);
    let mut builder = Builder::new(simplices, faces, clauses, literals)?;
    builder.non_emptiness()?;
    for &c in center {
        let literal = builder.literal(c)?;
        builder.cnf.literals.push(literal);
    }
    builder.cnf.end_clause();
    let central_pair_list = center.iter().enumerate().flat_map(|(a, &s)| {
        center
            .iter()
            .skip(a.saturating_add(1))
            .map(move |&t| (s, t))
    });
    builder.pairs(central_pair_list)?;
    builder.pairs(separation.into_iter())?;
    builder.face_clauses(faces)?;
    Ok(builder.cnf)
}

/// Number of face clauses and an upper bound of their literals.
fn face_clause_size(faces: &Faces) -> (usize, usize) {
    faces
        .iter()
        .filter(|f| f.internal)
        .map(|f| f.simplices.len())
        .fold((0usize, 0usize), |(c, l), k| {
            (c.saturating_add(k), l.saturating_add(k.saturating_mul(k)))
        })
}

/// A formula under construction, with its validated variable count.
struct Builder {
    cnf: Cnf,
    /// The number of simplices as given, for errors.
    simplices: usize,
}

impl Builder {
    /// Checks that `faces` has exactly `simplices` simplices and reserves
    /// room for `clauses` clauses of `literals` literals in all.
    fn new(
        simplices: usize,
        faces: &Faces,
        clauses: usize,
        literals: usize,
    ) -> Result<Self, CnfError> {
        let variables = u32::try_from(simplices)
            .ok()
            .filter(|&n| n <= MAX_VARIABLES)
            .ok_or(CnfError::TooManySimplices(simplices))?;
        let last = variables.checked_sub(1);
        if last.is_some_and(|s| faces.of_simplex(s).is_none())
            || faces.of_simplex(variables).is_some()
        {
            return Err(CnfError::Faces(simplices));
        }
        Ok(Self {
            cnf: Cnf {
                variables,
                simplex_variables: variables,
                literals: Vec::with_capacity(literals),
                ends: Vec::with_capacity(clauses),
            },
            simplices,
        })
    }

    /// The positive literal of simplex `s`; it fits `i32` because
    /// MAX_VARIABLES does.
    fn literal(&self, s: u32) -> Result<i32, CnfError> {
        if s >= self.cnf.variables {
            return Err(CnfError::Faces(self.simplices));
        }
        i32::try_from(s)
            .ok()
            .and_then(|s| s.checked_add(1))
            .ok_or(CnfError::TooManySimplices(self.simplices))
    }

    /// `1 ∨ 2 ∨ ... ∨ n`.
    fn non_emptiness(&mut self) -> Result<(), CnfError> {
        for s in 0..self.cnf.variables {
            let literal = self.literal(s)?;
            self.cnf.literals.push(literal);
        }
        self.cnf.end_clause();
        Ok(())
    }

    /// `-(i + 1) ∨ -(j + 1)` for each pair `i < j` of simplices.
    fn pairs(&mut self, pairs: impl Iterator<Item = (u32, u32)>) -> Result<(), CnfError> {
        for (i, j) in pairs {
            if i >= j || j >= self.cnf.variables {
                return Err(CnfError::Pair {
                    first: i,
                    second: j,
                    simplices: self.simplices,
                });
            }
            let clause = [
                self.literal(i)?.wrapping_neg(),
                self.literal(j)?.wrapping_neg(),
            ];
            self.cnf.literals.extend(clause);
            self.cnf.end_clause();
        }
        Ok(())
    }

    /// Julia's face covering clauses (`compute_face_clauses`).
    fn face_clauses(&mut self, faces: &Faces) -> Result<(), CnfError> {
        let mismatch = CnfError::Faces(self.simplices);
        for s in 0..self.cnf.variables {
            let slots = faces.of_simplex(s).ok_or(mismatch)?;
            // Julia's `combinations(simplex, d)` drops the last vertex first.
            for &f in slots.iter().rev() {
                let face = faces.get(f).ok_or(mismatch)?;
                if !face.internal {
                    continue;
                }
                let literal = self.literal(s)?;
                self.cnf.literals.push(literal.wrapping_neg());
                for &t in face.simplices.iter().filter(|&&t| t != s) {
                    let literal = self.literal(t)?;
                    self.cnf.literals.push(literal);
                }
                self.cnf.end_clause();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

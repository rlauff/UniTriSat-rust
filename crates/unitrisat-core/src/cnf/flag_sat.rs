//! Static flagness constraints, independent of the per-model flag checker.

use super::{Cnf, CnfError, MAX_VARIABLES};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Restricts `cnf` to flag complexes on the candidate simplices `labels`.
/// Each simplex consists of `width` distinct vertex labels below `points`.
/// Primary simplex variables are preserved; new variables follow them.
///
/// For every candidate face F with at least two vertices, a shared literal
/// p_F is equivalent to the disjunction of simplex variables containing F.
/// A face supported by just one simplex reuses that simplex's literal.
/// For every vertex v greater than max(F), the encoding requires
/// `p_F ∧ (∧_{u in F} p_{uv}) → p_{F ∪ {v}}`. A nonexistent extension is
/// false; a nonexistent edge makes the implication vacuous.
///
/// These clauses are necessary for flagness. Conversely, induction on clique
/// size proves sufficiency: remove the greatest vertex of a clique; its
/// remaining face exists by induction, so the clause forces the full face.
/// Full-size faces are included, excluding cliques larger than a simplex.
/// Every simplex assignment uniquely determines all auxiliary variables.
///
/// With S simplices of width w there are at most S(2^w - w - 1) face
/// incidences. Construction uses O(S w 2^w) support data and at most
/// O(n S w 2^w) extension literals, with expected constant-time hash lookups
/// (and deterministic sorting of the face table). At fixed dimension this
/// is quadratic in S since at most wS vertices occur. Dependence on dimension
/// is exponential; no triples of simplices are enumerated.
pub fn with_flag(
    mut cnf: Cnf,
    labels: &[u32],
    width: usize,
    points: usize,
) -> Result<Cnf, CnfError> {
    if width == 0
        || usize::try_from(cnf.simplex_variables)
            .ok()
            .and_then(|n| n.checked_mul(width))
            != Some(labels.len())
    {
        return Err(CnfError::FlagShape {
            labels: labels.len(),
            width,
            simplices: cnf.simplex_variables,
        });
    }
    let mut supports: BTreeMap<Vec<u32>, Vec<i32>> = BTreeMap::new();
    for (simplex, vertices) in (1_i32..).zip(labels.chunks_exact(width)) {
        let mut vertices = vertices.to_vec();
        vertices.sort_unstable();
        for &label in &vertices {
            if usize::try_from(label).map_or(true, |v| v >= points) {
                return Err(CnfError::FlagPoint { label, points });
            }
        }
        if let Some(pair) = vertices.windows(2).find(|pair| pair.first() == pair.last()) {
            return Err(CnfError::RepeatedPoint(pair.first().copied().unwrap_or(0)));
        }
        collect_faces(
            &vertices,
            &mut Vec::with_capacity(width),
            simplex,
            &mut supports,
        );
    }

    let mut faces = Vec::with_capacity(supports.len());
    let mut presence = HashMap::with_capacity(supports.len());
    let mut neighbours: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for (face, support) in supports {
        let literal = if support.len() == 1 {
            support.first().copied().unwrap_or(0)
        } else {
            let next = cnf
                .variables
                .checked_add(1)
                .filter(|&n| n <= MAX_VARIABLES)
                .ok_or(CnfError::TooManyVariables(
                    (cnf.variables as usize).saturating_add(1),
                ))?;
            cnf.variables = next;
            let literal =
                i32::try_from(next).map_err(|_| CnfError::TooManyVariables(next as usize))?;
            for &simplex in &support {
                cnf.literals.extend([simplex.wrapping_neg(), literal]);
                cnf.end_clause();
            }
            cnf.literals.push(literal.wrapping_neg());
            cnf.literals.extend(support);
            cnf.end_clause();
            literal
        };
        if let [u, v] = face.as_slice() {
            // Only increasing extensions are needed.
            neighbours.entry(*u).or_default().insert(*v);
        }
        presence.insert(face.clone(), literal);
        faces.push((face, literal));
    }

    let mut clause = Vec::with_capacity(width.saturating_add(2));
    let mut extension = Vec::with_capacity(width.saturating_add(1));
    for (face, literal) in &faces {
        // Pick the shortest candidate-neighbour list. A missing list means
        // that no vertex above max(F) can be adjacent to every vertex of F.
        let candidates = face
            .iter()
            .map(|u| neighbours.get(u))
            .collect::<Option<Vec<_>>>();
        let Some(candidates) =
            candidates.and_then(|sets| sets.into_iter().min_by_key(|set| set.len()))
        else {
            continue;
        };
        let top = face.last().copied().unwrap_or(0);
        for &v in candidates.range((std::ops::Bound::Excluded(top), std::ops::Bound::Unbounded)) {
            clause.clear();
            clause.push(literal.wrapping_neg());
            let adjacent = face.iter().all(|&u| {
                if let Some(&edge) = presence.get([u, v].as_slice()) {
                    clause.push(edge.wrapping_neg());
                    true
                } else {
                    false
                }
            });
            if !adjacent {
                continue;
            }
            extension.clear();
            extension.extend_from_slice(face);
            extension.push(v);
            if let Some(&larger) = presence.get(extension.as_slice()) {
                // Shared support can make an implication tautological.
                if clause.contains(&larger.wrapping_neg()) {
                    continue;
                }
                clause.push(larger);
            }
            clause.sort_unstable();
            clause.dedup();
            cnf.literals.extend_from_slice(&clause);
            cnf.end_clause();
        }
    }
    Ok(cnf)
}

/// Enumerates subsets without a machine-word mask or a dimension bound.
fn collect_faces(
    remaining: &[u32],
    face: &mut Vec<u32>,
    simplex: i32,
    supports: &mut BTreeMap<Vec<u32>, Vec<i32>>,
) {
    for (i, &vertex) in remaining.iter().enumerate() {
        face.push(vertex);
        if face.len() >= 2 {
            supports.entry(face.clone()).or_default().push(simplex);
        }
        if let Some(tail) = remaining.get(i.saturating_add(1)..) {
            collect_faces(tail, face, simplex, supports);
        }
        face.pop();
    }
}

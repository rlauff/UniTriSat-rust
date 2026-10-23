# SAT encoding of flagness

`--flag-sat` restricts the SAT formula to flag triangulations. It is independent
of `--flag`, which still checks each enumerated triangulation and rejects
non-flag ones. Combining them keeps the independent check and prints a warning
that it is redundant. The library option is `job::Settings::flag_sat`.

The encoding works with serial, split and incremental solving, and with CPU or
GPU intersection generation. Kissat supports it in first-solution serial/split
searches; its existing restrictions on `--flag`, `--regular`, `--all` and
incremental mode still apply. Boolean resume settings are restored unless
explicitly overridden, for example `--flag-sat=false`. Enabling it on resume
keeps completed results and warns about the changed request.

## Encoding and correctness

Let the candidate simplices have width `w = d + 1`. Their selected subsets
define a simplicial complex. A complex is flag exactly when every clique of
its edge graph is a face, including cliques larger than a candidate simplex.

For each distinct candidate face `F` of size at least two, define `p_F` to mean
that some selected candidate simplex contains `F`. Encode this as an equivalence
with the OR of those simplex variables. If only one candidate contains `F`,
reuse that candidate's variable. Thus the face variables are completely
determined by the simplex selection.

For every such face `F` and vertex `v > max(F)` connected to all of `F` in the
candidate edge graph, add:

```text
not p_F OR (OR over u in F: not p_{u,v}) OR p_{F union {v}}
```

If the extension is not a candidate face, omit the final literal. In particular,
these clauses forbid cliques with `d + 2` vertices. Candidate nonedges need no
clause because they cannot appear in any selected complex.

**Soundness.** An edge is already a face. Inductively, a selected clique of size
at least three has a prefix `F` obtained by deleting its largest vertex. The
prefix is a face by induction, and all its edges to the last vertex are
present. The extension clause therefore forces the whole clique to be a face,
or rejects it if that face is impossible. This covers higher missing faces,
not just missing triangles.

**Completeness.** In a flag complex, whenever a clause's face and incident edges
are present, their union is a clique and hence a face. Every clause is therefore
satisfied. The encoding removes exactly the non-flag complexes.

SAT model extraction and blocking use only the original simplex-variable
prefix, so auxiliary variables never appear as returned simplex indices or
cause duplicate triangulations. The unmodified independent flag checker is
used for equivalence tests and whenever `--flag` is also requested.

## Size and tradeoffs

Write `S` for candidate simplex count and `n` for the vertices occurring in
candidates. There are at most `S * 2^w` face incidences. Ordered extension
constraints use at most `O(n * S * w * 2^w)` literals, with sharing and candidate
edge pruning reducing actual size. Since `n <= w * S`, this is quadratic in
`S` at fixed dimension, rather than enumerating triples of candidate simplices.
Lookup and sorting costs also contribute to construction time.

The dimension factor is exponential. This is not a dimension-independent
quadratic algorithm, and extra clauses can cost more time or memory than
postchecking on some inputs. The feature remains opt-in; no universal runtime
speedup is claimed.

## Counts

With the encoding enabled, the internal encountered-triangulation count equals
the flag count. Human output therefore omits the generic `Triangulatable` tally;
CLI TSV omits `n_triangulations` and the uncomputed classification placeholders:

- `--flag-sat`: `idx`, `status`, `n_flag`, `seconds`.
- `--flag-sat --regular`: `idx`, `status`, `n_flag`, `n_quadratic`, `seconds`.

These combinations are valid and do not themselves produce warnings. The
canonical library row and resume schema retain their internal fields. Non-flag
assignments excluded by SAT are not enumerated and do not increment rejection
counters. `--flag` still reports actual completed checks. An exhausted encoded
search proves no flag triangulation using the selected simplex restriction;
with unimodular simplices it does not exclude other flag triangulations.
Existing cap and timeout rules still make incomplete counts lower bounds.

When combined with `--regular --regular-learn`, learned clauses can also skip
nonregular flag triangulations. The flag count and the
internal encountered-triangulation count are then observed lower bounds on all
flag triangulations using the selected simplex restriction. Requested regular/quadratic target counts retain their
complete-search meaning. See [regularity learning](REGULAR_LEARNING.md).

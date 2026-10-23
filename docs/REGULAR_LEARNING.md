# Exact regularity conflict learning

Use `--regular --regular-learn` to enable learning, optionally together with
`--flag` or `--flag-sat`. The library setting is
`job::Settings::regular_learn`, defaulting to false and requiring `regular`.
`--regular` alone retains its existing per-triangulation exact LP check.

This is a SAT-plus-exact-LP method, not Boolean encoding of height arithmetic.
It works with PicoSAT and CaDiCaL in serial, split and incremental modes, with
CPU or GPU intersection generation and independent polytope workers. Kissat
remains incompatible with regularity filtering because it cannot accept the
additional clauses after a solve.

## What is learned

The existing regularity checker assigns a height to every used point. Each
interior wall shared by selected simplices `s` and `t` requires a strict linear
inequality on those heights. For a finite system of homogeneous strict
inequalities, feasibility is equivalent to `A h >= 1`: scale any strict
solution until every positive margin is at least one. The complete wall system
characterizes regularity of the triangulation.

After a candidate fails that exact LP, learning searches for an exact Farkas
certificate:

```text
y >= 0,  sum(y) = 1,  transpose(A) y = 0.
```

All these conditions are verified using rational arithmetic. Multiplying a
hypothetical feasible `A h >= 1` by `y` would give `0 >= 1`, a contradiction.
Only rows with positive certificate weights are needed. Each retained row
keeps its originating pair of adjacent candidate simplices, including through
row deduplication. Let `U` be the union of those pairs. Learn:

```text
OR over s in U: NOT selected(s)
```

Any triangulation containing all of `U` has the same incompatible wall
constraints, so it cannot be regular. The learned clause therefore excludes
no regular triangulation. A small obstruction can eliminate many candidates
that ordinary blocking would enumerate and reject individually.

Certificate data belongs to the individual classification call. It is not a
shared mutable last-result buffer, which would be unsafe under split solving.
Clauses use original simplex variables, never flag-encoding auxiliary variables,
and are added between SAT calls. No certificate is learned merely because a
flag check rejects a triangulation. A skipped regularity test also cannot
produce a regularity obstruction.

## Counts, limits and resume

Learning changes which rejected candidates are visited. `n_triangulations`,
`n_flag`, and rejection counters count encountered candidates; they are lower
bounds on the corresponding unpruned totals, even when the search completes.
The CLI warns about this, including in quiet or TSV runs.

For a complete uncapped search, regular-only `n_regular` remains the exact
regular target count. With `--regular --flag`, `n_quadratic` remains exact;
`n_regular` retains the existing lower-bound behavior of the combined checker.
With `--regular --flag-sat`, regular counts refer to the flag-restricted search.
With both flag mechanisms, the independent checker still runs.

Solution caps count encountered candidates. A deadline, cap or interruption
continues to make unfinished target counts lower bounds. No formula claims an
exact count for triangulations that learning skipped.

The setting is saved in resume snapshots. Omitted flags inherit it;
`--regular-learn=false` disables it. Old snapshots default it to false. Switching
learning on or off retains completed rows and warns about mixed requests,
because the meaning of observed counts differs. An interrupted polytope restarts;
learned SAT clauses are recomputed, not persisted as trusted constraints.

## Cost and scope

Regular candidates do not need a certificate LP. Nonregular candidates incur
extra exact certificate work, which pays off only if the resulting clauses
avoid enough later search and LP checks. The feature is opt-in and is not
assumed to improve every workload. Certificate construction is skipped when
its dense input would exceed 1,000,000 rational cells (or its size calculation
overflows); the search then uses ordinary whole-model blocking. This bounds
the additional input allocation, though cddlib also uses internal workspace.

The related SAT-plus-LP method in [De Loera, Ferroni, Morales and Rambau,
section 4.3](https://arxiv.org/html/2606.11014v1#S4.SS3) learns certified
obstructions to quadratic coherence. This implementation uses the complete
wall system of the existing regularity checker rather than that necessary-only
condition.

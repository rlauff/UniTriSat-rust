# Bridge protocol 1

The Julia package uses this internal subprocess protocol. The command is:

```
unitrisat-julia --request request.json --response response.json
```

No shell is involved. A nonzero exit or missing/invalid response is a failure;
stderr contains diagnostics. The Julia adapter never treats a failed child as
an empty search. A batch succeeds only when every requested job succeeded.

Request (all fields required, unknown fields rejected):

```json
{
  "protocol": 1,
  "input": {"kind": "matrices", "matrices": [[["0", "0"], ["1", "0"], ["0", "1"]]]},
  "threads": 1,
  "settings": {
    "backend": "cpu",
    "unimodular": true,
    "regular": false,
    "flag": false,
    "find_all": false,
    "keep": "first",
    "solver": "picosat",
    "mode": "split",
    "max_solutions": null,
    "max_regularity_tests": null
  }
}
```

Alternatively, `input` is exactly `{"kind":"file","path":"/absolute/path"}`.
The file uses the existing Rust input parser. Matrices contain vertex rows and
may have different dimensions across a batch. An empty matrix batch succeeds
with no results; individual matrices must be nonempty and rectangular.

`backend` is `cpu|gpu`, `solver` is `picosat|cadical`, `mode` is
`serial|split|incremental`, and `keep` is `none|first|all`. `keep` retains target
triangulations, independently of whether `find_all` asks for exhaustive search.
Caps are null for disabled or positive decimal strings; no finite CLI defaults
are substituted. `threads` is a positive JSON integer and configures the worker
before any Rayon use. Every coordinate is a canonical signed decimal string
fitting i64. Counts are unsigned decimal strings fitting Rust u64; the adapter
also checks that they fit the local Julia Int.

Response:

```json
{
  "protocol": 1,
  "results": [{
    "counts": {"triangulations":"1", "regular":"0", "flag":"0", "quadratic":"0"},
    "status": "ok",
    "seconds": 0.01,
    "solutions": [[[["0","0"],["1","0"],["0","1"]]]],
    "steps": [{"name":"Lattice pipeline", "seconds":0.005}, {"name":"Formula", "seconds":0.002}, {"name":"Search", "seconds":0.003}]
  }],
  "seconds": 0.02
}
```

The `solutions` nesting is retained triangulations → simplices → vertex rows →
coordinate strings. Coordinates are in the pipeline's effective projected
lattice space. Preserve input batch order. Result fields are exact; do not add
fields without changing the version. A step list may be empty when no stage was
performed. Every timing is finite and nonnegative. Stage names describe actual
timed operations; neither side invents allocation measurements.

`status` is `ok|capped|reglimit` with the existing Rust job meanings; no skipped
row can occur in a successful bridge response. `ok` includes complete and
first-target outcomes. The adapter warns for lower-bound results because the
original public result layout has no status field. It must not report a
truncated zero as a proved absence.

There is no numerical Julia fallback. GPU availability uses Rust's already
reviewed exact CPU fallback. The original result field layout is preserved, but
this wire protocol is an internal adapter boundary, not the CLI TSV format.

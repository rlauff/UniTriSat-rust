# Plotting helper

`plot_triangulation.py` is copied verbatim from the Julia reference renderer.
`render.py` validates its input and calls its plotting function so malformed
input, missing dependencies and rendering failures produce a nonzero exit.
Julia propagates that failure instead of reporting a successful plot.

Install `numpy` and `plotly` in the Python environment used for plotting. The
default executable is `python3`; set `UNITRISAT_PYTHON` to another executable's
path if needed. The value is one executable, not a shell command with arguments.
The renderer retains Plotly's normal renderer selection (`PLOTLY_RENDERER` can
select one) and its existing 0.8 tetrahedron shrinkage for visibility.

D3 triangulations use axes relative to the first simplex's first vertex. This
common origin is subtracted in exact integer arithmetic before converting the
display copy to floating point, preserving unit edges even when the original
coordinates exceed 2^53. All simplices share the same origin, preserving their
relative placement. Returned coordinates retain their original values.

D4–6 display their boundary 3-faces in separate
views, as before. For these views, install and load `Polyhedra` and `CDDLib` in
the Julia environment (`using Polyhedra, CDDLib`). Hull incidence remains exact;
only the display projection is floating point. Unlike the original glue, it
does not round projected coordinates to integers. Returned simplex matrices
are never changed. Duplicate descriptions of the same boundary face are shown
once. Display projections can still lose precision for ill-conditioned data.

The legacy renderer did not support D2; a nonempty D2 result now produces an
explicit unsupported-dimension error instead of silently doing nothing. An
empty triangulation does nothing in every dimension.

Linux validation executed the real renderer headlessly for D3/D4 and checked
D5/6 boundary extraction. The numerical results are unaffected by display
projection. See [validation scope](../../../docs/VALIDATION.md).

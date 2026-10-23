"""Validate display data, then invoke the unchanged legacy Plotly renderer."""

import json
import math
import sys


def validate_tetrahedra(data):
    if not isinstance(data, list) or not data:
        raise ValueError("expected a nonempty list of tetrahedra")
    for tetrahedron in data:
        if not isinstance(tetrahedron, list) or len(tetrahedron) != 4:
            raise ValueError("every tetrahedron must have four vertices")
        for vertex in tetrahedron:
            if not isinstance(vertex, list) or len(vertex) != 3:
                raise ValueError("every vertex must have three coordinates")
            for coordinate in vertex:
                if isinstance(coordinate, bool) or not isinstance(coordinate, (int, float)):
                    raise ValueError("coordinates must be numbers")
                if not math.isfinite(coordinate):
                    raise ValueError("coordinates must be finite")
    return data


def main(argv):
    if len(argv) != 2:
        raise ValueError("usage: render.py TETRAHEDRA_JSON")
    with open(argv[1], encoding="utf-8") as stream:
        tetrahedra = validate_tetrahedra(json.load(stream))
    try:
        import numpy as np
        from plot_triangulation import plot_triangulation
    except ImportError as error:
        raise RuntimeError(
            "plotting requires numpy and plotly in the UNITRISAT_PYTHON environment"
        ) from error
    # Do not call the legacy script's CLI: it exits successfully on parse
    # failure. Validation and uncaught rendering failures now propagate to Julia.
    plot_triangulation([np.array(tetrahedron, dtype=float) for tetrahedron in tetrahedra])


if __name__ == "__main__":
    try:
        main(sys.argv)
    except Exception as error:
        print(f"UniTriSat plotting failed: {error}", file=sys.stderr)
        sys.exit(1)

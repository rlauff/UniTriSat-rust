module UniTriSatOscarExt
using UniTriSat
using Oscar

function _vertices(polytope::Oscar.Polyhedron)
    Oscar.is_bounded(polytope) || throw(ArgumentError("triangulate requires a bounded polytope"))
    matrix = Oscar.matrix(Oscar.vertices(polytope))
    # Broadcasting over an Oscar matrix preserves its MatSpaceElem container;
    # materialize a Julia Matrix explicitly for the public integer-matrix API.
    return [Int(matrix[row, column]) for row in 1:size(matrix, 1), column in 1:size(matrix, 2)]
end

UniTriSat.triangulate(polytope::Oscar.Polyhedron; kwargs...) =
    UniTriSat.triangulate(_vertices(polytope); kwargs...)
UniTriSat.triangulate(polytopes::AbstractVector{<:Oscar.Polyhedron}; kwargs...) =
    UniTriSat.triangulate(Matrix{Int}[_vertices(p) for p in polytopes]; kwargs...)
end

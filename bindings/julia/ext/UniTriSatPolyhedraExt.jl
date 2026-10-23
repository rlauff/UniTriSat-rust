module UniTriSatPolyhedraExt
using UniTriSat
using Polyhedra

function _vertices(polytope::Polyhedra.Polyhedron)
    representation = Polyhedra.vrep(polytope)
    isempty(collect(Polyhedra.rays(representation))) &&
        isempty(collect(Polyhedra.lines(representation))) ||
        throw(ArgumentError("triangulate requires a bounded polytope"))
    rows = [Int.(point) for point in Polyhedra.points(representation)]
    isempty(rows) && throw(ArgumentError("polytope has no vertices"))
    return permutedims(hcat(rows...))
end

UniTriSat.triangulate(polytope::Polyhedra.Polyhedron; kwargs...) =
    UniTriSat.triangulate(_vertices(polytope); kwargs...)
UniTriSat.triangulate(polytopes::AbstractVector{<:Polyhedra.Polyhedron}; kwargs...) =
    UniTriSat.triangulate(Matrix{Int}[_vertices(p) for p in polytopes]; kwargs...)
end

module UniTriSatPlotExt

using UniTriSat
using Polyhedra
using CDDLib

function plot_boundary(simplices::Vector{Matrix{Int}})
    # A returned triangulation covers the original polytope. Its exact vertex
    # coordinates therefore suffice for the same hull-boundary views without
    # retaining or running any of the Julia search pipeline.
    vertices = unique(reduce(vcat, simplices); dims=1)
    hull = Polyhedra.polyhedron(Polyhedra.vrep(Rational{BigInt}.(vertices)), CDDLib.Library(:exact))
    facets = collect(Polyhedra.halfspaces(Polyhedra.hrep(hull)))
    UniTriSat._plot_boundary(simplices, facets)
    return nothing
end

end

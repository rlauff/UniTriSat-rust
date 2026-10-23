# Run in an environment containing the adapter and these optional packages.
using Test
using UniTriSat
import Polyhedra, CDDLib, Oscar

@testset "Optional polyhedron inputs preserve exact integral vertices" begin
    vertices = [0 0 0; 1 0 0; 0 1 0; 0 0 1]
    polyhedron = Polyhedra.polyhedron(Polyhedra.vrep(vertices), CDDLib.Library(:exact))
    oscar = Oscar.convex_hull(vertices)
    for input in (polyhedron, oscar, [polyhedron], [oscar])
        result = triangulate(input; find_all=true, terminal_output="", parallel_split_solving=false)
        @test result.total_number_of_triangulations_found == 1
        @test Set(Tuple.(eachrow(only(only(only(result.triangulation_results).solution_simplices))))) == Set(Tuple.(eachrow(vertices)))
    end
    nonintegral = Rational{Int}.(vertices) ./ 2
    @test_throws InexactError triangulate(Polyhedra.polyhedron(Polyhedra.vrep(nonintegral), CDDLib.Library(:exact)); terminal_output="")
    @test_throws Exception triangulate(Oscar.convex_hull(nonintegral); terminal_output="")
    @test Base.get_extension(UniTriSat, :UniTriSatPlotExt) !== nothing
end

@testset "D4–6 boundary display extraction" begin
    for dimension in 4:6
        vertices = zeros(Int, dimension + 1, dimension)
        for coordinate in 1:dimension
            vertices[coordinate + 1, coordinate] = 1
        end
        original = copy(vertices)
        hull = Polyhedra.polyhedron(Polyhedra.vrep(Rational{BigInt}.(vertices)), CDDLib.Library(:exact))
        facets = collect(Polyhedra.halfspaces(Polyhedra.hrep(hull)))
        views = UniTriSat._plot_boundary_views([vertices], facets)
        @test length(views) == binomial(dimension + 1, 4)
        @test all(view -> length(view) == 1 && size(only(view)) == (4, 3), views)
        @test all(view -> all(isfinite, only(view)), views)
        @test vertices == original
    end
end

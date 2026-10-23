# Real rendering without opening browser windows: PLOTLY_RENDERER=json.
using Test
using UniTriSat
import Polyhedra, CDDLib

@testset "Actual plotting subprocess and optional boundary extension" begin
    @test get(ENV, "PLOTLY_RENDERER", "") == "json"
    vertices = [0 0 0; 1 0 0; 0 1 0; 0 0 1]
    mktempdir() do directory
        output = joinpath(directory, "plot.json-output")
        result = open(output, "w") do io
            redirect_stdout(io) do
                triangulate(vertices; plot=true, return_triangulations="none", terminal_output="")
            end
        end
        rendered = read(output, String)
        @test occursin("mesh3d", rendered)
        @test occursin("application/json", rendered)
        @test result.total_number_of_triangulations_found == 1
        @test isempty(result.triangulation_results)
        println("D3 rendered JSON bytes: ", sizeof(rendered))

        simplex4 = [0 0 0 0; 1 0 0 0; 0 1 0 0; 0 0 1 0; 0 0 0 1]
        open(output, "w") do io
            redirect_stdout(io) do
                UniTriSat._plot_triangulation([simplex4])
            end
        end
        rendered4 = read(output, String)
        @test length(collect(eachmatch(r"application/json", rendered4))) == 5
        @test occursin("mesh3d", rendered4)
        println("D4 boundary rendered JSON bytes: ", sizeof(rendered4))
    end
end

# Opt-in integration against the separately built Rust bridge:
# UNITRISAT_ADAPTER_INTEGRATION=1 julia --project=bindings/julia -e 'using Pkg; Pkg.test()'
@testset "Actual Rust worker preserves the Julia search interface" begin
    vertices = [0 0 0; 1 0 0; 0 1 0; 0 0 1]
    expected_vertices = Set(Tuple.(eachrow(vertices)))
    withenv("UNITRISAT_MAX_SOLUTIONS" => "", "UNITRISAT_MAX_REGULARITY_TESTS" => "") do
        for solver in ("picosat", "cadical"), mode in (:serial, :split, :incremental),
            (regular, flag) in ((false, false), (true, false), (false, true), (true, true))
            result = triangulate(vertices; solver=solver, find_all=true,
                regular=regular, flag_triangulation=flag, terminal_output="",
                parallel_split_solving=mode == :split, incremental_solving=mode == :incremental,
                return_triangulations="all")
            @test result.total_number_of_triangulations_found == 1
            @test result.total_number_of_regular_triangulations_found == Int(regular)
            @test result.total_number_of_flag_triangulations_found == Int(flag)
            @test result.total_number_of_quadratic_triangulations_found == Int(regular && flag)
            individual = only(result.triangulation_results)
            simplex = only(only(individual.solution_simplices))
            @test Set(Tuple.(eachrow(simplex))) == expected_vertices
        end
        none = triangulate([vertices, vertices]; return_triangulations="none", terminal_output="")
        @test isempty(none.triangulation_results)
        @test none.total_number_of_triangulations_found == 2
        first_result = triangulate(vertices; find_all=true, return_triangulations="first", terminal_output="")
        @test length(only(first_result.triangulation_results).solution_simplices) == 1
        square = [0 0; 1 0; 0 1; 1 1]
        square_all = triangulate(square; find_all=true, return_triangulations="all", terminal_output="")
        @test square_all.total_number_of_triangulations_found == 2
        @test length(only(square_all.triangulation_results).solution_simplices) == 2
        mixed = triangulate([square, vertices]; find_all=true, terminal_output="")
        @test [r.number_of_triangulations_found for r in mixed.triangulation_results] == [2, 1]
        withenv("UNITRISAT_MAX_SOLUTIONS" => "1") do
            @test_logs (:warn, r"lower-bound") begin
                capped = triangulate(square; find_all=true, return_triangulations="all", terminal_output="")
                @test capped.total_number_of_triangulations_found == 1
                @test length(only(capped.triangulation_results).solution_simplices) == 1
            end
        end
        if get(ENV, "UNITRISAT_ADAPTER_GPU", "0") == "1"
            cube = reduce(vcat, [reshape([x, y, z], 1, 3) for x in 0:1 for y in 0:1 for z in 0:1])
            cpu = triangulate(cube; solver="cadical", incremental_solving=true,
                find_all=true, return_triangulations="all", terminal_output="")
            gpu = triangulate(cube; solver="cadical", incremental_solving=true,
                intersection_backend="gpu", find_all=true, return_triangulations="all", terminal_output="")
            @test cpu.total_number_of_triangulations_found == gpu.total_number_of_triangulations_found == 72
            canonical(result) = Set(Tuple(sort([Tuple(sort(Tuple.(eachrow(simplex)))) for simplex in triangulation]))
                for triangulation in only(result.triangulation_results).solution_simplices)
            @test canonical(cpu) == canonical(gpu)
        end
        mktempdir() do directory
            path = joinpath(directory, "input with spaces.txt")
            open(path, "w") do io
                for row in eachrow(vertices)
                    println(io, join(row, ' '))
                end
            end
            from_file = triangulate(path; terminal_output="")
            @test from_file.total_number_of_triangulations_found == 1
            open(path, "a") do io
                println(io, "malformed input")
            end
            @test_throws Exception triangulate(path; terminal_output="")
        end
    end
end

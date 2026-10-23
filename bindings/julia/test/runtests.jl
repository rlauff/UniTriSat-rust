using Test
using UniTriSat
using JSON3

include("plotting.jl")

function response(; count="1", coordinate="9007199254740993", status="ok")
    return JSON3.write((protocol=1, seconds=0.01, results=[(
        counts=(triangulations=count, regular="0", flag="0", quadratic="0"),
        status=status, seconds=0.01,
        solutions=[[[[coordinate, "0"], ["1", "0"], ["0", "1"]]]],
        steps=[(name="Search", seconds=0.01)])]))
end

@testset "Plotting preserves first-any semantics independently of target retention" begin
    vertices = [0 0 0; 1 0 0; 0 1 0; 0 0 1]
    request = (protocol=1, input=(kind="matrices", matrices=[UniTriSat._input_matrix(vertices)]),
        threads=2, settings=(backend="gpu", unimodular=true, regular=true, flag=true,
        find_all=true, keep="none", solver="cadical", mode="incremental",
        max_solutions="5", max_regularity_tests="3"))
    # A triangulatable input with no target: no retained target can be plotted.
    primary = [UniTriSat.Structs.TriangulationResult(Vector{Matrix{Int}}[], 3, 0, 0, 0,
        0.1, UniTriSat.Structs.StepStat[])]
    plotting = [UniTriSat.Structs.TriangulationResult([[vertices]], 1, 0, 0, 0,
        0.01, UniTriSat.Structs.StepStat[])]
    received = Ref{Any}()
    rendered = Any[]
    invoke = r -> (received[] = r; plotting)
    UniTriSat._plot_results(request, primary; invoke=invoke, render=s -> push!(rendered, s))
    @test only(rendered) == [vertices]
    @test primary[1].number_of_triangulations_found == 3
    @test isempty(primary[1].solution_simplices)
    @test request.settings.keep == "none"
    @test received[].input == request.input
    @test received[].threads == request.threads
    @test received[].settings == merge(request.settings,
        (regular=false, flag=false, find_all=false, keep="first"))
    @test_throws Exception UniTriSat._plot_results(request, primary;
        invoke=_ -> UniTriSat.Structs.TriangulationResult[], render=_ -> error("must not display"))
    @test_throws Exception UniTriSat._plot_results(request, primary;
        invoke=_ -> error("child failure"), render=_ -> error("must not display"))
    @test UniTriSat._plot_results(request, UniTriSat.Structs.TriangulationResult[];
        invoke=_ -> error("empty batch must not invoke Rust")) === nothing
end

if get(ENV, "UNITRISAT_ADAPTER_INTEGRATION", "0") == "1"
    include("integration.jl")
end

@testset "Protocol preserves exact integers and legacy field types" begin
    results = UniTriSat._response(response())
    @test length(results) == 1
    @test results[1] isa UniTriSat.Structs.TriangulationResult
    @test results[1].solution_simplices[1][1][1, 1] == 9007199254740993
    @test results[1].step_stats[1].alloc_bytes == -1
    @test results[1].number_of_triangulations_found == 1
    @test_throws Exception UniTriSat._response(response(count="0"))
    @test_throws Exception UniTriSat._response(response(count="-1"))
    @test_throws Exception UniTriSat._response(response(coordinate="1.5"))
    @test_throws Exception UniTriSat._response(response(coordinate="-0"))
    @test_throws Exception UniTriSat._response(response(coordinate="9223372036854775808"))
    @test_throws Exception UniTriSat._response(response(status="skipped"))
    @test_throws Exception UniTriSat._response("{\"protocol\":2,\"results\":[],\"seconds\":0}")
    @test_throws Exception UniTriSat._response("{\"protocol\":1,\"results\":[]}")
    @test_throws Exception UniTriSat._response("{")
end

@testset "Options and empty batch need no numerical worker" begin
    empty_batch = Matrix{Int}[]
    result = triangulate(empty_batch; terminal_output="", return_triangulations="none")
    @test result isa UniTriSat.Structs.RunResult
    @test isempty(result.triangulation_results)
    @test result.number_triangulatable == 0
    @test result.total_number_of_triangulations_found == 0
    @test_throws ArgumentError triangulate(empty_batch; flag_SAT=true)
    @test_logs (:warn, r"use_normaliz is deprecated") triangulate(empty_batch; use_normaliz=true, terminal_output="")
    @test_throws ArgumentError triangulate(empty_batch; solver="unknown")
    @test_throws ArgumentError triangulate(zeros(Int, 0, 2))
    @test_throws InexactError UniTriSat._input_matrix(reshape([big(2)^64], 1, 1))
    withenv("UNITRISAT_MAX_SOLUTIONS" => "invalid") do
        @test_throws ArgumentError triangulate(empty_batch; terminal_output="")
    end
end

@testset "Kissat validates compatibility before an empty batch" begin
    empty_batch = Matrix{Int}[]
    for kwargs in ((incremental_solving=true,), (regular=true,),
                   (flag_triangulation=true,), (find_all=true,),
                   (regular=true, flag_triangulation=true))
        @test_throws ArgumentError triangulate(empty_batch; solver="kissat", kwargs...)
    end
    for split in (false, true), device in ("cpu", "gpu")
        if Sys.iswindows()
            @test_throws ArgumentError triangulate(empty_batch; solver="kissat",
                parallel_split_solving=split, intersection_backend=device, terminal_output="")
        else
            result = triangulate(empty_batch; solver="kissat", parallel_split_solving=split,
                intersection_backend=device, terminal_output="")
            @test result.total_number_of_triangulations_found == 0
        end
    end
end

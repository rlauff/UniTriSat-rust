"""Julia compatibility interface to the Rust UniTriSat implementation."""
module UniTriSat

using JSON3

export triangulate

include("structs.jl")
using .Structs
include("protocol.jl")
include("plotting.jl")

function _environment_limit(name::String)
    raw = get(ENV, name, "")
    isempty(raw) && return nothing
    value = tryparse(Int, raw)
    isnothing(value) && throw(ArgumentError("$name must be an integer"))
    return value > 0 ? string(value) : nothing
end

function _input_matrix(vertices::AbstractMatrix{<:Integer})
    size(vertices, 1) > 0 && size(vertices, 2) > 0 ||
        throw(ArgumentError("vertex matrices must be nonempty"))
    return [[string(Int64(value)) for value in row] for row in eachrow(vertices)]
end

triangulate(vertices::AbstractMatrix{<:Integer}; kwargs...) =
    _triangulate((kind="matrices", matrices=[_input_matrix(vertices)]); kwargs...)

triangulate(polytopes::AbstractVector{<:AbstractMatrix{<:Integer}}; kwargs...) =
    _triangulate((kind="matrices", matrices=[_input_matrix(p) for p in polytopes]); kwargs...)

triangulate(path::AbstractString; kwargs...) =
    _triangulate((kind="file", path=abspath(path)); kwargs...)

"""
    triangulate(vertices_or_path; kwargs...) -> UniTriSat.Structs.RunResult

Call Rust with the legacy Julia search options and result fields. Coordinates
and counts remain exact integers. `UNITRISAT_EXECUTABLE` selects the separately
built `unitrisat-julia` executable; importing this package never builds it.

`step_stats.alloc_bytes == -1` means Rust allocations were not measured.
Per-polytope timings come from Rust; the run total includes bridge and plotting
overhead. `check_full_dimensionality` is accepted, but Rust always projects
exactly as documented in the migration differences. `validate` was historically
unused; Rust always validates its input and exact arithmetic boundaries.
"""
function _triangulate(input;
    intersection_backend::String="cpu", unimodular::Bool=true,
    regular::Bool=false, flag_triangulation::Bool=false, flag_SAT::Bool=false,
    find_all::Bool=false, log_file::String="", terminal_output::String="final",
    validate::Bool=false, plot::Bool=false, use_normaliz::Bool=false,
    return_triangulations::String="first", solver::String="picosat",
    incremental_solving::Bool=false, check_full_dimensionality::Bool=false,
    parallel_split_solving::Bool=true)

    intersection_backend in ("cpu", "gpu") || throw(ArgumentError("intersection_backend must be cpu or gpu"))
    solver in ("picosat", "cadical", "kissat") || throw(ArgumentError("solver must be picosat, cadical, or kissat"))
    if solver == "kissat"
        option = incremental_solving ? "incremental_solving" : regular ? "regular" :
                 flag_triangulation ? "flag_triangulation" : find_all ? "find_all" : nothing
        isnothing(option) || throw(ArgumentError("Kissat is incompatible with $option: it cannot add clauses after solving; use PicoSAT or CaDiCaL"))
        Sys.iswindows() && throw(ArgumentError("Kissat is unavailable on Windows; use PicoSAT or CaDiCaL"))
    end
    flag_SAT && throw(ArgumentError("flag_SAT is retired; use flag_triangulation=true"))
    if use_normaliz
        @warn "use_normaliz is deprecated; UniTriSat now uses the exact Rust lattice engine"
    end
    if !(return_triangulations in ("first", "all", "none"))
        @warn "Unknown return_triangulations; falling back to first" return_triangulations
        return_triangulations = "first"
    end
    if incremental_solving && solver != "cadical"
        @warn "Incremental solving switches the Julia interface to CaDiCaL"
        solver = "cadical"
    end
    mode = incremental_solving ? "incremental" : parallel_split_solving ? "split" : "serial"
    request = (protocol=1, input=input, threads=Threads.nthreads(), settings=(
        backend=intersection_backend, unimodular=unimodular, regular=regular,
        flag=flag_triangulation, find_all=find_all, keep=return_triangulations, solver=solver, mode=mode,
        max_solutions=_environment_limit("UNITRISAT_MAX_SOLUTIONS"),
        max_regularity_tests=_environment_limit("UNITRISAT_MAX_REGULARITY_TESTS")))
    started = time_ns()
    results = _run_requests(request, plot)
    count_fields = (:number_of_triangulations_found, :number_of_regular_triangulations_found,
                    :number_of_flag_triangulations_found, :number_of_quadratic_triangulations_found)
    totals = [sum((BigInt(getproperty(r, field)) for r in results); init=BigInt(0)) for field in count_fields]
    exists = [count(r -> getproperty(r, field) > 0, results) for field in count_fields]
    run_result = RunResult(return_triangulations == "none" ? TriangulationResult[] : results,
        exists..., Int.(totals)..., (time_ns() - started) / 1e9)
    components = split(lowercase(replace(terminal_output, " " => "")), ',')
    if any(c -> c in ("all", "initial", "running", "table", "final"), components)
        _summary(stdout, run_result, length(results))
    end
    if !isempty(log_file)
        open(log_file, "a") do io
            _summary(io, run_result, length(results))
        end
    end
    return run_result
end

function _run_requests(request, plot::Bool)
    function execute_pair(effective_request)
        input = effective_request.input
        results = input.kind == "matrices" && isempty(input.matrices) ? TriangulationResult[] :
            _invoke(effective_request)
        if input.kind == "matrices" && length(results) != length(input.matrices)
            error("Rust bridge returned a different number of polytopes")
        end
        plot && _plot_results(effective_request, results)
        return results
    end
    if plot && request.input.kind == "file"
        # Both child calls must see identical bytes and order even if the user
        # edits the source file while the primary search is running.
        return mktempdir() do directory
            snapshot = joinpath(directory, "polytopes.txt")
            cp(request.input.path, snapshot)
            execute_pair(merge(request, (input=(kind="file", path=snapshot),)))
        end
    end
    return execute_pair(request)
end

# Legacy plotting uses the first triangulation, before target classification.
# A separate unfiltered search keeps that behavior without changing retention
# or the result of the primary search, including return_triangulations="none".
function _plot_results(request, primary; invoke=_invoke, render=_plot_triangulation)
    isempty(primary) && return nothing
    settings = merge(request.settings, (regular=false, flag=false, find_all=false, keep="first"))
    plotting = invoke(merge(request, (settings=settings,)))
    length(plotting) == length(primary) || error("Rust plotting search returned a different number of polytopes")
    # Both ordered responses refer to exactly the same ordered batch input.
    # Validate the whole batch before displaying any view.
    for (original, result) in zip(primary, plotting)
        (original.number_of_triangulations_found > 0) == (result.number_of_triangulations_found > 0) ||
            error("Rust plotting search disagrees with the primary triangulation existence result")
        if result.number_of_triangulations_found > 0
            length(result.solution_simplices) == 1 || error("Rust plotting search did not retain its first triangulation")
        elseif !isempty(result.solution_simplices)
            error("Rust plotting search returned geometry with a zero count")
        end
    end
    for result in plotting
        if isempty(result.solution_simplices)
            @warn "No triangulation available to plot"
        else
            render(first(result.solution_simplices))
        end
    end
    return nothing
end

function _summary(io::IO, result::RunResult, count::Int)
    println(io, "UniTriSat (Rust): $count polytopes; ",
        result.number_triangulatable, " triangulatable; ",
        result.number_regularly_triangulatable, " regular; ",
        result.number_flag_triangulatable, " flag; ",
        result.number_quadratic_triangulatable, " quadratic; ",
        result.total_time, " seconds")
end

end

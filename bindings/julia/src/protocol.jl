function _executable()
    override = get(ENV, "UNITRISAT_EXECUTABLE", "")
    if !isempty(override)
        found = isfile(override) ? abspath(override) : Sys.which(override)
        isnothing(found) && error("UNITRISAT_EXECUTABLE was not found: $override")
        return found
    end
    # The optional Julia package lives at bindings/julia in a source checkout.
    # Installed packages can select the bridge through PATH or the override.
    root = normpath(joinpath(@__DIR__, "..", "..", ".."))
    local_path = joinpath(root, "target", "release", "unitrisat-julia")
    isfile(local_path) && return local_path
    found = Sys.which("unitrisat-julia")
    isnothing(found) && error("Build unitrisat-julia or set UNITRISAT_EXECUTABLE to its path")
    return found
end

function _fields(value, expected::Tuple)
    value isa JSON3.Object || error("Rust bridge response expected an object")
    Set(String.(keys(value))) == Set(expected) || error("Rust bridge response has unexpected or missing fields")
    return value
end

function _integer(value; nonnegative::Bool=false)
    value isa AbstractString || error("Rust bridge integer must be a decimal string")
    occursin(r"^-?(0|[1-9][0-9]*)$", value) || error("Invalid Rust bridge integer")
    value == "-0" && error("Noncanonical Rust bridge integer")
    parsed = tryparse(Int, value)
    isnothing(parsed) && error("Rust bridge integer does not fit Julia Int")
    nonnegative && parsed < 0 && error("Negative Rust bridge count")
    return parsed
end

function _seconds(value)
    value isa Real && !(value isa Bool) || error("Rust bridge timing must be numeric")
    parsed = Float64(value)
    isfinite(parsed) && parsed >= 0 || error("Invalid Rust bridge timing")
    return parsed
end

function _simplex(value)
    value isa JSON3.Array && length(value) >= 2 || error("Invalid Rust bridge simplex")
    first(value) isa JSON3.Array || error("Invalid Rust bridge vertex")
    dimension = length(first(value))
    dimension > 0 && length(value) == dimension + 1 || error("Invalid Rust bridge simplex shape")
    matrix = Matrix{Int}(undef, length(value), dimension)
    for (i, row) in enumerate(value)
        row isa JSON3.Array && length(row) == dimension || error("Ragged Rust bridge simplex")
        for (j, coordinate) in enumerate(row)
            matrix[i, j] = _integer(coordinate)
        end
    end
    return matrix
end

function _response(raw::AbstractString)
    response = _fields(JSON3.read(raw), ("protocol", "results", "seconds"))
    response.protocol isa Integer && !(response.protocol isa Bool) && response.protocol == 1 ||
        error("Unsupported Rust bridge protocol")
    _seconds(response.seconds)
    response.results isa JSON3.Array || error("Rust bridge results must be an array")
    results = TriangulationResult[]
    for item in response.results
        _fields(item, ("counts", "status", "seconds", "solutions", "steps"))
        counts = _fields(item.counts, ("triangulations", "regular", "flag", "quadratic"))
        n, r, f, q = [_integer(getproperty(counts, field); nonnegative=true)
                      for field in (:triangulations, :regular, :flag, :quadratic)]
        r <= n && f <= n && q <= min(r, f) || error("Inconsistent Rust bridge counts")
        item.status in ("ok", "capped", "reglimit") || error("Invalid Rust bridge status")
        item.status == "ok" || @warn "Rust search returned lower-bound counts" status=String(item.status)
        item.solutions isa JSON3.Array || error("Rust bridge solutions must be an array")
        solutions = Vector{Matrix{Int}}[]
        for solution in item.solutions
            solution isa JSON3.Array && !isempty(solution) || error("Invalid Rust bridge triangulation")
            simplices = [_simplex(simplex) for simplex in solution]
            all(s -> size(s) == size(first(simplices)), simplices) || error("Mixed Rust bridge simplex dimensions")
            push!(solutions, simplices)
        end
        length(solutions) <= n || error("Rust bridge returned more solutions than its count")
        item.steps isa JSON3.Array || error("Rust bridge steps must be an array")
        steps = StepStat[]
        for step in item.steps
            _fields(step, ("name", "seconds"))
            step.name isa AbstractString || error("Rust bridge step name must be text")
            push!(steps, StepStat(String(step.name), _seconds(step.seconds), -1))
        end
        push!(results, TriangulationResult(solutions, n, r, f, q, _seconds(item.seconds), steps))
    end
    return results
end

function _invoke(request)
    executable = _executable()
    return mktempdir() do directory
        request_path = joinpath(directory, "request.json")
        response_path = joinpath(directory, "response.json")
        stderr_path = joinpath(directory, "stderr.txt")
        open(request_path, "w") do io
            JSON3.write(io, request)
        end
        command = `$executable --request $request_path --response $response_path`
        process = open(stderr_path, "w") do errors
            run(pipeline(ignorestatus(command), stdout=devnull, stderr=errors))
        end
        diagnostic = read(stderr_path, String)
        success(process) || error("Rust UniTriSat failed: " * diagnostic)
        isempty(diagnostic) || print(stderr, diagnostic)
        isfile(response_path) || error("Rust UniTriSat did not write a response")
        results = _response(read(response_path, String))
        settings = request.settings
        for result in results
            targets = settings.regular ?
                (settings.flag ? result.number_of_quadratic_triangulations_found : result.number_of_regular_triangulations_found) :
                (settings.flag ? result.number_of_flag_triangulations_found : result.number_of_triangulations_found)
            expected = settings.keep == "none" ? 0 : settings.keep == "first" ? min(targets, 1) : targets
            length(result.solution_simplices) == expected || error("Rust bridge did not honor requested target retention")
            settings.regular || result.number_of_regular_triangulations_found == 0 || error("Unexpected regular count")
            settings.flag || result.number_of_flag_triangulations_found == 0 || error("Unexpected flag count")
        end
        results
    end
end

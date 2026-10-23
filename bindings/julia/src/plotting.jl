import LinearAlgebra

# Plotting consumes copies of returned exact coordinates. Projection and the
# legacy renderer's 0.8 display scaling never alter triangulation results.
function _render_tetrahedra(tetrahedra::Vector{<:AbstractMatrix})
    isempty(tetrahedra) && return nothing
    all(t -> size(t) == (4, 3) && all(isfinite, t), tetrahedra) ||
        throw(ArgumentError("plotting requires finite 4 × 3 tetrahedra"))
    python = get(ENV, "UNITRISAT_PYTHON", "python3")
    isempty(python) && throw(ArgumentError("UNITRISAT_PYTHON must name a Python executable"))
    script = normpath(joinpath(@__DIR__, "..", "python", "render.py"))
    isfile(script) || error("UniTriSat plotting helper is missing: $script")
    data = [[collect(row) for row in eachrow(tetrahedron)] for tetrahedron in tetrahedra]
    mktemp() do path, io
        JSON3.write(io, data)
        close(io)
        try
            run(Cmd([python, script, path]))
        catch err
            if err isa Base.ProcessFailedException || err isa Base.IOError
                error("UniTriSat plot subprocess failed. Set UNITRISAT_PYTHON to a Python " *
                      "executable with numpy and plotly installed; inspect the child error above. " *
                      sprint(showerror, err))
            end
            rethrow()
        end
    end
    return nothing
end

function _plot_3d_coordinates(simplices::Vector{Matrix{Int}})
    isempty(simplices) && return Matrix{Float64}[]
    # One common display origin preserves relative placement across simplices.
    # Subtract in BigInt first: adjacent Int64 coordinates above 2^53 would
    # otherwise coalesce, and subtracting Int64 extremes could overflow.
    origin = permutedims(BigInt.(first(simplices)[1, :]))
    return [Float64.(BigInt.(simplex) .- origin) for simplex in simplices]
end

function _plot_triangulation(simplices::Vector{Matrix{Int}})
    # A search with no triangulation has nothing to display, as in legacy Julia.
    isempty(simplices) && return nothing
    dimension = size(first(simplices), 2)
    all(s -> size(s) == (dimension + 1, dimension), simplices) ||
        throw(ArgumentError("plotting received inconsistent simplex shapes"))
    if dimension == 3
        _render_tetrahedra(_plot_3d_coordinates(simplices))
    elseif 4 <= dimension <= 6
        extension = Base.get_extension(@__MODULE__, :UniTriSatPlotExt)
        isnothing(extension) && throw(ArgumentError(
            "D4–6 plotting requires the optional Polyhedra and CDDLib packages. " *
            "Install them in your Julia environment, then run `using Polyhedra, CDDLib`."))
        extension.plot_boundary(simplices)
    else
        throw(ArgumentError("legacy plotting supports dimensions 3–6; dimension $dimension is unsupported"))
    end
    return nothing
end

# A 3-face of a D-polytope is the intersection of D-3 independent supporting
# facets. These lazy combinations preserve the legacy D4, D5 and D6 views.
function _plot_facet_choices(count::Int, dimension::Int)
    dimension == 4 && return ((i,) for i in 1:count)
    dimension == 5 && return ((i, j) for i in 1:count for j in (i + 1):count)
    dimension == 6 && return ((i, j, k) for i in 1:count for j in (i + 1):count for k in (j + 1):count)
    throw(ArgumentError("boundary plotting supports dimensions 4–6"))
end

function _plot_boundary_views(simplices::Vector{Matrix{Int}}, facets)
    dimension = size(first(simplices), 2)
    seen_faces = Set{Tuple}()
    views = Vector{Vector{Matrix{Float64}}}()
    for indices in _plot_facet_choices(length(facets), dimension)
        # CDDLib supplies exact facets. Keep incidence and independence exact,
        # including products which may exceed machine integers.
        normals = [Rational{BigInt}.(facets[i].a) for i in indices]
        offsets = [Rational{BigInt}(facets[i].β) for i in indices]
        gram = [LinearAlgebra.dot(a, b) for a in normals, b in normals]
        iszero(LinearAlgebra.det(gram)) && continue
        tetrahedra = Matrix{Int}[]
        seen_tetrahedra = Set{Tuple}()
        for simplex in simplices
            on_face = [r for r in axes(simplex, 1) if all(
                j -> LinearAlgebra.dot(normals[j], view(simplex, r, :)) == offsets[j],
                eachindex(normals))]
            length(on_face) == 4 || continue
            tetrahedron = simplex[on_face, :]
            key = Tuple(sort([Tuple(row) for row in eachrow(tetrahedron)]))
            key in seen_tetrahedra && continue
            push!(seen_tetrahedra, key)
            push!(tetrahedra, tetrahedron)
        end
        isempty(tetrahedra) && continue
        # Non-simple polytopes may describe one face by several facet subsets.
        face_key = Tuple(sort!(unique([Tuple(row) for tetrahedron in tetrahedra for row in eachrow(tetrahedron)])))
        face_key in seen_faces && continue
        push!(seen_faces, face_key)
        first_tetrahedron = first(tetrahedra)
        origin = BigInt.(first_tetrahedron[1, :])
        # Differences are formed exactly before conversion, preserving small
        # display features even when absolute coordinates exceed Float64's
        # exact integer range. QR is only an orthonormal display projection.
        edges = [Float64(BigInt(first_tetrahedron[r, c]) - origin[c])
                 for c in 1:dimension, r in 2:4]
        basis = Matrix(LinearAlgebra.qr(edges).Q)[:, 1:3]
        projected = [Float64.(BigInt.(tetrahedron) .- permutedims(origin)) * basis
                     for tetrahedron in tetrahedra]
        all(t -> all(isfinite, t), projected) || error("boundary display projection is not finite")
        push!(views, projected)
    end
    isempty(views) && error("no boundary 3-faces were found for plotting")
    return views
end

function _plot_boundary(simplices::Vector{Matrix{Int}}, facets)
    views = _plot_boundary_views(simplices, facets)
    # Finish extracting before opening any browser windows so a geometry error
    # does not leave the caller with an apparently successful partial plot.
    for projected in views
        _render_tetrahedra(projected)
    end
    return nothing
end

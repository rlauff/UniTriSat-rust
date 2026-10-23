# Plotting protocol tests; no plotting process or browser needed.
@testset "D3 plotting uses an exact common display origin" begin
    shift = Int(2)^53 + 8
    first_tetrahedron = [shift shift shift;
                         shift+1 shift shift;
                         shift shift+1 shift;
                         shift shift shift+1]
    second_tetrahedron = first_tetrahedron .+ 2
    simplices = [first_tetrahedron, second_tetrahedron]
    original = deepcopy(simplices)
    display = UniTriSat._plot_3d_coordinates(simplices)
    @test display[1] == [0.0 0.0 0.0; 1.0 0.0 0.0; 0.0 1.0 0.0; 0.0 0.0 1.0]
    @test display[2] == display[1] .+ 2.0
    @test simplices == original

    # This display extent cannot retain unit resolution in Float64, but forming
    # its exact difference must not overflow Int or reverse its sign.
    extremes = [typemin(Int) 0 0; typemax(Int) 0 0; typemin(Int) 1 0; typemin(Int) 0 1]
    displayed_extremes = only(UniTriSat._plot_3d_coordinates([extremes]))
    @test displayed_extremes[2, 1] == Float64(BigInt(typemax(Int)) - BigInt(typemin(Int)))
    @test displayed_extremes[2, 1] > 0
    @test isempty(UniTriSat._plot_3d_coordinates(Matrix{Int}[]))
end

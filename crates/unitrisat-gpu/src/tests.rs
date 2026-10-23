use super::*;

#[test]
fn conservative_precision_handles_extremes_and_lower_minors() {
    assert_eq!(precision(&[0, 0, 1, 1], 2), Some(Lane::I32));
    assert_eq!(
        precision(&[-30_000, -30_000, 30_000, 30_000], 2),
        Some(Lane::I64)
    );
    assert_eq!(precision(&[i64::MIN, 0, i64::MAX, 0], 2), None);
    assert_eq!(precision(&[0, 0, 0, i64::MAX, 0, 0], 3), Some(Lane::I64));
    assert_eq!(precision(&[0, 0, 0, i64::MIN, 0, 0], 3), None);
    assert_eq!(precision(&[0], 2), None);
    assert_eq!(precision(&[], 8), None);
    for (magnitude, expected) in [
        (23_170, Some(Lane::I32)),
        (23_171, Some(Lane::I64)),
        (1_518_500_249, Some(Lane::I64)),
        (1_518_500_250, None),
    ] {
        assert_eq!(
            precision(&[-magnitude, -magnitude, magnitude, magnitude], 2),
            expected
        );
    }
}

#[test]
fn exact_hand_computed_triangles_and_validation() {
    let points = [0, 0, 4, 0, 0, 4, 1, 1, 2, 1, 1, 2, 4, 4];
    let simplices = [0, 1, 2, 3, 4, 5, 1, 2, 6];
    assert_eq!(
        intersecting_pairs(&points, 2, &simplices).unwrap(),
        vec![(0, 1)]
    );
    assert!(matches!(
        intersecting_pairs(&points, 2, &[0, 1, 99]),
        Err(IntersectError::PointOutOfRange { .. })
    ));
    assert!(matches!(
        intersecting_pairs(&[0], 2, &[]),
        Err(IntersectError::PointShape { .. })
    ));
    assert!(matches!(
        intersecting_pairs(&[], 11, &[]),
        Err(IntersectError::UnsupportedDimension(11))
    ));
}

#[test]
fn stream_has_collect_parity_and_honors_stop() {
    let points = [0, 0, 4, 0, 0, 4];
    let simplices = [0, 1, 2].repeat(40);
    let collected = std::sync::Mutex::new(Vec::new());
    stream_pairs(&points, 2, &simplices, 1, &|batch| {
        collected.lock().unwrap().extend_from_slice(batch);
        true
    })
    .unwrap();
    let mut streamed = collected.into_inner().unwrap();
    streamed.sort_unstable();
    assert_eq!(
        streamed,
        intersecting_pairs(&points, 2, &simplices).unwrap()
    );
    let calls = std::sync::atomic::AtomicUsize::new(0);
    stream_pairs(&points, 2, &simplices, 1, &|_| {
        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        false
    })
    .unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn generated_source_covers_every_dimension_and_lane() {
    for d in 2..=7 {
        for lane in [Lane::I32, Lane::I64] {
            let text = source::generate(d, lane).unwrap();
            assert!(text.contains("extern \"C\" __global__ void pairs"));
            assert_eq!(text.matches("normal(e,n);").count(), d);
        }
    }
    assert!(source::generate(8, Lane::I32).is_none());
}

#[cfg(feature = "cuda")]
#[test]
#[ignore = "requires a CUDA device and NVRTC; CPU fallback is a failure"]
fn real_cuda_matches_exact_cpu() {
    let mut state = 7_u64;
    for d in 2..=7 {
        let n = 24;
        let mut points = Vec::new();
        for _ in 0..n * d {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            points.push(((state >> 32) % 7) as i64 - 3);
        }
        let mut simplices = Vec::new();
        for i in 0..24 {
            for j in 0..=d {
                simplices.push(((i + j * 5) % n) as u32);
            }
        }
        let expected = intersect::intersecting_pairs(&points, d, &simplices).unwrap();
        let actual = search(&points, d, &simplices).unwrap();
        assert!(
            matches!(actual.backend, Backend::Cuda(_)),
            "D={d}: {:?}",
            actual.backend
        );
        assert_eq!(actual.pairs, expected, "D={d}");
    }
    for magnitude in [23_170, 23_171, 1_518_500_249] {
        let points = [
            -magnitude, -magnitude, magnitude, -magnitude, -magnitude, magnitude,
        ];
        let actual = search(&points, 2, &[0, 1, 2, 0, 1, 2]).unwrap();
        assert_eq!(
            actual.backend,
            Backend::Cuda(precision(&points, 2).unwrap())
        );
        assert_eq!(actual.pairs, vec![(0, 1)]);
    }
}

#[cfg(feature = "cuda")]
#[test]
#[ignore = "requires NVRTC; compile every generated dimension and integer lane"]
fn nvrtc_compiles_all_variants() {
    for d in 2..=7 {
        for lane in [Lane::I32, Lane::I64] {
            cudarc::nvrtc::compile_ptx(source::generate(d, lane).unwrap()).unwrap();
        }
    }
}

#[cfg(feature = "cuda")]
#[test]
#[ignore = "requires CUDA: incremental SAT uses an asserted real GPU source"]
fn real_cuda_incremental_both_solvers() {
    use unitrisat::{cnf, enumerate, families, hull, lattice, solve};
    let vertices = families::hypercube(3).unwrap();
    let facets = hull::facets(&vertices).unwrap();
    let points = lattice::lattice_points(&vertices, &facets).unwrap();
    let simplices = lattice::simplices(&points, lattice::Restriction::Unimodular).unwrap();
    let faces = lattice::faces(&points, &facets, &simplices).unwrap();
    let central = lattice::central(&points, &simplices, &faces).unwrap();
    let reduced = cnf::reduced(simplices.len() / 4, &faces, &central).unwrap();
    for backend in [solve::Backend::PicoSat, solve::Backend::Cadical] {
        let options = enumerate::Options {
            backend,
            goal: enumerate::Goal::All,
            keep: enumerate::Keep::None,
            max_solutions: None,
            deadline: None,
        };
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let result = enumerate::incremental_with_pairs(
            &reduced,
            &points,
            &simplices,
            &options,
            &enumerate::AnyTriangulation,
            &|p, d, s, _, sink| {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                assert_eq!(
                    cuda::run(p, d, s, &mut 0, &mut |batch| sink(batch)).unwrap(),
                    Lane::I32
                );
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(result.found, 72);
        assert_eq!(result.outcome, enumerate::Outcome::Complete);
    }
}

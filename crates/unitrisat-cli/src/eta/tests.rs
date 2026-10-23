use super::*;

fn seconds(estimate: Estimate) -> f64 {
    match estimate {
        Estimate::Ready { seconds, .. } => seconds,
        other => panic!("{other:?}"),
    }
}

#[test]
fn scaled_shapes_and_extreme_coordinates_have_safe_distinct_descriptors() {
    let square = |scale: i64| Matrix::new(2, vec![0, 0, scale, 0, 0, scale, scale, scale]).unwrap();
    assert!(descriptor(&square(2)) < descriptor(&square(16)));
    let extremes = Matrix::new(1, vec![i64::MIN, i64::MAX]).unwrap();
    assert_eq!(descriptor(&extremes), UNKNOWN);
    assert_eq!(descriptor(&Matrix::new(17, vec![0; 17]).unwrap()), UNKNOWN);
}

#[test]
fn ascending_descending_and_mixed_sizes_outperform_completed_average() {
    for order in [
        vec![2, 2, 3, 3, 4, 4, 5, 5],
        vec![5, 5, 4, 4, 3, 3, 2, 2],
        vec![2, 5, 3, 4, 2, 5, 3, 4],
    ] {
        let ids: Arc<[u16]> = order.clone().into();
        let mut estimator = Estimator::new(ids);
        let costs: Vec<f64> = order
            .iter()
            .map(|&band| 2_f64.powi(i32::from(band)))
            .collect();
        let observed: f64 = costs.iter().take(4).sum();
        for (offset, &cost) in costs.iter().take(4).enumerate() {
            estimator.finish(u64::try_from(offset + 1).unwrap(), Some(cost));
        }
        let actual: f64 = costs.iter().skip(4).sum();
        let prediction = seconds(estimator.estimate([], 1));
        let old = observed / 4. * 4.;
        assert!(
            (prediction - actual).abs() < 1e-8,
            "{order:?}: {prediction} vs {actual}"
        );
        assert!((prediction - actual).abs() <= (old - actual).abs() + 1e-8);
    }
}

#[test]
fn sparse_sizes_censoring_and_unstable_samples_keep_qualified_estimates() {
    let mut estimator = Estimator::new(vec![2, 2, 3, 9].into());
    estimator.finish(1, Some(4.));
    estimator.finish(2, Some(4.));
    assert_eq!(estimator.estimate([], 1), low_confidence(8.));
    estimator.finish(3, None);
    assert_eq!(estimator.estimate([], 1), low_confidence(4.));
    let mut unstable = Estimator::new(vec![2, 2, 2].into());
    unstable.finish(1, Some(0.01));
    unstable.finish(2, Some(100.));
    assert_eq!(unstable.estimate([], 1), low_confidence(50.005));
    let mut unsupported = Estimator::new(vec![UNKNOWN].into());
    assert_eq!(unsupported.estimate([], 1), Estimate::Learning);
    unsupported.finish(1, None);
    assert_eq!(seconds(unsupported.estimate([], 1)), 0.);
    assert_eq!(
        sample(
            crate::job::Status::Capped,
            Some(crate::job::StageTimes {
                lattice: 1.,
                formula: 1.,
                search: 1.
            })
        ),
        None
    );
}

#[test]
fn parallel_capacity_and_final_straggler_use_remaining_age() {
    let mut estimator = Estimator::new(vec![2; 6].into());
    for index in 1..=3 {
        estimator.finish(index, Some(10.));
    }
    assert_eq!(seconds(estimator.estimate([], 2)), 15.);
    assert_eq!(seconds(estimator.estimate([(4, 5.)], 2)), 12.5);
    estimator.finish(5, Some(10.));
    estimator.finish(6, Some(10.));
    assert_eq!(seconds(estimator.estimate([(4, 5.)], 8)), 5.);
    assert_eq!(estimator.estimate([(4, 15.)], 8), low_confidence(3.75));
    assert_eq!(estimator.estimate([(4, 41.)], 8), low_confidence(10.25));
}

#[test]
fn censored_mixture_qualifies_usable_success_timings() {
    let mut estimator = Estimator::new(vec![2; 8].into());
    for index in 1..=3 {
        estimator.finish(index, Some(1.));
    }
    estimator.finish(4, None);
    assert_eq!(estimator.estimate([], 1), low_confidence(4.));
}

/// Explicit microcheck only: no solver, native code, file parsing or correctness deadline.
#[test]
#[ignore = "run explicitly to measure the bounded ETA bookkeeping overhead"]
fn bookkeeping_overhead_100k_inputs() {
    use std::{hint::black_box, time::Instant};
    let vertices = Matrix::new(3, vec![0, 0, 0, 8, 0, 0, 0, 8, 0, 0, 0, 8]).unwrap();
    let started = Instant::now();
    let ids: Arc<[u16]> = (0..100_000)
        .map(|_| descriptor(black_box(&vertices)))
        .collect::<Vec<_>>()
        .into();
    let descriptors = started.elapsed();
    let started = Instant::now();
    let mut estimator = Estimator::new(ids);
    for index in 1..=90_000 {
        estimator.finish(index, Some(0.01));
    }
    let updates = started.elapsed();
    let started = Instant::now();
    for _ in 0..1000 {
        black_box(estimator.estimate([(90_001, 0.005)], 4));
    }
    let redraws = started.elapsed();
    let mut dense = Estimator::new(
        (0..100_000)
            .map(|i| (i % BUCKETS) as u16)
            .collect::<Vec<_>>()
            .into(),
    );
    for index in 1..=90_000 {
        dense.finish(index, Some(0.01 + ((index % 32) as f64) * 0.001));
    }
    let started = Instant::now();
    for _ in 0..1000 {
        black_box(dense.estimate([(90_001, 0.005)], 4));
    }
    let dense_redraws = started.elapsed();
    eprintln!(
        "ETA microcheck: 100000 descriptors={descriptors:?}; initialize+90000 updates={updates:?}; 1000 one-bucket redraws={redraws:?}; 1000 all-512-bucket redraws={dense_redraws:?}; descriptor bytes={}",
        100_000 * std::mem::size_of::<u16>()
    );
}

#[test]
fn real_scaled_descriptors_handle_wide_descending_tail_and_cautious_ascending() {
    let descriptor_for = |scale: i64| {
        descriptor(&Matrix::new(2, vec![0, 0, scale, 0, 0, scale, scale, scale]).unwrap())
    };
    let descending = [1024, 1024, 512, 512, 64, 16, 4, 1];
    let mut orders = vec![descending.to_vec()];
    orders.push(descending.into_iter().rev().collect());
    for (direction, scales) in orders.into_iter().enumerate() {
        let ids: Vec<u16> = scales.iter().map(|&s| descriptor_for(s)).collect();
        let costs: Vec<f64> = ids
            .iter()
            .map(|&id| 2_f64.powi(i32::from(id % 32)))
            .collect();
        let mut estimator = Estimator::new(ids.into());
        let trained = if direction == 0 { 4 } else { 6 };
        for (offset, &cost) in costs.iter().take(trained).enumerate() {
            estimator.finish((offset + 1) as u64, Some(cost));
            if direction == 1 && offset == 3 {
                let early = estimator.estimate([], 1);
                assert!(matches!(early, Estimate::Ready { rough: true, .. }));
                assert!(seconds(early) > 0.);
            }
        }
        let actual: f64 = costs.iter().skip(trained).sum();
        let old = costs.iter().take(trained).sum::<f64>() / (trained as f64)
            * ((costs.len() - trained) as f64);
        let estimate = estimator.estimate([], 1);
        assert!(matches!(estimate, Estimate::Ready { rough: true, .. }));
        let prediction = seconds(estimate);
        assert!(
            (prediction - actual).abs() < (old - actual).abs(),
            "scales={scales:?} predicted={prediction} actual={actual} old={old}"
        );
        eprintln!(
            "wide scaled direction={direction}: predicted={prediction:.3}s actual={actual:.3}s old={old:.3}s (synthetic costs)"
        );
    }
    let mut nonpositive = Estimator::new(vec![10, 10, 9, 0].into());
    nonpositive.finish(1, Some(1.));
    nonpositive.finish(2, Some(1.));
    nonpositive.finish(3, Some(2.));
    assert_eq!(nonpositive.estimate([], 1), low_confidence(2.));
}

fn low_confidence(seconds: f64) -> Estimate {
    Estimate::Ready {
        seconds,
        rough: true,
    }
}

#[test]
fn one_sample_covers_sorted_cold_buckets_in_both_directions() {
    for ids in [vec![1, 2, 9, 20], vec![20, 9, 2, 1]] {
        let mut estimator = Estimator::new(ids.into());
        assert_eq!(estimator.estimate([], 1), Estimate::Learning);
        estimator.finish(1, Some(10.));
        assert_eq!(estimator.estimate([], 1), low_confidence(30.));
    }
}

#[test]
fn nearest_size_fallback_preserves_observed_scale() {
    let mut estimator = Estimator::new(vec![1, 9, 2, 8].into());
    estimator.finish(1, Some(1.));
    estimator.finish(2, Some(100.));
    // Too few observations for a trend: each cold bucket borrows its neighbor.
    assert_eq!(estimator.estimate([], 1), low_confidence(101.));
}

#[test]
fn unsupported_descriptors_and_unseen_dimensions_use_observed_timings() {
    let mut estimator = Estimator::new(vec![UNKNOWN, UNKNOWN, 2, 80].into());
    estimator.finish(1, Some(10.));
    assert_eq!(estimator.estimate([], 1), low_confidence(30.));
    // Unknown active jobs use residuals too, rather than counting full duration.
    assert_eq!(estimator.estimate([(2, 5.)], 1), low_confidence(25.));
    estimator.finish(3, Some(10.));
    estimator.finish(4, Some(10.));
    assert_eq!(estimator.estimate([(2, 100.)], 8), low_confidence(25.));
}

#[test]
fn failed_jobs_alone_have_no_duration_evidence() {
    let mut estimator = Estimator::new(vec![2, 2, UNKNOWN].into());
    estimator.finish(1, None);
    estimator.finish(2, None);
    assert_eq!(estimator.estimate([(3, 100.)], 1), Estimate::Unpredictable);
    assert_eq!(estimator.observed.count, 0);
    estimator.finish(3, None);
    assert_eq!(seconds(estimator.estimate([], 1)), 0.);
}

#[test]
fn resumed_jobs_need_fresh_timing_but_only_one_sample() {
    let mut estimator = Estimator::new(vec![1, 2, 3, 9].into());
    estimator.restore_completed(1);
    estimator.restore_completed(2);
    assert_eq!(estimator.estimate([(3, 100.)], 1), Estimate::Learning);
    estimator.finish(3, Some(5.));
    assert_eq!(estimator.estimate([], 1), low_confidence(5.));
    estimator.finish(4, Some(10.));
    assert_eq!(seconds(estimator.estimate([], 1)), 0.);
}

#[test]
fn extreme_extrapolation_falls_back_to_a_finite_observed_mean() {
    for (ids, expected) in [
        (vec![0, 0, 1, 1, 31], 1e250),
        (vec![30, 30, 31, 31, 0], 1e-250),
    ] {
        let mut estimator = Estimator::new(ids.into());
        for (i, duration) in [1e-250, 1e-250, 1e250, 1e250].into_iter().enumerate() {
            estimator.finish((i + 1) as u64, Some(duration));
        }
        assert_eq!(estimator.estimate([], 1), low_confidence(expected));
    }
}

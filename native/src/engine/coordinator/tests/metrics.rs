use super::*;

#[test]
fn production_step_histogram_reports_bounded_conservative_percentiles() {
    let running = background_generation_handoff_fixture().unwrap().running;
    let metrics = RunningAuthorityMetrics::new(&running);
    for _ in 0..95 {
        metrics.record_step_duration(Duration::from_micros(150), None);
    }
    for _ in 0..4 {
        metrics.record_step_duration(Duration::from_micros(900), None);
    }
    metrics.record_step_duration(Duration::from_micros(300_000), None);

    let health = metrics.snapshot();
    assert_eq!(health.step_timing_samples, 100);
    assert_eq!(health.step_timing_total_micros, 317_850);
    assert_eq!(health.step_timing_max_micros, 300_000);
    assert_eq!(health.step_timing_p95_micros, 200);
    assert_eq!(health.step_timing_p99_micros, 1_000);
    assert!(health.step_timing_histogram_consistent);
    assert_eq!(health.step_timing_bucket_counts.iter().sum::<u64>(), 100);
    assert_eq!(
        health.step_timing_bucket_upper_micros,
        STEP_TIMING_BUCKET_UPPER_MICROS
    );
}

#[test]
fn production_step_histogram_rejects_a_partial_or_saturated_prefix() {
    let running = background_generation_handoff_fixture().unwrap().running;
    let metrics = RunningAuthorityMetrics::new(&running);
    assert!(metrics.snapshot().step_timing_histogram_consistent);
    // Model an authority preempted after its bucket update, before publish.
    saturating_increment(&metrics.step_timing_buckets[0], 1);
    assert!(!metrics.snapshot().step_timing_histogram_consistent);
    metrics.step_timing_samples.store(1, Ordering::Release);
    assert!(metrics.snapshot().step_timing_histogram_consistent);
    metrics
        .step_timing_samples
        .store(u64::MAX, Ordering::Release);
    assert!(!metrics.snapshot().step_timing_histogram_consistent);
}

#[test]
fn production_step_histogram_consistent_reads_are_complete_concurrent_prefixes() {
    let running = background_generation_handoff_fixture().unwrap().running;
    let metrics = RunningAuthorityMetrics::new(&running);
    let finished = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for sample in 0..10_000 {
                metrics.record_step_duration(
                    Duration::from_micros(if sample % 2 == 0 { 100 } else { 20_000 }),
                    None,
                );
                if sample % 16 == 0 {
                    std::thread::yield_now();
                }
            }
            finished.store(true, Ordering::Release);
        });
        while !finished.load(Ordering::Acquire) {
            let health = metrics.snapshot();
            if health.step_timing_histogram_consistent {
                assert_eq!(
                    health.step_timing_bucket_counts[0],
                    health.step_timing_samples.div_ceil(2)
                );
                assert_eq!(
                    health.step_timing_bucket_counts[16],
                    health.step_timing_samples / 2
                );
                assert_eq!(
                    health.step_timing_bucket_counts.iter().sum::<u64>(),
                    health.step_timing_samples
                );
            }
        }
    });
    let final_health = metrics.snapshot();
    assert!(final_health.step_timing_histogram_consistent);
    assert_eq!(final_health.step_timing_samples, 10_000);
}

#[test]
fn production_step_histogram_distinguishes_the_realtime_gate() {
    let running = background_generation_handoff_fixture().unwrap().running;
    let metrics = RunningAuthorityMetrics::new(&running);
    for _ in 0..99 {
        metrics.record_step_duration(Duration::from_micros(16_600), None);
    }
    metrics.record_step_duration(Duration::from_micros(20_000), None);

    let health = metrics.snapshot();
    assert_eq!(health.step_timing_p99_micros, 16_667);
    assert_eq!(health.step_timing_max_micros, 20_000);
}

#[test]
fn slow_step_costs_exclude_fast_and_generation_steps_and_bound_the_remainder() {
    let running = background_generation_handoff_fixture().unwrap().running;
    let metrics = RunningAuthorityMetrics::new(&running);
    let cost = RunningStepCostMicros {
        control_selection: 7_000,
        control_index: 2_000,
        control_neural: 4_000,
        control_sensing: 2_500,
        control_inference: 1_000,
        world_step: 8_000,
        ..RunningStepCostMicros::default()
    };
    metrics.record_step_duration(Duration::from_micros(16_667), Some(cost));
    metrics.record_step_duration(Duration::from_micros(50_000), None);
    metrics.record_step_duration(Duration::from_micros(20_000), Some(cost));
    metrics.record_step_duration(
        Duration::from_micros(18_000),
        Some(RunningStepCostMicros {
            control_selection: 19_000,
            control_index: 20_000,
            control_neural: 20_000,
            control_sensing: 20_000,
            control_inference: 20_000,
            world_step: 20_000,
            ..RunningStepCostMicros::default()
        }),
    );

    let health = metrics.snapshot();
    assert_eq!(health.step_timing_samples, 4);
    assert_eq!(health.slow_step_samples, 2);
    assert_eq!(health.slow_step_control_micros, 25_000);
    assert_eq!(health.slow_step_control_index_micros, 20_000);
    assert_eq!(health.slow_step_control_neural_micros, 4_000);
    assert_eq!(health.slow_step_control_sensing_micros, 2_500);
    assert_eq!(health.slow_step_control_inference_micros, 1_000);
    assert_eq!(health.slow_step_world_micros, 8_000);
    assert_eq!(health.slow_step_other_micros, 5_000);
}

/// Terminal attribution partitions the service clock, bounds nested costs and saturates counters.
#[test]
fn terminal_step_costs_partition_successful_service_without_changing_step_metrics() {
    let running = background_generation_handoff_fixture().unwrap().running;
    let metrics = RunningAuthorityMetrics::new(&running);
    let cost = RunningStepCostMicros {
        control_selection: 7_000,
        world_step: 8_000,
        generation_preparation: 20_000,
        generation_evolution: 17_000,
        generation_evolution_summary: 6_000,
        generation_evolution_reproduction: 10_000,
        generation_admission: 10_000,
        ..RunningStepCostMicros::default()
    };
    metrics.record_step_duration(Duration::from_micros(50_000), None);
    assert_eq!(metrics.snapshot().terminal_step_samples, 0);
    metrics.record_terminal_step_duration(Duration::from_micros(50_000), cost);
    metrics.record_terminal_step_duration(
        Duration::from_micros(18_000),
        RunningStepCostMicros {
            control_selection: 1_000,
            world_step: 2_000,
            generation_preparation: 20_000,
            generation_evolution: u64::MAX,
            generation_evolution_summary: 4_000,
            generation_evolution_reproduction: u64::MAX,
            generation_admission: u64::MAX,
            ..RunningStepCostMicros::default()
        },
    );
    let health = metrics.snapshot();
    assert_eq!(health.step_timing_samples, 1);
    assert_eq!(health.slow_step_samples, 0);
    assert_eq!(health.terminal_step_samples, 2);
    assert_eq!(health.terminal_step_total_micros, 68_000);
    assert_eq!(health.terminal_step_max_micros, 50_000);
    assert_eq!(health.terminal_step_control_micros, 8_000);
    assert_eq!(health.terminal_step_world_micros, 10_000);
    assert_eq!(health.terminal_step_preparation_micros, 35_000);
    assert_eq!(health.terminal_step_evolution_micros, 32_000);
    assert_eq!(health.terminal_step_evolution_summary_micros, 10_000);
    assert_eq!(health.terminal_step_evolution_reproduction_micros, 21_000);
    assert_eq!(health.terminal_step_admission_micros, 10_000);
    assert_eq!(health.terminal_step_other_micros, 5_000);
    metrics
        .terminal_step_total_micros
        .store(u64::MAX - 1, Ordering::Relaxed);
    metrics.record_terminal_step_duration(Duration::from_micros(2), cost);
    assert_eq!(metrics.snapshot().terminal_step_total_micros, u64::MAX);
    metrics
        .terminal_step_evolution_summary_micros
        .store(u64::MAX - 1, Ordering::Relaxed);
    metrics
        .terminal_step_evolution_reproduction_micros
        .store(u64::MAX - 1, Ordering::Relaxed);
    metrics.record_terminal_step_duration(Duration::from_micros(50_000), cost);
    let health = metrics.snapshot();
    assert_eq!(health.terminal_step_evolution_summary_micros, u64::MAX);
    assert_eq!(health.terminal_step_evolution_reproduction_micros, u64::MAX);
}

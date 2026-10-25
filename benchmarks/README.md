# Benchmark and experiment code

Maintained benchmark programs live under `scripts/qualification/` and run through the manual qualification workflow. Their generated outputs are ignored or retained as CI artifacts.

The rejected ray-reuse code is retained only for occasional inspection on `archive/experiment/sensor-ray-reuse-20261025`; it has no dedicated CI. The redundant crowded-sensors branch was retired. The rejected pellet-distance code remains at the existing `qualification-resume-20261025` tag.

That unchanged tag also preserves the historical one-off drivers, exceptional source patches and old migration metadata under `benchmarks/` and `docs/performance/`. They are archival Git content, not current runtime dependencies or reconstruction requirements. New alternatives use temporary `experiment/...` branches and ordinary commits.

Existing P1 profiling in `docs/performance/sensor-v4/README.md` attributes about
10.06 seconds of slow-step work to sensing versus 1.04 seconds to inference in
that recorded window. Serialization, persistence and boundary timings need a
separate workload-specific investigation. This cleanup changes none of those
paths and adds no SIMD or sensor optimization.

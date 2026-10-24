# Cached pellet-distance screen, 2026-10-25

The isolated `evolved-lives-pellet-distance` branch preserves the cached squared-distance prototype with a subnormal/exceptional-norm fallback. All 813 Windows native library tests pass, including exact Float32 ordinary/trace comparisons. The production branch does not adopt this candidate.

| Paired Windows median | Baseline | Candidate |
|---|---:|---:|
| P0 mean service | 3.47695 ms | 3.40981 ms |
| P0 sensing | 1.13357 ms | 1.17482 ms |
| P1 mean service | 14.26433 ms | 14.26230 ms |
| P1 sensing | 7.79987 ms | 7.89556 ms |

The maintained ABBA driver verifies exact continuation, authority/capture/distribution witnesses, complete requested work and unchanged positive native memory reservation. P1 service is flat and sensing is slower, so the screen does not justify adoption or another target-platform build. Shared-host aggregate counters do not establish an idle machine.

`summary.json` retains the compact decision; `catalog.json` identifies every report, counter and provenance object in the immutable [release asset](https://github.com/Tits-McGruff/slither_neuroevo/releases/tag/qualification-resume-20261025). The asset was downloaded afresh and all files restored and rehashed. Source is recorded by exact Git commits, path/hash/EOL inventories and toolchain/build/settings identities; no source copies are stored in the asset.

Restore with the maintained PyRL tool:

```sh
python ../PyRL-trainer/pyrl_trainer/evidence_store.py fetch --catalog docs/performance/sensor-v4/pellet-distance-screen-20261025/catalog.json --cache .cache/qualification-evidence
```

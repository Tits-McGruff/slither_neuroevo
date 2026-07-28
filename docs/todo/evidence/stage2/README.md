# Stage 2 evidence index

This directory holds reproducible evidence for the approved Rust-authoritative
migration. Raw JSON is retained under `windows-5800x/`. These are development-
machine results, not results from the required Ryzen 7 2700 Debian VM.

## Evidence classes

- Current-source facts are proved by the named source and tests.
- Git-history findings remain in
  `docs/todo/evidence/2026-07-29-stage1-git-history.md`.
- Prior planning measurements remain provisional until a retained fixture
  reproduces them.
- Derived arithmetic is labelled as calculation rather than measurement.
- Raw JSON produced by the Stage 2 runners is a new measured result and records
  its source commit, dirty flag, machine and runtime.

## Reproducible commands

Run from the repository root with the direct `tsx` entry point:

```powershell
node .\node_modules\tsx\dist\cli.mjs scripts\stage2\database-baseline.ts --db data\slither.db --output result.json
node .\node_modules\tsx\dist\cli.mjs scripts\stage2\codec-baseline.ts --scenario P0 --fixture fresh --output result.json
node .\node_modules\tsx\dist\cli.mjs scripts\stage2\codec-baseline.ts --scenario P2 --fixture evolved --evolution-generations 25 --output result.json
node .\node_modules\tsx\dist\cli.mjs scripts\stage2\runtime-baseline.ts --scenario P2 --backend native --workers 4 --warmup-steps 20 --steps 180 --frame-every 1 --output result.json
node .\node_modules\tsx\dist\cli.mjs scripts\stage2\behavior-baseline.ts --output result.json
node .\node_modules\tsx\dist\cli.mjs scripts\stage2\graph-baseline.ts --db data\slither.db --output result.json
```

The runtime runner uses the real `SimCore`, `World`, heterogeneous population,
sensors, recurrent brains, physics and frame-v1 serializer. A positive worker
count uses the canonical Node `BrainPool`. It deliberately labels itself as a
direct-engine measurement: it is not a substitute for the later real server,
LAN browser, RL client or Debian VM runs.

## Newly reproduced database result

The read-only artifact
`windows-5800x/database-slither-db.json` identifies the inspected database by
SHA-256
`9b8774387cff7aa82e64dbf75f4807d06ada9072814d24b71b8c76c4fe4bd8a4`.
It reproduces the prior 5,921,520/6,041,600-byte observation:

- database file: 6,041,600 bytes;
- two format-v2 generation-one `run-start` snapshots;
- 110 genome rows and 5,921,520 weight bytes in total;
- 2,960,760 weight bytes per 55-genome snapshot;
- 13,458 Float32 weights per genome;
- WAL: 0 bytes at capture; shared-memory file: 32,768 bytes;
- page size 4,096, page count 1,475, free-list count zero;
- Hall of Fame, graph presets and players are empty.

This file proves the raw default-population arithmetic. It does not reproduce
overnight accumulation, Hall-of-Fame growth, evolved compression, WAL growth or
vacuum behavior because the inspected database contains only two run-start
checkpoints.

## Newly reproduced codec results

All sizes below are exact weight payloads; future container, state, history and
small metadata are reported separately.

| Fixture | Raw | Decimal JSON bytes per Float32 | Decimal JSON/raw | Plain Zstd reduction | Shuffled Zstd reduction | Adaptive choice | Adaptive stored |
|---|---:|---:|---:|---:|---:|---|---:|
| P0 fresh | 2.82 MiB | 20.11998 | 5.030x | 7.678% | 14.646% | shuffled Zstd | 2.41 MiB |
| P0 evolved-like, 25 operator generations | 2.82 MiB | 20.04202 | 5.011x | 28.55% | 23.12% | plain Zstd | 2.02 MiB |
| P2 fresh | 84.53 MiB | 20.11 | 5.03x | 7.86% | 14.75% | shuffled Zstd | 72.06 MiB |
| P2 evolved-like, 25 operator generations | 84.53 MiB | 20.03 | 5.01x | 12.20% | 16.99% | shuffled Zstd | 70.17 MiB |

The P0 fresh fixture reproduced the prior planning percentages to rounding:
7.68% plain and 14.65% shuffled. Its exact plain result was 2,733,420 bytes,
not the prior unretained 2,733,479-byte claim. The exact JSON observation was
20.11998 bytes per Float32 for fresh P0 and about 20.03–20.04 for the evolved
fixtures; the plan must use the retained values rather than preserve the old
approximately-20.03 claim indiscriminately.

The evolved P0 fixture is an important reason to keep adaptive encoding:
plain Zstandard beat shuffled Zstandard there. Each decoded result was
bit-exact. Adding a Zstandard frame checksum added four bytes to each whole-
population frame. Single-run timing is retained in raw JSON but is not yet a
stable latency conclusion; repeated Windows and target-VM trials are still
required.

Derived from these measured payloads, 22 automatic P0 weight payloads would use
about 44.4–53.0 MiB. Twenty-two P2 payloads would use about 1.51–1.55 GiB.
Those are calculations from measured codec sizes, not complete-checkpoint disk
measurements. They support the selected retention counts and 4 GiB automatic
cap for P0/P2, subject to real full-checkpoint, Hall-of-Fame and VM free-disk
measurement.

## Initial Windows runtime measurements

The following are single direct-engine runs on a Ryzen 7 5800X, Windows 11,
Node 24.12.0. They are useful starting evidence, not Debian acceptance results.

| Workload/path | Sim seconds per wall second | Fixed-step p50 | p95 | p99 | Mean sensors | Mean brain |
|---|---:|---:|---:|---:|---:|---:|
| P0 JS serial | 1.72x | 9.41 ms | 11.60 ms | 12.44 ms | 6.84 ms | 0.86 ms |
| P0 native serial | 1.69x | 9.47 ms | 12.08 ms | 13.54 ms | 7.19 ms | 0.54 ms |
| P0 native, 4 Node workers | 1.79x | 9.10 ms | 11.12 ms | 11.81 ms | 7.04 ms | 0.33 ms |
| P1 JS serial | 0.36x | 44.81 ms | 66.01 ms | 69.18 ms | 38.97 ms | 4.17 ms |
| P1 native serial | 0.36x | 44.32 ms | 64.48 ms | 69.70 ms | 39.67 ms | 2.46 ms |
| P1 native, 4 Node workers | 0.37x | 44.12 ms | 61.70 ms | 68.01 ms | 39.93 ms | 1.07 ms |
| P2 JS serial | 0.51x | 31.76 ms | 40.88 ms | 42.93 ms | 8.62 ms | 21.71 ms |
| P2 native serial | 1.00x | 15.99 ms | 19.65 ms | 20.47 ms | 8.28 ms | 5.80 ms |
| P2 native, 4 Node workers | 1.43x | 11.24 ms | 14.05 ms | 14.95 ms | 6.93 ms | 2.69 ms |
| P3 native serial, capacity sample | 0.18x | 94.96 ms | 104.29 ms | 106.95 ms | 56.44 ms | 32.91 ms |
| P4 native dense fixture | 0.37x | 24.44 ms | 120.70 ms | 173.36 ms | 25.79 ms | 0.92 ms |

The P4 run starts with 217,000 body points and then rapidly loses snakes, so it
proves the initial collision/frame spike and current full-frame scale, not
sustained dense-world capacity. Its mean frame was about 2.06 MiB and its p99
step was 173.36 ms.

These results reproduce the product failure on the faster development CPU:
P1, P3 and the dense initial P4 state cannot sustain 1x. P1 remains about
0.36–0.37x because sensing dominates; the Node worker/native-kernel stack
barely helps. P2 benefits from native arithmetic and workers, but the production
path still performs three native layer calls per differently weighted
population evaluation. The worker artifacts retain the exact derived crossing
count from observed batch population counts.

## Still open before the Stage 2 exit gate

- the same clean runners on the Ryzen 7 2700 Debian VM;
- integrated real-server/browser/LAN/RL P5 and cadence measurements;
- sustained P4, P6 accelerated, P7 soak and P8 overnight-equivalent fixtures;
- browser heap traces for current import/export;
- P1/P3 codec fixtures and full managed-checkpoint container validation timing;
- the permitted narrow SQLite byte-volume experiment and page-reuse behavior;
- real owner save files outside the repository, if any;
- Debian graph-ordering output from the exact retained database/spec fixture.

No current Windows number is presented as proof that P0, P1 or P2 already meets
the target VM gate.

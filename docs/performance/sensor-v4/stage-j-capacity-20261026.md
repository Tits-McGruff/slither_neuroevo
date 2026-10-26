# Bounded Stage J capacity check (2026-10-26)

The production checkpoint envelope remains 512 MiB. Motion/B32 with 300 large
genomes requires 593,324,505 decoded bytes (565.84 MiB) and does not fit.
No graph dimensions, scratch limits or production envelopes were changed to pass.
This is a bounded Windows storage/live-overlap check, not maximum-capacity or
six-hour soak qualification. Motion/B16 default selection is measured separately.

## Source and configuration

Clean Slither `48e9337ecbb56539caa57e15ab2fa3925d7f4439`, native source identity
`6583c56c8231ae8aa7f83d51dcc353dac0b29079ebf02183a6b291c1d6074d57`, Windows
x86_64 MSVC, Rust 1.92, Node 24.19.0, seed 42, SSE2. PyRL for the live window:
clean `221cd6af46dfe38152ab2b32dbc3aefc7e64465b`, Python 3.12.10,
Torch 2.14.0+cu126, Float32, TF32 off, two trainer CPU threads.

Both graphs use Motion/B32 (504 inputs): small has 40,402 parameters per genome;
large has 494,306. Native checkpoint/worker scratch each remain 512 MiB and the
process admission ceiling is 4 GiB. Teacher and neural cadence both equal 1/60 s.
The portable fixture generator reads that admitted cadence rather than embedding
the old 0.06 value. Constructor tests also cover an explicitly requested 0.02 s.

## Storage and continuation

The existing `checkpoint-capacity-qualification` binary and
`scripts/qualification/checkpoint-capacity.ts` exercise initial/generation
checkpoints, exact restore/continuation, SQLite publication, 100 saved brains,
101st-record rejection without publishing files, four catalog pages, export of
the selected original files, import, three post-import generations and deletion
of the catalog while preserving population checkpoints. Source/replacement
inspector records remain bounded (at most 14,384 bytes here).

| Population / graph / encoding | Archive MiB | Native peak RSS MiB | Archive worker peak RSS MiB | Node peak RSS MiB | Elapsed s |
|---|---:|---:|---:|---:|---:|
| 300 / small / compressed | 56.85 | 112.92 | 115.97 | 242.06 | 12.94 |
| 55 / large / compressed | 279.80 | 226.72 | 228.12 | 183.89 | 25.86 |
| 55 / large / raw | 296.65 | 224.57 | 226.50 | 183.71 | 24.22 |

All listed correctness checks passed. Peaks belong to separate phases/processes;
they are not a summed simultaneous-memory guarantee. The metadata-only 300-large
audit allocates no numeric payload. Its conservative two-state estimate is
4,260,002,462 bytes, only about 32 MiB below 4 GiB. A 768-MiB decoded envelope
fits the metadata calculation but has not been admitted as a production change
or qualified through the allocated/live path.

## CUDA overlap and limits

The existing `live-distillation.ts` ran 55 large random genomes with evolved
respawn, ten stochastic v2 teachers, four separately stateful shadows, 100 actual
saved brains, a live inspector, 256-MiB replay and 4-GiB spool quota. Five Rust
workers were used. This capacity fixture deliberately retains its separate
900/2200 pellet/segment and 1/512/131072 ray tuple; it is not the selected B16 P0.

A fresh trainer reproduced the update-126 witness exactly and reached 250
updates. The overlap window added 93 updates in 60.671 active seconds, with
186,600 training samples. Three manually requested generations durably committed;
cold SQLite/native restore matched generation four and rejected seven invalid
restore admissions. All 100 catalog originals remained hash-valid after restart.
Each shadow advanced 646 samples. The inspector delivered 265 additional samples,
reacquired across all three boundaries and reached a maximum of 14,520 wire bytes.

The complete 65.415-second window advanced 3,722 steps (0.94831 sim/wall), including
generation durability holds. No wall debt was discarded, but maximum window lag
was 3.381 seconds; this does not meet sustained real time. P99 was at most 24 ms.
Neural queries and all rays were unsaturated; baseline pellet queries saturated
in 294/22,952 observations (1.281%). The stress configuration fails the ordinary
query-quality criterion. These results justify no broader real-time promotion.

Host RSS peaked at 397.68 MiB; trainer RSS at 1,477.27 MiB; replay at 183.21 MiB;
Torch device allocation/reservation at 49.22/58 MiB. Capture settled at chunk
1,524 with zero native/spool drops and 177.18 MiB retained. Trainer cursor advanced
16 -> 768 while producer advanced 1,121 -> 1,524: progress does not prove catch-up.
Eight exported aggregate observations cover Windows, running WSL and overall GPU
use: CPU 19..33%, GPU 0..29%, VRAM 1,303..1,627 MiB, including other workloads.
No unrelated process inventory is published; these samples do not prove idle load.

## Reproduction and local outputs

Use the existing capacity binary/host with `--coexistence library-inspector` and
the table's population/graph/encoding at `--envelope-mib 512`. Generate the large
portable inputs with the same binary, then give `live-distillation.ts`
`--workload P0 --capacity-graph large --checkpoint-envelope-mib 512 --workers 5
--device cuda:0 --generation-boundaries 3 --contention-seconds 60`, its generated
student/catalog inputs and the PyRL checkout/interpreter. Run/export aggregate
resource observations alongside it. No new benchmark/archive framework is used.

Ignored local outputs: `data/benchmarks/stage-j-capacity-20261026-cadence/`.
SHA-256 of `300-small-compressed-512.json`, `55-large-compressed-512.json` and
`55-large-raw-512.json`, respectively:

```text
52c0761cff62b9fe6f1c0a1e104cf1838aee1f1d2c00cd07edc26a46721a9035
6d77ee5e6ce22b664f68dae9f467b64ae532b1f553587b25d762436a81d9c22f
70d1a89147add2c306be39086c79943c78a50ab68c138b0bd4f1cfa3298e6dbd
```

Live report `integrated-cuda-p0-large-512.json`:
`082f64d42af4b0bc4ac79aaaed559662e077c4f91f46caf8f0257195ba935caf`.
Exported `integrated-resources.public.jsonl`:
`96658872527023924798a95938279bdea22e15392c35568d5412367e1014ef83`.
Full run outputs stay local/CI artifacts; models, databases and source snapshots
are not added to Git. Maximum occupancy, crowded learned policies and long-run
soak remain unproven by this bounded check.

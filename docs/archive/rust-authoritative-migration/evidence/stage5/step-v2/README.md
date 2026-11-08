# Stage 5 reusable single-worker complete-step evidence

These reports measure the current reusable single-worker Rust fixed-step
transaction on the owner VM after the first allocation-removal pass. They are
a Stage 5 profiling checkpoint, not a Stage 5 exit gate or production-cutover
claim.

## Source and environment

All eight reports were built from the same working-source overlay on base
commit `7925faf7aef3` and identify that source as:

- native build: `slither_native/0.1.0+7925faf7aef3.9912eff9755c5a52`;
- native source SHA-256:
  `4d83339805fe5f5737f7131b0e68faee3fd0a15731bc98cb1c5fa5462f14de50`;
- build contract SHA-256:
  `sha256:1fb16b35a1b51f7f0cd24193df2af9668ae71c93f702311812c144c61e047cd1`;
- transferred overlay SHA-256:
  `e1db2f7bb0bde6957b6bfa9e317c68cb5d4088f757584465a8a7c6c41858e0e9`;
- target: `x86_64-unknown-linux-gnu`, release test-hooks build with
  `rustc 1.92.0 (ded5c06cf 2025-12-08)`;
- VM: Debian host `oxygen`, AMD Ryzen 7 2700, eight available logical CPUs and
  16,775,352,320 bytes of assigned memory.

The source was transferred into the disposable
`/tmp/slither-stage5-7925faf7` checkout and built there. The live
`/opt/apps/slither_neuroevo` checkout and its saves were not changed. The
working overlay is not yet a clean Git commit, so these are exact source-hash
measurements but not clean-commit evidence. Repeat the required gates from a
committed source before production cutover.

## Measured path

Each report uses one Rust coordinator/calculation worker, one explicitly bound
neural math backend, differently weighted and stateful population brains, ten
baseline bots, five-point bodies, 3,500 pellets, corrected sensor v3, complete
heterogeneous graph evaluation, movement, food, swept collision, effects,
accounting and one atomic authoritative publication.

The measured interval excludes scheduler pumping, Node/N-API, browser and RL
delivery, frame packing, generation transition/evolution, persistence, a
sustained round and the future calculation-worker pool. Those gates remain
open.

Every report uses three stateful warm-up steps followed by 30 stateful measured
steps. All expected evolved and baseline snakes remained alive for the
retained interval. Longer round/soak behavior is a separate requirement.

The command shape was:

```text
/tmp/slither-stage5-7925faf7/native/target/release/stage5-step-benchmark \
  --scenario P0 \
  --math-backend sse2 \
  --warmup-steps 3 \
  --steps 30 \
  --environment owner-target-vm \
  --output /tmp/stage5-step-v2-current/oxygen-P0-sse2.json
```

The scenario and backend arguments were varied across P0-P3 and
`scalar`/`sse2`. Each JSON document retains its exact command.

## Results

Times are milliseconds. `sim/wall` is achieved simulated seconds per wall
second for the measured fixed steps. The final acceptance target is at least
0.98 over ten minutes through the production boundaries; this 30-step
synthetic measurement is an earlier diagnostic only.

| Scenario/backend | Mean | p95 | p99 | Control p95 | Physics p95 | Publish p95 | sim/wall | Alloc ops/step | Peak RSS |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| P0 scalar | 7.85 | 8.96 | 9.41 | 6.57 | 2.48 | 0.09 | 2.12 | 8.47 | 10.2 MiB |
| P0 SSE2 | 7.17 | 7.94 | 8.67 | 5.51 | 2.35 | 0.10 | 2.32 | 8.47 | 10.3 MiB |
| P1 scalar | 38.05 | 43.59 | 44.12 | 31.81 | 11.47 | 0.25 | 0.44 | 17.40 | 24.1 MiB |
| P1 SSE2 | 32.93 | 35.51 | 36.41 | 25.20 | 9.85 | 0.22 | 0.51 | 17.40 | 24.5 MiB |
| P2 scalar | 29.53 | 31.68 | 46.66 | 28.22 | 3.05 | 0.11 | 0.56 | 8.37 | 92.2 MiB |
| P2 SSE2 | 15.12 | 17.49 | 17.86 | 14.48 | 2.36 | 0.11 | 1.10 | 8.37 | 92.2 MiB |
| P3 scalar | 147.44 | 150.28 | 152.19 | 140.61 | 10.42 | 0.23 | 0.11 | 17.00 | 471.3 MiB |
| P3 SSE2 | 84.39 | 100.12 | 100.71 | 87.97 | 11.55 | 0.29 | 0.20 | 17.00 | 470.9 MiB |

P0 clears this isolated rate checkpoint. P2 SSE2 clears the short average-rate
checkpoint at 1.10 simulated seconds per wall second, but its 17.49 ms p95 and
17.86 ms p99 remain above one 60 Hz interval. The approved P2 rule permits the
later sustained ratio/generation gate when rare spikes do not accumulate
debt; this short run cannot establish that. P1 remains far below real time.
P3 remains the owner-selected capacity case and is also below real time.
Control selection, dominated by sensing and heterogeneous inference, remains
the largest share of every missed target. Bounded calculation-worker work is
still required; these results do not justify weakening sensors, collision,
physics or workload.

## Allocation interpretation

The earlier `step-v1` checkpoint recorded about 920 allocation operations per
P0/P2 step and 1,132 per P1/P3 step. This source records means of 8.37-17.40.
The large reduction follows reuse fixes in serialized RNG handling, effect and
physics staging, baseline-control results and authoritative validation.
Publication records zero allocation operations in all 240 measured passes.

The retained JSON reports count allocator operations by phase. They do not
correlate an individual operation with a particular retained-buffer capacity
change. Equal first/final capacities also cannot prove that no intermediate
growth occurred. These reports therefore do not classify the remaining
operations as a fixed hot-path floor, do not prove that every nonzero count was
capacity growth and do not claim an allocation-free steady state. A future
fixed-versus-growth claim requires per-step correlation or a controlled
warmed/no-growth fixture.

The P0 and P2 reports contain measured steps with zero allocation operations;
P1 and P3 do not during this short evolving-state interval. This is raw
evidence only, not a steady-state guarantee.

## Review correction retained before measurement

Independent review found that retained Gaussian-spare scratch could keep a
larger logical length after a baseline-count reduction and reject a later
pellet-generating effect. The implementation now uses only the active prefix
while retaining spare storage. A focused three-baseline-to-one-baseline
regression exercises generated death pellets and proves exact pellet, RNG,
allocator and baseline-death continuation against a fresh workspace, followed
by a second exact continuation. The eight reports were rebuilt only after that
correction.

## Comparison with `step-v1`

The two checkpoints are not a statistically controlled before/after study;
they are short sequential VM runs. The most useful comparison is the
allocation-order reduction and the unchanged location of the performance
limit.

| Scenario/backend | v1 mean | v2 mean | v1 alloc ops | v2 alloc ops |
|---|---:|---:|---:|---:|
| P0 SSE2 | 8.05 | 7.17 | 920.3 | 8.47 |
| P1 SSE2 | 36.67 | 32.93 | 1,132.4 | 17.40 |
| P2 SSE2 | 17.59 | 15.12 | 920.2 | 8.37 |
| P3 SSE2 | 80.36 | 84.39 | 1,132.3 | 17.00 |

P3 timing became noisier/slower rather than being presented as an improvement.
The data supports removing the repeat-allocation defect; it does not show that
allocation removal alone solves the many-snake or largest-brain throughput
problem.

## Artifact hashes

| File | SHA-256 |
|---|---|
| `oxygen-P0-scalar.json` | `bb9aafa045b4c37668e053d471eb798252db7779f6186630ee9d31aaa5080e8e` |
| `oxygen-P0-sse2.json` | `a188ad16b6d2c3a81610b31296af73729fb78173314dfdd29c738250c8abbfa5` |
| `oxygen-P1-scalar.json` | `c98ef423972574940ad27d4cfef9927bf2188d5e5dfb3c7fc3eec3cfff0eaa2b` |
| `oxygen-P1-sse2.json` | `e1e9d848baae67c2ed68d58274f2629c97cbf06c589a776ae52605b997eefcac` |
| `oxygen-P2-scalar.json` | `362d0ad734c731a1413866189f7553cf6aa3511bf38abd7cc2ccddf63cf01805` |
| `oxygen-P2-sse2.json` | `9faafe1c45e2bca35e51e3efdd1139f7bf81082d430ed6c28a12f6b6e9ed8398` |
| `oxygen-P3-scalar.json` | `933364bfe5675fb4e300394a64bc27e421b5f2addb8e6c4a9a246dd78bffaa1a` |
| `oxygen-P3-sse2.json` | `5134555c8de14e2dbf98d26219121f422ef0b0e7567cb1c7f7699ba3a26066e5` |

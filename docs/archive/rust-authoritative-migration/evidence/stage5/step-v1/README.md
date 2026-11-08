# Stage 5 single-worker complete-step evidence

These reports measure the first complete nonterminal Rust fixed-step transaction
on the owner VM. They are a profiling checkpoint, not a Stage 5 exit-gate or
production-cutover claim.

## Source and environment

All eight reports were built from the same working-source overlay on base commit
`7925faf7aef3` and identify that source as:

- native build: `slither_native/0.1.0+7925faf7aef3.382dbd50f9131fa5`;
- native source SHA-256:
  `b59cf03ff5a67db7fee1908290282621c5e465bd6fd41ac0bdce256ad20d7f97`;
- build contract SHA-256:
  `sha256:1fb16b35a1b51f7f0cd24193df2af9668ae71c93f702311812c144c61e047cd1`;
- target: `x86_64-unknown-linux-gnu`, release test-hooks build with
  `rustc 1.92.0 (ded5c06cf 2025-12-08)`;
- VM: Debian host `oxygen`, AMD Ryzen 7 2700, eight available logical CPUs and
  16,775,352,320 bytes of assigned memory.

The source was transferred into `/tmp/slither-stage5-7925faf7` and built there.
The live `/opt/apps/slither_neuroevo` checkout and its saves were not changed.
The working overlay is not yet a clean Git commit, so these are exact
source-hash measurements but not clean-commit evidence. Repeat them after the
same source can be committed and before treating the numbers as a cutover gate.

## Measured path

Each report uses one Rust coordinator/calculation worker, one explicitly bound
neural math backend, differently weighted and stateful population brains, ten
baseline bots, five-point bodies, 3,500 pellets, corrected sensor v3, complete
heterogeneous graph evaluation, movement, food, swept collision, effects,
accounting, and the atomic authoritative publication.

The measured interval excludes scheduler pumping, Node/N-API, browser and RL
delivery, frame packing, generation transition/evolution, persistence, a
sustained round and the future calculation-worker pool. Those gates remain
open.

Every valid report uses three stateful warm-up steps followed by 30 stateful
measured steps. Attempts to extend P1 to 60 or 120 measured steps encountered a
real collision/death, after which the remaining samples would no longer be a
full 300-snake workload; the benchmark rejected those runs and no failed report
was retained. Sustained round/death behavior needs its separate soak evidence.

The command shape was:

```text
/tmp/slither-stage5-7925faf7/native/target/release/stage5-step-benchmark \
  --scenario P0 \
  --math-backend sse2 \
  --warmup-steps 3 \
  --steps 30 \
  --environment owner-target-vm \
  --output /tmp/stage5-step-v1/oxygen-P0-sse2.json
```

The scenario and backend arguments were varied across P0-P3 and
`scalar`/`sse2`. Each JSON document retains its exact command.

## Results

Times are milliseconds. `sim/wall` is achieved simulated seconds per wall
second for the measured fixed steps. The target for real-time 1x execution is
at least 1.0 before adding the still-excluded production boundaries.

| Scenario/backend | Mean | p95 | p99 | Control p95 | Physics p95 | Publish p95 | sim/wall | Alloc ops/step | Peak RSS |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| P0 scalar | 8.76 | 11.22 | 11.64 | 7.92 | 3.10 | 0.39 | 1.90 | 920.3 | 10.1 MiB |
| P0 SSE2 | 8.05 | 9.42 | 9.54 | 6.49 | 2.66 | 0.31 | 2.07 | 920.3 | 10.4 MiB |
| P1 scalar | 39.89 | 43.89 | 44.31 | 31.51 | 11.67 | 0.63 | 0.42 | 1,132.4 | 24.1 MiB |
| P1 SSE2 | 36.67 | 47.73 | 52.96 | 32.95 | 12.57 | 0.69 | 0.45 | 1,132.4 | 24.2 MiB |
| P2 scalar | 28.93 | 30.31 | 30.67 | 27.57 | 2.66 | 0.30 | 0.58 | 920.2 | 92.1 MiB |
| P2 SSE2 | 17.59 | 19.16 | 19.66 | 16.08 | 2.68 | 0.35 | 0.95 | 920.2 | 92.5 MiB |
| P3 scalar | 151.18 | 155.71 | 156.49 | 144.46 | 11.51 | 0.62 | 0.11 | 1,132.3 | 470.9 MiB |
| P3 SSE2 | 80.36 | 89.14 | 89.56 | 77.86 | 11.45 | 0.60 | 0.21 | 1,132.3 | 471.1 MiB |

P0 clears the isolated complete-step real-time rate. P1 and P2 do not; P3 is
the owner-selected capacity case and is also below real time. Control selection
dominates each miss. Stage 4 evidence already separates that path further and
shows P1 corrected sensing, rather than graph arithmetic, as the largest known
share. The 920-1,132 measured allocator operations per step are also not
acceptable as a finished hot path. These results therefore direct the next work
toward removing repeat allocations and the approved bounded calculation-worker
path; they do not justify weakening sensors, collisions, physics or workload.

The higher P1 SSE2 p95/p99 despite a lower mean than scalar is retained rather
than smoothed away. Thirty samples are sufficient for this migration checkpoint
but not for a final tail-latency gate; the later sustained VM/server runs must
re-measure tail behavior.

## Artifact hashes

| File | SHA-256 |
|---|---|
| `oxygen-P0-scalar.json` | `821f4d014165dc0afef444b97116257f7b3f3dbbb34daedc5b2134b245ecc3a3` |
| `oxygen-P0-sse2.json` | `bf412f52d517e0aacb7dfc008b2419ac7ebe7d352a28745633be364edf9a4ae3` |
| `oxygen-P1-scalar.json` | `553389fbb8a807289617c27f299625f4ac8a57a9fd4151c8d7abe7a12e02906a` |
| `oxygen-P1-sse2.json` | `6b8f6458cb87b3fd980e6216c420bbbde4e4f863d1838ddde8f62a72c050b653` |
| `oxygen-P2-scalar.json` | `ac3da21a640307b691238d60a6a9a4cf82759479483b2635e28b35dac847f451` |
| `oxygen-P2-sse2.json` | `529d5f49fd2a98e476743f0207a3187ddcdc14d4791eef896a72d15414d9911d` |
| `oxygen-P3-scalar.json` | `ff0dc4141359c6efe8fa273e76d2e0da2e04fe4455d333292e7ba06dcd842cbd` |
| `oxygen-P3-sse2.json` | `9ccf7d16f4b255969fc2bd9eac2a1e7742ecfe9c0f34789a9dd4baf0af74a7f7` |

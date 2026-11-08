# Oxygen current-server P6 accelerated-control evidence

This directory retains the 30-run Stage 2 P6 matrix measured on the owner's
Debian VM, `oxygen`. It is current TypeScript-authoritative/current-server
evidence, not Rust-authoritative acceptance evidence.

Every run records clean source commit
`284ed4484f242be3c1dee2d8aa78ba514f48eac7`, the current real server, native
serial neural kernels, and a Protocol 2 wire-compatible synthetic loopback bot.
The environment declarations validate Debian Linux, hostname `oxygen`, an AMD
Ryzen 7 2700, eight logical CPUs, and 15.62 GiB visible to the VM (the owner's
16 GB allocation). P0, P1, and
P2 were each measured at requested 1x, 2x, 4x, 8x, and 12x speeds, once with no
display client and once with one complete-frame-v1 WebSocket spectator.

There is no browser player in these runs. Viewer-off runs contain one bot
controller socket. Viewer-on runs contain one bot controller and one spectator
socket. Both are loopback clients in the measured Node process. The spectator
counts frames and bytes but does not parse or render them in a browser and does
not cross the LAN.

Each independent launch waited until an observed tick at or beyond 300 and
then waited for at least 1,800 additional committed steps, equal to 30
simulated seconds at 60 Hz. Actual start ticks were 300–308 and measured spans
were 1,800–1,807 steps because health polling can overshoot a boundary. The
tables report the runner-monotonic simulated-seconds-per-wall-second result and
its ratio to the requested speed. Dropped seconds are discarded simulated-time
debt from the scheduler counters. Off/on values are independent runs in that
order, not paired causal measurements of spectator cost.

## P0 results

P0 is 55 evolved snakes, 10 baseline bots, the default graph, 16 sensor bins,
and approximately 3,500 pellets.

| Requested | Achieved off/on | Requested ratio off/on | Dropped seconds off/on | Bot observations/s off/on | Viewer-on frames (mean/p95 interval; mean bytes) | Event-loop p95 ms off/on | Maximum recorded RSS MiB off/on |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1x | 1.001x / 1.000x | 100.1% / 100.0% | 0.00 / 0.00 | 60.00 / 60.01 | 864 (34.7/39.8 ms; 75,029 B) | 21.5 / 18.8 | 212.2 / 211.8 |
| 2x | 1.818x / 1.769x | 90.9% / 88.5% | 5.72 / 5.28 | 107.59 / 101.03 | 16 (1,029.4/1,445.3 ms; 75,853 B) | 52.8 / 55.3 | 214.4 / 219.5 |
| 4x | 1.662x / 1.731x | 41.5% / 43.3% | 43.05 / 39.87 | 93.69 / 100.80 | 15 (1,134.1/1,324.7 ms; 75,638 B) | 52.4 / 54.2 | 216.7 / 216.4 |
| 8x | 1.740x / 1.941x | 21.8% / 24.3% | 112.77 / 96.97 | 98.85 / 109.91 | 15 (1,027.7/1,323.2 ms; 75,548 B) | 60.9 / 48.9 | 222.8 / 213.5 |
| 12x | 1.770x / 1.737x | 14.8% / 14.5% | 177.18 / 180.22 | 104.86 / 93.74 | 15 (1,142.2/1,420.8 ms; 75,873 B) | 57.2 / 53.1 | 219.9 / 215.5 |

## P1 results

P1 is 300 evolved snakes, 10 baseline bots, the default graph, 16 sensor bins,
and approximately 3,500 pellets.

| Requested | Achieved off/on | Requested ratio off/on | Dropped seconds off/on | Bot observations/s off/on | Viewer-on frames (mean/p95 interval; mean bytes) | Event-loop p95 ms off/on | Maximum recorded RSS MiB off/on |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1x | 0.793x / 0.867x | 79.3% / 86.7% | 14.73 / 7.00 | 45.99 / 49.96 | 103 (298.1/1,876.7 ms; 80,548 B) | 156.8 / 147.6 | 373.8 / 376.2 |
| 2x | 0.867x / 0.864x | 43.4% / 43.2% | 41.40 / 42.45 | 47.32 / 46.43 | 15 (2,197.1/3,547.9 ms; 81,434 B) | 152.0 / 160.3 | 374.1 / 383.2 |
| 4x | 0.834x / 0.840x | 20.8% / 21.0% | 125.07 / 125.27 | 49.46 / 48.50 | 15 (2,371.0/4,237.5 ms; 81,981 B) | 155.3 / 153.4 | 372.9 / 373.6 |
| 8x | 0.727x / 0.792x | 9.1% / 9.9% | 320.57 / 292.47 | 40.72 / 46.26 | 15 (2,450.6/3,865.5 ms; 81,735 B) | 188.5 / 164.2 | 375.0 / 375.4 |
| 12x | 0.743x / 0.833x | 6.2% / 6.9% | 482.28 / 429.20 | 40.40 / 41.58 | 15 (2,297.5/4,101.1 ms; 81,399 B) | 173.8 / 182.6 | 375.4 / 380.3 |

## P2 results

P2 is 55 evolved snakes, 10 baseline bots, a five-layer 256-wide feature MLP
with a 96-unit GRU, 32 sensor bins, and approximately 3,500 pellets.

| Requested | Achieved off/on | Requested ratio off/on | Dropped seconds off/on | Bot observations/s off/on | Viewer-on frames (mean/p95 interval; mean bytes) | Event-loop p95 ms off/on | Maximum recorded RSS MiB off/on |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1x | 1.033x / 1.064x | 103.3% / 106.4% | 1.18 / 0.07 | 59.86 / 62.33 | 397 (68.5/44.2 ms; 75,727 B) | 79.1 / 70.3 | 562.4 / 562.6 |
| 2x | 1.250x / 1.400x | 62.5% / 70.0% | 20.93 / 13.47 | 72.18 / 77.07 | 15 (1,334.6/2,121.7 ms; 75,431 B) | 95.9 / 82.3 | 575.5 / 564.4 |
| 4x | 1.305x / 1.332x | 32.6% / 33.3% | 63.07 / 61.25 | 73.15 / 74.06 | 15 (1,454.6/2,185.3 ms; 75,797 B) | 76.5 / 80.5 | 559.4 / 564.0 |
| 8x | 1.489x / 1.374x | 18.6% / 17.2% | 137.15 / 150.47 | 83.50 / 77.35 | 15 (1,435.8/1,933.9 ms; 75,069 B) | 77.6 / 78.6 | 566.1 / 564.0 |
| 12x | 1.334x / 1.407x | 11.1% / 11.7% | 246.60 / 233.88 | 75.38 / 73.79 | 15 (1,398.7/2,109.0 ms; 75,463 B) | 78.8 / 78.2 | 563.7 / 565.3 |

P2 1x viewer-on has a 68.5 ms mean frame interval despite a 44.2 ms p95.
That is not a transcription error: rare stalls raise the mean, with a 1,664.6
ms p99 and 2,053.3 ms maximum in the raw artifact.

## Interpretation and limits

P0 at 1x is the only clean pair in this matrix: it ran approximately in real
time and reported no discarded scheduler debt. P1 fails 1x materially. P2's
average 1x rate is above 1x, but it still records discarded debt and 70–79 ms
event-loop p95, so this short run is not a clean P2 real-time acceptance pass.
At every higher requested speed, the current server discards substantial
simulated time and starves frame publication.

These are declining-load averages, not measurements of a stable full
population. The tick-300 warm-up already misses the initial population spike:

| Scenario | Total alive at start | Total alive at end | Evolved alive at start | Evolved alive at end |
|---|---:|---:|---:|---:|
| P0 | 51–54 | 29–33 | 42–44 | 20–25 |
| P1 | 139–153 | 31–43 | 134–146 | 22–35 |
| P2 | 48–56 | 18–26 | 40–46 | 11–17 |

The especially important P1 case therefore does not measure the initial 311
configured snakes while they are all alive. It measures an already collapsed
population that continues shrinking during the window.

Viewer-on and viewer-off launches can diverge before and during measurement.
External join and action timing uses wall/event-loop timing, current external
joins advance authoritative RNG, viewer publication changes scheduling, and
health polling observes only at-or-beyond boundaries. Some viewer-on runs
appear faster than viewer-off runs; that must not be presented as evidence
that adding a viewer improves performance.

The synthetic bot immediately sends one action for each delivered observation.
Its recorded actions are client sends, not proof that the current server
accepted and applied every action. The current health endpoint has no accepted
or applied action counters. This runner also performs an O(snakes) diagnostic
world-load scan at every committed step and another after each pump. Server,
clients, health polling, memory sampling, and that instrumentation all share
the measured Node process.

Automatic current-reference SQLite checkpoints were configured every
generation. Nevertheless, all 30 windows remained at in-memory generation 1,
durable generation 1, and durable snapshot id 1. No run crossed a generation
boundary or published a later checkpoint. These artifacts therefore provide
no checkpoint-latency, restore, crash-recovery, checkpoint-v3, or managed-file
result.

Every artifact ends with `health.ok = true`, no simulation or collision-grid
fault, no out-of-bounds collision-grid entry, no reliable-message backlog or
failure, and the expected one- or two-socket composition. These checks make
the recorded windows usable, but do not turn them into browser, trusted-LAN,
owner-trainer, long-soak, or Rust-authoritative evidence.

## Peak-RSS telemetry qualification

The runner samples RSS every 50 ms, stops that timer, and then records one
final `memory.after` value. Its `peakRssBytes` field does not include that last
read. In `p6-p0-12x-viewer-off.json` and
`p6-p2-12x-viewer-on.json`, `memory.after.rss` is 131,072 bytes above the
reported sampled peak. The tables therefore use
`max(memory.before.rss, memory.after.rss, memory.peakRssBytes)` and label the
result **maximum recorded RSS**. The raw files remain valid; the runner field
should be corrected before later measurements rely on it as a complete peak.

## Integrity

`SHA256SUMS.txt` contains hashes for only the 30 JSON artifacts in this
directory, sorted by filename and stored as UTF-8 without a byte-order mark
using LF line endings. Its SHA-256 is
`09c94ff46a30e1f3e0271865121edcf6d4c0ab3b771e104ff78077c5ffbba4c9`.

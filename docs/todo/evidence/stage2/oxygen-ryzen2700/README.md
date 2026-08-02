# Oxygen Ryzen 7 2700 evidence index

This directory contains 24 retained JSON artifacts from `oxygen`, the owner's
Debian VM. It characterizes the current TypeScript-authoritative reference,
the real current server with synthetic external clients, and disposable
checkpoint/codec prototypes. It is not evidence that the Rust-authoritative
runtime is implemented, and it does not close Stage 2.

## Evidence labels used here

- **Current-source proof** means a structural fact proved by the named source
  revision or retained fixture.
- **New target-VM measurement** means a generated-runner JSON embeds and
  validates hostname `oxygen`, Debian/Linux, the Ryzen 7 2700 CPU, eight
  logical CPUs, and at least 15 GiB of RAM. The manually assembled browser
  summary records those facts through external provenance but is explicitly
  not self-validating raw runner output.
- **External provenance** means a read-only SSH inventory or transfer record
  supplied the fact, but the raw JSON does not carry enough host identity to
  prove it by itself.
- **Derived interpretation** is arithmetic or a conclusion drawn from retained
  measurements; it is not another measurement.
- **Limitation** identifies a required boundary that the artifact did not run.

The `source.commit` in each JSON is the clean source revision used to produce
that artifact. It is not the later commit that eventually retains the evidence.

## Target and deployment inventory

The following facts were collected by read-only system inventory. The CPU,
memory, kernel, hostname, Node, and Zstandard facts are also embedded in the
runtime or checkpoint artifacts where noted.

| Item | Recorded value | Evidence |
|---|---|---|
| Host | `oxygen`, x86-64 KVM/QEMU VM | Embedded target-VM provenance and SSH inventory |
| OS | Debian GNU/Linux 13 (`trixie`), Debian point release 13.5 | External provenance; the artifacts embed Linux kernel identity |
| Kernel | `6.12.94+deb13-amd64` | Embedded target-VM provenance |
| CPU allocation | AMD Ryzen 7 2700; one virtual socket, four virtual cores, two threads per core, eight online logical CPUs | Embedded model/count plus external topology |
| RAM | 16,775,352,320 bytes total (15.62 GiB) | Embedded target-VM provenance |
| Swap | 8,589,930,496 bytes, unused at the audit | External provenance |
| Application filesystem | `/dev/mapper/oxygen--vg-srv`: 399,603,867,648 bytes total, 169,858,342,912 used, 213,114,433,536 available | External provenance |
| Temporary filesystem | `/tmp` tmpfs: 8,387,678,208 bytes total and 7,466,950,656 available at the later inventory | External provenance; checkpoint artifacts retain their own before/after free-byte samples |
| Runtime | Node `v24.12.0`; V8 `13.6.233.17-node.37`; ICU `77.1`; Zstandard `1.5.7` | Embedded across runtime, graph, codec, and checkpoint artifacts |
| Rust toolchain | `rustc`/`cargo` 1.92 from the user's rustup toolchain | External provenance |

The deployed checkout was
`/opt/apps/slither_neuroevo` at
`027f5b28d61e69ab0dc4e9f2e6103daf7136aa4d`. No simulation service or
listener was running during the audit. Its ignored configuration retained the
intended trusted-LAN bind (`host` and `uiHost` `0.0.0.0`, server port 5174, UI
port 5173), with eight Node inference workers configured. Those are deployment
facts, not proof of current LAN latency or capacity.

Benchmarks did not replace or run from that deployed source tree. Clean local
commits were transferred by Git bundle into the isolated detached checkout
`/tmp/slither-stage2-e79ebd5`; later commits were fetched into that same
directory despite its original name. Dependencies were copied into this
temporary checkout, and the native addon was rebuilt there. The native addon
used by the runtime artifacts reports build identity
`slither_native/0.1.0+e79ebd578699.2299894198bc853c`; its native source had not
changed when later benchmark-runner commits were checked out. The release Rust
kernel tests passed 3/3 in the isolated tree. The copied Node dependency tree
was not pristine: `npm ls` reported Mermaid CLI 11.12.0 against the declared
`^10.9.1`. Mermaid is not on these simulation benchmark paths, but the mismatch
is retained as a limitation rather than hidden.

## Artifact source map

| Artifacts | Embedded clean source | Classification | Principal limitation |
|---|---|---|---|
| `runtime-*.json` | `93e22c1e92c725b52c5e0ed4d224c9ffc61c44c7` | New target-VM measurement | Direct `SimCore`/`World`, not the real server, LAN browser, trainer, or Rust-authoritative runtime |
| `external-p5-*.json` | `284ed4484f242be3c1dee2d8aa78ba514f48eac7` | New target-VM current-server measurement | Real server with synthetic Protocol 2 clients over loopback, not the browser, LAN, or owner trainer |
| `browser-lan-p0-60hz.json` | `284ed4484f242be3c1dee2d8aa78ba514f48eac7` | Target-VM/current-server and real-browser LAN measurement summary with external provenance | Real desktop browser and P0 only; no sensor suppression, accepted/applied-step correlation, laptop, owner trainer, or Rust runtime |
| `checkpoint-validation-*.json` | `cd9ab281117f55a97e080dd8e3c60c3075f2dd6f` | New target-VM prototype measurement | Disposable Node USTAR/Zstandard prototype, not production Rust, SQLite authority, restore, or power-loss proof |
| `codec-*.json` | `4cd1c004b78cf5d192eb7d69b35c9d052f25db7c` | New target-VM codec measurement | Offline weight payloads only, not the managed checkpoint container, full server memory, or production Rust codec |
| `graph-debian.json` | `e79ebd5786991a62e02315dc83d3e3b8784273b0` | Reproducible fixture on Debian | One graph and one locale/runtime pair cannot prove all-platform ordering |
| `owner-database.json` | `e79ebd5786991a62e02315dc83d3e3b8784273b0` | New measurement of a stable owner-database copy | External provenance connects the local inspection copy to the remote path; it is not an exact-resume checkpoint or a search of client download folders |

## Current reference runtime

P0 is 55 evolved snakes plus 10 baseline bots with the default 13,458-weight
graph. P1 raises the evolved population to 300. P2 uses 55 evolved snakes and
the 402,914-weight, 32-bin large graph. P3 combines 300 evolved snakes with
that large graph. P4 starts with 300 evolved snakes, 10 baseline bots, 700
body points per snake, and 12,000 pellets.

The table reports the exact current schema-v2 files. `sim/wall` is simulated
seconds per wall second while directly computing fixed steps; times are
milliseconds. It does not include server scheduling, socket traffic, browser
work, or pacing. Different row durations are shown so they are not mistaken
for perfectly matched repeated trials.

| Scenario/path | Warm-up + measured steps | sim/wall | step p50 | p95 | p99 | max | alive at end | peak RSS MiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| P0 JS serial | 30 + 180 | 1.032 | 15.58 | 19.31 | 20.76 | 23.43 | 55 | 145.7 |
| P0 native serial | 30 + 600 | 1.220 | 12.93 | 16.82 | 18.13 | 19.99 | 41 | 144.8 |
| P0 native, four Node workers | 30 + 180 | 1.023 | 15.88 | 18.49 | 20.43 | 22.86 | 55 | 294.8 |
| P1 JS serial | 20 + 180 | 0.216 | 76.80 | 95.01 | 106.21 | 140.16 | 178 | 227.9 |
| P1 native serial | 20 + 180 | 0.228 | 73.16 | 89.63 | 96.32 | 126.39 | 179 | 225.6 |
| P1 native, four Node workers | 20 + 180 | 0.231 | 70.43 | 89.14 | 92.23 | 107.65 | 179 | 406.9 |
| P2 JS serial | 20 + 180 | 0.321 | 51.12 | 59.22 | 63.58 | 63.74 | 57 | 408.4 |
| P2 native serial | 20 + 180 | 0.672 | 23.92 | 28.19 | 29.72 | 30.96 | 56 | 407.2 |
| P2 native, four Node workers | 20 + 180 | 0.906 | 17.33 | 19.82 | 31.60 | 80.17 | 56 | 790.5 |
| P3 native serial | 10 + 120 | 0.126 | 131.85 | 143.71 | 152.37 | 202.08 | 228 | 1,575.9 |
| P4 native serial | 0 + 600 | 1.003 | 8.69 | 26.23 | 45.36 | 396.49 | 8 | 374.3 |

These measurements reproduce the current failure plainly. P1 is only about
0.22x real time in every tested current path. Four Node workers improve P2
relative to current serial execution but reach only 0.906x and have a long
tail. P3 is a measured capacity case at 0.126x. P0/P1/P2 remain mandatory
targets for the eventual Rust engine; these current-reference results neither
weaken those targets nor prove that the future engine can meet them.

### P4 load collapse and collision omissions

P4 installed 310 live snakes, 217,000 body points, and exactly 216,690
collision-index entries. The initial capacity admission was complete and the
cumulative out-of-bounds counter was zero. The first completed step took
396.49 ms, killed all but 52 snakes, and reduced the current collision-index
load to 37,333 entries. Only eight snakes remained at step 600.

The reported 1.003x average is therefore dominated by a drastically smaller
world after the first-step collapse. It is not sustained-dense-world capacity
and cannot pass P4 by itself.

The out-of-bounds counter first increased at measured step 107 and reached
829,938 at step 600. This is a cumulative count of rejected segment-midpoint
insertions across repeated index rebuilds, not the number currently absent at
the end. Every increase nevertheless proves that the current TypeScript index
silently omitted segments without raising a capacity fault. The initial
capacity-growth assertion therefore does not repair the later geometric
out-of-bounds defect. Rust Stage 5 must provide complete collision storage or
fail clearly; it must not preserve these omissions as reference behavior.

## P5 real current-server baseline

These schema-v4 runs started the real current server on Oxygen with its native
single-thread backend. Each used three loopback WebSocket connections: one
synthetic Protocol 2 bot, one synthetic Protocol 2 UI player, and one synthetic
Protocol 2 full-frame-v1 spectator. The UI player used a timer independent of
sensor delivery at the named 30 or 60 Hz candidate; the synthetic bot remained
observation-driven. Each measurement lasted about 15 seconds after a two-
second warm-up. Automatic generation persistence was deliberately excluded by
setting the checkpoint interval to 1,000,000 generations.

`sim/wall` below is derived exactly as `schedulerDelta.simulatedSeconds /
(measuredWallMs / 1000)`, rather than copied from a differently bounded health
counter. The `frames / frame p95` column gives received frame count followed by
the p95 interval between those frames.

| Scenario | Player candidate | derived sim/wall | player sends | player interval p95 | bot observations/s | frames / frame p95 | event-loop p95 | peak RSS | dropped debt |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| P0 | 30 Hz | 1.009497 | 409 | 47.47 ms | 60.636 | 378 / 56.87 ms | 28.52 ms | 210.8 MiB | 0 s |
| P0 | 60 Hz | 1.003389 | 789 | 31.24 ms | 60.336 | 393 / 59.27 ms | 31.03 ms | 209.8 MiB | 0 s |
| P1 | 30 Hz | 0.263499 | 245 | 79.85 ms | 14.579 | 2 / 7,055.48 ms | 79.95 ms | 372.5 MiB | 11.916667 s |
| P1 | 60 Hz | 0.268123 | 248 | 85.51 ms | 16.283 | 2 / 6,529.80 ms | 85.98 ms | 374.2 MiB | 11.8 s |
| P2 | 30 Hz | 0.833822 | 364 | 49.51 ms | 45.403 | 6 / 2,679.21 ms | 24.84 ms | 561.7 MiB | 2.1 s |
| P2 | 60 Hz | 0.789101 | 709 | 26.61 ms | 47.544 | 6 / 2,683.04 ms | 26.03 ms | 553.4 MiB | 3.0 s |

P0 accumulated no dropped simulation debt in either run and remained just
above 1x by the stated derived calculation. The higher requested player rate
did not yield a 16.67 ms p95 timer interval even at P0. P1 reproduced severe
server starvation: both the 30 Hz and 60 Hz timers collapsed to about 16
actual sends per wall second (15.87 and 16.15 respectively), while only two
display frames arrived in roughly 15 seconds. P2 delivered only six frames and
also accumulated debt. These are current-reference failures to preserve as
evidence, not acceptable targets for Rust.

The counters require careful interpretation:

- player and bot `actionCount` count client-side `socket.send` calls. The
  current health endpoint exposes no accepted/applied action counter, so these
  values do not prove that `ControllerRegistry` accepted each command or that
  any particular command affected an eligible fixed step;
- `actionToNextSensorMs` measures the time from a client send to the next
  sensor message observed by that client. It does not establish that the
  action caused that sensor, was accepted, or was applied before it, so it is
  not end-to-end control latency; and
- all clients were wire-compatible synthetic loopback clients. The runs did
  not execute `src/main.ts`, render a browser frame, cross the trusted LAN, or
  run the owner's Python trainer.

Every run ended with healthy server status, no simulation fault, and no
reported reliable-message failure. That evidence is limited to the exercised
loopback path and does not cure the observability gaps above.

## Real browser over the trusted LAN

`browser-lan-p0-60hz.json` is a retained measurement summary of a real
Chromium 150 desktop-browser run
against the real current server on Oxygen over the home LAN. The browser
loaded the Vite UI from `192.168.0.200`, joined a P0 snake, reclaimed its
assignment after a reload, rendered frames, and transmitted 396 real player
actions over 6.776742 seconds. The measured rate was 58.288 actions/s; action
interval p50/p95/p99/max was 16.988/20.534/23.108/23.476 ms. Pointer movement
produced 41 distinct turn values. A captured boost-on was followed by
boost-off 18.207 ms later.

The CDP WebSocket send observation proves that changed steering and boost
release left browser JavaScript on the socket whose recorded endpoint was
Oxygen's LAN address. It does not prove server acceptance or which fixed step
applied each action, and this window did not suppress sensors or display
frames. The observed cadence is consistent with the current source's
independent browser timer; the Stage 1 suppression integration test, rather
than this live capture, is the present proof that transmission does not depend
on sensors. The final point-in-time browser heap sample used 19,835,497 bytes;
it is not a peak or allocation trace.

The temporary P0 process had also continued accidentally for 32,876.982 wall
seconds before it was stopped. Its final health sample reported 32,869.667
simulated seconds, 0.999778x achieved speed, 7.3 seconds of cumulative dropped
debt, generation 142, one connected externally owning browser, no reliable
failure, and no simulation fault. This is a long-duration survival
observation, not a controlled soak with interval samples, and does not prove
P1/P2 or future Rust capacity.

After graceful shutdown, its disposable database was 41,385,984 bytes. It had
only one generation-one population checkpoint, but 142 Hall-of-Fame rows with
38,188,674 bytes of repeated decimal genome JSON; `hof_entries` occupied
38,346,752 allocated bytes. This independently exercises the Hall-of-Fame
duplication defect. It does not replace the separate evidence that automatic
full-population checkpoints also grow without retention.

Unlike the generated Stage 2 runner artifacts, this summary was assembled
from live tool results. Raw CDP events, health/process output, and SQLite query
transcripts were not retained, so an independent reader cannot recompute its
percentiles or re-prove assignment reclaim from the JSON alone.

## Packed-weight codec measurements

Both fixtures contain 55 separately weighted evolved genomes after 25
deterministic operator generations. Decoding was bit-exact. Sizes below are
numeric weights only, before the rest of the checkpoint container and state.

| Fixture | Raw Float32 | Decimal JSON | JSON/raw | Plain Zstd reduction | Shuffled Zstd reduction | Archive-v1 stored | Legacy gzip JSON | Legacy encode/decode |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| P0 | 2.82 MiB | 14.15 MiB | 5.011x | 28.55% | 23.12% | 2.17 MiB shuffled | 6.37 MiB | 807.6/69.6 ms |
| P2 | 84.53 MiB | 423.40 MiB | 5.009x | 12.20% | 16.99% | 70.17 MiB shuffled | 190.12 MiB | 22,244.0/2,232.9 ms |

Archive v1 intentionally chooses between raw packed Float32 and shuffled
Zstandard per numeric payload. Plain Zstandard remains a comparison rather
than an archive-v1 encoding, so P0 selects shuffled Zstandard even though plain
Zstandard happened to be smaller in this fixture. Packed binary alone reduces
these decimal-JSON weights by about 80%; compression is measured and is not
promised to beat an arbitrary percentage on high-entropy data.

The current schema-v3 codec JSON files embed and validate the target hostname,
Debian distribution, Ryzen CPU, eight logical CPUs, and memory allocation.
They are therefore self-contained target-VM measurements, while still limited
to an offline codec fixture rather than the production archive path.

## Managed-checkpoint write-validation prototype

Each target artifact contains three trials of four policies. `p95 barrier` is
generation-boundary time through archive construction, file flush/fsync,
validation selected by the variant, same-filesystem rename, and parent-
directory fsync. RSS includes the TypeScript fixture, contiguous packed source
copy, and codec scratch; it is not a future Rust-state measurement.

| Fixture | Raw weights | Archive | Single-pass p50/p95 | Frame-checksum p95 | Lightweight-scan p95 | Full-decode p95 | maximum process RSS |
|---|---:|---:|---:|---:|---:|---:|---:|
| P0 evolved-25 | 2.82 MiB | 2.31 MiB | 41.64/44.69 ms | 43.95 ms | 48.90 ms | 66.57 ms | 195.1 MiB |
| P2 evolved-25 | 84.53 MiB | 72.15 MiB | 708.66/712.91 ms | 718.15 ms | 717.59 ms | 1,100.46 ms | 740.5 MiB |

All 12 measured publications per fixture completed file fsync, atomic rename,
and parent-directory fsync, and temporary fixture cleanup restored the sampled
free space. This is useful target timing, not a power-loss durability test.

The strict fault matrix shows why a payload-blind scan is not a replacement
for restore validation. A lightweight scan rejected a truncated trailer, a
bad USTAR header, and a changed logical-root manifest, but accepted corrupted
compressed weight bytes. Full decode rejected that payload corruption. A
Zstandard frame checksum gave earlier codec-level diagnosis only when the
payload was decoded; it did not make the lightweight scan inspect payloads.

The approved minimum policy therefore remains a single pass for ordinary
automatic checkpoints: calculate logical counts/hashes while writing,
complete the codec/container, flush and fsync, verify final completion/length,
rename atomically, and fsync the parent directory. It accepts that a latent
codec or storage defect may first be discovered during a strict startup,
import, or restore, with recovery to the previous retained checkpoint. Manual
exports and pinned checkpoints receive a full post-write decode when that
production path exists. The optional frame checksum and milestone validation
remain measurement-gated; these Node results do not silently turn them into a
new product rule.

The prototype does not implement the Rust writer/reader, SQLite metadata
pointer, restored authoritative state, retention, HTTP import/export, legacy
compatibility, recovery branch, exhaustive malformed-input corpus, or crash
state machine. Its Node decoder also cannot preflight the Zstandard frame
window as strictly as the Rust importer must.

## Owner database and save inventory

External provenance identifies the source as
`/opt/apps/slither_neuroevo/data/slither.db`, 11,726,848 bytes, SHA-256
`54f736b19be9df7e4f9195205361fb064390c4e8453870b521cde837f8f3dc2d`.
The retained artifact analyzes a stable local copy with that exact size and
digest. The remote main file's digest, size, and modification time
(`2026-02-07 12:57:59.231606700 +1100`) remained unchanged by the audit.
SQLite `quick_check` succeeded before the copy was analyzed.

The database predates `format_version`. It has the legacy combined
`genomes_blob` column but no `snapshot_genomes` child table. Its four relevant
table counts are:

- zero population snapshots, so there is no exact experiment checkpoint to
  resume;
- zero graph presets and zero players; and
- two Hall-of-Fame rows stored as decimal `genome_json`.

The Hall-of-Fame rows contain 13,458 and 569,346 weights. Their JSON records
are 270,763 and 11,404,408 bytes respectively, for 11,675,171 bytes combined.
SQLite `dbstat` attributes 11,689,984 of the database's bytes to the Hall-of-
Fame table. The smaller row uses the default GRU graph; the larger uses an
83-to-512-to-512-to-512-to-2 MLP. These are legacy genomes that compatibility
work must preserve, but they cannot reconstruct a run's RNG, generation state,
population, or exact continuation.

The scoped search under `/opt/apps/slither_neuroevo/`, excluding dependency,
build, and Git material, found no standalone exported archive or population
save. This does not search browser download directories on other machines.
Compatibility readers therefore cannot be retired merely because this server
directory lacks an exported file.

## Graph ordering comparison

`graph-debian.json` and the retained Windows
`../windows-5800x/graph-windows.json` used the same database fixture, SHA-256
`9b8774387cff7aa82e64dbf75f4807d06ada9072814d24b71b8c76c4fe4bd8a4`.
The Windows artifact used source `6949ae3`; the Debian artifact used
`e79ebd5`. Both ran Node 24.12.0, ICU 77.1, and locale `en-AU`.

No Windows-versus-Debian ordering difference reproduced. The locale probe,
compiled node order (`input`, `mlp`, `gru`, `head`), parameter offsets, total
13,458 parameters, 16 recurrent-state floats, stored architecture key, and
computed architecture key all match. Any prior claim of an observed
cross-platform difference for this saved fixture is therefore corrected.

The current compiler's use of `localeCompare` remains a structural portability
risk: equal output from one locale/ICU pair does not prove stable ordering on
every supported environment. The Rust contract still needs explicit portable
ordering and compatibility checks; the actual retained fixture must not be
described as having failed this comparison.

## Disclosed audit side effects

The audit was intended to preserve the deployed checkout and owner data. Three
filesystem side effects occurred and have not been silently removed:

- `git submodule status` refreshed
  `/opt/apps/slither_neuroevo/.git/index` to 16,635 bytes with modification
  time `2026-08-02 03:52:13.543294443 +1000`, and refreshed the `.git`
  directory metadata. No tracked source, Git ref, branch, or commit changed.
- Opening the original WAL-mode database through a read-only, query-only
  `better-sqlite3` connection created
  `/opt/apps/slither_neuroevo/data/slither.db-wal` (zero bytes) and
  `/opt/apps/slither_neuroevo/data/slither.db-shm` (32,768 bytes). The main
  database's size, digest, modification time, and logical contents remained
  unchanged. The two sidecars remain present pending an explicit cleanup
  decision.
- Dependency inspection wrote npm error logs
  `/home/james/.npm/_logs/2026-08-01T17_51_44_225Z-debug-0.log` for `npm ls` in
  the deployed checkout and
  `/home/james/.npm/_logs/2026-08-01T18_14_22_440Z-debug-0.log` for `npm ls` in
  the isolated checkout. Both report the same pre-existing Mermaid version
  mismatch. They have not been deleted.

The old checkout also already contained untracked generated WASM files before
the audit. No benchmark was written there. Benchmark builds, fixture databases,
bundles, outputs, and temporary checkpoint files used `/tmp`; this index does
not claim those disposable files have all been removed.

## Reproduction

Run from a clean checkout. The runtime and checkpoint JSON files retain the
exact absolute command arrays used on Oxygen. Equivalent portable forms are:

```sh
node ./node_modules/tsx/dist/cli.mjs scripts/stage2/runtime-baseline.ts --scenario P2 --backend native --workers 4 --warmup-steps 20 --steps 180 --frame-every 1 --environment owner-target-vm --output result.json
node ./node_modules/tsx/dist/cli.mjs scripts/stage2/runtime-baseline.ts --scenario P4 --backend native --workers 0 --warmup-steps 0 --steps 600 --frame-every 1 --sample-every-steps 10 --environment owner-target-vm --output result.json
node ./node_modules/tsx/dist/cli.mjs scripts/stage2/external-control-baseline.ts --scenario P1 --player-hz 60 --warmup-ms 2000 --duration-ms 15000 --workers 0 --environment owner-target-vm --output result.json
node ./node_modules/tsx/dist/cli.mjs scripts/stage2/codec-baseline.ts --scenario P2 --fixture evolved --evolution-generations 25 --environment owner-target-vm --output result.json
node ./node_modules/tsx/dist/cli.mjs scripts/stage2/managed-checkpoint-validation.ts --scenario P2 --fixture evolved --evolution-generations 25 --trials 3 --environment owner-target-vm --output result.json
node ./node_modules/tsx/dist/cli.mjs scripts/stage2/graph-baseline.ts --db /tmp/stable-graph-fixture.db --output result.json
```

Run `database-baseline.ts` only against a stable inspection copy made while the
service is stopped or through SQLite's supported backup operation. Preserve a
WAL database's main, `-wal`, and `-shm` files as one source set when making the
copy; do not open the owner's only copy to produce a supposedly immutable
inventory.

## Artifact SHA-256

These hashes cover the raw files present when this index was written. If an
artifact is rerun, its hash and the relevant prose must be updated together or
replaced by a checked-in `SHA256SUMS.txt` manifest.

```text
7a42572059edebb5c32828c33cae4858e6882ab3a0b9f4c2dffda6228dc5669b  checkpoint-validation-p0-evolved25.json
c82310ba7444e4645d0cdc8a02c8c6c09029ecbdf66b1cdd4cc23787422e078b  checkpoint-validation-p2-evolved25.json
af89fc9183c6d4ac48e53e67248ff33d133a31087cf38c97d41461ba8db28f6f  codec-p0-evolved25.json
a3241c9152a3d1f1d501b151ffcac7accdadc002c3fe136d3f6b395ce168268a  codec-p2-evolved25.json
c6e49cb46287d48ce249310cc6f198110e20bd4503e393186bc95e76ddfc7445  external-p5-p0-30hz.json
21ef00a7e318264715eb3414b6f284cba8e9edd6487c1c83c8b5b7ec4af770fa  external-p5-p0-60hz.json
019007b822993b64fb958dc31a2a5d80b891209836370e82f8ac6e285116f724  external-p5-p1-30hz.json
35d34cf5b0261c12b1bc65bad770b4a2ef3606297f2c81f892612ac1590ed299  external-p5-p1-60hz.json
bdd1fac78a2f43bb2cd032c97df4a2e84d447982c9d547d93c504ef0c9383a83  external-p5-p2-30hz.json
1d838e651975801b39d671b752c2b5598e23e3bb40fd5b0606619a1b4a359c4a  external-p5-p2-60hz.json
469031c6e9d9057c05bd96af2162b7f4e723b321d039bc3f0651c28819ad6038  browser-lan-p0-60hz.json
51a80c08d6d052ad412fa529d52d9ea9820385fed640dbb1ba34b705c485016a  graph-debian.json
eee093d5cfd08701194853e4eec1d64283e1c715b25bf4bc848e20d9a8dec34f  owner-database.json
770d5be6d5dae38de3830446d39f0137e8171323c517a97a8957fdf73ccc169a  runtime-p0-js-serial.json
8988d97c0e6a0b17ad46d92d8570c2bd046d7293b0efa22d68ce54d72f5d5f4f  runtime-p0-native-serial.json
1f2ec098ff370009e3ab9381beb0f65fe98ce299480c9135dc6e8455fbc1b6a2  runtime-p0-native-workers4.json
50f181a73eb15e5af56cc4534e3bf3f3de6528e2d9e3d7691296f342535144f1  runtime-p1-js-serial.json
8f47d67cf5d6162020dc579b3cb356be187e95384eb1908b88ac06ec45c0d662  runtime-p1-native-serial.json
73cee2f102282e5ecb9186bb781d0ddc13dad2e0bd3a04d24ac0c9a7aeb73eb1  runtime-p1-native-workers4.json
6294ec1c9e890b09953313a9fbd60c242de915e79137811d81760023fcd837c7  runtime-p2-js-serial.json
a1652251d53323701213aef607ec7bbca1bbf701093e489d8f708092781dfb49  runtime-p2-native-serial.json
05dd7265d25db7e577d06030ea6e151176a746f3db1c0eb127f906950d2e339b  runtime-p2-native-workers4.json
32bd47b8e15590e60d2c811e184897f36513c38623246bd015de8f5a64681386  runtime-p3-native-serial.json
07d294490b1760113bb1cee5b77b2003e55571ed19a9720f730461cf0d52a9c6  runtime-p4-native-dense-600.json
```

## Open gates

This evidence does not complete Stage 2. At minimum, the following remain
open:

- P5 now has Oxygen synthetic-loopback P0/P1/P2 and a real desktop-browser LAN
  P0 baseline. It still needs P1/P2 real-browser load, the laptop, delayed/
  suppressed sensors and display frames, lifecycle priority under backlog,
  server accepted/applied-action observability, genuine browser-input-to-step
  latency, and a final measured 30-Hz-versus-60-Hz selection;
- the owner's real RL trainer path. The separately retained trainer audit shows
  that its current commit sends Protocol 1 while this server requires Protocol
  2, so this needs a coordinated trainer change rather than weakening the
  server contract;
- P6 accelerated 1x/2x/4x/8x/12x curves through the actual server on Oxygen,
  both headless and with a LAN spectator;
- P7 controlled target-VM/browser/trainer soak with interval samples,
  intentional reconnects, memory, handles, queues, database/WAL, and durable-
  checkpoint behavior. The accidental nine-hour P0 survival observation does
  not close this gate;
- P8 direct browser download/upload, bounded browser and server memory,
  retention, recovery branches, corruption cases, and overnight-equivalent
  growth using the production managed-file implementation; and
- every production Rust correctness, restore, persistence, LAN, browser,
  trainer, parallelism, and performance gate. Nothing in this directory is a
  Rust-authoritative cutover result.

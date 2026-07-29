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

The source- and Git-history-backed compatibility matrix is retained in
`persistence-format-inventory.md`. It inventories current format-2 SQLite,
historical combined-gzip and all-parent JSON rows, current/older standalone
JSON files, browser-local legacy state and the planned archive-v1 boundary.
It also records the limited local owner-file search; actual files on the
Debian VM, Unraid storage, other disks or browser profiles remain unexamined.

## Reproducible commands

Run from the repository root with the direct `tsx` entry point:

```powershell
node .\node_modules\tsx\dist\cli.mjs scripts\stage2\database-baseline.ts --db data\slither.db --output result.json
node .\node_modules\tsx\dist\cli.mjs scripts\stage2\codec-baseline.ts --scenario P0 --fixture fresh --output result.json
node .\node_modules\tsx\dist\cli.mjs scripts\stage2\codec-baseline.ts --scenario P2 --fixture evolved --evolution-generations 25 --output result.json
node .\node_modules\tsx\dist\cli.mjs scripts\stage2\runtime-baseline.ts --scenario P2 --backend native --workers 4 --warmup-steps 20 --steps 180 --frame-every 1 --output result.json
node .\node_modules\tsx\dist\cli.mjs scripts\stage2\behavior-baseline.ts --output result.json
node .\node_modules\tsx\dist\cli.mjs scripts\stage2\graph-baseline.ts --db data\slither.db --output result.json
node .\node_modules\tsx\dist\cli.mjs scripts\stage2\create-current-db-fixture.ts --scenario P1 --output C:\temporary\stage2-p1.db
node .\node_modules\tsx\dist\cli.mjs scripts\stage2\browser-baseline-host.ts --db C:\temporary\stage2-p1.db --server-port 55194 --ui-port 55193 --ui-rate 30 --duration-ms 1800000
node .\node_modules\tsx\dist\cli.mjs scripts\stage2\external-control-baseline.ts --scenario P1 --player-hz 60 --warmup-ms 2000 --duration-ms 15000 --workers 0 --output result.json
node .\node_modules\tsx\dist\cli.mjs scripts\stage2\retention-baseline.ts --generations 480 --scenario P0 --output result.json
node .\node_modules\tsx\dist\cli.mjs scripts\stage2\managed-checkpoint-validation.ts --scenario P0 --fixture evolved --evolution-generations 25 --trials 7 --output result.json
node .\node_modules\tsx\dist\cli.mjs scripts\stage2\managed-checkpoint-validation.ts --scenario P2 --fixture evolved --evolution-generations 25 --trials 7 --output result.json
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

## Reproduced unbounded P1 checkpoint growth

An intended browser test launch accidentally remained attached to its shell
while the disposable simulation server continued running. The UI process
failed before serving a page, so this is not browser evidence. It is, however,
a real current-server P1 checkpoint-growth run. The retained database inventory
and raw process logs identify the source and preserve that distinction.

From the generation-one checkpoint to generation 128:

- 127 additional automatic generation checkpoints accumulated over 8.906
  hours;
- the disposable database grew from 16,338,944 bytes to 2,155,540,480 bytes
  (2.008 GiB);
- 38,400 genome rows held 2,067,148,800 raw weight bytes (1.925 GiB);
- the mean observed file growth was about 16.84 MB per additional checkpoint,
  or 229.1 MiB per wall-clock hour;
- SQLite reported 526,321 pages and zero free-list pages because no snapshot
  was pruned; and
- scheduler logs repeatedly reported dropped wall-clock debt, so this is not a
  claim that P1 met real-time performance.

This was an accidental, uncontrolled development-machine soak, not the
approved P8 target-VM acceptance run. It directly reproduces the current
unbounded full-population accumulation mechanism and its practical disk
effect. It does not replace the selected managed-file retention, compaction,
recovery, or overnight-equivalent tests.

## Newly reproduced codec results

All sizes below are exact weight payloads; future container, state, history and
small metadata are reported separately.

| Fixture | Raw | Decimal JSON bytes per Float32 | Decimal JSON/raw | Plain Zstd reduction | Shuffled Zstd reduction | Archive-v1 choice | Archive-v1 stored |
|---|---:|---:|---:|---:|---:|---|---:|
| P0 fresh | 2.82 MiB | 20.11998 | 5.030x | 7.678% | 14.646% | shuffled Zstd | 2.41 MiB |
| P0 evolved-like, 25 operator generations | 2.82 MiB | 20.04202 | 5.011x | 28.55% | 23.12% | shuffled Zstd | 2.17 MiB |
| P1 fresh | 15.40 MiB | 20.11916 | 5.030x | 7.676% | 14.675% | shuffled Zstd | 13.14 MiB |
| P1 evolved-like, 25 operator generations | 15.40 MiB | 20.04165 | 5.010x | 22.42% | 23.91% | shuffled Zstd | 11.72 MiB |
| P2 fresh | 84.53 MiB | 20.11 | 5.03x | 7.86% | 14.75% | shuffled Zstd | 72.06 MiB |
| P2 evolved-like, 25 operator generations | 84.53 MiB | 20.03 | 5.01x | 12.20% | 16.99% | shuffled Zstd | 70.17 MiB |
| P3 fresh | 461.10 MiB | 20.11236 | 5.028x | 7.86% | 14.75% | shuffled Zstd | 393.09 MiB |
| P3 evolved-like, 25 operator generations | 461.10 MiB | 20.03389 | 5.008x | 11.15% | 16.82% | shuffled Zstd | 383.55 MiB |

The P0 fresh fixture reproduced the prior planning percentages to rounding:
7.68% plain and 14.65% shuffled. Its exact plain result was 2,733,420 bytes,
not the prior unretained 2,733,479-byte claim. The exact JSON observation was
20.11998 bytes per Float32 for fresh P0 and about 20.03–20.04 for the evolved
fixtures; the plan must use the retained values rather than preserve the old
approximately-20.03 claim indiscriminately.

Plain Zstandard remains comparison evidence, but it is not an archive-v1
numeric encoding selected by Draft 4. The first version of the Stage 2 runner
incorrectly let that comparison win evolved P0's `selectedAdaptive` field.
Version 2 corrects the label and selection: archive v1 chooses only
`raw-f32le-v1` or `f32le-shuffle4-zstd-v1`. Evolved P0 therefore stores the
2,276,354-byte shuffled result rather than the smaller 2,115,432-byte plain
comparison. This correction was made before any production archive format was
implemented or any retention fixture consumed the value.

Each decoded result was bit-exact. Adding a Zstandard frame checksum added four
bytes to each whole-population frame. Single-run timing is retained in raw JSON
but is not yet a stable latency conclusion; repeated Windows and target-VM
trials are still required.

Derived from the approved archive-v1 choices, 22 automatic P0 weight payloads
would use about 47.8–53.0 MiB. Twenty-two P2 payloads would use about
1.51–1.55 GiB.
Twenty-two P3 payloads would use about 8.24–8.45 GiB, so the 4 GiB byte cap,
not the count limit, necessarily controls that capacity case. The protected
four-checkpoint P3 minimum calculates to about 1.50–1.54 GiB. These are
calculations from measured codec sizes, not complete-checkpoint disk
measurements. They support the selected retention rules, subject to real full-
checkpoint, Hall-of-Fame and VM free-disk measurement.

## Accelerated 480-generation retention fixture

The retained
`windows-5800x/retention-p8-p0-480.json` artifact has SHA-256
`72fd0d7fbdb28a105305970210e93d01c78a7869952556e049e26cf4e40347c5`.
It was produced from clean source commit
`e489b3cc9689dccb7feef21b7d522656cbb5dde4`.

The fixture represents 480 60-second generations, or eight hours by generation
count and checkpoint volume. It physically creates size-matched managed files
and real SQLite checkpoint metadata, current-pointer, compact-history and
Hall-of-Fame rows. The files are not checkpoint-v3 USTAR archives, are not
importable, are not fsynced, and do not prove restore, publication durability
or target-VM throughput.

The materialized P0 run modeled 1,097,202,628 checkpoint-payload bytes,
23,809,737 Hall-of-Fame payload bytes and 26,880 compact-history bytes. After
automatic pruning:

- 22 checkpoints used 50,079,788 bytes: one latest, seven other recent,
  twelve milestones and two distinct prior-run anchors;
- 460 superseded checkpoint files totaling 1,047,122,840 bytes were deleted;
- 50 unique Hall-of-Fame genome files used 2,480,183 bytes, while all 480
  compact Hall-of-Fame metadata rows remained;
- 430 superseded Hall-of-Fame weight files totaling 21,329,554 bytes were
  deleted;
- all file-to-metadata, Hall-of-Fame, fixed-width-history and current-pointer
  accounting assertions passed; and
- the final SQLite database contained 480 compact 56-byte history records,
  22 checkpoint references, 50 Hall-of-Fame genome references, 480
  Hall-of-Fame entries and two immutable definition records.

The metadata transaction p95 was 2.746 ms, p99 was 12.924 ms and maximum was
118.624 ms on the development machine. Those are non-durable metadata timings,
not complete checkpoint-publication latency. The final passive WAL checkpoint
took 4.503 ms; peak WAL was not sampled. The fixture's short wall time benefits
from copying one size-matched template repeatedly and is not archive encoding
or storage-device throughput evidence.

Derived from the retained evolved-codec artifacts, the approved automatic
retention policy projects the following weight-payload state:

| Workload | Unpruned 480-generation payload | Retained checkpoints | Retained weight payload |
|---|---:|---:|---:|
| P0 | 1.02 GiB | 22 | 47.76 MiB |
| P1 | 5.49 GiB | 22 | 257.83 MiB |
| P2 | 32.89 GiB | 22 | 1.51 GiB |
| P3 | 179.79 GiB | 10 | 3.75 GiB |

P1 through P3 are derived arithmetic rather than materialized retention runs.
The figures cover genome-weight payloads, not complete checkpoint containers,
recurrent/configuration state, SQLite metadata, filesystem allocation or
pinned data. Pinned checkpoints and downloaded exports remain outside the
automatic cap. The Hall-of-Fame fixture intentionally models every generation
as a new qualifying unique genome; duplicate, non-qualifying, pinned and
multi-run cases remain for later persistence tests.

## Managed-checkpoint write-validation comparison

The retained artifacts are:

- `windows-5800x/checkpoint-validation-p0-evolved25.json`, SHA-256
  `efa57db87552c61452ebb48240d49c562c50b33ecb254a4d4a5a2cfd40bb7e96`;
- `windows-5800x/checkpoint-validation-p2-evolved25.json`, SHA-256
  `59bbd3013ea85bb65b0894b24633305f2293b115ede99f9047ad1d3f2055caed`.

Both were produced from clean source commit
`ac905db49bbb912bf49cf3a91b36934c72932229`. They use a disposable Node
prototype of the selected bounded shuffled-Zstandard entries inside a strict
USTAR container. This is Stage 2 measurement code, not the production Rust
checkpoint contract, a restore implementation, a SQLite payload schema, an
HTTP path or proof of durability on the target VM.

The evolved-25 fixtures exactly match the retained codec artifacts' population
architecture, raw byte counts and logical SHA-256 values. Each of four
write-validation variants completed seven accepted trials:

| Fixture | Raw weights | 1 MiB blocks | Stored candidate | Archive bytes | Reduction from raw weights |
|---|---:|---:|---:|---:|---:|
| P0 | 2,960,760 | 3 | 2,416,675 | 2,426,368 | 18.143% including container |
| P2 | 88,641,080 | 85 | 75,641,920 | 75,652,096 | 14.657% including container |

The bounded blocks cost 140,321 bytes more than P0's retained whole-frame
selected encoding and 2,058,911 bytes more than P2's. That is a measured
bounded-memory/compression-ratio trade-off rather than evidence that the
previous whole-population frame is a safe production decoder.

| Fixture/variant | Hash + compression p95 | File fsync p95 | Validation p95 | Publication barrier p95 |
|---|---:|---:|---:|---:|
| P0 single pass | 24.870 ms | 3.556 ms | — | 33.613 ms |
| P0 lightweight scan | — | — | 2.079 ms | 34.541 ms |
| P0 full decode | — | — | 20.285 ms | 50.803 ms |
| P2 single pass | 520.061 ms | 28.610 ms | — | 587.623 ms |
| P2 lightweight scan | — | — | 9.581 ms | 594.904 ms |
| P2 full decode | — | — | 408.597 ms | 965.716 ms |

The lightweight scan read only 6,296 P0 bytes or 6,322 P2 bytes; full
validation read the complete 2,426,368-byte or 75,652,096-byte archive. The
frame-checksum variants added exactly four bytes per compressed block: 12
bytes for P0 and 340 bytes for P2. Timing differences between checksum-off and
checksum-on trials are too small and noisy to claim a speed effect.

The retained fault matrix distinguishes the mechanisms:

- truncating the terminal blocks fails the strict scan with `USTAR_TRAILER`;
- corrupting a header fails with `USTAR_HEADER_CHECKSUM`;
- corrupting an unchecked compressed payload passes the structural scan but
  fails full decode with `LOGICAL_ROLE_MISMATCH`;
- corrupting the root text fails with `LOGICAL_ROOT_MISMATCH`; and
- corrupting a checksummed compressed payload still requires a decode before
  it fails, then reports `SHUFFLED_BLOCK_DECODE`.

The selected provisional Stage 3 policy is therefore:

- ordinary automatic generation checkpoints use one pass that calculates
  logical hashes and counts while encoding, completes the codec and container,
  flushes and fsyncs the file, checks its final length, atomically renames it,
  and fsyncs the parent directory on Debian;
- Zstandard frame checksums remain off because they add no creation-time
  detection without a decode and the logical role hash already verifies
  decoded content;
- an automatic checkpoint does not require a second lightweight scan because
  that scan does not verify payloads; strict scanning remains mandatory when
  startup, import or restore consumes an archive;
- manual exports and pinned checkpoints receive full post-write decode once
  those paths exist; whether periodic milestones should also receive it remains
  measurement-gated on the production Rust codec and target VM; and
- recovery to a previous retained valid checkpoint is the protection against a
  latent codec or storage fault discovered on restore.

This is the simplest policy supported by the development-machine evidence. It
does not prove the target checkpoint-latency gate: P2's single-pass p95 was
587.623 ms on a Ryzen 7 5800X, so performance on the Ryzen 7 2700 remains
unknown. Windows regular-file fsync succeeded, but Windows parent-directory
fsync returned `EPERM`; the required Debian directory-fsync behavior remains
open. The observed peak process RSS, 185.65 MiB for P0 and 715.69 MiB for P2,
also includes the TypeScript `World`, evolved population and packed source
buffer, so it is not a prediction of Rust engine memory.

All temporary benchmark files were removed and every artifact assertion
passed. Free-space sampling after cleanup differed from the initial reading by
4,096 bytes for P0 and 565,248 bytes for P2; filesystem free-space readings
are not a byte-exact leak or reclamation test. No owner database or save file
was written.

## Narrow SQLite byte-volume comparison

The approved disposable experiment wrote representative already-compressed P0
and P2 byte volumes to one-megabyte SQLite BLOB rows with WAL, `synchronous =
FULL`, and automatic checkpointing disabled. It built no checkpoint schema,
reader, backup, pruning, recovery or export implementation.

| Payload | Synchronous insert | Main-loop timer delay | WAL after commit | WAL/payload | WAL checkpoint | Read-back | Delete |
|---|---:|---:|---:|---:|---:|---:|---:|
| P0, 2,527,124 bytes | 12.51 ms | 13.05 ms | 2.43 MiB | 1.009x | 7.60 ms | 3.33 ms | 4.00 ms |
| P2, 75,563,269 bytes | 273.60 ms | 274.15 ms | 72.59 MiB | 1.007x | 201.04 ms | 104.35 ms | 55.44 ms |

After deletion and checkpoint, the P2 database still occupied its prior file
size with 18,472 free pages. Re-inserting the same volume added zero main-file
pages, proving normal free-page reuse; only explicit `VACUUM` returned the file
to its two-page baseline. The same synchronous work delayed a scheduled
main-loop timer for essentially the operation duration.

This is development-SSD evidence, not VM latency evidence. It confirms the
written architectural comparison: population-sized SQLite transactions create
population-sized WAL work and event-loop stalls if run on the main thread,
while deletion does not return filesystem space without compaction. No
correctness requirement has appeared that justifies replacing the selected
immutable managed files with this more complicated payload-in-SQLite path.

## Current browser import/export reproduction

The current browser path was exercised through the built UI against a
disposable real server and P1 database at the normal 30 Hz display publication
rate. The production source was commit
`ec1cc708423c4337f1d5f0ed73ac7a1f7b9ecdf8`; the tested built JavaScript asset
had SHA-256
`032ffa5518ca21938e02d79acbfb10d5df35638540ea71c944cd34db7988182c`.
Browser automation used the Codex in-app Browser plugin version
`26.721.41059` on the Windows environment recorded by the retained source and
runtime artifacts.
The delay while the browser tool awaited owner approval to monitor loopback was
excluded from every timing below.

The fixture contained 300 differently weighted evolved snakes with 16,149,600
packed weight bytes. Its exact current server JSON export was 81,293,145 bytes
(77.53 MiB), SHA-256
`011b7b3b2ec30ea9d6b4f72fd4a15ade9d3a9cbe9eb75fd1159428ec1fb3cecf`.
The large payload was temporary rather than committed.

For browser export:

- combined JavaScript heap plus backing storage began at 5,964,125 bytes;
- it peaked at 256,485,515 bytes 6.222 seconds after the Export click:
  168,733,600 used-heap bytes plus 87,751,915 backing-store bytes;
- that is about 244.6 MiB in the two reported browser memory categories for a
  population with only 15.4 MiB of packed weights;
- the browser materialized the response, parsed it, rebuilt a population
  object, stringified it again and created a Blob exactly as the current source
  audit predicted; and
- the in-app browser did not expose a download event within the bounded
  50-second listener despite no page error or alert. This event observation is
  not treated as proof that Chromium itself failed to write a file; the
  population-sized memory peak proves the defect independently.

For browser import, the exact 77.53 MiB JSON file was selected through the
normal file picker:

- the file-selection operation remained busy for 151.337 seconds before the
  first heap sample could execute;
- because the browser action itself blocked that interval, the trace does not
  claim to contain the parse-time peak;
- after the action released, combined reported heap and backing storage rose
  from a 6,002,453-byte baseline to at least 48,878,389 bytes before failure;
- the UI logged `TypeError: Failed to fetch` and displayed its failure alert;
  and
- a direct replay of the same body proved the server limit: it stopped after
  53,018,624 uploaded bytes and returned HTTP 400 with
  `{"ok":false,"message":"payload too large"}`.

The retained raw traces are
`windows-5800x/browser-current-export-p1.jsonl` and
`windows-5800x/browser-current-import-p1.jsonl`. Their SHA-256 digests are
`4be16b5eecd7246d8d868d7ef25e7c7310f119e46e7bb90448713a78893a472b`
and
`b4c02d6790dd1d33b9787b68c972acaf7db13b1c6545c73d18127a9e9aef1556`.
The HTTP reproduction is retained separately beside them. These are Windows
development-machine defect measurements, not acceptance results for the future
direct compressed archive path.

## Current real-server external-control measurements

The real current server was run with three simultaneous Protocol 2 loopback
clients: a periodic latest-value UI controller, an observation-driven
wire-compatible bot, and a spectator receiving complete frame-v1 buffers.
Each measurement used a disposable database, the native serial backend, two
seconds of warm-up and fifteen seconds of runner-monotonic measured wall time.
The source was clean commit
`24dca58a4fa36fcfc183ca8e21d0df2bc007bfc6`.

| Workload/cadence | Sim seconds per wall second | Player actions | Player-send p95 | Bot sensor p95 | Display frames | Frame-interval p95 | Event-loop p95 | Peak RSS | Dropped sim debt |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| P0, requested 30 Hz | 1.0035x | 347 | 54.33 ms | 30.98 ms | 260 | 70.67 ms | 31.64 ms | 162.5 MiB | 0 s |
| P0, requested 60 Hz | 1.0024x | 521 | 33.59 ms | 31.38 ms | 262 | 72.75 ms | 32.26 ms | 163.1 MiB | 0 s |
| P1, requested 30 Hz | 0.6365x | 349 | 61.45 ms | 41.03 ms | 5 | 3,870.71 ms | 41.55 ms | 280.3 MiB | 6.900 s |
| P1, requested 60 Hz | 0.6261x | 542 | 44.20 ms | 44.35 ms | 5 | 3,895.29 ms | 43.71 ms | 282.5 MiB | 7.033 s |

The periodic client did not sustain either requested cadence even on P0:
347/521 actions over fifteen seconds are about 23.1/34.7 sends per second.
P1 reproduced the owner-visible failure more severely. It advanced only
0.63–0.64 simulated seconds per wall second, discarded about seven simulated
seconds of scheduler debt, and delivered just five display frames during each
fifteen-second measurement. Requesting 60 Hz produced fresher timer callbacks
than requesting 30 Hz but did not repair the overloaded server and slightly
reduced simulated throughput in these single runs. These results do not select
the final browser cadence.

The retained `actionToNextSensorMs` field is only a latest-action-to-next-
sensor receipt upper bound. In overloaded P1, several player actions can occur
between sensor deliveries, so its near-zero percentile records the action
nearest a delayed sensor rather than end-to-end command-to-step latency. It is
not used as proof of acceptable control latency. The Rust vertical slice must
add authoritative accepted-action and applied-step correlation.

The raw artifacts are
`windows-5800x/external-p5-p0-30hz.json`,
`windows-5800x/external-p5-p0-60hz.json`,
`windows-5800x/external-p5-p1-30hz.json`, and
`windows-5800x/external-p5-p1-60hz.json`. Their SHA-256 digests are,
respectively,
`cd3036ba73ce7aa76ece95fe5775918a46353262ad2835a9823fb6d680ae7621`,
`d56f675b73456d8f3385363b35c7c1652419dbb3b2a7b14d74680176e77da925`,
`22c46fae875ba9a61d7b4f54509a212e8b5e0fbd74c2fdf0169026739b2e4367`,
and
`942bebdcdae6e2c7cee1e8e06518fa2f18a154656647ec86467ff502cf37e786`.

This is a real-server compatibility baseline, not the full P5 gate. The
clients run in the same Node process on loopback: they are not the actual
browser renderer, the owner's separate RL trainer, another LAN device, or the
target Debian VM. The earlier wait for owner approval to let the browser tool
observe a loopback address/port is outside the runner and outside every timing
reported here.

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
- real browser/LAN/owner-trainer/target-VM P5 and cadence measurements (the
  current-server synthetic loopback baseline is retained above);
- sustained P4, P6 accelerated and P7 soak fixtures;
- P8 full checkpoint-v3 publication, durability and restore testing beyond the
  retained size-matched 480-generation retention fixture;
- repeat the selected checkpoint-v3 publication/restore policy in Rust on the
  target Debian VM, including parent-directory fsync, state restoration and the
  target checkpoint-latency barrier;
- real owner save files outside the repository, if any;
- Debian graph-ordering output from the exact retained database/spec fixture.

No current Windows number is presented as proof that P0, P1 or P2 already meets
the target VM gate.

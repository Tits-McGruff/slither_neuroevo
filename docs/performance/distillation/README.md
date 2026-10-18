# Live distillation qualification

The `live-canonical-distillation-v1` reports exercise real committed Rust
Basic B8 / deterministic baseline-v2 samples through the production HTTP spool,
PyRL's independent client/replay/trainer, a process restart, native candidate
validation and four Rust shadow slots. The initial population uses the small
qualification Dense graph; the trained student uses GRU8/Dense2 with 1,962
parameters. These are functional integration and resource observations for this
explicit workload, not the final P0/P1/P2 capacity, ablation or six-hour soak gate.

Each run uses the default batch of 32 windows, 32 burn-in deliveries and at most
64 BPTT deliveries. The first process checkpoints update 125 and records the
next actual optimizer update. A new process restores that checkpoint and must
reproduce update 126's exact weights, optimizer, sampler, recurrence and private
RNG witnesses before continuing to update 250. Its immutable candidate then
passes independent Rust validation and real shadow evaluation. Initial retained
live data stays fixed during the update comparison; asynchronous arrival is not
claimed to produce identical replay across separate runs.

Reports retain source hashes, numerical environment, process peak RSS, Torch
allocated/reserved GPU peaks, native step timing and bounded diagnostic HTTP route
counts. Torch allocator statistics exclude CUDA context/driver allocations and
are not a measurement of whole-process VRAM. Shared-host wall timings are
observations, not release performance thresholds. The existing native exact
continuation tests remain the dependency evidence for capture/four-shadow
isolation across one and three workers; these HTTP runs prove live provenance,
world progress and absence of actor/controller traffic.

Build the production addon and the separate source-identified test-hooks addon,
then run from the Slither checkout (replace the interpreter/checkout paths):

```powershell
npm --prefix native run build
node node_modules/tsx/dist/cli.mjs scripts/prepare-panic-test-addon.ts
node node_modules/typescript/bin/tsc -p scripts/qualification/tsconfig.json
node node_modules/tsx/dist/cli.mjs scripts/qualification/live-distillation.ts --pyrl-root C:/path/to/PyRL-trainer --python C:/path/to/PyRL-trainer/.venv/Scripts/python.exe --device cpu --workers 1 --report live-cpu-w1.json
```

`--device` accepts `cpu` or explicit `cuda:0`; `--workers` accepts 1 or 3.
`--spool-gib` selects an admitted 1..64 GiB producer disk ceiling, default 4.
The trainer requires matching reported policy before consuming samples.
Reports use exclusive creation. The host creates and verifies a private temporary
root, respects the normal disk-admission reserve, services native generation
persistence through the existing router, and stops/cleans its processes and files
before exiting. It never opens an owner database or changes production defaults.
The pinned PyRL CI job runs the same gate on Linux CPU with one and three workers;
local Windows CPU/CUDA reports are retained here.


The v1 Windows cases are CPU with one/three workers and CUDA with one
worker. The CUDA report's host bytes correspond to commit `ac360d2` (Windows
line endings); CPU reports use `b39b741`, which adds strict host type checking and
scheduler-debt telemetry. Native/core trainer math is unchanged between these
hosts. The later v2 CUDA three-worker gate is recorded below; the named
larger-workload/soak gates remain unqualified. A report's source hashes and explicit workload define its scope;
none of these small-student runs selects production v4/v2 defaults.


`live-canonical-distillation-v2` adds the managed-library handoff. A second
250-update student matches the Dense world graph. Both trained architectures are
saved with distilled provenance; GRU live deployment is rejected in that world,
and the compatible Dense original exports byte-for-byte. The fixture explicitly
applies the matching 0.06-second neural cadence after training, then checks native
deployment, exact retry receipts, catalog deletion, cold file reclamation and
continued inference/provenance from the retained live brain. The four-GRU-shadow
snapshot is taken before the compatible-student handoff. The new
`live-library-cpu-w1-v1.json` and `live-library-cpu-w3-v1.json` reports pass on
Windows; the pinned Linux job runs the same extended gate. The earlier v1 reports
retain their original scope and source identities.


`live-library-cuda-w3-v1.json` passes the extended v2 gate on Windows with three
native workers, Torch 2.14.0+cu126/CUDA 12.6 and an RTX 4080. GRU and compatible
Dense students each reach 250 updates, with independent-process update-126
restart witnesses, native shadows and the complete library/deletion/retained
inference handoff. Exact sources are Slither `8c2df37` and PyRL `9aa4eee`; the
report also binds the executed file and native hashes.

The GRU processes take 46.0 and 41.203 seconds; Dense takes 11.625 and 10.953.
The reported host peak is 159.0 MiB, GRU Python peak 1,297.2 MiB, Torch allocated
peak 17.8 MiB and reserved peak 22 MiB. CUDA allocator peaks exclude context and
driver allocations. Native p95/p99 buckets are 6/8 ms with zero dropped wall debt
for this explicitly small Dense world, not the required modest/large graph cost
gate. All four shadow slots deliver actual samples, and forbidden controller
requests remain zero. Larger resource workloads and sustained held-out quality
remain separate gates.

The configured 8 GiB device allowance is a ceiling. Admission checks the calculated
job estimate against both that ceiling and current free CUDA memory; it does not
require 8 GiB free for this small job. This run uses the unchanged job and sequence
configuration.


`live-storage-cpu-w1-q4-v1.json` and `live-storage-cuda-w3-q64-v1.json`
qualify admitted producer storage-policy provenance at 4/64 GiB on Windows with
one/three native workers. Slither sources are `c328d4d` / `1988993`, paired with
PyRL `da21983`; native source is unchanged from the preceding sensor qualification.
Both GRU and compatible Dense students reach 250 updates, reproduce update 126
after a fresh-process checkpoint restore, and pass native shadows plus the
retained-library deployment/deletion handoff. Reported producer policy matches
the saved trainer policy; the later host explicitly passes the selected quota to
both students and retains the Dense policy in its deployment witness.

The 64 GiB ceiling does not allocate or fill 64 GiB. These checks establish
admission and checkpoint provenance, leaving loaded spool rotation, sustained
throughput, maximum-quota memory and integrated-soak gates open. Native worker
and checkpoint scratch remain 512 MiB.

## Loaded opaque-spool qualification

The dedicated host exercises the actual isolated storage worker without creating a
game or decoding samples. It writes real synthetic opaque objects through ordinary
ingestion, preserves the physical disk reserve, tests retention and leased byte
identity, and restarts under both the original and a smaller admitted quota.

```powershell
node node_modules/tsx/dist/cli.mjs scripts/qualification/spool-capacity.ts --quota-gib 4 --restart-quota-gib 1 --object-kib 1024 --workspace native/target/spool-default4g --output spool-default4g.json
```

Workspace and report must be new, with the report outside the disposable workspace.
The workspace parent must exist. Default object count fills the selected byte or
65,536-object ceiling; `--objects 64 --object-kib 64 --quota-gib 1` is an explicitly
partial smoke run. Reports distinguish byte capacity, object capacity and partial
loads using the actual completed initial-object count. All runs keep the production
startup/request deadlines and lease/reserve bounds. Cleanup follows worker shutdown.
`--retain-on-failure true` keeps only a failed task-owned fixture for diagnosis; it
must be removed after its follow-up checks. The outer qualification deadline can
be selected up to 10,800 seconds to accommodate 65,536 durable file publications.
This does not change the worker's 60-second operation/startup deadlines.

Windows and Debian baseline reports in `spool-capacity-*-v1/` fill the actual 4 GiB
default with 4,096 one-MiB objects, then rotate new objects and reopen at 1 GiB.
Both pass with zero native/spool drops and settled watermarks. They exercise 32
pending metadata requests, four concurrent downloads, 64 leases, 512 MiB of unique
pins with overlapping leases, expiry/renewal, 256 manifests and complete pagination.
All manifests, stopped state and exact pinned bytes survive both restarts.

Windows records about 207 MiB sampled peak RSS, 5.579-second loaded restart and
21.172-second smaller-quota restart. Debian records about 288 MiB, 8.655 seconds
and 57.350 seconds respectively. These are shared-host storage measurements; they
exclude native sample encoding, HTTP, training, shadows and the integrated soak.
The original Windows 65,536-object case (64-KiB objects, 4 GiB retained under a
64-GiB quota) fills successfully and reopens in 37.132 seconds, but reduction to
1 GiB exceeds the 60-second startup deadline. Its failed report is retained.
Retention formerly rescanned all object totals and committed one deletion per
victim. It now keeps worker-exclusive totals, updates them after durable metadata
commits, and deletes the minimal oldest unpinned prefix in batches of at most 64.
Metadata removal commits before file unlink; interrupted cleanup leaves only
unadvertised orphan files for the existing recovery pass.

The revised Debian 4-GiB case reduces smaller-quota restart from 57.350 to 9.292
seconds, with about 256 MiB sampled peak RSS. Its 65,536-object case also passes:
12.168-second loaded restart, 32.348-second smaller-quota restart and about 398 MiB
sampled peak RSS. That case reaches the object ceiling and the 1,024-object pin
ceiling, while retaining only 4 GiB; it does not qualify 64 GiB of retained bytes.
Thirty-five spool tests pass on each platform, including committed retry, multiple
retention batches, active pins and replacement across datasets. The full 64-GiB byte gate remains open; the corrected Windows count result is recorded below.

A subsequent Windows candidate run stops during publication at about 16,000
objects with `EPERM` while renaming a complete temporary file over its exclusive
final reservation. Its failed fixture was retained for the corrected continuation below, then removed after successful worker shutdown. Windows
publication now retries only `EPERM`/`EACCES`/`EBUSY`, at most six attempts with
310 ms total requested worker backoff; other I/O failures propagate immediately.
Metadata publication still follows complete durable file publication. All 39
Windows spool tests pass, including transient/persistent locks, permanent I/O
errors and preservation of an existing final file; eight real capture/shadow
checks also pass. The focused error-path checks precede the corrected maximum-object run below.

Measured hosts are retained losslessly as `spool-capacity-host-v1.ts.gz` and
`spool-capacity-host-v2.ts.gz`; their decompressed canonical-LF digests match the
corresponding source receipts. Current reports bind their host/store sources
independently. Fill progress uses acknowledged durable inventory and verifies
watermarks and drop counters after every batch. CI runs the small smoke on
Ubuntu/Windows Node 24 and retains its honest partial-load report.

A retained failed Windows count fixture can be checked with
`scripts/qualification/spool-capacity-continue.ts --workspace PATH --failed-report PATH --output PATH`.
The original, unredacted failed report must identify that exact directory. Before
starting a mutable worker, the harness opens metadata read-only and verifies the
synthetic run, sensor/teacher contracts, seed, state and uniform 64-KiB inventory.
It recovers the old capture as interrupted, creates a separate capture for new
objects, and checks global inventory at 65,536 objects. Leased objects from both
captures must remain exact after loaded and 1-GiB restarts. A failure retains the
fixture; success closes the worker and verifies directory identity before cleanup.
This count test retains 4 GiB, so it cannot qualify 64 GiB of stored bytes.

Shared opaque fixture helpers keep the fresh-load and retained-load hosts aligned.
An automated integrity check verifies historical host archives, patches, unique
report paths, outcome labels and recorded host/store identities on both platforms.

The corrected Windows continuation passes the global 65,536-object ceiling with
64-KiB objects (4 GiB retained under the selected 64-GiB quota). Read-only ownership
checks find 16,040 complete prior objects; production recovery closes that capture
as interrupted, and a new capture adds 49,496. Exact pinned objects from both
captures survive loaded and 1-GiB restarts, taking 37.888 and 56.717 seconds within
the unchanged 60-second startup deadline. Sampled peak RSS is about 358 MiB;
the full continuation takes 854.615 seconds, including fill and cleanup. Both
captures' zero-drop watermarks remain recorded. The task-owned fixture is removed.
The narrow remaining deadline margin is visible; this is shared-host evidence,
not a guaranteed restart time. It does not qualify 64 GiB of retained bytes.

## Canonical training-window capacity

PyRL checkpoint `040f543` exposes the unchanged job device estimate to a dedicated
capacity host. Rust's identified offline compiler emits the large Motion/B32
MLP/GRU96/Dense2 graph with 494,306 parameters and 504 inputs, using isolated
initialization seed one. The request, one zero input, canonical manifest, compiler
receipt, exact host and five reports are retained in `canonical-capacity-windows-v1/`.
The native compiler source is identified in its receipt; this is independent of
current live-game execution qualification.

All admitted cases complete three actual canonical Adam/TBPTT updates with full
active synthetic Float32 windows and event weight four. GPU measurements use the
qualified Torch 2.14/CUDA 12.6 RTX 4080 environment, two CPU threads, FP32 and an
8-GiB admission budget.

| Device/case | Batch | BPTT/burn-in | Estimate MiB | Peak CUDA allocated/reserved MiB | Result |
|---|---:|---:|---:|---:|---|
| CPU default window | 32 | 64/32 | 647.55 | — | Three updates |
| CUDA default window | 32 | 64/32 | 647.55 | 49.24 / 58 | Three updates |
| CUDA longest BPTT, admitted boundary | 129 | 256/0 | 8,162.93 | 339.86 / 358 | Three updates |
| CUDA maximum batch and burn-in | 256 | 128/128 | 8,101.00 | 400.06 / 414 | Three updates |
| Next BPTT batch | 130 | 256/0 | 8,225.04 | No device/model allocation | Rejected before allocation |

The conservative ceiling is not allocated. These cases qualify these synthetic
training windows under the current estimate, not loaded replay RAM, teacher codec,
HTTP, trainer checkpoint peaks, live game contention, four shadows or the integrated
soak. The host reports process RSS separately from device memory. Automated
integrity checks bind every result to exact inputs and completed optimizer work;
two PyRL tests also verify real training and rejection before device allocation.
The existing 73 focused job/control tests pass and changed Python files pass lint.

## Loaded replay RAM

From `native/`, the standalone fixture harness writes a new disposable directory:

```powershell
cargo run --release --locked --manifest-path ../scripts/qualification/teacher-replay-fixture/Cargo.toml --target-dir target -- target/replay-fixture 10000
```

It calls the production Rust encoder and decoder for every synthetic Motion/B32
chunk, checks each exact roundtrip, and preserves one GiB of physical free-space
reserve. Its separate source and locked dependency receipts identify the offline
harness; the production native source/lockfile remain unchanged. The 10,000-chunk
fixture contains 600,000 synthetic rows in a 1,226,610,000-byte length-framed bundle.
Python never encodes a teacher sample.

PyRL checkpoint `4593011` loads this bundle through the real decoder/replay at the
minimum, default and maximum public RAM budgets. Every eight-object proposal
checks exact retry idempotence, constant per-chunk accounting and complete FIFO
rotation. A second decode constructs independent retained objects and restores
the replay snapshot with identical cursor, counters, retry state and raw witnesses.
Reports and measured sources are retained in `replay-capacity-windows-v1/` and
`replay-capacity-debian-v1/`. Debian decodes the same portable Rust-produced bytes;
the fixture was encoded on Windows.

| Replay budget MiB | Retained chunks | Evicted chunks | Fill / restore seconds | Peak RSS after fill / dual-owner restore MiB |
|---|---:|---:|---:|---:|
| 64 | 268 | 9,732 | 6.824 / 0.395 | 563.55 / 622.91 |
| 256 | 1,076 | 8,924 | 6.939 / 0.849 | 762.56 / 1,010.37 |
| 2,048 | 8,616 | 1,384 | 7.886 / 5.497 | 2,544.69 / 4,574.64 |

Each retained budget has less than one chunk of unused capacity. Process RSS also
includes the Python/Torch environment; restore intentionally retains the original
and independently decoded replay at once. These are descriptive Windows memory
measurements, not a claim that whole-process memory equals the selected replay
budget. Seventeen focused replay/framing/capacity tests pass; the standalone Rust harness passes
rustfmt and Clippy. The same 17 focused tests also pass on Debian with Python
3.13.5 and Torch 2.14 CPU. Debian fill/restore times are 20.569/0.797,
21.073/2.619 and 25.785/18.052 seconds respectively. Peak RSS during independent
restore is 347.22, 728.36 and 4,285.50 MiB. These shared-host measurements do not
qualify HTTP/optimizer coexistence, durable trainer checkpoint peaks, live-game
contention or integrated soak.

The Debian full-byte case writes all 65,536 one-MiB objects (64 GiB), passes
pressure rotation, exact downloads, lease/manifest admission and pagination, then
fails the first loaded reopen at the unchanged 60-second startup deadline.
Its report remains failed with `qualificationComplete: false`; the full fixture
was retained for the recovery retest below. Sampled host RSS is about 352 MiB. This does not
close the full-byte gate or select a larger production default.

A subsequent read-only diagnostic verifies four disjoint 4-GiB ranges. Sequential
read/stat/SHA checks take 16.832 and 20.658 seconds; bounded groups of four async
read/stat/WebCrypto checks take 13.933 and 12.910 seconds. Kernel physical-read
counters record approximately 4 GiB per case. Shared-host/cache states differ, and
these partial ranges are not full-startup qualification. Their source and compact
measurements are retained alongside the capacity reports; the historical RSS
field records end-of-case snapshots. Windows also fills the actual 64 GiB and
fails the former 60-second loaded startup; that failed report remains retained.

Recovery now prices one absolute startup deadline from the actual retained
inventory, independently of the newly selected quota. Initial metadata opening
gets 60 seconds. One validated receipt permits 10 additional seconds per GiB
beyond 4 GiB, up to 660 seconds for 64 GiB. Receipt delivery never resets the
clock; malformed, repeated or late receipts fail. Readiness still requires every
retained file's integrity check and complete quota readmission. Ordinary requests
retain 60 seconds. Initialization cancellation stops the worker immediately, and
leases keep their declared expiry, checked again after potentially long recovery.

`scripts/qualification/spool-capacity-recover.ts --workspace PATH --failed-report PATH --output PATH`
reuses only a read-only-verified, owned synthetic full-byte fixture. It verifies
full recovery, smaller-quota recovery, exact watermarks and pinned downloads or
honest lease expiry. It fsyncs an exclusive report outside the fixture before
verified cleanup. Windows passes full recovery in 142.707 seconds and reduction
to 1 GiB in 173.445 seconds, both within the inventory-priced bound. The two old
lease downloads remain exact; a fresh recent-object pin is exact too. Watermarks,
contracts and zero drops match the failed predecessor. Sampled peak RSS is
372.38 MiB, and successful cleanup releases the fixture. Debian also passes full
recovery in 408.298 seconds and 1-GiB reduction in 457.589 seconds, within the
same bound, with about 404.23 MiB sampled peak RSS. Its five-minute old lease
expires during smaller-quota recovery and rejects as declared; a fresh recent
pin downloads exact bytes. Watermarks, contracts, zero drops and 256 manifests
remain intact, and its fixture is removed after the report is durably saved.
These complete both opaque full-byte recovery gates, without selecting larger
production defaults or qualifying codec/HTTP/trainer coexistence.


## Loaded durable trainer checkpoints

PyRL checkpoint `b0793dc` fills the same Rust-produced fixture through actual replay
retention, trains the 494,306-parameter Motion/B32 graph, and saves one complete
job through the production serializer. Adam moments are populated. The original
job then takes its next update; an independently restored job must reproduce its
windows, weights, Adam state, detached recurrence, metrics and private RNG exactly.
Both jobs coexist during restore. Reports and exact host are retained in
`checkpoint-capacity-windows-v1/`.

| Device | Replay MiB | Checkpoint bytes | Save / restore-plus-next-update seconds | Peak process RSS MiB |
|---|---:|---:|---:|---:|
| CPU | 64 | 38,925,133 | 0.179 / 1.087 | 870.23 |
| CPU | 256 | 138,172,581 | 0.728 / 2.421 | 1,253.35 |
| CPU | 2,048 | 1,064,318,324 | 4.186 / 17.067 | 4,817.39 |
| CUDA | 2,048 | 1,064,318,636 | 6.642 / 15.726 | 5,388.61 |

All four cases pass exact next-update continuation and preserve disk reserve.
Each restored job has completed two updates, 4,096 training samples and 2,048
burn-in samples. CUDA's observed allocated peak is 57.37 MiB; the default windows
and two resident model/optimizer owners do not allocate the 8-GiB admission ceiling.
The full Windows trainer suite passes 269 tests with two existing optional-fixture
skips. These reports cover one offline durable checkpoint with no pending candidate
backlog. Debian passes the same three replay budgets: save/restore-plus-next-update
times are 0.590/8.216, 1.404/13.525 and 11.558/48.622 seconds. Peak process RSS is
484.63, 866.71 and 4,429.39 MiB. Exact source and reports are in
`checkpoint-capacity-debian-v1/`.

The follow-up `--checkpoint-saves 4` host (`f75032a`) exercises real production
retention at 2,048 MiB replay. Windows CPU/CUDA both retain three complete
checkpoints, remove serial one, retain about 3.19 GB of physical files, and restore
serial four with exact next-update continuation. Four saves take 28.003/27.393
seconds; restore plus next update takes 12.157/12.531 seconds. Peak process RSS
is 4,816.72/5,373.52 MiB. The measured host and both reports remain in
`checkpoint-rotation-windows-v1/`. Four focused trainer tests pass, including
actual CPU/CUDA restore, rotation inventory and disabled-assertion rejection.
Debian also passes maximum replay rotation: four saves take 65.057 seconds,
latest restore plus next update takes 45.646 seconds, peak process RSS is
4,429.14 MiB, and exactly three files retain 3,192,954,885 bytes. Its source and
report remain in `checkpoint-rotation-debian-v1/`. Candidate backlog, HTTP/game
contention and integrated soak remain open. Successful checkpoint workspaces are removed after
the complete measurement report is fsynced outside them.


The subsequent request-admission change reserves the client data slot before
awaiting readiness and caps complete HTTP routes at 32. An empty graceful-close
request has reserved admission even when all data slots are occupied. Focused
client/store/category/shadow checks pass 66 tests, including real HTTP saturation
and cleanup. This queue ownership change reuses the preceding serial full-byte
recovery evidence; it does not change file verification, retention or deadlines.

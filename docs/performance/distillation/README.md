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
retention batches, active pins and replacement across datasets. The corrected Windows maximum-object and full 64-GiB byte gates remain open.

A subsequent Windows candidate run stops during publication at about 16,000
objects with `EPERM` while renaming a complete temporary file over its exclusive
final reservation. Its failed fixture remains task-owned for follow-up. Windows
publication now retries only `EPERM`/`EACCES`/`EBUSY`, at most six attempts with
310 ms total requested worker backoff; other I/O failures propagate immediately.
Metadata publication still follows complete durable file publication. All 39
Windows spool tests pass, including transient/persistent locks, permanent I/O
errors and preservation of an existing final file; eight real capture/shadow
checks also pass. These focused checks do not substitute for the remaining
maximum-capacity Windows run.

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

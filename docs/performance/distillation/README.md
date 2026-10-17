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

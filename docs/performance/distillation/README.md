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
Reports use exclusive creation. The host creates and verifies a private temporary
root, respects the normal disk-admission reserve, services native generation
persistence through the existing router, and stops/cleans its processes and files
before exiting. It never opens an owner database or changes production defaults.
The pinned PyRL CI job runs the same gate on Linux CPU with one and three workers;
local Windows CPU/CUDA reports are retained here.

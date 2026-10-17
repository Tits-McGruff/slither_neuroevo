# Evolved lives, long generations, sensor v4 and canonical-graph distillation

Status: frozen behavioral plan, incorporating the owner's decisions and all accepted review amendments. Created 2026-10-11. Implementation is in progress; completed work and remaining stages are recorded below.

## Implementation progress (2026-10-17)

Stage A is implemented in the current worktree: the 8..604800-second duration
control/parser, hour-scale timers, generation-scoped manual completion through
Rust's ordinary terminal transaction, atomic completion chronology, and bounded
archive-v2 sidecars with archive-v1 compatibility. The native checkpoint-v3
boundary and frame-v1 contracts remain unchanged.

Stage A acceptance passed 501 native tests plus the doctest, the 721-test
JavaScript aggregate (18 existing skips), and the 169-test required-native
overlay (6 existing skips), with type/lint/client-build checks. Oxygen passed
its seven-day clock, manual scope, checkpoint/archive and real HTTP roundtrip
checks. The implementation is now committed and pushed on
`evolved-lives-sensors-distillation`, beginning with `9275610` and the
checkpoint-codec Clippy correction `0031b70`; GitHub CI now starts automatically
on this project branch at each pushed checkpoint.

Stage B's native accounting core is implemented: dense population-slot lives,
ordered prefix/substep credits and actual burn attribution, independent
life/generation wallets, best-life and non-point assessment tuples, growth,
simultaneous death/corpse diagnostics, and atomic running-state publication.
The evolution assessment adapter preserves exact one-life legacy evolution
and supports configured-duration scaling and finite negative life penalties.
Terminal evolution now consumes the selected slot assessments and life penalties,
including negative fitness. The life settings/UI and strict legacy/modern schema
selection are connected; the fixed history/HoF contract is described below.

Stage C's evolved-only bounded placement API and full-width generation-labelled
RNG derivation are implemented. RNG-bundle v2 now stores dense, uniform-only
per-slot continuations in the existing checkpoint-v3 state role, with strict
v1/v2 dispatch, bounded decode and unchanged v1 encoding. Retained phase copies,
external replacement isolation/proofs and successor derivation preserve the
new continuations.

The versioned life-policy extension, explicit legacy dispatch, one-time death
delay sampling, integer-step deadlines, finite exhaustion, quarter-second retry
cooldowns, deadline/attempt/slot ordering and eight-slot attempt selection are
implemented and unit-tested. The running prefix now replaces due evolved lives
in place before shared control, retains genome/brain ownership, clears motor
state, awards newborns once, cleans pellet ownership and compacts the batch.
Reset/Preserve recurrence publishes with the complete step. A private sealed
witness binds replacement identities and per-slot RNG to both running and
terminal admission. Pending time excludes participating newborns; collision
zero-delay deaths wait for the next newborn boundary. Automatic early completion
uses finite exhaustion and leaves unlimited/pending experiments running.
Complete-step regressions cover these rules, identity-exhaustion rollback,
carried versus local wallets, point assessments and manual terminal penalties.
Normal/None/Subtract replacement mass corpse policies now share ordinary and
cold God Mode effects. Compensation permits zero and caps actual randomized
value; exhausted deaths retain normal rules. Discard processes all canonical
attempt draws, allocates only retained IDs, preserves existing pellets and
records attempted/emitted/discarded count/value. Running saturation and cold
God Mode failure remain atomic; cold delay sampling commits before ambient RNG.
Dropdown controls now support labelled integer choices and atomic draft
validation. Production startup, reset/New Run and checkpoint/archive restore
recognize the complete life contract while preserving recorded legacy settings.
The UI requires explicit opt-in for legacy upgrades, rejects malformed life
values atomically and includes the respawn preset. V3 relative-score scalars
use selected assessments and a monotonic selected-score peak; wallets and
delivery deltas stay physical. Scalar/parallel sensing regression evidence
passes. Bounded native life-end records now capture simultaneous contacts,
exclusive cause, historical IDs, economics, food/kills, length/age and actual
corpse emission before scratch is lost. Each transaction retains at most one
end per population owner; the running recent-event ring retains the latest 256.
Both buffers are admitted and reused. Running publication verifies append-only
event chronology with death counters; cold God Mode shares capture. Boundary
checkpoints remain free of running events. Regressions cover 300 simultaneous
deaths, 600 successive lives, source-event integrity and capacity reuse.
On-demand native selection now binds world incarnation and full physical ID.
God Mode requires and revalidates that token; queued drags preserve their
original identity. Selection requests bind the observed process/run/generation,
are limited to one pending request per connection and 128 globally, and use
no gameplay RNG. The UI reacquires selected population owners at four Hz,
holds the follow camera during pending lives and clears stale physical-only
selections even when frame IDs wrap. Run/generation/reconnect/import boundaries
invalidate pending intent. Native visualization retains valid captures between
neural deliveries, keys lifetime to full physical ID/world, and emits bounded
sequenced clear markers on retirement. Frame-v1 remains unchanged.
Committed display stats now include fixed-size lifecycle telemetry: the five
owner-state counts, exact u64 life/death/respawn/failure counters, pending time,
bounded recent-event count/cursor, and actual placement work with explicit
eight-slot, 2,048-candidate and 250,000-geometry limits. The retained native
projection stays below 256 bytes and does not copy life logs or advance RNG.
The Stats tab validates this optional version-one contract and clears absent
or unsupported projections. Frame-v1 and boundary checkpoints are unchanged.
Windows qualification passes 595 release library tests with hooks, three
benchmark checks and the doctest, plus 43 focused protocol/UI tests and the
rebuilt addon real-server legacy/respawn/New Run and archive-retry scenarios.
TypeScript, Clippy, rustfmt, repository ESLint and the client build pass.
The final geometry-counter ordering correction reuses this broad release
evidence and passes the focused newborn-work regression after rebuilding.

Modern terminal generations now construct a 512-byte experiment history v1
and a 256-byte HoF assessment v2 from committed terminal inputs. The existing
56-byte fitness history and 120-byte archive HoF index retain their physical
meanings. SQLite commits the combined fixed payload atomically with completion,
summary, HoF and checkpoint pointers. Immutable retries reject changed or
omitted extensions. Archive v3 carries dense bounded 776-byte records (generation
plus payload), with explicit absent sentinels for inherited legacy rows; v1/v2
readers remain available. Validation binds schema, counts, completion chronology
and retained physical HoF source, without lifetime-sized experiment allocations.
Bounded on-demand history and HoF projections expose explicit assessment/bundle
and display-length semantics; frame-cadence stats retain compact fitness history.
The HoF UI labels Reset latest-life, Cumulative generation-peak and Best Life
selected-life-peak lengths. Focused native round trips compare exact imported
inventory and subsequent single/multiworker continuation; SQLite failpoints
cover publication/import before commit and reply loss after commit. Current
Windows qualification passes 594 release library tests with hooks, three
benchmark checks and the doctest; the narrow N-API inventory-version addition
is verified by the rebuilt addon and real HTTP export/import. The complete
JavaScript/native aggregate plus the corrected archive-helper file rerun covers
747 passing tests and 18 existing skips. Changed/omitted retry records, SQLite
publication/import failpoints, malformed projections and corrupted generation/
physical HoF bindings reject. TypeScript, Clippy, rustfmt, repository ESLint and
client build pass. Current Linux qualification remains part of the cohesive
respawn integration gate.

The preceding settings/restore/sensing and bounded-event checkpoint passed 591 Windows
release library tests with hooks, three benchmark-binary checks and the doctest.
Clippy, formatting, 69 focused selection/client/protocol tests, TypeScript,
repository-wide ESLint and the client build pass. The rebuilt addon passes the real-server legacy-upgrade,
respawn and New Run scenario. That preceding JavaScript/native aggregate passed 741 tests with
18 existing platform/capability skips, including all native-required files.
The rebuilt addon passes source identity, stale-life/world token rejection,
ordinary God Mode/visualization, checkpoint/archive and exact multiworker
import/restart continuation. The bounded archive test downloader uses streaming
Node HTTP with its existing deadline to support every OS-assigned loopback port.
Oxygen's earlier 517-test ledger-foundation evidence remains
scoped to that slice; current Linux qualification belongs to the cohesive
respawn integration gate. The copied-deployment fixture includes nonignored
new source and verifies source identity before startup.

Stage D now has native structural identities for frozen v3 and candidate v4
Basic/Motion, with exact labels/offsets, normalization and geometry meanings,
absence/validity encodings and a domain-separated digest. Equal-width profiles
remain incompatible. V4 revisions stay explicitly unfrozen pending workload
qualification and ablation. The pure geometry prototype covers capsule sides/
endpoints/tangency, disc-aware walls and all-head relative-motion encounters.
An exhausted intersection budget returns explicit Incomplete rather than Clear.
The allocation prototype validates the complete maximum-due bound, uses stable
population/baseline slots and non-population brains, grants equal base work and
rotating single-unit remainders, and retains private revision-guarded proposals.
It has no production limit defaults or finish-order redistribution. Shared
control staging now integrates the cursor as described below.
The physical-life first-delivery flag is integrated into existing actual Rust
delivery/acceptance, replacement initialization and authority witnesses. Pure
probes and rejected external deliveries preserve it; takeover keeps it and
both evolved recurrence modes reopen it on physical replacement.
Windows release qualification passes 617 native library tests with hooks,
three benchmark checks and the doctest; Clippy and formatting pass.
The rebuilt production addon and isolated release test-hooks addon pass all
13 required-native files: 170 tests, with six existing capability/platform skips.
TypeScript and repository ESLint pass; the client-build evidence is reused
from the preceding telemetry checkpoint because client sources are unchanged.
The five historical source-identity fixtures remain byte-for-byte unchanged.

The complete native v4 candidate evaluator now constructs every Basic/Motion
scalar and channel, including local economics, exact kind/life-budget sentinels,
food rings, body sectors/capsule rays, walls, nearest-head representatives and
all-head predicted urgency. Existing movement owns the shared effective-cost
formula. Complete-step heading/speed changes and applied turn are staged with
physics and reset at physical births, independently of neural delivery cadence.
A reused sensor-only hierarchy binds exact source-body records and stores each
segment once. A ray packet shares hierarchy/candidate visits across at most 32
directions; each exact capsule intersection remains individually charged.
Incomplete output identifies unresolved directions conservatively. Oracle
comparisons cover every exhaustion boundary; serial/multiworker vectors match.
Initial synthetic P0/P1 exploration at B16/B32 reports zero incomplete samples
with batch 65,536 and cap 256. At P1 the 32,768 batch still saturates; cap 128
also saturates at 65,536. This is preliminary source-shape evidence, not actual
gameplay/target-platform cost, histogram or ablation qualification. No production
minimum, default or normalization revision is frozen by this result.
Windows release qualification passes 636 library tests with hooks, three
benchmark checks and the doctest; Clippy, formatting, types and ESLint pass.
The rebuilt production and isolated test-hooks addons pass all 13 required-native
files: 170 tests, with six existing capability/platform skips. Client build
evidence is reused from the prior checkpoint because browser sources are unchanged.

V4 candidate layouts now route through the existing shared native control,
neural sensing/inference and packed external-observation staging. Due neural
and external owners receive immutable, canonical quotas before dispatch; v1
baselines retain their exact v3 evaluator and layout. The generation-scoped
cursor is copied through prefix/control/physics/replacement scratch, bound into
authority witnesses and published with the complete step. Empty generation
boundaries require its reset value. Failed preparation, stale control proposals
and rejected physics preserve source cursor, recurrence and delivery flags.
New tests cover both profiles at B8/B16/B32, 504-input inference, exact v1
baseline continuation, stable storage ordering, real parallel sensing/inference
with 12 due observations and 1/2/3/4 workers, and warm retained capacities.
Coordinator hierarchy/owner/grant capacities and worker-local v4 arrays/query/
packet storage are reported explicitly. Production settings, wire/persistence
propagation and full admission/target-platform qualification remain open.
Windows qualification passes 639 release library tests with hooks, three
benchmark checks and the doctest. All-target Clippy with hooks, rustfmt, types
and repository ESLint pass. The rebuilt production and source-identified
test-hooks addons pass all 13 required-native files: 170 tests with six existing
skips, including archive cancellation and exact multiworker continuation.
Client-build evidence is reused because browser sources are unchanged.

V4 now has strict native settings/state admission: a complete version-one
structural extension records profile and canonical digest, checks the exact
Input width and rejects incomplete, revised or same-width reinterpretations.
Numeric environment changes preserve the structural identity. Existing v3
schemas and compact v3 checkpoint-format metadata retain their compatibility.
The checkpoint-v3 settings role carries the new fields without changing its
packed population format. Production restore recognizes both complete v4
schema families. Candidate runs use the normal durable run-start, native
control/physics and checkpoint restore paths with explicit ray limits; ordinary
constructors still select v3 until workload/ablation qualification chooses limits.
Native startup emits bounded canonical contract bytes and their digest. Node
verifies those bytes against the digest, graph and recorded settings, then
forwards them in welcome metadata. Browser stack/editor sizing follows the exact
profile. Seven shared conformance fixtures cover v3 and both v4 profiles at
B8/B16/B32 and are included in both independent source-identity policies; the
five historical fixture files remain unchanged. Native reply admission prices
wider vectors, and controller routing checks the active width with metadata
cached across observations. Existing v3 snapshot-marker conversion remains.
Native regressions prove durable v4 restoration, first-step frame agreement
across one/three workers, structural rejection and environment compatibility.
Production selectors, ray/normalization freeze, workload/tactical qualification,
full resource accounting and saved-brain/trainer propagation remain open.
Windows qualification passes 643 release library tests with hooks, three
benchmark checks and the doctest; all-target Clippy with hooks and rustfmt pass.
The rebuilt production and isolated source-identified hooks addons pass all
13 required-native files: 170 tests with six existing skips. Unit/component
validation passes 384/61 tests (three existing unit capability skips), and the
focused browser startup set passes. TypeScript, repository ESLint and the
client build pass.

An actual native qualification runner now constructs the six specified graph
sizes, publishes a normal immutable run-start checkpoint, measures complete
steps plus frame packing, and reads consumed observations after publication.
Bounded raw/normalized histograms, due counts, cost percentiles and work/quality
counters retain exact source/contract identity. Observer on/off and worker-count
continuation tests guard noninterference. Windows Motion/B16 P1 at minimum 1,
cap 256 and batch 65,536 measured 3.02% incomplete with eight-segment BVH leaves;
four-segment leaves reduced this to 0.384% at unchanged limits. Native step p95
still misses the Windows 1x gate. These actual-run results supersede synthetic
zero-incomplete exploration for this configuration; limits/revisions remain
unfrozen. See [the runner and retained reports](../performance/sensor-v4/README.md).
Loaded Oxygen runs preserve the same source identity and bounded RSS, but even
frozen v3 misses the cost gate under concurrent Node load; they do not establish
clean target performance. Target-platform, trained-distribution, all-consumer
and ablation gates remain open. This checkpoint passes 646 release library
tests, three benchmark checks and the doctest, plus all 13 required-native files
(170 tests, six existing skips), type/lint/build, rustfmt and all-target Clippy.

Stage E's pure policy core now consumes strict Basic/Motion v4 inputs and the
recorded numeric sensing environment. It retains size-stage weights, discounts
food traps, scores capsule/wall openings and crowd/closing/crossing risk, and
uses visible relative size/velocity for pursuit and threat escape. Intercept
geometry is explicitly approximate from encoded clearance, without target IDs
or world reads. World-heading commitment lasts .2 seconds while safe; normalized
safe-sector improvement uses .15 with exact angular/index ties. Urgency uses the
declared TTC < .5-second threshold. Economic boost checks actual eligibility,
Float32 half-second budget, safe route and complete geometry. Deterministic mode
preserves its behavior RNG word; stochastic wander uses a separate versioned
run/seed/full-generation/slot label. Four real native sensor fixtures and eleven
policy/contract tests pass, alongside all ten retained v1 strategy and thirteen
RNG regressions. The native due/quota/control join is described below. UI
selector propagation and full tactical batches remain open; ordinary fresh
defaults remain v1 until workload qualification.
The policy checkpoint passes all 661 release library tests, 20 focused rebuilt-
addon source/surface/fresh-run checks, rustfmt and all-target Clippy. The previous
checkpoint's broader JS/type/lint/client-build and unchanged benchmark/doctest
results remain the dependency evidence for these untouched paths.

The v2 persistence foundation now has a strict three-field setting identity
(version, mode, cadence) and RNG bundle v3. Dense uniform-only behavior streams
are separate from lifecycle/placement Gaussian streams, with full-generation
labels when configured. V1/v2 packed bytes and derivation remain frozen. V3
encoding/decoding bounds count, proves packed bytes before allocation, and
rejects missing/zero/reordered streams or silent elision into older versions.
State admission binds the complete v4/v2 setting group to exactly bundle v3;
managed checkpoint restoration preserves exact words and subsequent draws.
Retained phase copies reuse capacity, generation successors derive both roles,
and memory/replacement witnesses cover the added streams. That checkpoint
rejected v2 activation until the native control join described below.
The RNG checkpoint passes 668 release library tests, three benchmark checks and
the doctest; the rebuilt production/hooks addons pass all 13 required-native
files (170 tests, six existing skips), rustfmt and all-target Clippy.

V2 now joins native shared control with immediate first-life evaluation and the
same bounded cadence algorithm as neural deployment. Held actions retain their
motor history and consume no observation or behavior RNG. Due baseline slots
join evolved/external consumers in the canonical fair quota ring before any v4
sampling. The policy reads only decoded fields; no v2 world/target scan is added.
Each existing lifecycle slot carries bounded physical-life policy and consumed-
step state, reset on replacement. Private control commitment seals exact policy
and behavior words; pending/resolved publication checks preserve that seal across
physics and reliable delivery. Late neural failure preserves source/cursor/state.
A real sixteen-step cold run agrees across one/three workers; mixed-controller
fixtures prove cadence, actual grants, held-state preservation and stale-seal
rejection in both profiles/modes. Explicit cold candidate construction performs
ordinary preflight and production restore recognizes all six complete schemas,
with a bounded excess-list rejection test. Workload/tactical qualification and
production/UI selection remain open; public defaults stay recorded v3/v1.
The control checkpoint passes 671 release library tests, three benchmark checks
and the doctest, rustfmt and all-target Clippy. The process allocation guard
measures zero warmed policy allocations for Basic/Motion, B8/B16/B32 and both
modes; whole-v2-workload performance qualification remains separate.
The rebuilt production/hooks addons pass all 13 required-native files (170 tests,
six existing skips), including actual sockets, checkpoint/archive and multiworker
continuations.

Stage F's Rust manifest and bounded offline whole-graph runner are implemented.
Compiler-derived named parameter views include interleaved affine bias strides,
exact recurrent gate/state layouts, resolved ports and ordered outputs. The
manifest records canonical sensor bytes/digest, separately versioned equations,
initializer semantics and deployment cadence/life/output rules. The shared
source-graph decoder now serves production admission and the offline tool.
The tool uses isolated Rust initialization or finite/count-validated canonical
weights, evaluates up to 256 delivery rows, and writes immutable Float32 outputs,
every next recurrent state and digest receipts. See
[`canonical-training-graph.md`](../canonical-training-graph.md) for its bounded
request and file contract. PyRL's canonical graph implementation is committed at
[`6e3bdd8`](https://github.com/Tits-McGruff/PyRL-trainer/commit/6e3bdd840389594db1387ca0d23eed76e5968dda)
on `canonical-graph-distillation`. It uses one flat Float32 parameter tensor and
the supplied views, exact custom recurrences and routing/output order. Strict
manifest bounds, equation revision/digest, sensor bytes/digest and complete
parameter/state ownership are checked before tensor construction. Method-v1
loss uses raw turn/boost outputs; deployed sigmoid/value heads are absent.
All 81 local tests and full Python lint pass, including the existing PPO suite.
Ten Rust reference sequences cover every operation and zero/nonzero state over
256 deliveries. Live actual-optimizer/export/Rust-admission/exact-byte-reimport
gates pass all six parameterized graph cases on CPU and RTX4080, each with both
native scalar and SSE2 backends. Maximum output/state error is 6.56e-7 on CPU and
5.29e-7 on CUDA; fixed tolerances were not widened. Torch 2.14.0+cu126, FP32 and
disabled TF32/autocast define this envelope. Compact source/dataset/environment
reports are retained in
[PyRL's canonical graph evidence](https://github.com/Tits-McGruff/PyRL-trainer/tree/6e3bdd840389594db1387ca0d23eed76e5968dda/docs/performance/canonical_graph);
its CI now builds the pinned native runner for a mandatory live CPU roundtrip.
PyRL's bounded sequence API is implemented at
[`59a8569`](https://github.com/Tits-McGruff/PyRL-trainer/commit/59a856904dc8ef041536af03ef6cc96b1c5ddcdb):
batch32, no-gradient burn-in32, TBPTT64, Adam3e-4 and clipping1.0. Explicit
generation/life/gap/capture-start boundaries apply Reset/Preserve semantics;
padding never advances recurrence and interior omissions reject. Continuations
detach at truncation and are identified as pre-update-weight training state.
The exact weighted objective and unweighted metrics exclude burn-in/padding.
Constant routing/output-independent graphs return metrics without inventing
parameters or optimizer updates. An independent gradient reference and
CPU/CUDA lifecycle/gap tests pass; the complete suite has 104 passing tests
and full Python lint. The mandatory live roundtrip now exercises the actual
32/32/64 trainer with 1,024 burn-in and 2,048 training samples per update.
Both native backends pass on CPU/RTX4080; maximum output/state error is 5.07e-7
and 5.96e-7 respectively. Sequence reports remain in PyRL's evidence directory.
Replay/sampling, committed transport and job integration remain open, as do
normal deployed-candidate acceptance and later stages. Stage G now has a bounded
Rust-owned teacher chunk v1 codec with exact Float32 bits, full stream/contract
identities, request/outcome separation, explicit boundary reasons and strict
chronology/size/finite validation. The
[`teacher-chunk-v1.md`](../teacher-chunk-v1.md) wire contract records its field
order and distinguishes cold life ends from post-step death labels. Seven
focused native tests cover exact bytes/signed zero, every truncated prefix,
trailing/reserved data, malformed dimensions and physical/capture boundaries.
Exact selected baseline-v2 rows now copy from the existing evaluated observation
into cold-reserved reusable buffers. Held intervals produce no row; preparing
capture does not resample or consume another marker. Control proposals compare
bit-for-bit with capture-disabled Basic/Motion and deterministic/stochastic
preparations. The running coordinator exposes them only after the matching
complete-step or acknowledged generation publication succeeds; failed/pending
transactions have no published boundary. A managed-checkpoint test confirms
equal full continuation hashes for capture on/off and one/three workers.
Complete physics now attaches actual post-step death and boost realized in any
collision substep, including boost exhausted before the final substep. A bounded
Rust stream assembler consumes published boundaries, including held-control
steps, retains exact rows in cold-reserved buffers, and seals on elapsed time,
physical death, generation end or capture stop. It assigns sample ordinals and
chunk sequences before the separate queue admits whole immutable objects;
overflow consumes sequences, records drops and labels the next fragment as a
gap. The managed-checkpoint test also compares encoded chunks across one/three
workers. Automatic capture now runs on ordinary, resolved-delivery and
acknowledged-generation publications. Admission derives sensor/teacher/cadence
identity from current native run settings. Cold deaths seal old physical streams;
live configuration changes stop the old epoch without dropping retained bytes.
The production handle exposes ordered arm/stop and a separate bounded opaque
drain. Its owner uses a nonblocking handoff and a coalesced diagnostic wake, so
drains can progress at a retained generation barrier without resolving it.
Native tests exercise a disconnected consumer, whole-chunk overflow, exact sensor
metadata, partial drains, shutdown tails and no additional authority commands.
The Node spool now keeps opaque objects and bounded SQLite metadata in a
dedicated diagnostic directory. Its worker validates whole handoffs before
publication, fsyncs bytes, publishes final names, then commits descriptor
visibility. Oldest unpinned retention, bounded expiring/renewable storage leases,
partial publication/retry recovery, interrupted epochs and coherent paginated
watermarks have direct and actual-worker coverage. The server opens this path
lazily through `/api/distillation/v1/`; metadata, leased immutable downloads and
ordered capture controls do not instantiate an actor or acquire a controller
lease. Real HTTP/native-server tests cover these boundaries. PyRL now decodes
Rust-encoded bytes through an independent bounded HTTP consumer, validates exact
manifest/descriptor identities, releases finite storage pins on success/failure,
and proposes local cursors with explicit settled gaps. Restart binds the native
run seed, full contract and exact canonical sensor bytes. The spool records seeds
and migrates older diagnostic metadata with an explicit unknown seed. Frozen
codec fixtures are synthetic; they do not prove committed-world provenance.
PyRL also has bounded whole-chunk replay, stable stream partitioning, independent
window sampling and an initial standalone DistillationJobV1 owner. Self-contained
checkpoints retain exact replay bytes with optimizer, private RNG, sampler/cursor,
window recurrence, source/environment and metrics. CPU/CUDA restart tests compare
the next actual optimizer update and private RNG draws. The runner supplies
periodic/final checkpoints and unweighted validation. Shared PyRL v4 metadata
now preserves complete structural identity and legacy v3 PPO compatibility.
Portable .slither-brain candidates freeze every 250 successful updates, with
four pending exports retained across job restart. Rust independently compiles
complete graph/packing/sensor/deployment manifests and validates raw or shuffled-
Zstandard weights. Bounded uploads reach that source-identified validator in the
diagnostic worker and return idempotent receipts without controller actions.
Native shadow inference now has four empty-by-default slots, immutable candidate
registration, ordered activation and activation-scoped stop commands. Each slot
uses the exact committed teacher vectors with private Rust recurrence and reusable
scratch. Reset/Preserve births, generation resets, unweighted strategy/life-stage/
rare-stratum errors and a 32-delivery pre-death window are bounded. Fixed summaries
use a separate nonblocking diagnostic mailbox; inference failures disable only
that slot. Managed-run regressions compare exact authority continuation and
teacher bytes with capture off/on and four shadows across one/three workers.
The native bridge and dedicated HTTP controls now connect explicit immutable
activation, activation-scoped stop and fixed metrics. Worker-side preparation
separates cache pressure from ordinary artifact validation/recovery. Receipts
have request/process/observed-slot binding, durable bounded chronology and exact
retry behavior; timeout keeps an admitted identity pending for its late outcome.
Actual admitted v4/v2 HTTP tests evaluate four shadows with one/three workers,
selecting a real continuing teacher stream. PyRL's explicit opt-in journals the
complete pending request with immutable bytes before activation, retries lost
replies after CPU/CUDA job restart, and binds online metrics to that deployment.
Saved-library/spawn integration and integrated resource/gameplay qualification
remain open. This checkpoint passes 710 release library tests with hooks,
two canonical-runner checks, three benchmark checks and the doctest, plus
all-target Clippy, rustfmt, TypeScript, repository ESLint and the client build.
The rebuilt source-identified addon and complete JavaScript aggregate pass
810 tests with 18 existing skips. PyRL passes 175 broad tests with two dedicated
native-tool skips, plus focused CPU/CUDA pending-request restart and saved-journal
integrity checks; repository-wide Pylint passes at 10/10.
Codec validation alone remains insufficient proof of a committed example.
The manifest/runner checkpoint passes 678 release library tests, two offline-tool
checks, the three benchmark checks and doctest, rustfmt and all-target Clippy.
The rebuilt production addon passes 14 fresh-run/source/surface tests and the
real-socket custom-graph reset/reconnect/resume regression. The focused metadata
set passes 27 tests with three existing platform skips; CI supplies the full
cross-platform native/JavaScript matrix.

The local PyRL environment now uses CUDA-enabled Torch `2.14.0+cu126` on the
RTX 4080. The twelve-case CUDA sequence optimizer/export/native scalar/SSE2/
reimport gate passes against the `d6c1def` release runner; its compact report is
retained in PyRL alongside the prior canonical evidence. The exact 60-second
checkpoint deadline uses direct deadline comparison, with the CI rounding case
covered by a deterministic regression.

The live distillation gate now consumes actual committed v4/v2 samples over the
production HTTP spool with the default 32-window/32-burn-in/64-BPTT sequence.
A second Python process restores update 125 and exactly reproduces update 126's
weights, optimizer, sampler, recurrence and private RNG witnesses. Update 250
freezes an immutable trained candidate for independent Rust validation and four
actual shadow slots. Windows CPU one/three-worker and CUDA one-worker cases pass;
compact source/environment/RSS/allocator/step/route reports are retained under
`docs/performance/distillation/`. No actor/controller traffic is admitted, and
world steps continue through training and activation. PyRL CI pins the Slither
host and runs this gate on Linux CPU with one/three workers. Unchanged native
capture/four-shadow continuation tests remain the mathematical isolation
witnesses; these live runs add committed provenance and cross-process handoff.
At that checkpoint, CUDA three-worker qualification, larger-workload resource/VRAM
measurements, held-out tactical/ablation evaluation and final integrated soak
remained open.
The extended live gate also trains a compatible Dense student to update 250 with
the same exact process-restart check. It saves both trained architectures,
rejects GRU deployment in the Dense world, exports the compatible original
byte-for-byte and applies an explicit matching neural cadence in the disposable
fixture. Native deployment and immutable receipt retry then succeed. Deleting
the catalog record reclaims its cold original/weight files while the live snake
continues actual Rust inference and retains its distilled origin. Windows CPU
one/three-worker reports cover this path under `docs/performance/distillation/`.
The preceding v1 Linux live gate passed both worker counts in PyRL CI; the
extended library gate is pinned for subsequent CI runs. Other Stage I resource,
capacity and final acceptance obligations remain open.



Stage I's native origin foundation is implemented: retained brains own bounded
immutable HoF or saved-brain origins through shared Arcs. Admission binds origin
to exact packed weights, population/external owners reject retained provenance,
and running continuation plus opaque replacement proofs bind the complete origin.
Custom copies share the Arc and memory admission reserves its allocation overhead.
HoF commands now carry the actual leased entry ID into Rust; a common native
retained spawn initializes zero recurrence and the pending-first-action sentinel.
Catalog deletion cannot revoke live values. The origin checkpoint passes all
713 release library tests with hooks, including immutable-copy and publication
proof regressions, plus all-target Clippy. Its complete JavaScript set passes
810 cases with 18 existing skips, including the focused built-client recheck.
Retained spawn now reuses the oldest eligible dead retained owner at full record
capacity, reserves fresh physical/frame/brain IDs, removes the displaced body
range and clears old pellet ownership. Cold rollback scratch is admitted before
mutation; rejection restores body ranges, weights/origin, owners, allocators and
world RNG. The replacement receives its first real control delivery immediately.
All 715 release library tests and all-target Clippy pass for the reuse slice.
The saved-brain storage core now shares `ManagedNeuralWeightsDescriptor` and
immutable objects beneath HoF, preserving older raw/compressed bytes when today's
codec would choose differently. Rust publishes the original portable bytes and
canonical graph/sensor/deployment/provenance metadata only after source and quota
admission. The isolated persistence worker provides explicit save/read/list/delete
and bounded export/spawn leases, with 100-record/2-GiB quotas and the existing disk
reserve. Metadata digests and scalar bindings disable GC on corrupt references;
shared GC counts both catalog and HoF consumers and retains deleted-but-leased
objects. A production-addon/real-worker test covers restart, exact original bytes,
both reference directions, lease deletion and interrupted original publication.
All 719 release library tests, Clippy, type/lint/client build and focused storage
checks pass. Candidates still do not auto-save.
Saved records now have a separate coarse native deployment command. Rust checks
exact run/configuration/world incarnation, recompiles the portable source and
requires the active graph, structural sensors and neural cadence. Shared weight
shape/size admission precedes decoding. Deployment uses the common atomic retained
spawn and owns its origin/weights after catalog deletion. Regressions cover equal
width Basic B20/Motion B8 sensor rejection, graph/cadence/stale-scope rejection,
corrupt or oversized shared objects, and actual one/three-worker addon deployment.
The deployment checkpoint passes all 722 release library tests, all-target Clippy,
type/lint/client build and 820 JavaScript tests (18 existing skips).
HTTP library controls now provide explicit candidate save, standalone portable
import/export, paginated metadata, deletion and scoped native deployment. Native
cold admission validates managed originals before export or an exact save retry.
Timeouts retain pending command leases; process identity, a deployment cursor and
bounded receipts prevent duplicate spawns after lost replies or receipt eviction.
Archive/world replacement gates and shutdown preserve live file consumers.
Actual one/three-worker HTTP tests cover byte-identical export/import, explicit
save, scope/retry controls, same-size corruption and continued live simulation.
The HTTP checkpoint passes type/lint/client build and all 826 JavaScript tests
(18 existing skips); Rust validation reuses the unchanged native checkpoint.
The capability-gated browser library now supports bounded pagination, explicit
candidate save and original-File import, direct export, delete, provenance details
and scoped spawn. It compares applied graph descriptions/sensor/cadence facts separately from
editor drafts, reports source-configuration differences and never coerces settings.
Graph description comparisons are presentation only; the editor's legacy key is
never compared to Rust's canonical compiled key. Rust owns exact admission.
Uncertain replies retain their exact request body; known rejections clear it.
Connection revisions and fresh native scope prevent stale confirmations from
following a snake in another world. Metadata is rendered as text and only user
actions fetch catalog/details; no animation-frame polling was added. Browser
verification uses an isolated native/Vite instance and synthetic portable fixture.
The UI slice passes 401 unit cases (three existing skips), 67 component cases,
22 main UI smoke cases and two actual one/three-worker HTTP cases, plus type,
lint and client build. The preceding full HTTP/native evidence remains applicable
to unchanged engine and persistence paths.
Archive selection now has an atomic private batch lease for up to 100 explicitly
selected originals. The lease returns at most 256 KiB of compact metadata, shares
the existing 16-lease admission and pins deleted originals/shared weight objects
through export preparation. Native source validation precedes all-or-nothing
selection; worker replies bind the exact requested identities. Tests cover a full
100-record selection at the physical-byte quota, partial failure, retries and
heterogeneous real native sources overlapping an ordinary single-brain lease.
The batch slice passes 97 focused persistence/native cases (one existing skip),
401 unit cases (three existing skips), 67 component cases, type/lint and client
build. The archive roles, opt-in selection/export/import wiring and integrated
qualification remain open; ordinary archive versions/default exports are unchanged.
The optional association implementation now adds archive v4 with a separately
versioned 1..3 record inventory and up to 100 exact portable originals. Its root
binds the selected membership metadata as well as original bytes. Native readers
validate each heterogeneous graph/sensor/weight contract and preserve originals;
SQLite restores library membership with the checkpoint transaction, retaining
existing names on retry and enforcing the existing quota. Browser selection is
explicit, capability gated and cleared on connection changes; default exports
retain their existing format. Native mixed-version roundtrip and tampered-label
rejection pass, together with all 679 release library tests. Actual production
HTTP verifies original-byte export, unchanged default exports, heterogeneous
membership restoration and complete library/world rollback when the second
catalog insertion fails, followed by a successful retry.
The checkpoint also passes 402 unit cases (three existing skips), 68 component
cases, 96 focused persistence/client cases (one existing skip), type/lint/client
build and rustfmt/Clippy. Integrated Stage I and later qualification remain open.

Selected live provenance is now additive metadata on the existing native identity
response. Rust copies only fixed-width origin values, without a catalog lookup,
RNG draw or Arc ownership change. Stats distinguishes HoF entries from saved
models and shows exact artifact/weight identities and source class; disconnect
clears the display. Source deletion does not erase live provenance. Focused native
origin/storage/selection checks and browser selection/clear tests pass. Actual
production WebSocket selection verifies the exact retained HoF identity without
changing durable metadata. This slice passes 43 focused native origin/storage/
selection/queue/bridge cases, 402 unit cases (three existing skips), 70 component
cases, 23 main UI cases, type/lint/client build and rustfmt/Clippy. The existing
one/three-worker deployment fixture additionally checks saved provenance after
catalog deletion through the native bridge in CI.

The selected inspector now has an opt-in native delivery cache. A single validated
physical target owns a fixed-size pending/published pair, admitted within the
existing worker scratch ceiling. Neural, baseline-v1/v2 and external rows retain
their actual sampler layout and boundary scalars/diagnostics. Whole-step publication
and exact accepted socket receipts gate visibility; held controls preserve the last
row and its age. Tests cover late failure, mixed receipts, equal-width profiles,
B32 Motion, closed capture, unchanged native continuation and scratch rejection.
The cache now also copies actual pre-normalization scalar, food, sector, wall,
body-ray and motion values from that same sample. Incomplete rays retain an
explicit unknown result. One exclusive fixed buffer follows the selected neural
owner into its worker partition; closed capture performs no diagnostic query or
allocation. Frozen v3 vectors, actual one/two/three/four-worker v4 observations,
quota use, warm capacities and held sample ages remain unchanged. Transport,
probes, lifecycle/shadow joins and browser inspection remain open; no inspector
capability or large debug stream is advertised yet. This raw-trace slice passes
all 731 release library tests with engine hooks, 26 focused sensor cases, 45
control cases, eight library/runtime cases, three inspector cases, rustfmt and
Clippy. The preceding delivery-cache checkpoint passes the complete Windows/Linux
and Node 24/26 CI matrix; unchanged client/server code reuses that evidence.
The next native transport slice adds an ordered exact-token subscription command,
a separate replaceable mailbox limited to 10 Hz, a sequenced clear on close, and
a bounded Rust-authored JSON read. Its additional copy/encoding reserve is charged
to the existing worker ceiling. Full-width source IDs and age chronology remain
exact strings; raw graph outputs are separate from clipped motor actions. A
strict presentation decoder checks profile widths, source/action combinations,
finite scalars, exact bin arrays and explicit unknown rays. Actual production
addon tests exercise select/enable/read/stale-run rejection/close through the
command queue. All 731 release library tests with hooks pass for that transport
checkpoint, together with focused display/queue/control tests, type/lint and the
client build. Rows and their clear messages additionally retain the exact queue
sequence of the subscription that admitted them, so an old mailbox row cannot
be confused with a newly acknowledged target. Close/reopen and rejected-subscribe
regressions exercise that identity through the real addon. WebSocket routing now
admits one joined UI owner and one pending command globally, isolates diagnostic
failures, filters old subscriptions and retains cleanup across disconnect or
scope replacement. Each socket keeps one replaceable diagnostic row behind
reliable replies, stats and frames. Actual sockets verify exclusive ownership,
native rows, disconnect/reclaim and explicit close. The capability-gated Stats
inspector is now opt-in and default-off. Canvas or keyboard slot selection uses
the existing native identity resolver; pending lives retain their individual.
The display shows exact step/age/source identity, raw scalars/sectors/rays/motion,
actual graph/v2 action values and the normalized vector. Missing profile channels
remain labelled not-sampled. Browser tests cover scope changes, stale rows,
coalesced opt-out and clearing; a disposable real browser run verifies delivered
data, drawing and opt-out. The inspector now attaches exact pre-movement evolved
life counters, current/best tuples, ledger attribution and selected assessment.
Provisional fitness reuses the existing native formula and population maximum;
the browser only presents its seven components and native total. Actual frozen-v1
actions include effective strategy and retained timers. Four shadow outputs join
only the matching committed teacher row, with full candidate/capture/activation
identity; held samples preserve their original values. Respawn/wallet modes,
continuation with capture and four shadows, stale shadow rejection, bounded wire
decoding and UI clearing are covered by regression tests. Explicit one-shot probes
now use the same immutable pre-movement indexes, with an independent meter bounded
by the admitted per-observation cap. They execute no controller/model, clear no
delivery/first flag, advance no authoritative RNG or fair quota cursor, and keep
their original step/age until another request. Successful whole-step publication
gates visibility; failed attempts cannot expose a probe. The default delivered
view remains unchanged, and closed or completed one-shot capture performs no more
debug sampling. Native held-control continuation checks cover Basic/Motion through
B32, including recurrent state and full RNG/quota witnesses. The browser labels
probe rows and supports Probe once / Show delivered with bounded pending intent.
Integrated capacity qualification remains open. The pure-probe checkpoint passes
all 734 release library tests with engine hooks, actual addon and socket probes,
413 unit cases (three existing skips), 88 component cases, 24 main UI cases,
type/lint/client build and rustfmt/Clippy. The preceding lifecycle/action/shadow
checkpoint passes all 732 release library tests with engine hooks, actual addon
and socket inspector fixtures, 411 unit cases (three existing skips), 85 component
cases, 24 main UI cases, type/lint/client build and rustfmt/Clippy.

The live distillation gate now trains GRU and Dense candidates for 250 updates
with an independent-process restart at update 125, validates immutable artifacts,
activates native shadows, saves/exports/deploys compatible Dense weights and
retains native inference/provenance after catalog deletion. Linux CPU CI passes
with one/three native workers; Windows CPU reports retain the same path. CUDA
one-worker training/restart/shadow evidence passes. The extended three-worker
CUDA gate now also passes both 250-update students, exact next-update restarts,
four native shadows and the compatible library/deletion/retained-inference path.
Integrated larger-workload resource qualification remains open. Reports and their exact scope
live under `docs/performance/distillation/`.

The sensor workload runner accepts explicit v2 mode/cadence and records committed
neural/baseline/external work separately, with shared ray-budget witnesses and
OS peak RSS. Observer-on/off and one/three-worker complete continuation witnesses
match across v1 and both v2 modes; all 734 release library tests pass. Retained
version-2 Windows/Oxygen measurements are described in
`docs/performance/sensor-v4/README.md`. These random-population workloads do not
qualify representative tactical batches, stochastic exploration or ablation.

A native tactical runner now drives fourteen seeded paired geometries through
actual prefix/control/three-substep physics, recording survival, food, kills,
boost spend, collision causes, first-action v1 comparison and full continuation
witnesses. Windows and Oxygen release batches contain 1,120 trials per worker
count and match exactly at one/three workers within each platform. They exercise
stochastic behavior RNG and verify zero v2 action-selection world reads. The runner exposed a Motion crossing
error: projected relative contact in a different sector did not veto holding the
colliding forward velocity. Selection now applies a front-contact escape veto
and lateral pressure using declared observations only; rear boost escape remains
covered. All 738 release library tests pass. Reports and remaining wall/crowd/
food-trap/stochastic-crossing limitations are explicit under
`docs/performance/baseline-v2/`; this is not final teacher qualification. CI runs
and retains a bounded Windows/Ubuntu tactical matrix.

Avoidance now retains stronger steering until heading alignment, preserves a
reachable escape side and suppresses exploration during the manoeuvre. Bounded
actual-delivery decision tails exposed the earlier relaxation/reversal failures;
odd-bin layouts also check both forward neighbours. All 256 native wall/food-trap
regressions pass across 16 seeds, both profiles/modes and 60 Hz/0.06-second
cadences. Windows/Oxygen release matrices agree across one/three workers at
60 Hz and 0.06 seconds; frozen-v1 cohorts match the previous platform records. All 740 release
library tests and 16 actual-addon identity/runtime/capture/shadow tests pass.
Version-2 reports retain compressed complete JSON and diagnostic tails under
`docs/performance/baseline-v2/`.

Basic v2 now permits economic escape boost away from close declared body/head
geometry; it does not infer unavailable target size, velocity or urgency. Walls
alone do not supply an escape reason. Both profiles require half a second of
clearance along the current forward path as well as the selected route, with
existing eligibility, wallet and saturation checks. All 744 native release
library tests pass, including 128 actual boosted-rear-escape trajectories across
16 seeds, both profiles/modes and 60 Hz/0.06-second cadences. Four retained
Windows batches contain 4,480 complete trials with one/three-worker agreement,
unchanged frozen-v1 records and no cohort survival regression. Motion crowd
opening reaches 16/16 at 0.06 seconds in both modes; it remains 12/16 at 60 Hz.
Rebuilt addons pass 16 identity/runtime/capture/shadow tests; rustfmt and both
Clippy configurations pass. Crowded/late contacts, scarce ray budgets and representative quality evaluation
remain open.

The version-3 native workload runner now measures ordinary teacher capture,
exact source/drained sample witnesses, per-slot rates, due-teacher activity,
queue peaks/drops and settled ownership under slow draining. Windows P0/P1
20-second batches preserve authority and observations across capture off/on and
one/three workers; normal capture loses no rows and count-limited slow capture
drops only complete chunks with reconciled accounting. All 746 release library
tests and 16 rebuilt-addon tests pass. The actual rates are 422.2/204.45 rows per
simulated second; full-stack throughput remains unqualified. P1's 24.998 ms
three-worker p95 exceeds one fixed-step interval, and the 4.103% pooled
incomplete rate fails the quality gate. Actual 1x scheduling remains unqualified.
Matching Oxygen batches also preserve exact native capture/authority isolation and retain
verified reports. P1 records 178.75 teacher rows per simulated second, 4.093%
pooled incompleteness and 150.829 ms three-worker p95 on the loaded target;
this is no timing pass. Full reports and their scope live under
`docs/performance/sensor-v4/`; no limits/defaults are raised. Spool/trainer resources and the integrated soak remain.

Version-4 workload reports now partition existing native timers without adding
hot-path timing calls. The matching P1 Windows/Oxygen baselines attribute about
66% of service time to sensing. A shared inflated-bounds ray-packet experiment
preserves exact observations, counters and authority continuations in adjacent
ABBA comparisons, but provides no consistent target-platform timing benefit;
the implementation is reverted. The comparison utility and eight fingerprinted
reports remain under `scripts/qualification/` and
`docs/performance/sensor-v4/` for reproducibility. The final runtime retains
only the bounded qualification attribution, with no admission/default changes.

The browser now has an explicit Distillation tab for teacher capture start/stop,
bounded dataset discovery and watermarks/drop diagnostics, four candidate
activations/stops and native action-error strata. Applied welcome capabilities
control admission; connection/run/settings changes invalidate old presentation.
Uncertain shadow replies retain an immutable retry body; server-assigned capture
starts are never automatically replayed. Wire dataset types and pure shadow
metadata validators now live under `src/protocol/`, shared with Node. The
browser routes only small metadata/commands and executes no graph. Tests cover
strict metadata bounds, exact identities, stale responses, retry ownership,
text-safe rendering and real HTTP manifest compatibility. The broad JavaScript
run plus its corrected startup-DOM fixture rerun passes 898 tests with 18 existing
skips; 16 rebuilt-addon tests, type/lint/client build, rustfmt and Clippy pass.
Native runtime validation reuses the unchanged source's 747-test release run.
Real-browser inspection uses synthetic metadata, separately from native/API
evidence; it does not qualify sustained capture, training or a new sensor default.

V4 now shares validated head facts across one pre-movement observation boundary.
The cold buffer is included in staging admission and warm capacity diagnostics;
both 512 MiB scratch ceilings remain unchanged. Exact cached/reference vectors,
receipts, traces and work counters match across both profiles, every B8..32 and
multiple grants. All 750 native release library tests and 183 required-native
checks pass, with six existing skips in that overlay; rustfmt and both Clippy
configurations pass. Twenty retained
Windows/Oxygen ABBA reports preserve complete continuations and sample witnesses,
including Oxygen P1 across a fixed-affinity follow-up. Windows P1 mean service
improves about 8%; fixed-affinity Oxygen P1 improves about 6.3%, while the
unrestricted Oxygen result remains noisy. P1 remains above the fixed-step timing
reference and fails the pooled ray-quality gate; actual 1x scheduling is open.
Source commits, full measurements and scope are in
`docs/performance/sensor-v4/README.md`.

PyRL commit `34f1e33` adds `python -m pyrl_trainer --mode distillation`, delegating
the remaining arguments to the existing canonical job owner. Default/explicit
PPO startup remains available. Owner-specific imports are lazy: Distillation
does not load PPO settings or import its actor/learner modules. A fresh-process
test rejects malformed PPO environment leakage; exact argument forwarding,
owner-specific help and default PPO routing are covered. All 184 trainer tests
pass with two existing optional-fixture skips, and full Pylint passes. Canonical
training equations, optimizer/checkpoint ownership and the standalone runner
are unchanged.

Native capture now has a measured byte-pressure cohort using ten ordinary v2
teachers at 60 Hz, the admitted 0.5-second respawn minimum and Motion B32. Windows
and Oxygen fast drains preserve every source/drained row with one/three workers;
capture off/on and byte pressure preserve complete authority continuations.
Their 16 MiB queues reach the encoded byte allowance with only 103 of 8,192
chunk slots occupied, dropping 89/90 whole chunks with exact row/release
accounting. The aggregate rates are 589.5/592.5 rows per simulated second;
ten-teacher intervals reach 600, but ordinary death gaps preclude a sustained
600-row claim. Eight retained reports and the bounded reproducible cohort driver
are under `docs/performance/sensor-v4/` and `scripts/qualification/`. Node 24 CI
runs the cohort on both supported platforms. All 751 Windows release library
tests and 183 rebuilt-addon required-native checks pass, with six existing skips
in that overlay; rustfmt and both Clippy configurations pass. Native byte-pressure behavior
is qualified for this cohort; spool/trainer resources and the integrated soak
remain open. Both scratch ceilings remain 512 MiB and production teacher delay
remains 20 seconds.

PyRL commit `61d0e00` exposes the section-18 new-job controls for seed, CPU
threads, device/replay budgets, learning rate and sequence windows through the
canonical runner. Public ranges reject before file/network access; fresh CLI
seed is 1, while library defaults and exact resumed choices retain their meanings.
CPU counts respect process affinity, including an actual one-CPU Windows child
under an overridden Python CPU count. Explicit CUDA admission checks availability
and physical capacity; the 8 GiB ceiling need not be entirely free if the admitted
estimate fits. Batch 256 and both 256-delivery window endpoints train on CPU/CUDA
and checkpoint correctly. Replay quotas span 64..2048 MiB with a 256 MiB default
and no eager capacity allocation. Streamed checkpoint disk limits derive from the
recorded replay quota and bounded roles, with unchanged directory/object caps
and disk reserve; ZIP64 and pre-read raw replay admission have regressions.
All 230 trainer tests pass with two existing optional-fixture skips; full Pylint
passes. Loaded maximum-quota RAM/VRAM and integrated-soak qualification remain
open. Native scratch ceilings stay at 512 MiB.

V4 shares stack-only centered-sector boundaries across dense food, body, head
and predicted-contact queries, retaining original normalization at ambiguous
boundaries, zero/subnormal norms and large headings. Sparse queries keep the
original path. Native reference regressions cover both profiles, every B8..32,
raw accumulations/minima/IDs/urgency, extreme vectors and squared-distance
underflow; frozen v3 meanings and all five identity-fixture bytes are preserved.
All 754 Windows release library tests and 183 rebuilt-addon required-native checks
pass, with six existing skips in that overlay; rustfmt and both Clippy configurations
pass. Sixteen retained Windows/Oxygen ABBA reports preserve exact continuation,
consumer work, raw/normalized histograms and captured row witnesses. Windows P1
mean/CPU fall about 17.5%/21.4%; loaded Oxygen P1 mean/CPU fall 13.1%/17.1%.
These descriptive gains still leave P1 above the fixed-step p95 reference and
around 4.1% pooled ray incompleteness. Measurements and reproduction scope are in
`docs/performance/sensor-v4/README.md`. No production defaults or resource ceilings
change; unchanged capture transport reuses the preceding byte-pressure evidence.

The v4 body-ray index now uses bounded eight-bin spatial splits with a median
fallback, retaining unique segment ownership, the existing node-capacity bound,
all charged work units and fair quotas. Complete geometry/exhaustion regressions
and 771 layout-bound checks pass; Windows passes 701 default-feature Rust tests
and 183 rebuilt-addon required-native checks (six existing skips), rustfmt and
both Clippy configurations. Twelve retained probes on each supported platform reduce P1 Motion B16
pooled incompleteness from 4.103% / 4.093% to 0.202% / 0.048% on Windows /
Debian at unchanged cap 256/batch 65,536.
Basic/Motion B8 and B16 pass this seeded quality probe; both B32 profiles and
smaller-budget probes fail. One/three-worker continuation, consumer work,
raw/normalized samples and captured rows agree exactly. Actual delivered counts
remain explicit when a boundary temporarily has fewer due snakes. P1 Motion p95,
representative/tactical/ablation and full-stack gates remain open. Measurements
are in the existing sensor-v4 performance report. Both scratch ceilings remain
512 MiB; ordinary v3/v1 defaults remain unchanged.

The Node dataset-disk ceiling is now configurable through
`--distillation-spool-gib`, `DISTILLATION_SPOOL_GIB` or TOML
`distillationSpoolGiB`, with exact integer admission 1..64 and unchanged 4 GiB
default. Startup validates before owner-database access; existing configuration
files are preserved. Smaller restart quotas rotate oldest unpinned objects,
count overlapping leases once and refuse impossible pin sets before eviction.
Object/lease/download/reserve bounds remain unchanged, with an additive lease
lookup index for bounded catalog work. All 918 JavaScript tests and 184
required-native checks pass (18/six existing skips), plus type/lint/client build;
unchanged native source reuses the preceding Rust evidence. This exposes the
producer's global disk ceiling; loaded spool/resource qualification and the
final settings-surface audit remain open.
Native scratch ceilings remain 512 MiB.

Read-only `GET /api/distillation/v1/storage` now exposes the actual admitted
producer policy. Canonical trainer checkpoints bind its exact byte/count/lease
ceilings before consuming rows; policy changes require a new job, offline resume
preserves saved admission, and legacy 404 providers remain explicitly unreported.
The paired trainer passes 251 tests (two existing skips) and full Pylint; 34
focused spool/HTTP tests and host type/lint checks pass. Retained Windows CPU
one-worker/4-GiB and CUDA three-worker/64-GiB live reports prove both students'
250 updates, fresh-process exact restart, four shadows and saved-library handoff.
Reports reside under `docs/performance/distillation/`. This closes trainer quota
provenance; maximum-capacity storage and integrated resource gates remain open.

A retained Windows P1 sensing CPU profile identifies pellet collection/sorting
and angular lookup as the largest measured targets. A prepartitioned angular
lookup passes exact comparison but shows no whole-workload speed gain and is
reverted; its compact ABBA evidence and patch remain in the existing sensor-v4
performance directory. The comparison driver now checks raw histograms explicitly.

Spatial body/pellet candidates now cache the reversible IEEE total-order key
once per distance, preserving original distance bits, ties, candidate sizes and
admission. Sixteen retained Windows P0/P1 ABBA reports preserve complete
continuations, raw/normalized samples, consumer work and capture across builds
and P1 worker counts 1/3/5. CPU medians fall about 1..4%; wall timing is mixed,
including a three-worker P1 regression. These descriptive probes do not close
the actual P0/P1 1x throughput/debt gate. All 702 release library tests and 184
rebuilt-addon required-native checks pass (six existing skips), plus rustfmt and
both Clippy configurations. Production defaults remain unchanged.

Stages D through J and remaining B/C acceptance gates remain open: representative
sensor/tactical/ablation evaluation, sustained teacher throughput, larger workload
and device-memory admission, crowded/saturated geometry, and the integrated
six-hour-equivalent soak. Inspector and library implementation are present; their
capacity qualification and the final section-18 settings-surface audit remain
open. The main entry point selects the independent distillation runner and its
bounded job controls. Existing v3/v1 runtime defaults remain until their stages
qualify.

Repository inspection baseline:

- `slither_neuroevo/main`: `fa0cb3e934a81feeb57edfda95c1f6b01cbd8f62`.
- `PyRL-trainer`: `7240da036df6f4cfb606b1f20d6dc1c3561399a2`.
- Slither's local worktree was clean before this document was added.
- `AGENTS.md` is the architectural reference. Historical TypeScript execution is not a design authority except where current Rust deliberately retains frozen provenance fixtures.

The behavioral contracts below are settled. Ray-work values, the final pre-release v4 layout following ablation, checkpoint admission, and resource defaults explicitly marked for qualification are evidence gates. They must be resolved through the specified measurements before release, not silently chosen during implementation. There are no remaining unselected product alternatives in the initial scope.

## 1. Architectural boundaries and current systems

Rust owns the sole authoritative simulation, fixed-step scheduling, sensors, heterogeneous graph inference, recurrent state, movement, food, collisions, controllers, evolution, RNG/allocators, checkpoints and frame packing. Node remains the HTTP/WebSocket/static-file/archive-byte/SQLite-metadata layer. The browser remains presentation, control and configuration drafts.

Do not restore the retired TypeScript world/sensors/neural execution, Node inference pool, browser fallback, second authoritative loop or per-layer native bridge calls.

### State and transaction ownership

The inspected production path has these concrete foundations:

| System | Current references | Implication |
|---|---|---|
| Population | `state.rs::PopulationGenome`, `GenomeLineage`, `BrainHandle` | Genome, weights and population slot are independent of physical lives. |
| Neural ownership | `BrainRuntimeState`, `BrainOwner::PopulationSlot` / `Entity` | Population recurrent state can survive physical replacement without changing its owner. |
| World | `SnakeState`, `WorldState`, `BodyRange`, `PelletState` | Physical IDs, frame IDs, array indexes and population slots are distinct. Bodies are pooled. |
| Running authority | `running_loop.rs::RunningAuthorityLoop`, `running_step.rs::RunningStepCoordinator` | Extend retained keyed transactions and existing delivery/persistence barriers. |
| Prefix | `fixed_step.rs::FixedStepPrefixWorkspace`, `accounting.rs` | Alive awards, ambient spawning and baseline replacement precede control. |
| Controls | `control_phase.rs`, `control.rs::NeuralControlPipeline`, `calculation.rs` | One stable pre-movement view, due cadence, packed observations and staged output. |
| Inference | `inference.rs::GraphExecutionPlan` and reset-aware heterogeneous evaluation | Reuse canonical execution and transactional recurrent publication. |
| Physics | `movement.rs`, `food.rs`, `collision.rs`, `effects.rs`, `physics.rs`, `world_step.rs` | Complete collision substeps never partially publish authority. |
| Evolution | `evolution.rs::calculate_fitness`, `generation.rs`, `generation_start.rs` | Current fitness consumes terminal physical state; multi-life assessment needs an explicit replacement input. |
| RNG | `rng.rs`, `RngStateBundle` | World, evolution, external and baseline streams are independent and versioned. |
| Baseline lifecycle | `baseline.rs`, baseline replacement in `fixed_step.rs` | In-place replacement, new IDs, one newborn award, owner cleanup and batch compaction already exist. |
| External lifecycle | `external_replacement.rs`, `controllers.rs`, `server/rustEngine/externalRouting.ts` | Preserve leases, tokens, assignment receipts, takeover and generation reassignment. |
| Retained brains | `resurrection.rs::resurrect_hall_of_fame`, `checkpoint.rs` | Generalize managed weights and spawn machinery, not neural execution. |
| Display | `frame_v1.rs`, `display.rs`, `src/protocol/frame.ts`, `src/render.ts` | Keep compact frame-v1 and on-demand diagnostics. |
| Settings/graphs | `src/protocol/settingDefinitions.ts`, `settings.ts`, `sensors.ts`, `src/brains/graph/*`, `stackBuilder.ts`, `parameterCounts.ts` | TypeScript remains schema/compiler/editor/presentation tooling only. |
| Persistence | `checkpointPersistenceWorker.ts`, generation handoffs, `export_archive.rs` | Preserve immutable files, transactional pointers and bounded archive readers. |

The existing joined step is: ordered command service -> prefix accounting/spawn -> shared observation/control -> movement/food/collision/effects -> generation guard -> either nonterminal external replacement/delivery/publication or evolution/checkpoint/metadata/successor transaction.

Respawn belongs at the established pre-control newborn boundary. Life-end accounting belongs in the physical transaction. Manual finish belongs in the existing post-physics generation guard.

### Current limits and contracts

- Evolved population: at most 300; baseline slots: at most 120.
- Production world ceiling includes additional external/retained entities; it is not just population plus baselines.
- Body-point storage: 1,000,000; pellet storage: 250,000.
- Graph tensor width: 512; packed parameters: 1,000,000 per graph.
- Production checkpoint envelope: currently 512 MiB, with separate workspace/process/disk admission.
- Protocol 2 and frame-v1 remain. Frame-v1 has seven header floats, eight scalars plus XY body pairs per alive snake, then pellet count and five floats per pellet.
- Current sensors: only v3, `19 + 4 * bins`, bins 8..32.
- Current generation duration: 8..480 seconds; default 240.
- Checkpoint-v3 is pre-spawn generation-boundary population durability with zero recurrent state, not a running-world snapshot.
- Exact replay is scoped to matching contracts/build/environment, completed steps and ordered actions. Cross-platform/build long-horizon bit identity is not promised.

## 2. Sandbox principles and defaults

Engine correctness means deterministic/versioned transitions, coherent ownership, explicit RNG, transactional failure, bounded memory/work, valid persistence and compatibility. It does not mean removing exploitable incentives.

Cheap death, corpse farming, accumulated resources, repeated best-life attempts, cross-life memory, stochastic teachers and imperfect sensing are selectable experiments or observable consequences.

### Three default layers

| Situation | Rule |
|---|---|
| Recorded checkpoint/import | Preserve recorded contracts/settings; use only explicit provenance-labelled compatibility handling. |
| Reset/New Run from selected configuration | Preserve that configuration. Reset repeats seed; New Run selects a new seed. Neither silently changes sensor/teacher. |
| New configuration using current defaults | v4 Motion, 16 bins, baseline v2 stochastic, respawn off, 240 seconds. |

Restore current defaults explicitly selects the new defaults. Browser drafts never override authoritative welcome values. Label v3/v1 as compatibility/frozen while keeping them easy to select.

Revise authoritative defaults in `fresh_run.rs`, `step_config.rs` and graph construction. Introduce `production_default_settings()` and a sensor-aware production default graph constructor. Preserve separately named frozen v3 fixture/compatibility constructors; do not rewrite historical P0/P1/P2/P3 expectations. Update browser `CFG_DEFAULT`, input synchronization and server stack presentation together.

Confirmed initial choices:

- Selectable v3 and v4.
- Reset/Cumulative/Best Life point assessment.
- Cumulative Net/Gross earned, Net default.
- Cumulative Life-local/Carried wallet, Life-local default.
- Non-point fitness aggregation independently selectable, Generation totals default with respawn.
- Per-slot/World respawn RNG, Per-slot default.
- Recurrent Reset/Preserve, Reset default.
- Normal/None/Subtract replacement mass corpse rules.
- Fail/Discard new excess pellet overflow; explicit respawn preset selects Discard.
- Separate PyRL/PyTorch CUDA trainer using the canonical graph.
- Founders, DAgger and exact mid-generation recovery are future stages.

## 3. Stage A: duration parsing and manual completion

### Flexible duration

Retain authoritative `generationSeconds` as numeric seconds. Replace its browser slider with a text duration control and shared pure parser/formatter.

Grammar:

- Plain unsigned decimal integer: seconds.
- One to three unsigned integer terms using `h`, `m`, `s`, case-insensitive; any term order, no duplicate unit.
- Terms separated by spaces/tabs, a single comma with optional surrounding spaces, or mixtures. Optional spaces between number and unit.
- Two-field colon: `M:SS`, first field one or more digits, seconds exactly two digits in 00..59.
- Three-field colon: `H:MM:SS`, hours one or more digits, minutes/seconds exactly two digits in 00..59.
- Maximum input length 128 characters; checked accumulation.

Examples: `240`, `1h`, `2m`, `30s`, `1h 2m`, `2m 3s`, `1h 3s`, `1h 2m 3s`, `1H, 2m 3s`, `2 m`, `2m,3s`, `1:02`, `1:02:03`.

Reject fractions in this first text grammar, negatives/signs, exponent/nonfinite notation, arithmetic expressions, garbage, empty input, duplicate units, malformed colon fields, doubled/trailing commas and overflow. Invalid drafts block Apply and reset rather than silently clamping.

Range: **8..604,800 seconds**, default 240. Seven days is an operational contract, not a numerical cliff. It represents 36,288,000 nominal 60 Hz steps. Existing numeric checkpoint values retain their recorded interpretation.

Update settings definitions/coercion/UI/main, `step_config.rs`, `SensorConfig::validate`, hour-scale timers and `scripts/stage6/runtime-integration-probe.ts`, which also hard-codes 480. Preserve existing fixed-step duration chronology rather than incidentally replacing elapsed-time comparisons.

### Clock qualification

A read-only V8 Float64 probe performed repeated addition of `1/60`:

| Nominal duration | Steps | Accumulated error, seconds |
|---|---:|---:|
| 4 minutes | 14,400 | +0.0000000000831335 |
| 1 hour | 216,000 | +0.0000000182276 |
| 6 hours | 1,296,000 | -0.000000230171 |
| 24 hours | 5,184,000 | +0.00000142524 |
| 7 days | 36,288,000 | +0.000161911 |

This is arithmetic evidence, not a native authority acceptance run. Native qualification must compare accumulated clock with integer generation-step-derived time, measure maximum error and test actual duration completion near the boundary.

### Finish generation now

Add joined-UI `finishGeneration { requestId, expectedRunId, expectedGeneration }`, routed through Protocol 2 validation, shared command admission and the coarse Rust queue.

- Retain one idempotent generation-scoped request.
- Finish the next eligible complete authoritative step normally.
- Evaluate after physics, effects and life accounting.
- Use `GenerationTransitionReason::Manual` and the ordinary evolution/HoF/checkpoint/commit/successor path.
- Do not bypass pending external receipts or persistence.
- Reject stale requests and prevent successor-generation finish.
- Precedence: Manual -> Duration -> early-end rule.
- Persist reason, actual elapsed time, configured duration and completed step.

UI confirms generation/elapsed time, then shows pending authoritative completion. No pause system is added; the current scheduler has Background/Interactive service modes, not pause.

**Contracts/cost:** additive commands/results/capability, constant request state, no extra simulation or RNG.

**Acceptance:** parser matrix, seven-day clock, repeated/stale finish, coincident terminal conditions, delivery barriers, persistence failures/process death and ordered-action replay.

## 4. Stage B: population-slot life state and economic ledger

Add `EvolvedGenerationState` in running continuation with one dense `EvolvedSlotRuntime` per population slot.

### Lifecycle and accounting data

| Layer | Required state |
|---|---|
| Identity | Slot, current physical ID, existing population brain/genome ownership |
| Lifecycle | Life ordinal, lives started, deaths, successful respawns, status, deadline/attempt state, fallback cursor |
| Current life | Age, gross credits, actual boost spent, spend attribution, net life points/local wallet, peak net points, food, kills, current/final and peak length |
| Generation | Total alive time, gross credits, actual boost spent, net earned balance, food, kills, completed-life net growth, pending waiting time |
| Assessment | Selected point assessment and independent non-point tuple |
| Best-point life | Ordinal, peak points, age, food, kills, current/final length, peak length |
| Diagnostics | Last cause, cause counts, failed placements, corpse emitted/discarded count/value |

Lifecycle states: Alive, Waiting, PendingPlacement, Exhausted, DeadNoRespawn. Initial life is ordinal 1; increment only on successful spawn.

Evolution reads `EvolvedAssessment` by slot instead of pretending final `SnakeState` represents all lives.

### First-class ledger invariants

Maintain these values in every score mode:

- `lifeGrossEarned`.
- `lifeBoostSpent`.
- `lifeBoostSpentFromLifeCredits`.
- `lifeBoostSpentFromPriorBank`.
- `lifeLocalWallet` / `lifeNetPoints`.
- `priorBankRemaining`.
- `generationGrossEarned`.
- `generationBoostSpent`.
- `generationNetEarned`.
- `assessmentPoints`.
- `SnakeState.points`: actual physical spendable wallet.

Mathematical ledger identities:

```text
lifeBoostSpent = lifeBoostSpentFromLifeCredits + lifeBoostSpentFromPriorBank
lifeNetPoints = lifeGrossEarned - lifeBoostSpentFromLifeCredits
generationNetEarned = generationGrossEarned - generationBoostSpent
```

Implement explicit finite nonnegative balances; compare independently accumulated Float64 identities with documented rounding bounds where necessary. Do not hide genuine overdraw with a debt/clamp trick.

`priorBankRemaining` is maintained directly, not reconstructed from old life records. In Carried mode the accessible wallet is current-life credits plus the remaining prior bank. Otherwise the accessible wallet is life-local only; inaccessible historical net earnings remain useful accounting, not spendable capital.

Use `MovementProposal::boost_points_spent` as the actual burn. Attribute it at that movement substep, before subsequent food/kill credits: current-life credits first, then inherited bank. Preserve physical movement's existing wallet arithmetic and transaction boundaries.

Example: inherit 50, spend 50 before earning, then earn 10. The spend is prior-bank expenditure; life net points and physical wallet are now 10. `max(lifeGrossEarned - allLifeBoostSpent, 0)` would incorrectly erase them and is prohibited.

### Point assessment and wallet

| Mode | Assessment |
|---|---|
| Reset | Latest/current life's net points; retain the just-dead final value while waiting; successful newborn starts at zero |
| Cumulative Net | All generation credits minus actual boost expenditure |
| Cumulative Gross | All generation credits, unaffected by boost spending |
| Best Life | Highest net life-point peak achieved across lives, including unfinished final life |

Best Life ties retain earlier ordinal. Its tuple updates while that life is active and finalizes at death. Spending after a peak does not reduce the recorded peak.

Cumulative wallets:

- Life-local, default: zero newborn wallet/prior bank.
- Carried: zero new-life credits and inherited net generation bank.

Gross never makes gross score spendable. Reset and Best Life use life-local wallets in the first UI. The internal bank is independent of score mode so future broader persistence is possible without reconstructing history.

### Non-point fitness bundles

- Generation totals, default with respawn: accumulated alive time, food, kills, completed-life net growth plus unfinished-life current net growth.
- Follow score: Reset uses latest life; Cumulative uses totals; Best Life uses the point-best life's tuple.
- Legacy terminal: latest physical-life values.

Point mode changes do not change the non-point bundle unless Follow score was selected.

Growth is `max(final_life_length - starting_length, 0)` per completed life, or current length for the unfinished life. Peak length is separate. This defines net retained growth rather than silently scoring every shrink/regrow event.

### Transaction integration and newborn invariant

Use authoritative deltas: prefix alive awards; food claims/credits; collision kills/credits; actual movement burn; complete substep aggregation; one life finalization.

Invariant for each generation/step/slot:

- Source-live life: exactly one alive award.
- Source-dead slot successfully replaced at the newborn boundary: exactly one newborn award.
- Waiting/failed/exhausted slot: no alive award.

The existing prefix accounts for source-live snakes, then awards a separately constructed baseline newborn once. Reuse that chronology. A replacement cannot receive both the dead life's award and a newborn award for the same step. A snake dying during physics retains the award already made that step; no fractional collision-time survival is introduced. Participating newborn time is not also counted as a full pending interval.

**Modules/contracts:** state/accounting/movement/food/collision/effects/physics/world-step/evolution/generation; new running life/assessment version and settings schema. Slot-sized reusable scratch, no historical-life allocation.

**Acceptance:** all score/bundle/wallet combinations; gross 80/net 30 after spending 50; inherited spending/new earnings; unfinished life; repeated/simultaneous deaths; deltas across substeps; no double/skipped awards; God Mode; rollback; legacy one-life equivalence.

## 5. Stage C: evolved replacement, transient state and bounded placement

### Identity model

- Slot, lineage, weights and population `BrainHandle` persist for the generation.
- `BrainOwner::PopulationSlot` remains unchanged.
- Each successful life gets a new general-domain internal ID and available frame-v1 ID different from its immediate predecessor.
- Replace its world record in place; never append historical evolved lives.
- Successor population/brain epochs use the existing generation transaction.

Add a private evolved-replacement publication proof containing slots and old/new physical identities. Narrowly extend allocator/brain continuation checks rather than weakening generic validation.

### Fresh physical state

Construct collision-safe start-length body, initial radius/base speed, placement heading, identical current/previous head, zero current/previous turn and boost request, actual boost false, zero life age/food/kills/points, starting target length and selected wallet.

Initialize delivered-wallet boundary to the starting wallet, preventing inherited capital from appearing as new earnings. Initialize first-delivery flag false. Clear life-local motion/sensor state. Apply exactly one newborn prefix award.

Set `ControlPhaseConfig::initial_neural_accumulator_seconds()` (currently 0.06 sentinel) so first inference precedes newborn movement even if control interval changes.

### Recurrent reset through existing inference

Reuse `NeuralControlBatchInputs::reset_brains`, `NeuralControlPipeline::prepare_recurrent_resets`, `evaluate_heterogeneous_population_with_resets` and staged recurrent publication.

Reset mode marks the life/brain pending zero-source first evaluation. Preserve mode omits it and freezes recurrence while dead. Both force evaluation; neither preserves old motor actions.

Split `prepare_neural`'s currently conflated force-due/reset-source decisions. External takeover retains its existing reset behavior. Reset source comes from the existing exact-zero block, not prefix-time mutation of authoritative recurrence. Clear marker only with successful complete publication; failure preserves old authority and pending reset.

### Death records and reference cleanup

Capture `EvolvedLifeEndRecord` before collision metadata disappears: slot, physical ID, ordinal, flags/primary cause, body owner, scores, food/kills, length and age. Retain simultaneous flags; exclusive chart precedence Wall -> Head-to-head -> Body. God Mode has its own cause and shares finalization.

Transaction event capacity is population-sized; reduce to counters and retain at most 256 recent events. Do not persist every life.

On replacement:

- Clear old pellet owners, preserve value/kind/color.
- Release retired bodies after final accounting and compact once per batch.
- Preserve population brain owner/handle.
- Rebuild calculation/index references; reject old delivery markers.
- Clear/invalidate contact scratch, baseline target diagnostics, visualization and physical selection.
- Treat historical killer IDs as values, not pointers.
- Signal life boundaries to capture/shadows.
- Leave external leases/routes separate.

Frame IDs can wrap. Selected entity mutation uses an on-demand token containing world incarnation/internal physical identity, preventing old frame IDs from targeting later lives. Individual-follow reacquires by slot. No frame-v1 expansion.

### Delay/life limits and congestion

Enabled off by default. Delay 0..3,600; Fixed or Uniform `[0.5D,1.5D]`, sampled once at death. Zero means next eligible pre-control boundary, never replacement inside a death substep. Maximum lives includes initial life; zero unlimited. Failed placement consumes neither life nor successful-respawn penalty.

Use an evolved mid-generation bounded spawn API/wrapper returning Placed, PendingNoPlacement or PendingWorkLimit, including consumed work, RNG continuation and fallback cursor. Invalid/corrupt/overflow/allocation state remains error.

Preserve existing generation-start, baseline v1, external and retained-spawn error semantics. Do not turn their `WorkBudgetExceeded` errors into pending implicitly.

Initial evolved work defaults: eight attempted slots/step, 256 candidates/slot, 2,048 candidates/batch, 250,000 geometry checks/batch, unsuccessful retry interval 0.25 simulated seconds. These are internal limits, not experiment sliders.

Order by deadline step, last-attempt step, slot. Earlier successes immediately become obstacles. Up to 32 random candidates, then bounded fallback with retained cursor. Revalidate against current world; no partial geometry proof survives a step.

Failed attempts advance selected RNG by consumed draws. Allocate IDs after success, staged. Pending retries last until generation completion; congestion neither faults nor silently exhausts lives. Report waiting/failures.

**Acceptance:** identity/brain/state resets; reset-aware rollback; 300 due slots; fairness/ordering; safe placement; impossible arena; continuation/ID rollback; owner cleanup/body capacity; no historical world growth; external lifecycle regressions.

## 6. Ordering and RNG contracts

### Physical versus individual ordering

Preserve existing physical ties and document death changing priority:

| Work | Rule |
|---|---|
| Independent accounting/movement/control lookup | Existing physical order remains |
| Neural calculation | Existing brain epoch/ID first, then snake/index ties |
| Equal-distance food | Lower physical ID wins |
| Body contact tie | Time, owner physical ID, segment |
| Spatial candidate ties | Lower bound/distance then owner/segment |
| Effect emissions/shared draws/overflow | Existing canonical physical order |
| Baseline v1 scans/reductions | Preserve ID-sorted behavior |
| Evolution and new accounting aggregates | Stable population slot |
| Evolved respawn attempts | Deadline/attempt/slot |
| V4 quotas | Equal fair grants over stable observation owners |
| Teacher streams | Baseline slot/life identity |

Do not globally substitute slots for physical IDs. Test changed snake ID with unchanged unique population brain handle: calculation ordinal must remain unchanged when brain IDs disambiguate. Metamorphic tests distinguish container/worker invariance from intentional physical-tie sensitivity.

### Evolved RNG

Offer Per-slot continuation and World, Per-slot default. Per-life derivation is considered but deferred to avoid a third initial regime.

Centralize `derive_evolved_respawn_rngs(run_seed: u32, generation: u64, population_count)` using:

```text
evolved-respawn/v1/generation/<decimal-u64>/slot/<decimal-u32>
```

ASCII decimal, no locale/padding/leading zeros. Use existing labelled `derive_seed`, never numeric `hash_seed([seed,generation,slot])`. That numeric contract normalizes to Uint32 and loses high bits. Exclude new run ID so same-seed Reset repeats derivation.

Initialize at generation boundary; delay/placement share selected stream; preserve failure draws and boundary continuation. Test 1, `2^32+1`, above `2^53`, `u64::MAX`, exact labels and frozen selected derivations. A 32-bit result is not globally injective; requirement is full-identity mixing, not collision impossibility.

Respawn never advances evolution RNG. Isolated placement does not isolate death: evolved corpse/boost effects currently consume world RNG. Preserve v1 baseline derivation; v2 separates behavior from lifecycle RNG. Training has its own RNG.

## 7. Fitness, ecology and generation completion

### Exact legacy fitness

Current default:

```text
F = 0.70 * age
  + 80 * food_value_eaten
  + 100 * max(body_length - start_length, 0)
  + 400 * kills
  + 42 * clamp(ln(1 + points) / ln(1 + maximum_points), 0, 1)
  + top_bonus
```

Terminal evolved points determine maximum; use 1 when nonpositive. Top bonus 600 for points within `1e-6` of maximum. All-zero points therefore do not automatically receive top bonus. Coefficients remain existing settings.

Respawn fitness substitutes assessment points for normalization/top comparison and independent selected non-point tuple for other terms. Waiting never counts as alive. Optional amount 0..10,000, default zero, penalizes Deaths or Successful respawns after rewards. Finite negative fitness is valid. Final exhausted death counts under Deaths only.

### Duration scaling

| Duration | Survival fitness at 0.70/s | Passive credits at 0.60/s |
|---|---:|---:|
| 4 minutes | 168 | 144 |
| 30 minutes | 1,260 | 1,080 |
| 1 hour | 2,520 | 2,160 |
| 6 hours | 15,120 | 12,960 |

Point normalization is bounded at 42 and top bonus at 600. A food-value unit gives 80 food fitness, potentially 100 retained growth fitness and 20 credits. A kill gives 400 fitness and 400 credits. Small-snake boost costs about 7/s, so passive earning supports about 8.6% boost duty before size effects.

Absolute is default. Optional 240-second normalized fitness multiplies extensive survival/food/growth/kills by `240/configured_duration`; leaves normalized points/top bonus/penalties and gameplay economics unchanged. Manual finish uses configured duration, not actual shortened time. Version objective semantics.

### Corpse mass audit

Normal body/head deaths use fraction `clamp(0.95 - (0.95-0.33)*size^1.6,0,1)`, size based on configured max/start length. Budget is recycled length / growth-per-food. Big/small base values are 3x/1x normal food; big share .78; floored counts with mandatory one big pellet; randomized values; default 420 pellets/corpse. Wall deaths do not drop corpses.

Starting length 5 emits one big + one small, expected value about 4, while replacement creates five segments.

| 300 snakes, body/head death interval | Deaths/hour | New segments/hour | Expected corpse value/hour | Pellets/hour |
|---|---:|---:|---:|---:|
| 60 seconds | 18,000 | 90,000 | 72,000 | 36,000 |
| 20 seconds | 54,000 | 270,000 | 216,000 | 108,000 |
| 5 seconds | 216,000 | 1,080,000 | 864,000 | 432,000 |

These exclude consumption, wall deaths, growth, caps and congestion. Six hours at 20 seconds attempts about 648,000 pellets versus 250,000 capacity.

Modes: Normal default; None on replaceable evolved death; Subtract replacement mass with budget `max(f*length - starting_length,0)/growPerFood`. Compensation mode permits zero, removes mandatory-big floor for that mode and bounds total randomized emitted value by budget. Final exhausted death uses normal rules because no replacement follows. This is not a claim of universal mass conservation.

### Overflow

Fail atomically preserves legacy default. Explicit respawn preset selects Discard new excess; enabling respawn alone does not overwrite other settings.

Discard retains existing pellets, processes new effects in existing canonical order, consumes defined draws for all attempted emissions, allocates IDs only for retained pellets and reports attempted/emitted/discarded count/value. Apply consistently to normal and cold God Mode effects. Corruption/allocation failures still fail.

### Early completion

- Automatic default: legacy physical threshold when respawn off; all slots exhausted for finite respawn; duration/manual only for unlimited.
- Physical alive: existing threshold even with pending lives, intentional experiment.
- Duration only: no early count rule.

Respect existing minimum early time. Pending congestion is not extinction.

**Acceptance:** all fitness/economy modes, growth and penalties, unfinished life, duration arithmetic, corpse budget/wall/exhaustion rules, saturated overflow/RNG, early/manual/duration completion.

## 8. History, HoF and long-generation durability

### Bounded history

Preserve existing 56-byte fitness summary rows. Add fixed-size versioned experiment sidecar in the same checkpoint/SQLite transaction, with bounded archive role:

- Reason, actual/configured time.
- Total lives/deaths/respawns; mean/median/max deaths/slot.
- Primary death causes.
- Current/gross/net/cumulative/best/assessment score summaries.
- Pending time and exhausted slots.
- Corpse attempted/emitted/discarded values/counts.
- Fitness component means/maxima and penalties.
- Scoring/accounting/scaling identities.

Median uses at most 300 counters at generation end. Keep per-step saturation, queue/strategy metrics and recent events bounded/ephemeral rather than permanent life logs.

HoF still selects best fitness genome. New record version retains assessment/bundle, selected life ordinal, economic scores/deaths, latest terminal/current length, selected-life peak and generation best length. Display length: Reset latest life; Cumulative generation best; Best Life selected-life peak. Label semantics. Do not reinterpret old 120-byte index records silently.

### Recovery comparison

| Option | Scope | Initial decision |
|---|---|---|
| Boundary durability | Restore pre-spawn population boundary | Implement/preserve |
| Periodic progress | Counters/digests/diagnostics, not exact world resume | Future optional |
| Exact mid-round | Full running authority and recovery protocol | Separate future project |

Multi-hour failure can lose the round's computation. Boundary saves do not imply a durable journal of mid-round external/settings/God Mode/manual actions.

Exact continuation would require world/snakes/bodies/pellets and owners, all brains/recurrent state, life accounting/pending cursors, baseline strategies/timers/cadence, RNG/spares, allocators, ambient/time/steps, observation/cadence/motion continuation, controller leases/actions, scheduler debt/wall rebasing, commands/manual state, and resolved transaction/delivery recovery. Rebuild derived indexes canonically. Display and trainer state remain separate.

Never turn diagnostic/trainer progress into implied world durability, rewrite owner databases or claim cross-build exact replay. Use existing labelled compatible-build recovery branches with explicit provenance.

## 9. Stage D: v3 audit and v4 sensor contract

### V3 exact layout

Let `bipolar(x)=clamp(2*x-1,-1,1)` and `s=clamp((L-start)/(max-start),0,1)`.

| Index | Label | Meaning |
|---:|---|---|
| 0..1 | heading_sin/cos | Absolute heading |
| 2 | size_norm | bipolar(s) |
| 3 | boost_margin | `(points-minBoost)/max(minBoost,1e-6)`, clamped |
| 4 | points_pct | Bipolar logarithmic ratio to monotonic generation-best points |
| 5 | speed_norm | Bipolar speed/configured boost speed |
| 6 | boost_state | Actual boost -1/+1 |
| 7 | points_norm | Bipolar points/max(best,1) |
| 8 | points_delta_norm | Delivered-sample delta / 10, clamped |
| 9 | length_norm | Bipolar length/configured max length |
| 10 | boost_points_frac | `(points-minBoost)/max(minBoost,1)`, clamped |
| 11 | boost_cost_norm | Bipolar effective cost / max(3*baseCost,1) |
| 12 | wall_dist_norm | Bipolar center-based wall distance/world radius |
| 13 | nearest_food_dist_norm | Bipolar proximity within far radius |
| 14..15 | nearest_food_dir_sin/cos | Relative bearing, zeros when absent |
| 16 | nearest_body_dist_norm | Bipolar nearest other-body proximity |
| 17 | nearest_head_dist_norm | Bipolar nearest other-head proximity |
| 18 | age_norm | Bipolar age/max(duration,1), capped |

Then B food, B hazard, B wall and B head values:

- Food sums `(1-distance/far)*clamp(value/foodValue,0,6)`, saturates with `strength/(strength+foodK)` and bipolar mapping.
- Hazard takes minimum segment clearance/near radius.
- Wall uses center ray to boundary minus radius/near.
- Head takes minimum other-head clearance/near.

Nearest-first pellet/body queries are capped (default 900/2,200, with existing effective minimum floors). Body cap returns conservative zero hazard clearances. Head scan covers bounded world records; wall is O(B). Sensor detail caps never truncate collision truth.

Losses: near/far food separation, ray route geometry, nearest-head bearing precision, relative size/heading/velocity, approaching versus receding, crossing/closing/TTC, crowd structure and proprioceptive state. V1 baseline's direct target length/heading/speed and crowd reads demonstrate useful missing information.

| Length | V3 size s | Default near radius | V4 tactical size candidate |
|---|---:|---:|---:|
| 5 | 0 | 520 | 0 |
| 25 | .0020 | 520.52 | .333 |
| 80 | .0075 | 521.95 | .652 |
| 1,000 | .0995 | 545.88 | .961 |

Existing sensor fixture lengths 2..20 and points 2..50 establish formula correctness, not trained-policy distribution. Collect bounded native workload histograms for lengths, wallets/credits/boost time, food value, wall/body/head clearance, deltas and new motion values before qualification. Preserve physical size curves.

V3 generation-best remains monotonic; respawn experiments define it from peak selected assessment, initialized before first sensing. Retain non-stationarity explicitly. V4 omits population-relative neural scores; inspector/history show them instead.

### Structural identity versus environment

`SensorContractIdentity` includes version/profile, width, exact labels/offsets, channel meaning/ordering/geometry, normalization revision/constants and missing/validity encodings.

Numeric world/range/food/cost/detail-limit settings remain run settings/provenance. They can alter behavior without making identical structural weights undecodable. Display differences without rejecting load. Structural mismatch rejects even if widths match. Cadence/recurrent life rules are separate deployment contracts.

### V4 scalar prefix (24)

| Index | Label |
|---:|---|
| 0..1 | heading_sin, heading_cos |
| 2 | tactical_size |
| 3 | radius_norm |
| 4 | speed_norm |
| 5 | spendable_points_norm |
| 6 | boost_seconds_norm |
| 7 | boost_cost_norm |
| 8 | boost_eligible |
| 9 | actual_boost |
| 10 | held_boost_request |
| 11 | held_turn |
| 12 | previous_applied_turn |
| 13 | angular_velocity_norm |
| 14 | speed_change_norm |
| 15 | delivered_wallet_delta_norm |
| 16 | current_life_points_norm |
| 17 | assessment_points_norm |
| 18 | life_age_norm |
| 19 | generation_remaining_fraction |
| 20 | life_budget_used_norm |
| 21 | first_sample_of_life |
| 22 | pellet_query_saturated |
| 23 | body_or_ray_query_saturated |

### V4 profiles and spatial semantics

Basic channels: near food, far food, other-body sector clearance, body capsule-ray clearance, wall-ray clearance, other-head clearance. Width `24+6B` (120 at B16, 216 at B32).

Motion adds nearest-head presence, bearing residual, relative length, heading sine/cosine, relative-velocity forward/lateral, earliest projected collision urgency, head density. Width `24+15B` (264 at B16, 504 at B32).

Nearest fields describe one head selected by minimum clearance then ID. Collision urgency considers all relevant heads, binned by predicted encounter bearing, so nearest-only selection does not hide a crossing threat. Compute relative-motion disc intersection over two seconds using authoritative current heading/speed. It is a held-motion estimate, not another simulation. Closing/closest approach/intercept diagnostics can expose derived quantities without redundant neural channels.

Food rings are `[0,near]` and `(near,far]`, declared in manifest with bounded distance/value weighting and saturation. Rays use body capsules and declared collision radii. Complete channel formulas/absence behavior receive exact geometry fixtures before release.

B8..32, default B16. Motion/B32 is a ceiling case under tensor width 512; future channel changes need another contract or explicit admission change. No arbitrary feature subsets. Third radial band is deferred.

### Normalization candidates

- Tactical size: bipolar `d/(d+40)`, `d=max(L-start,0)`; same fraction scales configured sensor radii.
- Radius: bipolar radius/configured maximum radius.
- Speed: bipolar speed/configured boost speed.
- Wallet: bipolar `P/(P+140)`.
- Life points: bipolar `P/(P+200)`.
- Assessment: bipolar `P/(P+1000)`.
- Boost time: headroom above minimum points / actual effective cost, saturated by bipolar `T/(T+2)`.
- Cost: bipolar `cost/(cost+7)`.
- Life age: bipolar `age/(age+60)`.
- Delivered wallet delta: `delta/(abs(delta)+10)`.
- Angular velocity: completed fixed-step heading change/time divided by configured turn rate.
- Speed change: completed-step speed change/configured boost speed.
- Relative length: clamp `log2(otherLength/ownLength)/4`.
- Relative velocity: divide by twice configured boost speed, clamp.
- Collision urgency: `1-TTC/2` within horizon, zero otherwise.
- Density: `count/(count+4)`.
- Declared clearance/proximity transforms use actual sensing radii.
- Booleans/presence/saturation: -1/+1.

Freeze normalization revision after representative histogram/tactical qualification, never silently retune a released contract.

### Kind-specific meanings and exact sentinels

| Kind | Life/assessment points | life_budget_used_norm |
|---|---|---|
| Evolved, respawn disabled | Life score / current legacy assessment | -1.0 |
| Evolved, finite maximum M | Explicit life score / selected assessment | Float32(-livesStarted/M) |
| Evolved, unlimited | Explicit life score / selected assessment | +1.0 |
| Baseline | Ordinary physical-life points for both | 0.0 |
| External | Ordinary physical-life points for both | 0.0 |
| Resurrected retained neural | Ordinary physical-life points for both | 0.0 |

Finite values are [-1,0), +1 unlimited evolved, 0 non-applicable. Baseline/external replacement entitlements are separate. Archived HoF score is provenance, not the live snake's input.

`first_sample_of_life` is +1 until actual internal consumption or matching external acceptance, then -1. Add physical-life delivery flag and extend `ObservationDeliveryMarker` validation/commit. Pure probes, inspector and shadow do not clear it. Rejected delivery preserves it; takeover in the same physical life does not restart it.

Zero effective boost cost maps budget to +1 (no point-cost bound). Eligibility follows actual movement: currently zero spend leaves boost off, so eligibility is -1. Do not introduce free boosting as a sensor change. All output values are finite Float32.

### Fair bounded ray work

Prototype spatial traversal with broad-phase rejection, nearest-first/early termination, reusable deduplication and bounded cell/candidate/intersection work. No 65,536-per-snake ceiling.

`RayWorkLimits`: qualified profile-specific minimum/observation, cap/observation, batch budget. Admission proves positive minimum <= cap and `maximum_due_observations * minimum <= batch_budget`. Maximum includes all simultaneously admissible evolved, baseline, retained and external consumers and coincident first/takeover evaluations, not just population count.

Allocate equal base quotas, then single-unit remainder quanta around rotating canonical owner ring. Stable evolved/baseline slot and non-population brain ownership identifies observations; classes get equal opportunity. Commit cursor with observation transaction. Workers never redistribute by finish order; initially unused quota is not reclaimed after dispatch. Inspector probes use separate budget; shadows reuse observations.

Illustration only: B65,536/N300/C512 yields 218 each plus 136 rotating extra units. Continuous fixed-set saturation must keep cumulative grants within one quantum.

Qualification candidates: per-observation exact-work caps 128/256/512/1,024; batch budgets 16,384/32,768/65,536. Select charged work units and minimum through benchmark, not an arbitrary fixed 16. A 65,536 batch would bound exact tests at about 3.93 million/s at 60 due boundaries, not a billion.

Unknown/incomplete rays are conservative and validity-marked. Measure cap/quality rates, not only theoretical bounds. Select smallest candidate meeting geometry/tactical correctness, ordinary saturation below 1% and supported P0/P1 1x gate. If none qualifies, revise traversal/profile visibly before freeze; do not simply raise limits.

Ablate bearing residual, heading pair, velocity pair, urgency and density with identical workloads/seeds/datasets. Compare tactical quality, agreement, width/parameter/inference/sensing costs. The 15-channel Motion contract is the initial candidate; any pre-freeze removal updates all counts. Post-release alterations are a new contract.

### Compatibility propagation and acceptance

Trace version/profile/width/digest through native layout/sensors, graph Input validation, stack/custom editor, inference/workers/packed observations, external RL, inspector/visualization, service bounds, startup/welcome, checkpoint/HoF/saved brains, import/legacy conversion, PyRL and memory admission.

Profile/bin change uses Apply and reset/new weights. No arbitrary-network input migration or v3 padding. Preserve v3 native/frozen fixtures. Test exact ordering, every normalization/geometry/motion/absence/sentinel, query saturation, fairness, environmental compatibility versus structural rejection, and worker-count equivalence.

## 10. Stage E: baseline v2 and tactical evaluation

V1 already has Small/Medium/Large stages at 25/80; Roam/Seek/Avoid/Boost; persistent timers/wander RNG; food/clearance sector scoring and hazard veto/fallback; medium direct target selection, half-second lead/cutoff-like bias; large crowd logic. Preserve versioned v1.

V2 consumes decoded v4 observations only for action selection. Metadata IDs may identify inspected sector representatives but do not supply undeclared privileged control information.

Add relative-motion/head-on/crossing risk, capsule routes, smaller-target pursuit/intercept, larger-threat escape, wall openings, food-trap discounting, crowd pressure, commitment/hysteresis and economic safe boost.

Initial versioned rules: urgent TTC < .5s; safe commitment .2s unless unsafe; switch safe sector at normalized improvement >= .15; smallest absolute turn then sector-index ties. Basic omits unavailable motion tactics. Preserve size-stage weighting as starting policy, not a simplistic replacement.

Modes: Deterministic uses fixed timers/ties/no strategy RNG; Stochastic uses isolated behavior RNG for wander/timers/safe exploration. Lifecycle RNG remains separate in v2. V1 streams/behavior remain unchanged.

V2 `baselineBots.controlDt` uses shared due algorithm. Teacher/student deployment cadence must match, both first-life evaluations immediate. Capture never changes cadence. V1 teacher remains fixed-step; downsampling labels is not falsely described as equivalent held-action training.

Native fixtures use real sensor/control/physics: approaching/receding, crossing/head-on, corridor, safe crowd opening, wall escape, food-body trap, smaller intercept, larger threat, boost pursuit/escape, predicted crossing and saturation. Short qualitative tests plus deterministic batches measure survival/food/collision causes/interceptions/agreement. V1 comparison is evidence, not a universal optimality requirement.

**Acceptance/cost:** declared-information teacher, no stale target dependence, mode/cadence reproducibility, isolated RNG, fixed sector/state work, no extra v2 full-world scan.

## 11. Stage F: canonical graph training in PyRL

### Chosen method and process

PyRL currently has CUDA device selection, Adam/autograd, train/inference copies, sensor equality and atomic optimizer checkpointing. Its `PolicyValueNet` is a ReLU MLP with policy/value heads and is not the exported Neuroevo format.

Add separate Distillation mode beside PPO. Selected solution is PyTorch/CUDA canonical graph, not new Rust autodiff/BPTT, Rust ML adapters or imitation-only neuroevolution. Alternatives support different tradeoffs, but implementing native gradients/GPU would have much higher maintenance; neuroevolution would be less sample efficient.

PyRL is trainer only. Rust owns simulation, observations/teacher, gameplay evaluation and deployed inference. No live control weights change halfway through a calculation.

### Rust manifest

`CanonicalTrainingGraphManifestV1` from canonical Rust compilation includes graph spec/key, layout/equation revision/digest, topological order, resolved incoming ports/order, node widths/MLP layers, named parameter blocks/offsets/lengths, recurrent layout, outputs/order, totals, sensor structural identity and deployment cadence/output mapping.

PyRL constructs differentiable views over one flat canonical Float32 parameter tensor. It never independently chooses ordering/layout.

Implement exact tanh Dense/MLP and custom recurrences:

- GRU Wz/Wr/Wh/Uz/Ur/Uh/bz/br/bh; candidate `tanh(Wh*x + Uh*(r*h) + bh)`; `(1-z)*h + z*candidate`.
- LSTM Wi/Wf/Wo/Wg/Ui/Uf/Uo/Ug/bi/bf/bo/bg; hidden then cell.
- RRU Wc/Wr/Uc/Ur/bc/br; `(1-gate)*h + gate*candidate`.
- Explicit Split, Concat, branching/fan-out and output ordering.

No stock GRU/LSTM substitution based only on dimensions. Rust initialization/bias semantics remain canonical; isolated trainer seed initialization must not consume authoritative evolution RNG.

### Exact loss and recurrent training

For raw canonical turn t, boost b, committed teacher turn a and boost request y:

```text
Lturn = (t-a)^2
z = (b-0.35)/0.10
Lboost = BCEWithLogits(z,y) = softplus(z)-y*z
L = eventWeight * (Lturn + Lboost)
```

Runtime remains `clamp(b,-1,1)>0.35`, equivalent to b>.35 for finite b. Temperature .10 is fixed/versioned method v1. No deployed sigmoid/value head. Realized boost is a separate outcome, not the label.

Defaults: Adam lr3e-4, clip1.0, batch32 sequence windows, TBPTT64, burn-in32, Float32. State advances on deployment cadence, resets/preserves on declared life boundary, detaches at truncation. Gaps terminate continuity. Burn-in is an approximation, not exact unlimited-memory reconstruction. Short full-sequence BPTT is allowed within bounds; unbounded full-life BPTT is not initial scope.

### Parity and deployment roundtrip

Offline whole-graph Rust runner accepts bounded graph/weights/observations/state, emits outputs/next state; no per-layer production bridge.

Test every operation, biases, widths, mixed recurrences, Split/Concat/fan-out/outputs, zero/nonzero state and 256-step recurrence.

Initial tolerances: CPU step atol1e-5/rtol1e-4; CUDA step 1e-4/1e-4; 256 steps 1e-3/1e-3 with error growth reported. Disable TF32/mixed precision initially. Investigate failures, do not automatically widen.

Mandatory full roundtrip: actual optimizer update -> export flat Float32 bytes -> ordinary Rust graph/weight validation -> Rust batch inference -> reimport exact bytes into PyTorch -> compare output/next recurrent state.

Reproducibility envelope is dataset/order/seed and recorded Torch/CUDA/device/environment. CPU/RTX4080/different CUDA versions need numerical semantic parity, not byte-identical trained weights. Current PyRL dependencies pin Torch2.14 with CUDA12.6 wheel index; do not alter PPO dependencies casually.

**Acceptance:** all canonical supported DAGs train, all parity/roundtrip gates, finite validated export, normal Rust deployment, existing PPO regressions.

## 12. Stage G: committed teacher capture and bounded transport

### Samples and sequence semantics

Capture exact observation already constructed by authoritative due control and actual committed turn/boost request. Publish only after complete transaction; failed steps produce no examples. Do not resample or commit an extra delivery marker.

`TeacherSampleV1`: run/config/sensor/teacher/cadence identity, generation/step, baseline slot/physical life/ordinal, Float32 vector, applied actions, strategy/life stage, hazard/saturation stratum, sample sequence and start/end/gap reasons. Include post-step death/actual boost as outcomes.

Contiguous stream chunks at most256 observations, sealed early at life/contract/capture boundaries or after one simulated second. Chunk boundary alone is not recurrent reset. Capture starting mid-life is labelled capture-start, not fabricated physical-life start.

Default rare-event weight4x cruising, bounded. Strata: imminent collision/wall escape, pursuit/escape, boost, transitions, crowd/saturation, post-respawn. Preserve raw counts/unweighted validation. Partition whole recurrent streams with stable run-seed/baseline-slot/generation hash, 20% validation; final held-out run seeds. Measure conditional label variance from hidden strategy/stochastic state; do not promise exact instantaneous cloning.

### DistillationTransportV1

Use immutable binary spool chunks over dedicated bounded HTTP. New PyRL `DistillationClient` must not instantiate `ActorClient`, send player join/action, acquire lease or resume token. Existing ActorClient bot/player path stays PPO/external control.

```text
Committed Rust samples -> sequence buffers -> separate bounded native chunk queue
-> Node spool worker (opaque bytes) -> PyRL HTTP consumer -> canonical training
-> candidate artifact upload -> Rust validation -> optional shadow/library save
```

Rust owns encoding, boundaries and descriptors. Node stores/routes opaque bytes and bounded metadata, not parsed samples. Authoritative thread does no disk I/O and never waits on trainer/storage.

Separate coarse `drainTeacherChunks(maxChunks,maxOwnedBytes)` diagnostic queue; do not use reliable controller/generation output reservations.

Route namespace `/api/distillation/v1/` provides dataset metadata/manifest, paginated chunk listing after exact cursor, immutable downloads and bounded candidate upload. Candidate IDs/digests make retries idempotent. Node routes uploaded canonical bytes to Rust validation, never Python model/pickle as a deployed brain.

Descriptors: dataset/capture epoch, monotonic u64 chunk sequence (hex wire), stream/generation/life, first/last sample sequence, count/width, contracts, boundary reasons, byte count and SHA256. Assign sequences when sealing committed chunks, before queue admission. Whole dropped chunks create visible gaps.

### Quotas, atomic spool and watermarks

Initial configurable defaults to qualify: native16MiB, replay RAM256MiB, spool4GiB, at most65,536 retained chunks. At 264 inputs plus ~64 metadata bytes, 1,120 bytes/sample; ten60Hz teachers ~672kB/s, ~14.5GB/six hours.

Native overflow drops complete newly sealed chunks and records counters; no authority wait. Spool rotates oldest complete unpinned chunks. Leases have finite count/byte budget and expiry/renewal. Pinned saturation drops new data. Bound download concurrency/buffers separately.

Spool descriptor visibility requires complete bytes written, byte/digest validation, required durability, rename/publication, then metadata transaction. Interrupted writes never advertise partial objects.

| Watermark | Exact meaning |
|---|---|
| productionWatermark | Highest assigned committed chunk sequence, including dropped-before-spool |
| storageWatermark | Highest successfully published immutable chunk, not contiguous guarantee |
| retentionFloor | Lower bound below which chunks are no longer retrievable |
| resolvedThrough | Highest contiguous sequence with settled stored/unavailable disposition |

Coherent bounded listings include those values/descriptors/drop counters. Missing above resolvedThrough may be pending; missing at/below is unavailable, after respecting pagination. Do not infer loss from storageWatermark alone.

No consumption ACK gates capture/simulation. PyRL persists cursors/snapshot/sampler state with job checkpoint, reconnects after cursor, deduplicates retries and marks gaps. Never fabricate missing observations. Dataset continuity failure cannot corrupt world.

After Slither failure, epoch is interrupted; retain last durably observed watermark, do not invent unpersisted tail or reuse epoch. New capture has new epoch. Retained data supports offline jobs. A committed world step whose chunk never becomes durable is allowed to become a diagnostic transport gap.

**Acceptance:** exact sample/action bytes; cadence/first delivery; failed steps; overflow, slow/disconnected consumer, leases/eviction, incomplete writes, duplicate/reconnect/pagination, watermarks, no controller lease and authority on/off equivalence.

## 13. Stage H: shadow evaluation and durable jobs

### Shadows

At most four explicitly requested candidates, absent by default. Teacher owns snake. Shadow uses exact same vector, normal Rust inference, separate recurrence/cadence/life rule; never publishes action, fitness or observation marker.

Metrics: turn MAE/MSE, boost/threshold disagreement, strategy/hazard/pursuit/post-respawn strata, bounded pre-death window. Immutable candidate activates through explicit boundary command; reset shadow recurrence and metric segment on version change. Errors disable job, not partial controlling update.

### Isolation and reproducibility

Own Python/NumPy/Torch CPU/CUDA RNG. Default two CPU threads normalized to available CPUs; no loader workers initially; preferred configured CUDA0 with explicit availability validation/CPU option. Initial admitted device budget8GiB. CUDA allocation/crash pauses job, not world.

No world/baseline/evolution RNG, ordering, physics or live controlling mutation. CPU contention can affect wall throughput/debt, not identical completed-step/action results. Record candidate arrival/activation. Live retention may differ with transport pressure; record dataset decisions rather than promise identical asynchronous jobs.

### DistillationJobV1 owner

New schema/checkpoint owner alongside PPO, not extra fields stuffed into PPO schema2. Reuse atomic utilities/rotation only.

Persist graph/structural sensor/deployment/teacher/method identities, hyperparameters/loss revision, canonical weights, optimizer, all trainer RNG, counters, immutable dataset snapshot/chunks/leases, partition/sampler, sequence cursors/detached recurrence, validation/online metrics, source provenance and numerical environment.

Atomic checkpoint every60 wall seconds or1,000 updates; newest three valid. Candidate every250 updates. Sampler/cursor/optimizer/RNG checkpoint together. Missing required dataset state makes exact restart fail explicitly or start labelled segment; never silently skip/reconstruct.

PyRL structural contract extends current layout_version+labels with profile, normalization revision, structural digest and widths, retaining exact labels. Legacy PPO v3 schemas remain readable; v4 missing digest is rejected. Numeric settings remain provenance.

**Acceptance:** offline same-environment update reproducibility, restart/optimizer/RNG/sequence, dataset-loss behavior, immutable handoff, bounded RAM/VRAM, candidate export roundtrip, multiworker authority unchanged with capture/training/shadow.

## 14. Stage I: retained origins, saved brains and portable deployment

### Managed objects/catalog

Generalize managed weight storage beneath HoF: `ManagedNeuralWeightsDescriptor`, `SavedBrainRecordV1`, separate catalog and reference-aware retention/export leases. Preserve old HoF bytes/references; GC counts both consumers.

Saved record: graph/key/packed weights/structural sensor/cadence, teacher/version/mode, method/seed, samples/validation/gameplay metrics, source run/generation/settings and environment. Distilled brains are not HoF winners.

Initial100 retained records/2GiB objects, paginated metadata, explicit save/delete/export. Candidates do not auto-populate library.

### Runtime origin ownership

Keep four-value `SnakeKind`; `Resurrected` is the retained non-population neural runtime class for HoF and saved brains. No SavedNeural variant in initial implementation.

Add to `BrainRuntimeState`:

```rust
retained_origin: Option<Arc<RetainedNeuralOrigin>>
```

Bounded immutable origin identifies HallOfFame(entryId,weightDigest) or SavedBrain(brainId,weightDigest,sourceKind). Distilled/Imported/Manual is provenance, not simulation behavior. Population/external brains have None; production retained brains supply validated origin.

Spawn/removal/reuse publishes origin with brain/world/allocators. Custom `copy_brains_reusing`, external replacement, validation, admission and fixtures explicitly preserve/compare `Arc::clone`, never reconstruct provenance strings per step. Charge allocation overhead/text. Removing brain releases Arc; failed publication retains old state.

Node owns durable catalog, but Rust owns live origin. Catalog deletion cannot invalidate a running brain; its weights/origin are retained values, not dangling lookup pointers. Skin/metadata shows distinction; God Mode/control/effect RNG remains existing runtime kind.

### Portable .slither-brain and spawn

Versioned bounded manifest + graph/spec/digest + sensor/deployment contract + canonical Float32 weights/counts/encoding/digests + provenance. Reuse raw/shuffled-Zstandard codecs/validation. Max64MiB and current graph limits. Rust independently recompiles/validates finite values, layout/width/count/digest. Browser/Node route bytes only.

Store differing architecture artifacts, but live spawning requires active graph and structural sensor match, as current single-graph authority. Numeric environment differences are displayed, not structurally rejected. No per-entity graph runtime redesign.

Optional associated saved brains in .slither-save, default off. Trainer datasets/optimizer states are separate exports.

Generalize retained spawn: safe placement, Entity brain owner, zero recurrence, first-action sentinel, admitted capacities/atomic proof. Reuse eligible dead retained records, freeing old brain/origin and clearing owners/body; never active/population records. No frame-v1 change.

**Acceptance:** reference GC, origins/custom copies/no warm allocation, catalog deletion live, artifact/archive mixed versions, finite/graph/sensor rejection, compatible numeric settings, ordinary Rust spawn, initial inference/dead-record reuse.

## 15. Stage J: inspector and capacity qualification

### Selected inspector

One target, replaceable at most10Hz, enabled on demand. Payload identity: world epoch, step, physical ID/slot, exact sample age. Show raw/normalized scalars/rings/sectors/rays, body/head/wall/food/motion/saturation, baseline strategy/sector/tactical quantities, teacher/shadow outputs, evolved ordinal/economics/assessment/deaths/pending and provisional fitness.

Non-due snake shows last delivered sample, not fresh pretend input. Explicit pure probe is labelled and does not clear delivery/first flag or quota cursor. Closed inspector stops expensive debug capture/large stream. Small selection identity remains on demand.

### Parameter/capacity estimates

Modest:64/64 MLP + GRU16 + Dense2. Large:five256 affine layers + GRU96 + Dense2.

| Graph/profile | Inputs | Params/brain | Raw MiB55 | Raw MiB300 | Expected mutation attempts/child |
|---|---:|---:|---:|---:|---:|
| Modest v3 B16 | 83 | 13,458 | 2.82 | 15.40 | 384 |
| Modest Basic B16 | 120 | 15,826 | 3.32 | 18.11 | 455 |
| Modest Motion B16 | 264 | 25,042 | 5.25 | 28.66 | 732 |
| Large v3 B32 | 147 | 402,914 | 84.53 | 461.10 | 11,579 |
| Large Basic B32 | 216 | 420,578 | 88.24 | 481.31 | 12,109 |
| Large Motion B32 | 504 | 494,306 | 103.71 | 565.69 | 14,321 |

First affine increase is deltaInput*H; direct recurrent input uses gate multiplier GRU3/LSTM4/RRU2. Mutation expectations use .03 nonrecurrent/.025 recurrent, not guaranteed effective changes after clamps. Modest parameter work rises ~86%, large ~23%. RMS species threshold .35 remains; extra dimensions change relative contributions, measure rather than automatically retune.

### Checkpoint gate

565.69MiB weights alone exceed current512MiB. **768MiB is a candidate, not approved answer.** Audit payload/small roles, raw/compressed candidate files, `checkpoint_workspace_bound`, old/successor weights, decoded state/recurrent buffers, HoF/saved objects, archive/import overlap, Node spool/disk reserves and process peak RSS.

Qualify on Windows/Oxygen before aligning native/descriptor/disk limits. Preserve global admission; reject unfit configurations with estimates, never silently reduce dimensions or raise envelope until bytes happen to fit.

### Bounded storage and performance

Life/accounting: population-sized. Respawn: candidate/geometry limits and one body compaction. Rays: qualified fair batch cap. Sensor scratch: reusable worker buffers. Shadow: at most4. Capture: one due sample/teacher plus bounded chunks. Replay/spool/jobs/library: quotas. Inspector: one target. Logs: aggregate/rate-limited, not per-life/sample/retry.

Audit world.snakes/body capacity, pellet owners, event rings, origin Arcs, recurrent masks, visualization, data queues/leases, telemetry and candidates. Non-population stale records are reused/bounded. Persistent history grows by compact generation records under existing export bounds, not by every historical life.

Run real native P0/P1/P2 and P3-style admission, crowded geometry and six-hour-equivalent soak with repeated deaths, pellet/ray saturation, shadows and slow consumers. Measure step percentiles, throughput/debt, warmed allocations/capacities, compaction, queue work, RSS/VRAM and checkpoint peaks. CPU/GPU contention does not loosen determinism assertions.

## 16. Persistence/version rollout and external-controller safety

Preserve Protocol2 and frame-v1; add capabilities. Version state/running accounting, settings/hash, RNG bundles, sensor structural contract, baselinev2, experiment sidecar/HoF, manifest/samples/transport/jobs/candidates, saved-brain objects and archive roles explicitly.

Checkpoint-v3 remains boundary container/purpose with revised contained state schema. Dispatch readers by recorded version. Existing source/schema recovery uses labelled compatible-build branch with explicit legacy defaults/provenance. Never call cross-build recovery exact. Preserve original database/files, health-only failed-resume diagnostics and disk reserve.

Legacy TS-v2/gzip/JSON conversion remains bounded and v3-specific; no arbitrary v4 padding. Update startup metadata currently hard-coded sensor3, browser reconstruction currently assuming bins->v3, service reply bounds, settings coercion and admission estimates together. Sensor/graph key and packed counts remain compatibility boundaries.

External controllers retain joined/RL death replacement, assignment receipt/token/reclaim, reliable routing, input hold/grace/takeover, generation reassignments and authority replacement. Evolved respawn shares only underlying geometry/storage/reset helpers, never external lease semantics. Distillation transport cannot masquerade as an external controller.

## 17. Ordered implementation and acceptance matrix

| Stage | Deliverable | Dependency/gate |
|---|---|---|
| A | Duration/manual finish | Existing terminal transaction and clock/parser checks |
| B | Life/economic/death accounting | Legacy one-life equivalence first |
| C | Respawn identity/reset/placement | Accounting and private mutation proofs |
| Following B/C | RNG/scoring/ecology/termination/history | All selectable rules and durable records |
| D | v4 prototype/qualification | Stable life/economic meanings, fair budgets and pre-freeze ablation |
| E | baselinev2/tactics | Shared qualified observation model |
| F | canonical manifest/PyTorch parity | Can parallelize development with D/E once contracts stabilize |
| G | capture/transport | Commit-safe samples and cadence/teacher identities |
| H | shadows/training/jobs | Manifest parity and bounded datasets |
| I | library/artifacts/spawn | Qualified candidates, shared retained objects/origins |
| J | inspector/integrated capacity | Diagnostics incrementally; final cross-platform soak |
| Future | founders/DAgger/exact recovery | Separate explicit projects |

### Required tests

- Identity: same slot/genome/brain, new internal/public IDs, reset/preserve, first action due, changed physical ID unchanged calculation ordinal.
- Transactions: failed reset inference, later physics/delivery failure, pending reset retained, no partial origin/lifecycle/ledger publication.
- Storage: owner cleanup, compact bodies, no historical evolved growth, retained reuse, origin Arc preservation/no provenance allocations.
- Accounting: every scoring/gross-net/wallet/bundle, unfinished peak, spend attribution/prior bank, food/kills/growth/alive/wait, penalties, multiple/simultaneous deaths, one award per step.
- Placement: one/300 due, stable fair attempts/earlier obstacles, work limits/fallback cursor, failed RNG, impossible arena/pending, allocation rollback.
- Ecology: corpse modes/budget/quantization, wall/final-exhausted behavior, capacity discard draws/order, repeated deaths.
- Completion: duration/manual/stale/duplicate/coincident, physical count, unlimited/pending, finite exhaustion, durability barriers/process death.
- Duration: all grammar combinations, commas/case/spaces/colon, fractions/negative/duplicates/garbage/nonfinite/overflow/min/max, long f64 versus integer-step clock and actual completion.
- Sensors: exact scalar/channel offsets, geometry, food rings, motion/intercept/TTC, proprioception, kind/sentinels/first delivery/zero-cost, saturation, quota fairness/all consumers, v3 preservation/environment compatibility/structural rejection.
- Baseline: tactics, deterministic no strategy draws, stochastic replay, cadence, declared inputs and hidden-state variance.
- Training: all graph operations/DAGs/recurrences, initialization/order, actual update-export-Rust load-inference-PyTorch reimport, numeric environments.
- Capture/transport: exact observations/actions/committed-only, sequences/lives/cadence, gap/partition, queue/spool/leases, atomic files/watermarks/pagination/reconnect/duplicates, no controller entitlement.
- Jobs/shadows: independent state, version activation, bounded pre-death metrics, optimizer/RNG/sampler restart, interrupted saves/missing data, no world RNG/order/weights mutation.
- Persistence: old/native/legacy import, sidecar/HoF versions, saved-brain file/archive/reference GC, catalog deletion live, incompatible brain, startup defaults/version branches.
- Multiworker: identical matching-step/actions authority and captured observations across supported worker counts; scalar/SSE2 comparisons use existing explicit tolerances.

Register JS files once in `scripts/test-categories.ts`; native-required overlay must load actual source-identified addon. Keep five native identity fixtures/historical bytes, no restored TS generators. Focused tests during slices; broad mandated Rust/JS/type/lint/build at cohesive checkpoints; CI cross-platform confirmation. Follow AGENTS CI efficiency/Oxygen/scratch cleanup rules. No new prose dossier for ordinary progress.

## 18. Concrete settings surface

Enum labels use stable numeric wire codes and typed native enums; extend SettingDefinition for select/duration. R=Apply/reset+world settings/hash/checkpoint; L=ordered live operation; J=durable trainer-job config; P=presentation preference. Job settings are outside world checkpoints.

### Generation, evolved lives and fitness

| Path | UI label | Type/range/options | Default | Apply | Structural compatibility | Tooltip/consequence |
|---|---|---|---|---|---|---|
| generationSeconds | Generation duration | Numeric8..604800; duration text | 240 | R | No | Simulated duration; absolute long rewards differ |
| observer.earlyEndMode | Early completion | Automatic/Physical alive/Duration only | Automatic | R | No | Whether temporary death can end round |
| observer.earlyEndMinSeconds | Earliest early completion | Existing0..50 | 8 | Existing L, checkpointed | No | Minimum time for count rules |
| observer.earlyEndAliveThreshold | Alive threshold | Existing1..25 | 2 | Existing L, checkpointed | No | Physical-alive experiment |
| evolvedRespawn.enabled | Respawn evolved snakes | Bool | Off | R | No | Same genome, multiple physical lives |
| evolvedRespawn.delaySeconds | Respawn delay | Number0..3600 | 0 | R | No | Zero next newborn boundary, wait loses alive reward |
| evolvedRespawn.delayMode | Delay variation | Fixed/Uniform | Fixed | R | No | Uniform0.5..1.5D |
| evolvedRespawn.maxLives | Lives per individual | Integer0..10000;0 unlimited | 0 | R | No | Includes initial life; finite exhaustion |
| evolvedRespawn.recurrentState | Memory across lives | Reset/Preserve | Reset | R | Separate deployment rule | Cross-life recurrent experiment |
| evolvedRespawn.rngMode | Respawn randomness | Per-slot/World | Per-slot | R | No | Shared environmental continuation |
| lifeScoring.mode | Point assessment | Reset/Cumulative/Best Life | Reset | R | No | Points independently of other fitness |
| lifeScoring.cumulativeBasis | Cumulative score | Net/Gross earned | Net | R | No | Gross ignores burn |
| lifeScoring.cumulativeWallet | Cumulative boost wallet | Life-local/Carried | Life-local | R | No | Carried inherits net bank |
| reward.lifeAccounting | Fitness across lives | Generation totals/Follow score/Legacy terminal | Generation totals | R | No | Independent non-point bundle |
| reward.durationScaling | Duration reward scale | Absolute/240-second normalized | Absolute | R | No | Extensive fitness only |
| reward.lifePenalty | Life penalty | Number0..10000 | 0 | R | No | Explicit cost, zero permits cheap death |
| reward.lifePenaltyBasis | Penalize | Deaths/Successful respawns | Deaths | R | No | Final death only under Deaths |
| evolvedRespawn.corpseMode | Replaceable death food | Normal/None/Subtract replacement mass | Normal | R | No | Ecology/mass experiment |
| world.pelletOverflow | Full pellet storage | Fail/Discard new excess | Fail | R | No | Discard reports resource-limited food loss |

Explicit Respawn preset enables respawn and selects Discard, otherwise uses defaults. Checkbox alone does not overwrite other settings. Existing reward component sliders retain recorded semantics; live changes record config provenance. Respawn-specific score/fitness overrides are inactive when off under default legacy behavior. Gross/net/bank telemetry remains explicit.

### Sensors and baseline

| Path | UI label | Range/options | Fresh default | Apply | Compatibility | Tooltip/consequence |
|---|---|---|---|---|---|---|
| sense.layoutVersion | Sensor version | v3/v4 | v4 | R | Structural | v3 frozen compatibility |
| sense.profile | V4 information | Basic/Motion | Motion | R | Structural | Limited versus motion-aware |
| sense.bubbleBins | Angular resolution | Integer8..32 | 16 | R | Structural | More input/search cost;32 ceiling |
| Existing sense.rNear*/rFar* | Sensor ranges | Existing bounds | Existing numeric defaults | Existing settings semantics, checkpointed | Numeric environment | Tactical scale in v4, frozen v3 |
| sense.foodKBase | Food saturation | Existing bound | Existing | Existing semantics | Numeric environment | Density saturation |
| sense.maxPelletChecks | Food detail limit | Existing100..3000 | 900 | Existing semantics | Numeric environment | Visible cap, no collision truncation |
| sense.maxSegmentChecks | Body detail limit | Existing200..4000 | 2200 | Existing semantics | Numeric environment | Conservative valid-limit signal |
| baselineBots.algorithm | Baseline algorithm | v1/v2 | v2 | R | Teacher identity | v1 frozen, v2 requires v4 |
| baselineBots.behaviorMode | Baseline variation | Deterministic/Stochastic | Stochastic | R | Teacher identity | Clean labels versus variation |
| baselineBots.controlDt | V2 decision interval | Number0.008..0.06 | 1/60 | R | Deployment cadence | Must match job/student |
| Existing count/seed/randomize/respawn delay | Existing labels | Existing ranges | Existing defaults | Existing semantics | Provenance | Preserve baseline experiments |

Recorded/imported defaults are not fresh defaults. Inactive/incompatible UI controls explain constraints rather than silently coercing.

### Distillation and diagnostics

| Path | UI label | Type/range | Initial default | Apply/durability | Tooltip/consequence |
|---|---|---|---|---|---|
| distillation.enabled | Teacher capture | Bool | Off | L/J | Diagnostic capture, no control change |
| distillation.seed | Training seed | UInt32 | 1 | J | Own RNG only |
| distillation.device | Training device | CPU/available CUDA | CUDA0 | J | Validate device; explicit CPU alternative |
| distillation.cpuThreads | Trainer CPU threads | Integer1..available | min(2,available) | J | Bound contention |
| distillation.deviceMemoryMiB | Device budget | 512..available | 8192 if admitted | J | Reject oversized job |
| distillation.learningRate | Learning rate | 1e-6..1e-2 | 3e-4 | J | Adam scale |
| distillation.batchSequences | Sequence batch | Integer1..256 | 32 | J | VRAM/work |
| distillation.sequenceLength | BPTT window | Integer16..256 | 64 | J | Gradient history |
| distillation.burnIn | Recurrent burn-in | Integer0..128 | 32 | J | Approximate truncated context |
| distillation.ramMiB | Replay RAM | Integer64..2048 | 256, qualify | J | Bounded retention |
| distillation.spoolGiB | Dataset disk | Integer1..64 | 4, qualify | J | Complete chunk rotation |
| distillation.shadowCount | Shadow candidates | Integer0..4 | 0 | L/J | Optional native inference |
| diagnostics.inspectorEnabled | Decision inspector | Bool | Off | P/L | One target on demand |
| diagnostics.followIndividual | Follow population individual | Bool | On | P | Reacquire physical life by slot |

Graph/sensor/teacher/cadence/life rule are immutable job identities; changes create job. Loss temperature is methodv1, not another slider. Start/stop/activate candidate/save/import/export/finish are commands. Ray limits are admitted engine configuration, not dozens of UI toggles.

## 19. Separate future stages

### Founders

Reset/New Run only: exact founder plus mutated descendants, or entire population mutations around compatible saved brain. Existing canonical graph/mutation/evolution RNG and durable run-start barrier; record artifact digest/mode. No foreign mid-generation insertion.

### DAgger

Student controls real snake; advisory teacher labels encountered observations with separate strategy/RNG. Teacher advisory does not change world or baseline streams. Same sample contract supports source identity/sequence; PyRL retrains; candidate deployment remains explicit. Addresses teacher-distribution coverage without a second simulator.

### Exact mid-round recovery

Separate running-state/delivery/wall-clock durability project using Section8 inventory. Neither trainer checkpoint nor periodic counters imply it.

## 20. Emergent interaction review

| Interaction | Origin and handling |
|---|---|
| Suicide/corpse farming | Fresh mass plus corpse food; corpse settings and emitted/discarded telemetry |
| Cheap cumulative death | Assessment survives body; finite lives/penalties selectable |
| Gross aggressive boost | Credits remain assessment despite spending; expose gross/spent/net |
| Newborn boost | Carried net bank, never gross credits |
| Best-life lottery | Unlimited repeated attempts; life/death count and finite alternative |
| Boost after best peak | Peak survives later spending; inspector shows peak/current difference |
| Reckless aggression/survival | Unlimited versus finite lives/bundles/penalties, not automatic correction |
| Cross-life memory | Preserve recurrence, reset motors, show life boundaries |
| Reorientation | Randomized safe placement/headings and post-respawn metrics |
| World RNG coupling | Shared placement selectable; corpses still world-owned under isolated placement |
| Physical ordering priority | New physical ID changes exact ties/shared draw assignment; documented/tests |
| Stochastic distillation | Intentional label noise/strategy history; recurrent student and strata |
| Population-relative moving target | v3 preserved, v4 stationary local economics |
| Absolute long survival | Multi-day selection differs; explicit normalized alternative |
| Wider mutation search | Counts/admission/ablation, no automatic mutation/species retuning |
| Intentionally limited sensors | v3/Basic can encourage memory/reactive policies |
| Capacity ecology | Discard-new resource limit and lost-value telemetry |
| Spawn congestion | Pending waiting remains experiment; no fake extinction |
| Environment-transfer mismatch | Same structural brain allowed under different numeric settings; provenance and gameplay evaluation |

Observe and reproduce these outcomes. Do not call them bugs merely because evolved policies exploit them. Preserve Rust authority, deterministic transactions, explicit RNG, compatibility and bounded retained capacity throughout implementation.

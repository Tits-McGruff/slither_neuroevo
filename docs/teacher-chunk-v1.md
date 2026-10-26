# Teacher chunk v1 binary contract

`native/src/engine/teacher_samples.rs` owns the bounded encoder and decoder.
The native publication/capture join, queue and coarse handoff are implemented.
The Node worker stores/routes opaque objects; it never decodes their samples.
The dedicated HTTP dataset/lease/download and arm/stop routes are implemented;
PyRL consumption and candidate upload/validation remain in progress.
A valid encoded chunk is descriptive data; codec validation does not certify
that a world step committed.

All integers and IEEE floats are little-endian. The eight-byte magic is ASCII
`SLTHTCH1`. A chunk is at most 1 MiB, contains 1..256 observations of width
1..504, and covers at most 60 completed 60 Hz steps between its first/last row.
The producer seals at size/time, life, generation, contract or capture boundaries.

The header has 168 fixed bytes plus the UTF-8 run identifier:

| Field in wire order | Bytes |
|---|---:|
| Magic | 8 |
| Capture epoch | 16 |
| Run identifier byte length | 1 |
| Run identifier | 1..128 |
| Config SHA-256 | 32 |
| Structural sensor SHA-256 | 32 |
| Teacher implementation/spec SHA-256 | 32 |
| Teacher revision (currently 2) | 4 |
| Teacher mode (0 deterministic, 1 stochastic) | 1 |
| Control interval seconds, Float64 (0.008..0.06) | 8 |
| Input width | 2 |
| Assigned chunk sequence | 8 |
| Generation | 8 |
| Dense baseline slot (0..119) | 4 |
| Full physical snake ID | 8 |
| Start reason | 1 |
| End reason | 1 |
| Sample count | 2 |

Every sample contains 25 fixed bytes followed by `inputWidth * 4` observation
bytes: step u64, sample ordinal u64, applied turn Float32, flags u8, strategy u8,
life stage u8, strata u16, then the exact normalized Float32 vector. Sample
ordinals count deliveries within the capture epoch/baseline stream. They do not
pretend to count pre-capture deliveries during a physical life.

Flag bits are requested boost, realized boost, post-step death and first physical
sample, from bit zero through three. Other bits reject. Request and realization
are distinct. Strategies encode Roam/Seek/Avoid/Boost as 0/1/2/3; life stages
Small/Medium/Large as 0/1/2. Strata bits are imminent hazard, pursuit/escape,
boost, transition, saturation and post-respawn. Applied turn and observations
are finite and in [-1,1]. Signed zero is preserved bit-for-bit.

Start reasons 0..4 are CaptureStart, PhysicalLifeStart, Continuation, Gap and
GenerationStart. End reasons 0..6 are MaximumSamples, SimulatedSecond,
LifeEnded, GenerationEnded, ContractChanged, CaptureStopped and Interrupted.
Size/time seals preserve recurrent continuity. Capture-start is distinct from
physical birth. Cold life-ending commands can seal between controls without
rewriting the preceding sample's post-step outcome.

Within a chunk, steps increase strictly and sample ordinals are contiguous.
No sample follows a committed death outcome in the same physical-life chunk.
Only its first row can be a first physical sample. A PhysicalLifeStart requires
that flag; a Continuation cannot set it. The decoder proves the exact remaining
payload size before reserving sample/observation storage and rejects unknown
versions/reasons/flags, malformed UTF-8, truncated or trailing bytes.

SHA-256 covers the exact complete encoded bytes and belongs to the external
descriptor. Storage/download validation must compare that digest and byte count
before advertising a completed immutable object.

The native publication join attaches outcomes only after complete world-step
publication or acknowledged generation publication. Realized boost means that
movement burned positive boost points in at least one collision substep. It is
accumulated across the whole fixed step; the final snake boost flag alone can
miss early boost followed by exhaustion. Death refers to the same physical snake
at that step's end.

`TeacherStreamCapture` consumes every published fixed step, including steps
without a due observation. It seals an existing fragment after sixty elapsed
steps or on physical death without creating an extra observation. Its row and
observation capacities are admitted cold; normal appends reuse that storage.
Sample ordinals continue across physical lives within each capture/baseline
stream. One-second seals use Continuation. A dropped fragment consumes both
sample and chunk sequences; the next fragment uses Gap.

`TeacherChunkQueue` owns a separate admitted ring and encoded payload budget,
including one maximum encoding buffer as headroom. Overflow drops the newly
sealed whole object. Its bounded drain transfers complete bytes and Rust-built
descriptors, with no controller reservation or consumption acknowledgement.
`production_watermark` includes drops; `released_through` identifies sequences
that have left native ownership or were dropped. The latter is **not** the
durable spool's `resolvedThrough`: Node must settle storage independently after
opaque bytes pass digest/length validation and complete durable publication.
The native background authority captures on ordinary complete publication,
resolved external delivery and acknowledged generation publication. A pending
generation's terminal observation is retained privately until its final
publication succeeds. Cold God Mode deaths seal existing physical streams
without rewriting the previous sample; a live config revision seals and stops
the old epoch while retaining its queued bytes.

The production `ExperimentalRunningAuthority` handle exposes:

- `submitStartTeacherCapture(sequence, request)`: exact own fields
  `captureEpoch`, `expectedRunId`, `expectedConfigRevision`, `baselineSlots`,
  `pendingBytes`, `queueChunks`, `queueBytes`. Epoch is 32 lowercase hex digits;
  revision/command sequence are exact 16-digit u64 words. Slot selection is
  strictly increasing and bounded to existing baseline-v2 owners. Run/config
  checks occur at the ordered clean boundary, followed by complete cold memory
  admission. Ordinary production v3 does not admit this teacher capture.
- `submitStopTeacherCapture(sequence, captureEpoch)`: rejects stale epochs and
  seals only the matching capture at a clean boundary. An epoch retains its
  queue after stopping; a new arm requires that prior queue to be drained.
- `drainTeacherChunks(maxChunks, maxOwnedBytes)`: transfers complete Rust bytes
  and descriptors independently of `drainOutputs`. Count is 1..256 and the
  ownership ceiling is at most 16 MiB, including returned native metadata.
  It returns the capture contract, exact `sensorContractJson`, native counters,
  stopped/error status and `moreWork` even without a chunk.

Teacher SHA-256 binds the `slither.baseline-teacher.v2\0` domain to the compiled
native source SHA-256's lowercase ASCII hex. Mode/revision/cadence are separate
contract fields. Node must persist unique epochs and exact native metadata; it
must not reinterpret a valid source digest as gameplay qualification.

The owner transfers at most 1 MiB through a separate mailbox using only
`try_lock`; a busy or slow consumer leaves chunks in the native queue, where new
whole chunks can drop. Queue admission also charges the mailbox and two bounded
sensor metadata copies. A coalesced payload-free diagnostic wake requests more
handoff service even when physics is blocked on delivery/durability. It consumes
no command sequence, reliable reservation or gameplay acknowledgement.
Partial mailbox drains keep `releasedThrough` below any untransferred object.
Orderly shutdown seals the last committed tail; failure labels it interrupted.
Stopped runtimes retain bounded opaque drain access. These APIs do not yet
provide candidate upload/validation; the dataset HTTP transport is described
in `docs/API-instructions.md`.

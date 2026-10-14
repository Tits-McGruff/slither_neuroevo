# Portable brain artifact v1

`.slither-brain` is a bounded neural artifact, separate from population saves and
trainer-job checkpoints. Rust independently compiles its source graph, rebuilds
the canonical training contract and checks finite canonical Float32 weights.
An accepted artifact is not automatically activated, resurrected, or saved into
the brain library. Spawning additionally requires the active graph and structural
sensor contract to match. Diagnostic shadows may use a different compiled graph;
their input contract and deployment cadence must match the selected teacher.

The complete file is at most 64 MiB. Integers and Float32 values are little-endian.
The fixed header is 88 bytes:

| Offset | Bytes | Meaning |
|---|---:|---|
| 0 | 8 | ASCII `SLTHBRN1` |
| 8 | 4 | Manifest UTF-8 byte count, at most 1 MiB |
| 12 | 1 | Numeric encoding: 0 raw-f32le-v1, 1 f32le-shuffle4-zstd-v1 |
| 13 | 3 | Reserved zero bytes |
| 16 | 4 | Canonical parameter count, at most 1,000,000 |
| 20 | 4 | Stored numeric payload byte count |
| 24 | 32 | SHA256 of decoded canonical Float32 bytes |
| 56 | 32 | SHA256 of stored numeric payload bytes |

The manifest follows immediately, then the complete stored payload. Their checked
length sum must equal the file length; truncation and trailing bytes reject. Raw
encoding requires exactly four payload bytes per parameter. Shuffled encoding
reuses the checkpoint codec's bounded independent `SFZ1`/Zstandard blocks and
all its decoded-length/frame/digest validation.

The manifest has exactly `schemaVersion: 1`, `graphManifest` and `provenance`.
`graphManifest` is the complete canonical training graph manifest v1, including
source graph, compiler key/digest, named parameter/state views, equations,
structural sensor bytes/digest, initialization and deployment contracts. Rust
compiles `graphSpec` and reconstructs every manifest field; supplied offsets,
ports, counts, equations and sensor/deployment claims cannot override its result.

Provenance has exactly `schemaVersion: 1`, `sourceKind` (`distilled`, `imported`,
`manual`) and an object-valued `details`. The complete provenance JSON is bounded
to 64 KiB, with bounded depth, fields and text. Provenance is immutable metadata;
it introduces no new snake kind or Hall-of-Fame winning claim. Trainer exports
include job/method/source identities, teacher/run/config identity, seed, updates,
processed sample counts, trained generation extent, validation and environment.
Numeric environment differences are provenance, not structural rejection rules.

The artifact ID is SHA256 of all complete file bytes. It identifies retries and
immutable candidate records. Different stored encodings can represent the same
canonical weight digest while having different artifact IDs.

The dedicated API currently provides:

- `POST /api/distillation/v1/candidates/<artifact-sha256>`: opaque binary body with
  exact Content-Length and application/octet-stream; returns a Rust descriptor.
- `GET /api/distillation/v1/candidates/<artifact-sha256>`: descriptor/byte-count
  metadata for an available candidate.

Existing browser origin admission and loopback/trusted-LAN configuration apply.
Candidate upload has one admitted request at a time; file/native/database work
runs in the diagnostic worker. Transient candidate retention is independent of
teacher data: at most eight records and 64 MiB, with the configured disk reserve.
Oldest transient candidates rotate as complete objects. It does not populate the
saved-brain catalog. IDs/digests make lost-response retries idempotent. Native
validation never submits a controller action or resolves a generation barrier.

Cold native validation also registers the immutable graph/weights in a bounded
64-MiB cache. Queued activations and active shadows pin their candidate; only
unreferenced candidates can be evicted. Four shadow slots are absent by default,
with a separate 64-MiB workspace including replacement staging. The coarse native
bridge exposes explicit activation, activation-scoped stop and bounded metrics;
HTTP/job shadow coordination remains pending. A cache miss requires cold
revalidation of the stored artifact before activation. These budgets do not
change population or teacher-spool admission.

The offline `brain_artifact_validate` tool takes a file and absent output directory,
then writes its independently reconstructed manifest, exact weights and descriptor.
It creates no world, controller, run RNG, or live recurrent state.

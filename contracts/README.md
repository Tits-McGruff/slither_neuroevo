# Slither contracts

One small cross-project contract project, shipped from this repository. The
Rust authority is `native/src/contracts.rs`; the engine imports those types.
`contracts.schema.json`, Python TypedDicts and browser types are generated using
Schemars, datamodel-code-generator and json-schema-to-typescript. Python installs
this directory as `slither-contracts`, pinned to an exact repository commit.
No extra repository, Rust/Python FFI or execution service is involved.

The definitions cover graph operations, parameter/state views, canonical training
metadata, brain provenance, teacher/dataset and external reward wire shapes,
format magic/header layouts, version and domain identities. Browser types include
graph, teacher/dataset and routed reward metadata. Game validation and mathematical execution stay in the
consumers. Python keeps validated tensor views and strict numeric/range checks.
TypedDict shapes provide exact field inventories without a second hand-maintained
list. Installed contract bytes participate in exact trainer restart identity.

External reward version 1 is structurally independent of sensor layout and binary
packet versions. `ExternalRewardEnvelopeV1` carries an initial/live/terminal/truncated
interval, explicit run/world/lease/physical/public identities, hexadecimal ordered
observation and step boundaries, and unweighted `ExternalRewardDeltasV1` quantities.
Food value and pellet count are distinct; growth and point deltas are signed.
Survival, food, kill, other point credits and boost spending preserve the economic
accounting identity. No field reconstructs an event from a sensor feature.

`external-reward-v1-fixture.json` is a compact interoperability fixture checked by
real native physics: both external snakes eat, one dies on the other's body, the
owner receives a kill, and generation cutoff closes the two lives with different
terminal/truncation semantics. PyRL checks this same packaged fixture.

Generation assignments retain an old-life reward closure and final sensors for
live truncation; terminal closures omit bootstrap sensors. Ordinary reward markers
commit with complete native output admission. A rebuilt server advertises reward
v1 and binary envelope v2, and Node routes closures before successor assignments.
Legacy peers retain their original format unless hello requests the extension.
See `docs/API-instructions.md` for negotiation and wire semantics. The behavioral plan is
[`docs/todo/ppo-reward-action-policy-repair-plan.md`](../docs/todo/ppo-reward-action-policy-repair-plan.md).

Regenerate after changing a native contract:

```sh
python -m pip install -e './contracts[dev]'
npm ci
python contracts/generate.py
```

CI regenerates and checks the diff. Generated JSON is a schema, not a source or
experiment archive. Existing canonical JSON declaration order, binary bytes and
domain-separated hashes are unchanged. Canonical equation field order remains
explicit because JSON object order is not a portable serialization contract.

Protobuf/FlatBuffers would replace existing compatible wire bytes or add a second
format/adapter. JSON Schema describes the existing cold metadata directly. The
small little-endian binary codecs keep their standard `struct`/Rust primitives;
sharing their definitions does not justify a new container or FFI runtime.

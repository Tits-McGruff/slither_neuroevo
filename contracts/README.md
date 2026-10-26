# Slither contracts

One small cross-project contract project, shipped from this repository. The
Rust authority is `native/src/contracts.rs`; the engine imports those types.
`contracts.schema.json`, Python TypedDicts and browser types are generated using
Schemars, datamodel-code-generator and json-schema-to-typescript. Python installs
this directory as `slither-contracts`, pinned to an exact repository commit.
No extra repository, Rust/Python FFI or execution service is involved.

The definitions cover graph operations, parameter/state views, canonical training
metadata, brain provenance, teacher/dataset wire shapes, format magic/header
layouts, version and domain identities. Browser types include only graph and
teacher/dataset metadata. Game validation and mathematical execution stay in the
consumers. Python keeps validated tensor views and strict numeric/range checks.
TypedDict shapes provide exact field inventories without a second hand-maintained
list. Installed contract bytes participate in exact trainer restart identity.

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

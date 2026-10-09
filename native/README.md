# Slither native inference kernels

This private napi-rs crate supplies the Dense, MLP, GRU, LSTM, and RRU math
kernels used by the Node simulation server. It does not implement world state,
physics, sensors, evolution, persistence, rendering, or networking.

The supported targets are x86_64 Windows MSVC and x86_64 Linux GNU. There is
no WASM build and no non-x86_64 scalar native fallback.

## Build and test

From the repository root:

```powershell
npm --prefix native run build
cargo test --manifest-path native\Cargo.toml --release
cargo fmt --manifest-path native\Cargo.toml -- --check
cargo clippy --manifest-path native\Cargo.toml -- -D warnings
```

The build generates `index.js`, `index.d.ts`, and one platform-specific
`slither-native.*.node` file in this directory. These outputs are ignored by
Git.

Normal server startup requires this addon. The JavaScript reference backend is
available only when explicitly selected for diagnosis with `--backend js`.
Threading is a separate setting: either backend can run without workers or
with the canonical pool enabled by `--mt` and optionally `--mt-workers N`.

## Safety boundary

Every public N-API call validates dimensions, lengths, writable aliasing, and
checked arithmetic before entering a private raw-pointer kernel. Keep each
unsafe block narrow and document its exact valid-range and non-overlap
contract. Do not add a second Rust implementation of the simulation world.

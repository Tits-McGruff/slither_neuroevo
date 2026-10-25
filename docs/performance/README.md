# Performance results

Keep selected human-readable results and compact checksums here. Run benchmark
programs under `scripts/qualification` explicitly or dispatch the Qualification
workflow; routine CI does not reproduce historical studies. Outputs belong in
CI artifacts. Significant published results may use an ordinary release asset
with its SHA-256 recorded beside the summary.

Experiments identify clean Git commits. Commit experimental code on a temporary
`experiment/...` branch or make a WIP commit before promotion. Explicit local dirty
runs must say they are dirty; never copy source trees into their outputs.

Historical payloads remain at the unchanged
[qualification evidence release](https://github.com/Tits-McGruff/slither_neuroevo/releases/tag/qualification-evidence-v1-20261024).
Its original archive is 99,140,262 bytes with SHA-256
`ba676e79474345879dda0717de4f60b3d4dec4d7e85a668f34bba6500a2ed39b`.
The old catalog, exceptional source patches and migration metadata can be inspected
at Git tag `qualification-resume-20261025`. Their migration is complete; there is
no ongoing source-tree reconstruction or historical qualification service.

The existing [pellet-distance result](sensor-v4/pellet-distance-screen-20261025/README.md)
retains its own compact summary and immutable asset receipt. Older reports below
are historical measurements: paths they name may refer to archived outputs.

Current correctness uses deliberate fixtures in `native/fixtures` and
`scripts/qualification/fixtures`, including learned cohorts, numerical
counterexamples and public aggregate resource counters. These do not depend on
historical release downloads. Existing semantic and native source-identity
fixtures remain supported.

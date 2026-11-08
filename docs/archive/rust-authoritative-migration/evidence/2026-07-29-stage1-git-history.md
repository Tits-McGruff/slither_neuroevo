# Stage 1 Git-history evidence

## Evidence class and repository identity

This file records Git-history evidence required by approved plan
`2026-07-29-draft-4`. The earlier audit ZIP had no `.git` directory, so none of
these claims is described as proof from that ZIP.

The evidence was reproduced on 2026-07-29 from repository
`slither_neuroevo`, implementation base
`46c2f634c4cf48b1c1d30b7b55e3373495773d4e`. Git retains the complete file and
diff bytes under the commit and blob identities below; the commands listed
here retrieve the exact contents without copying large historical files into a
second documentation archive.

## HIST-GOV-001: false owner-history documents

Commands:

```powershell
git show 3fe62d0:docs/todo/project-recovery-plan.md
git show 3fe62d0:docs/todo/native_refactor_plan.md
git diff 3fe62d0^ 3fe62d0 -- docs/todo/project-recovery-plan.md docs/todo/native_refactor_plan.md
git show 258ac69:AGENTS.md
git show 258ac69:README.md
git show 258ac69:docs/decisions/0001-native-kernels-and-threading.md
git diff 258ac69^ 258ac69 -- AGENTS.md README.md docs/decisions/0001-native-kernels-and-threading.md docs/todo/project-recovery-plan.md
```

Reproduced identities:

| Role | Commit/blob |
|---|---|
| First combined recovery-plan commit | `3fe62d0bdec4e9964f7f7c0da9d67ee4249612d2` |
| `project-recovery-plan.md` at that commit | `b7b4ceb1bed963f15967f5ceb67dc189820f3bc4` |
| `native_refactor_plan.md` at that commit | `1ba29abea6c27c9b3b0b2009de63a68c4de2fc00` |
| Expanded active-document commit | `258ac69e80df411fa724ad16f1b2cb19e1ae210c` |
| `AGENTS.md` at that commit | `283821cab91c97629686b36cd3e1b92435eacbf8` |
| `README.md` at that commit | `83ebee29fec6a29b3bbac1e2aaf23de1a6d3e14c` |
| ADR 0001 at that commit | `fabe7f0255381a6306089417041e12c762df3c82` |

The retrieved recovery plan says it is “authoritative, owner-approved,” says
the owner locked the rule that Rust was only the neural-kernel accelerator,
and says the owner explicitly rejected the complete Rust simulation. The
native-refactor warning, later `AGENTS.md`, README, and ADR 0001 repeat that
boundary. The owner has directly stated those attributions are false.

Result: the current documents are current-source proof of the false text; the
commit/blob table above is the separately reproduced Git-history evidence for
when it appeared.

## HIST-CTRL-002: ten-tick hold and twenty-tick release

Commands:

```powershell
git show 3989d26:server/config.ts
git show 3989d26:server/controllerRegistry.ts
git diff 3989d26^ 3989d26 -- server/config.ts server/controllerRegistry.ts
```

Reproduced identities:

| Role | Commit/blob |
|---|---|
| Historical controller commit | `3989d2653c1df97ebc6ca6284a5eb14cdb7c48c9` |
| `server/config.ts` | `7b33a5c97074688e9b10ea6ee15c6aa7cdc1e729` |
| `server/controllerRegistry.ts` | `ea64f7b5b7868e8e64864374a03c92dec9e05a41` |

The retrieved config sets `actionTimeoutTicks: 10`.
`ControllerRegistry.getAction()` holds the last action while
`delta <= actionTimeoutTicks`, calculates
`releaseAfter = actionTimeoutTicks * 2`, releases the snake when
`delta > releaseAfter`, and returns neutral control between those thresholds.

Result: the 10-tick/20-tick historical behavior reproduces. The approved
replacement uses the separately selected 500 ms input hold and 30-second
disconnect grace in wall time.

## HIST-RUST-001: incomplete Rust game at commit 8330065

Commands:

```powershell
git show 8330065:native/src/lib.rs
git diff 8330065^ 8330065 -- native/src/lib.rs native/index.d.ts README.md
```

Reproduced identities:

| Role | Commit/blob |
|---|---|
| Historical Rust commit | `8330065dfdb00c93314129ae2ddbb43c89dfc8d4` |
| `native/src/lib.rs` | `06be86b0a6b6742a716f8e63136e12fb3de49227` |

The complete historical file contains hard-coded snake/world constants, a
fixed 16-bin v2-style partial sensor vector with placeholder channels, only
head positions in its spatial hash, fixed `0.016` steering/movement deltas,
head-to-head circle collision only, and per-call/per-step `Vec` allocation and
collection in neural and sensor paths.

Result: the reported defects reproduce. The file remains historical evidence;
the approved migration reads current TypeScript behavior and does not restore
this implementation.

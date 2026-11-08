# Stage 2 external RL trainer contract audit

## Scope and evidence class

This is a source audit of the separate
[`PyRL-trainer`](https://github.com/Tits-McGruff/PyRL-trainer) repository at
commit `5932884fa35065f19eabb1abb05b39ab3a3c112f`, compared with the current
source in this repository. It is not a successful connection test, a trainer
benchmark, or evidence that either program has been changed. No file in the
external repository was modified.

- **External-repository proof** below is limited to that exact trainer commit.
- **Current-source proof** below is limited to this repository's checked-out
  Protocol 2 implementation.
- The compatibility conclusion is derived by comparing those two source facts.
  A live trainer run remains required after a coordinated update.

## Current server proof: Protocol 2 is required

`server/protocol.ts` exports `PROTOCOL_VERSION = 2`. Its
`getProtocolVersionError()` names a mismatched `hello` version explicitly, and
`isHello()` accepts only version 2. `server/wsHub.ts::handleMessage()` checks
that mismatch before generic message parsing; `WsHub.protocolError()` sends an
error where possible and closes with WebSocket code `1008`. The same public
contract is documented in `docs/API-instructions.md` under “Protocol 2
connection sequence.” `server/integration.test.ts` also covers a version-1
hello being closed with `1008`.

On 2026-08-02, the focused command shown below passed all seven tests,
including that Protocol 1 rejection case. This verifies the current server
boundary only; it is not a live external-trainer test.

The current server supports opaque `resumeToken` reclaim and reports reclaim
outcomes including `ambiguous` (`server/protocol.ts`,
`server/controllerRegistry.ts::reclaimSnake()`). The approved migration also
requires a reliable `state-replaced` lifecycle result after reset, New Run or
import; see the Stage 6A/controller requirements in
`docs/todo/rust-authoritative-runtime-plan.md`.

## External trainer proof at the named commit

| Area | Evidence | Consequence |
|---|---|---|
| Handshake version | `pyrl_trainer/config.py` sets `PROTOCOL_VERSION = 1`. Both `pyrl_trainer/agent.py::Actor._handshake()` and `pyrl_trainer/__main__.py::discover_obs_dim()` send it in `hello`. | The trainer cannot complete the current server handshake. |
| Default connection shape | `Config` in `pyrl_trainer/config.py` defaults to `ws://localhost:5174` and four actors. | A LAN deployment needs its configured WebSocket URL changed, but that is secondary to the version mismatch. |
| Startup retry | `__main__.py::main()` makes one `discover_obs_dim()` attempt and returns on connection/protocol failure. | Initial server unavailability is not retried by the main process. |
| Actor reconnect | `agent.py::Actor.run()` catches connection/loop failures and waits `0.5` seconds before retrying. | Established actors retry, provided the process got past startup. |
| Control shape | `_handshake()` joins in `player` mode and sends `{"type":"viz","enabled":false}`. `_loop()` ignores byte messages; `_handle_sensors()` constructs a JSON `action` only after a JSON `sensors` message. | This is an observation-driven JSON client, consistent with the approved first-cutover direction; it does not require browser-player periodic cadence or binary frames. |
| Reclaim/lifecycle handling | The trainer accepts `assign`, `error`, `stats`, and `sensors` in `Actor._loop()`. Its join omits `resumeToken`; it does not branch on `ambiguous` reclaim or `state-replaced`. Rejoining by the same bounded actor name can still use the server's legacy unambiguous identity path when conditions permit. | It does not use the preferred reliable reconnect/lifecycle surface and needs an explicit update/test for it. |
| Trainer persistence | `pyrl_trainer/learner.py::save_checkpoint()` writes the trainer model as `.pt` files in its own checkpoint directory. | These are trainer-model files, not Slither experiment checkpoints or population archives. |

## Derived compatibility conclusion

At the audited trainer revision, its first `hello` is version 1 and the
current server rejects it before `welcome`. Therefore it cannot presently
serve as the approved “existing Protocol 2 trainer” acceptance client. This is
a compatibility blocker, not authorization to weaken the server or accept
Protocol 1.

The first Rust-authoritative cutover must preserve the current Protocol 2 JSON
handshake, sensors, actions, assignment, death/reassignment and reconnect
semantics. A coordinated external-trainer update must first change the trainer
to Protocol 2 and then be exercised against the server. That update, any
change to trainer behavior, and any change to the external contract require
owner review before finalization. No Protocol 1 compatibility shim is planned.

## Reproduction commands

The following commands inspect source only after obtaining a disposable clone;
they do not modify either repository's tracked content. Substitute a temporary
path for `$audit`.

```powershell
$audit = "$env:TEMP\slither-pyrl-trainer-contract-audit"
git clone https://github.com/Tits-McGruff/PyRL-trainer $audit
git -C $audit checkout --detach 5932884fa35065f19eabb1abb05b39ab3a3c112f
git -C $audit rev-parse HEAD
rg -n -S "PROTOCOL_VERSION|hello|resumeToken|state-replaced|ambiguous|viz|sensors|action|sleep\(0\.5|torch\.save" "$audit\pyrl_trainer"

rg -n -S "PROTOCOL_VERSION|getProtocolVersionError|isHello|protocolError|resumeToken|ambiguous|state-replaced" server docs\API-instructions.md
node .\node_modules\tsx\dist\cli.mjs scripts\run-tests.ts integration --reporter=dot
```

The final command validates this repository's existing integration coverage;
it does not test the external trainer. A real connection test requires the
trainer's Protocol 2 update and a configured server endpoint.

## Remaining uncertainty and required work

- The trainer audit is source-only. Its installed dependencies, runtime
  behavior, model-training throughput and actual reconnect behavior were not
  measured.
- The user may have a different local trainer revision or configuration than
  the audited public commit. That copy must be identified before compatibility
  support is narrowed or retired.
- The exact update design (including resume-token storage and handling of
  `ambiguous` and `state-replaced`) has not been approved or implemented.
- A desktop-to-Oxygen LAN test remains open after a coordinated update; it
  must cover assignment, observations, actions, death/reassignment, reconnect
  and a reset/import state replacement.

# Local Server API and Bot Client Guide

## Scope

The Slither Neuroevolution server is authoritative. External bot clients send
turn and boost commands; the server owns simulation time, physics, sensors,
randomness, evolution, and persistence.

This is a hobby-project API for loopback or deliberate use on the owner's
trusted home LAN, not a public service. The default listeners are:

- WebSocket and HTTP API: `ws://127.0.0.1:5174` and
  `http://127.0.0.1:5174`.
- Vite UI: `http://127.0.0.1:5173`.

The server has no accounts, authentication, authorization, TLS termination,
or hardened public-hosting boundary. Trusted-LAN clients may connect when
`host` and `uiHost` bind to a LAN interface (or `0.0.0.0`). Do not forward the
ports through a router or expose them on an untrusted network.

`publicWsUrl` is the WebSocket address injected into the webpage when the
simulation server's hostname differs from the UI hostname. It can be set in
TOML, through `PUBLIC_WS_URL`, or with `--public-ws-url`. The browser resolves
its server address in this order: `?server=...`, the saved browser override,
`publicWsUrl`, the current UI hostname plus the configured server port, then
the localhost fallback. The legacy word “public” means “advertised to the
browser”; it is not a security claim.

For a typical trusted-LAN setup, use:

```toml
host = "0.0.0.0"
uiHost = "0.0.0.0"
publicWsUrl = "ws://192.168.1.25:5174"
```

Replace the example address with the computer's actual home-network address.
On Windows, allow Node.js on the Private firewall profile (or allow inbound TCP
5173 and 5174 on that profile). The launchers print the usable network URLs.
CORS permits the separate UI and API origins; it is not authentication.

The server writes `server/config.toml` with defaults when the file does not
exist. Command-line and environment overrides are described by
`node ./node_modules/tsx/dist/cli.mjs server/index.ts --help` and in
`server/config.ts`.

## Protocol compatibility

The WebSocket protocol version is exactly `2`. A client must send a Protocol 2
`hello` before any other message. Protocol 1 is incompatible and is rejected
with an explicit error before the socket closes with code `1008`.

The binary world-frame serializer is version `1`. Protocol and serializer
versions are independent and both appear in `welcome`.

Client messages are UTF-8 JSON text and use strict schemas:

- Unknown fields are rejected.
- Required fields must be present.
- Numeric fields must be finite numbers.
- A message may be at most 64 KiB by default.
- Binary client messages, invalid JSON, invalid schemas, and lifecycle errors
  produce an `error` message and close code `1008`.
- Correlation `requestId` values must be non-empty strings of at most 64
  characters.

Server JSON messages may gain fields in a later protocol version. Clients
should switch on `type` and ignore fields they do not use after confirming the
protocol version.

## Connection lifecycle

A bot-player connection follows this sequence. Replace `127.0.0.1` with the
printed LAN address when the bot runs on another trusted home device:

1. Open `ws://127.0.0.1:5174`.
2. Send `hello` with `clientType: "bot"` and `version: 2`.
3. Read `welcome` and verify its protocol, sensor, serializer, seed, and
   inference metadata.
4. Send `join` with `mode: "player"` and a non-blank name.
5. Read `assign` to learn the controlled `snakeId`.
6. For each `sensors` message, send an `action` for that assigned snake.
7. If the controlled snake dies, read the replacement `assign` and use its new
   `snakeId`.

A UI client may join as a `spectator` or `player`. The authoritative control
messages `settings`, `godMode`, `reset`, and `newRun` require a joined UI
client. Bots cannot invoke them.

The server does not implement an application-level idle timeout. `ping` is
accepted as a no-op and has no matching `pong` response.

## Client-to-server JSON messages

### `hello`

The first message on a connection:

```json
{
  "type": "hello",
  "clientType": "bot",
  "version": 2
}
```

- `clientType` is exactly `"bot"` or `"ui"`.
- `version` must equal `2`.
- Sending a second `hello` is a protocol error.

### `join`

Register as a spectator or request a controlled snake:

```json
{
  "type": "join",
  "mode": "player",
  "name": "trainer-1"
}
```

- `mode` is exactly `"spectator"` or `"player"`.
- `name`, when present, is at most 24 characters.
- Player mode requires a non-blank name at the server behavior boundary.
- Joining spectator mode releases any snake assigned to that connection.

### `action`

Update the held input for the assigned snake:

```json
{
  "type": "action",
  "tick": 312,
  "snakeId": 100000,
  "turn": -0.25,
  "boost": 1
}
```

- `tick` is the most recent authoritative sensor tick observed by the client.
  It is recorded for diagnostics but is not required to equal the current
  server tick.
- `snakeId` must match the connection's current assignment. A mismatched ID is
  ignored.
- `turn` is clamped to `[-1, 1]`.
- `boost` is clamped to `[0, 1]`.
- The latest accepted input is held until another action is accepted or the
  assignment is released.

Default limits are one accepted action per authoritative tick and 120 action
attempts per wall-clock second per controller. Excess actions are dropped
without an acknowledgement. Send one action in response to each sensor packet
instead of flooding the socket.

### `ping`

Optional no-op heartbeat:

```json
{
  "type": "ping",
  "t": 1721620000000
}
```

`t` is optional and may be any finite number. The server sends no `pong`.

### `view`

Accepted for UI compatibility:

```json
{
  "type": "view",
  "viewW": 1920,
  "viewH": 1080,
  "mode": "follow"
}
```

`viewW` and `viewH` are optional finite numbers. `mode` is optionally
`"overview"`, `"follow"`, or `"toggle"`. The current server accepts this
message but does not use it to mutate the authoritative world.

### `viz`

Enable or disable neural-visualization data in stats:

```json
{
  "type": "viz",
  "enabled": true
}
```

Visualization works in both serial and multi-threaded inference modes. It is
intended for UI clients but structurally accepted from any joined client.

### `settings` (joined UI only)

Queue one atomic group of live setting updates for the next fixed-step
boundary:

```json
{
  "type": "settings",
  "requestId": "settings-17",
  "updates": [
    { "path": "simSpeed", "value": 12 },
    { "path": "baselineBots.respawnDelay", "value": 5 }
  ]
}
```

The request must contain between 1 and 64 updates. Only paths marked live in
`src/protocol/settingDefinitions.ts` are accepted. The complete request is
normalized and applied atomically, or rejected without incrementing
`configRevision`. Successful results are broadcast to all joined UIs as
`settingsApplied`; rejection is returned to the requester.

### `godMode` (joined UI only)

Kill one snake through the normal death path:

```json
{
  "type": "godMode",
  "requestId": "god-8",
  "action": "kill",
  "snakeId": 42
}
```

Or translate its complete body:

```json
{
  "type": "godMode",
  "requestId": "god-9",
  "action": "move",
  "snakeId": 42,
  "x": 125.5,
  "y": -80
}
```

The command is queued for a fixed-step boundary. Movement is clamped so the
body remains within the world. The requester receives `godModeResult`.

### `reset` (joined UI only)

Rebuild generation one with the current seed and a new run ID:

```json
{
  "type": "reset",
  "settings": {
    "snakeCount": 300,
    "simSpeed": 1
  },
  "updates": [
    { "path": "sense.bubbleBins", "value": 16 }
  ],
  "graphSpec": null
}
```

All fields after `type` are optional. `settings` accepts only the core settings
defined in `server/protocol.ts`; `updates` accepts the reset/import paths in
`src/protocol/settings.ts`; `graphSpec` is an object or `null`.

Reset is a deterministic experiment restart: the same seed and authoritative
configuration reconstruct the same initial state. Before the new run is made
current, the server writes a durable run-start checkpoint. A failed transition
produces `error`; there is no separate successful reset acknowledgement.

### `newRun` (joined UI only)

Start generation one with a new entropy-derived seed and a new run ID:

```json
{
  "type": "newRun",
  "requestId": "new-run-3"
}
```

The server preserves existing snapshots and returns `newRunResult` only after
the new run-start checkpoint is durable. New Run is rejected when durable
persistence is unavailable.

## Server-to-client JSON messages

### `welcome`

Sent immediately after a valid `hello`:

```json
{
  "type": "welcome",
  "protocolVersion": 2,
  "sessionId": "process-session-id",
  "tickRate": 60,
  "worldSeed": 123456789,
  "runId": "lineage-id",
  "configRevision": 0,
  "configHash": "sha256-content-identity",
  "settings": {
    "core": {},
    "updates": []
  },
  "inferenceMode": {
    "requestedBackend": "native",
    "activeBackend": "native",
    "requestedMt": false,
    "activeWorkerCount": 0,
    "poolEpoch": null,
    "weightEpoch": null,
    "graphKey": "graph-key",
    "parameterCount": 1234,
    "seed": 123456789,
    "nativeAddonStatus": "ready",
    "nativeAddonBuildIdentifier": "source-derived-id"
  },
  "sensorSpec": {
    "sensorCount": 83,
    "order": [],
    "layoutVersion": "v3"
  },
  "serializerVersion": 1,
  "frameByteLength": 4096
}
```

The example abbreviates `settings` and `sensorSpec.order`; real messages
contain the complete authoritative values. Treat `inferenceMode` as two
independent axes: `requestedBackend`/`activeBackend` describe native versus JS
math, while `requestedMt`/`activeWorkerCount` describe serial versus worker
execution. Normal mode requires native. JS is an explicit diagnostic mode,
not an automatic fallback.

`configRevision` is a monotonic accepted-change sequence. `configHash` is a
canonical content identity, so returning to an older configuration can repeat
a hash at a newer revision.

### `assign`

Sent after player join and whenever a dead controlled snake is replaced:

```json
{
  "type": "assign",
  "snakeId": 100000,
  "controller": "bot"
}
```

`controller` is `"bot"` for a bot client and `"player"` for a UI player.

### `sensors`

Sent at the stable pre-movement observation boundary of each completed fixed
step for a controlled snake:

```json
{
  "type": "sensors",
  "tick": 312,
  "snakeId": 100000,
  "sensors": [0.0, 1.0],
  "meta": { "x": 10.5, "y": -4.25, "dir": 1.57 }
}
```

The example sensor array is abbreviated. Use `welcome.sensorSpec.sensorCount`
and `welcome.sensorSpec.order`; do not hard-code the default length. `meta` is
optional in the type but the current server includes the controlled snake's
pose.

### `stats`

Broadcast about once per wall-clock second to every joined client. It contains
the last committed `tick`, generation number and timing, population and
baseline-bot alive counts, pump rate, and optional fitness, bounded history,
visualization, and Hall-of-Fame data. Optional payloads are omitted when there
is no new value to send.

### `settingsApplied`

```json
{
  "type": "settingsApplied",
  "requestId": "settings-17",
  "applied": true,
  "updates": [
    { "path": "simSpeed", "value": 12 }
  ],
  "configRevision": 1,
  "configHash": "sha256-content-identity",
  "sequence": 4,
  "step": 313
}
```

On rejection, `applied` is false, `updates` is empty, `reason` is present, and
`sequence`/`step` are absent. A successful response reports normalized
authoritative values and is broadcast to all joined UIs.

### `godModeResult`

Reports `requestId`, `action`, `snakeId`, and `applied`. Successful results
also include the accepted `sequence` and boundary `step`. A move can include
actual `x`/`y`; a kill can include `pelletsDropped`. Rejection includes
`reason` and does not stop the server.

### `newRunResult`

```json
{
  "type": "newRunResult",
  "requestId": "new-run-3",
  "applied": true,
  "worldSeed": 987654321,
  "runId": "new-lineage-id"
}
```

On rejection, `applied` is false and `reason` is present. Existing saved rows
are not deleted by New Run.

### `error`

```json
{
  "type": "error",
  "message": "human-readable reason"
}
```

Protocol errors are followed by socket close code `1008`. Operational errors
such as a failed reset or a faulted simulation may be reported without closing
the socket.

## Sensor contract

The only supported sensor layout is `v3`. Its size is:

```text
19 + 4 * max(8, floor(bubbleBins))
```

With the default 16 bins, `sensorCount` is 83. The first 19 values are:

1. `heading_sin`
2. `heading_cos`
3. `size_norm`
4. `boost_margin`
5. `points_pct`
6. `speed_norm`
7. `boost_state`
8. `points_norm`
9. `points_delta_norm`
10. `length_norm`
11. `boost_points_frac`
12. `boost_cost_norm`
13. `wall_dist_norm`
14. `nearest_food_dist_norm`
15. `nearest_food_dir_sin`
16. `nearest_food_dir_cos`
17. `nearest_body_dist_norm`
18. `nearest_head_dist_norm`
19. `age_norm`

`points_delta_norm`: Score change accumulated since this snake's previous
delivered sensor sample, or since construction for its first sample; unsampled
control intervals accumulate. The value is divided by 10 and clamped to
[-1, 1].

The remaining channels contain `bubbleBins` values each, in this order:

1. `food_0` through `food_N-1`
2. `hazard_0` through `hazard_N-1`
3. `wall_0` through `wall_N-1`
4. `head_0` through `head_N-1`

The `welcome.sensorSpec.order` array is the definitive index-to-label mapping.
It must agree with the neural input size for the active graph.

## UI binary frames

Joined UI clients also receive binary world frames; bot clients do not. The
serializer version is reported in `welcome`. Version 1 is a packed
`Float32Array`:

1. Seven-float header: generation, snake count, alive count, world radius,
   camera X, camera Y, zoom.
2. One block per alive snake: ID, radius, skin flag, X, Y, direction, boost
   flag, point count, then `pointCount * 2` body coordinates.
3. Pellet count, followed by five floats per pellet: X, Y, value, type, color
   ID.

Consumers should use `src/protocol/frame.ts`; pointer arithmetic is a hard
contract shared with the serializer and renderer.

## HTTP API

The HTTP routes share port 5174 with WebSocket upgrade handling. Request bodies
are JSON and are limited to 50 MiB. These unauthenticated routes are intended
only for the local UI and local tooling.

### `GET /health`

Returns `{ "ok": true, ... }` plus current tick, connected client count,
inference mode, scheduler diagnostics, fault state, run identity,
`configRevision`, and `configHash`.

### `POST /api/save`

Writes the current population as a typed, non-resumable `population-export`
snapshot and returns `{ "ok": true, "snapshotId": number }`. This is a
population transfer, not a complete mid-tick checkpoint. Automatic generation
and run-start checkpoints are separate resumable records.

### `GET /api/export/latest`

Streams the newest snapshot as JSON without constructing one
population-sized JSON string. Returns 404 when the database has no snapshots.
The payload includes `generation`, `archKey`, `genomes`, `cfgHash`,
`worldSeed`, and available settings/run/boundary metadata.

### `POST /api/import`

Accepts an exported snapshot directly or as `{ "payload": snapshot }`.
Required fields are `generation`, `archKey`, a non-empty `genomes` array,
`cfgHash`, and `worldSeed`. Use `?force=1` or top-level `force: true` to
override a configuration-hash mismatch deliberately.

Import replaces compatible population genomes at a recurrent reset boundary;
it does not apply the exported seed. Success reports `importedWorldSeed`,
`activeWorldSeed`, `seedApplied: false`, and a metadata-only seed disposition.
Use Protocol 2 New Run for a new seed or Reset for a same-seed reconstruction.

### `POST /api/resurrect`

Accepts a genome directly or as `{ "genome": genome }`. A genome contains a
non-empty `archKey`, a finite `weights` array, and optional `brainType` and
`fitness`. Success returns the spawned `snakeId`.

### Graph presets

- `GET /api/graph-presets?limit=50` lists preset metadata; limit is clamped to
  1 through 200.
- `GET /api/graph-presets/:id` loads one preset or returns 404.
- `POST /api/graph-presets` accepts `{ "name": string, "spec": object }` and
  returns `presetId`.

### Hall of Fame

- `GET /api/hof?limit=50` returns `{ "ok": true, "hof": [...] }`.
- `POST /api/hof` accepts `{ "hof": [...] }` and replaces/saves the supplied
  entries.

Unknown routes return 404.

## Minimal Node bot

Install dependencies in the repository, start the server, then run an ES
module containing:

```js
import WebSocket from 'ws';

const socket = new WebSocket('ws://127.0.0.1:5174');
let snakeId = null;

socket.on('open', () => {
  socket.send(JSON.stringify({
    type: 'hello',
    clientType: 'bot',
    version: 2
  }));
});

socket.on('message', (data, isBinary) => {
  if (isBinary) return;
  const message = JSON.parse(data.toString());

  switch (message.type) {
    case 'welcome':
      if (message.protocolVersion !== 2) {
        throw new Error(`Unsupported protocol ${message.protocolVersion}`);
      }
      console.log('run', {
        seed: message.worldSeed,
        backend: message.inferenceMode.activeBackend,
        workers: message.inferenceMode.activeWorkerCount,
        sensors: message.sensorSpec.sensorCount
      });
      socket.send(JSON.stringify({
        type: 'join',
        mode: 'player',
        name: 'example-bot'
      }));
      break;

    case 'assign':
      snakeId = message.snakeId;
      break;

    case 'sensors':
      if (message.snakeId !== snakeId) break;
      socket.send(JSON.stringify({
        type: 'action',
        tick: message.tick,
        snakeId,
        turn: 0,
        boost: 0
      }));
      break;

    case 'error':
      console.error('server error:', message.message);
      break;
  }
});

socket.on('close', (code, reason) => {
  console.log('closed', code, reason.toString());
});
```

## Common integration mistakes

- Sending Protocol 1 or omitting `hello`.
- Sending binary client messages instead of JSON text.
- Adding unknown keys to otherwise valid messages.
- Joining player mode without a non-blank name.
- Continuing to use an old `snakeId` after a replacement `assign`.
- Flooding actions instead of replying once per sensor sample.
- Hard-coding 83 sensors instead of using `welcome.sensorSpec`.
- Treating `requestId`, accepted `sequence`, and boundary `step` as the same
  identity.
- Treating `configRevision` as configuration content identity instead of using
  `configHash`.
- Treating a population export as an exact resumable checkpoint.
- Assuming an imported `worldSeed` changes the active run.
- Assuming JavaScript is a transparent fallback when native loading fails.
- Exposing the unauthenticated local server to another machine.

# Phase 2 plan: Client reroute to server frames

## Purpose and scope

This phase moves the browser client from worker-driven simulation to
server-driven rendering. The client becomes a renderer and does not advance the
simulation locally unless the server is unavailable. The worker remains as a
fallback so the app can still run offline or during development without the
server.

## Architecture narrative

The client maintains a WebSocket connection to the server and treats it as the
primary source of frames and stats. The render loop stays unchanged: it draws
whatever buffer is stored in `currentFrameBuffer`. The only difference is the
source of that buffer. A small WS client module isolates connection logic and
exposes callbacks to the UI. A connection state machine chooses between server
frames and the fallback worker.

## Decisions locked for Phase 2

The default server URL is `ws://localhost:5174`. The UI frame rate target is 30
Hz, but the client renders only what it receives. Worker fallback remains
enabled, and connection status is visible in the UI.

## Module changes

### Pseudocode: `src/net/wsClient.ts`

This module owns WebSocket connection logic and emits events to the UI. It does
not manipulate DOM elements or render buffers directly. It is responsible for
resolving the initial connection, parsing inbound messages, and reporting
connection state transitions.

Interface sketch:

```ts
export interface WsClientCallbacks {
  onConnected: (info: WelcomeMsg) => void;
  onDisconnected: () => void;
  onFrame: (buffer: ArrayBuffer) => void;
  onStats: (msg: StatsMsg) => void;
  onAssign?: (msg: AssignMsg) => void;
  onSensors?: (msg: SensorsMsg) => void;
  onError?: (msg: ErrorMsg) => void;
}

export interface WsClient {
  connect(url: string): void;
  disconnect(): void;
  sendJoin(mode: "spectator" | "player", name?: string): void;
}
```

### Pseudocode: `src/main.ts` integration

`main.ts` resolves the server URL, creates the WS client, attaches callbacks
that update `currentFrameBuffer` and stats, and starts a fallback timer that
launches the worker if WS does not connect. It also updates the connection
status indicator in the UI.

State flow narrative: the client starts connecting to the server, uses server
frames once connected, starts the worker if the connection times out or drops,
and switches back to server frames if the server reconnects later.

## Server URL resolution

Resolution order is: the query param `?server=ws://host:port`, then the last
successful URL stored in `localStorage.slither_server_url`, and finally the
default `ws://localhost:5174`. The client saves the URL after a successful
connection.

## Worker fallback rules

If WS does not connect within two seconds, start the worker. If WS disconnects
after being primary, start the worker. If WS reconnects, stop the worker and
prefer WS frames. Stopping the worker terminates it and clears any worker-only
state like pending exports.

When switching away from server mode, the client must invalidate any
server-issued identity. That includes `sessionId`, `snakeId`, and any cached
`sensorSpec` assumptions. On reconnect, the client performs a fresh join and
accepts a new assignment rather than reusing old identity. This avoids subtle
bugs where a reconnecting client sends actions for a stale snake id.

## UI changes

Add a small connection status indicator using the element id
`connectionStatus`. The text should be `Server`, `Worker`, or `Connecting`, and
the color should be green for Server, blue for Worker, and yellow for
Connecting.

## Protocol handling in the client

The client handles only a subset of protocol messages. It stores `tickRate` and
`sensorSpec` from `welcome`, keeps `snakeId` from `assign` for later phases,
updates UI stats from `stats`, and ignores `sensors` for now while still
providing a handler for future use.

Binary frames are handled separately and must never be JSON parsed. The WS
message handler should branch on the message type and treat `ArrayBuffer`
payloads as frame buffers.

## Detailed design notes

The WS client should treat connection loss as normal and fall back gracefully
to the worker without user intervention. The fallback timer prevents the UI
from stalling on startup if the server is not running. When a connection does
exist, the worker should be terminated quickly to avoid dual rendering and to
ensure that the UI reflects server authority rather than local simulation.

Frame handling should be low overhead. The WS handler should store the latest
frame buffer and return immediately; rendering happens only on the animation
frame so the UI stays smooth. This separation prevents a burst of incoming
frames from forcing extra renders or main-thread jank.

Connection status is a user experience feature and a debugging tool. It should
be kept accurate and updated on every state transition so testers can quickly
tell whether they are looking at server or worker output. This is especially
important when the server is running on a different port or machine.

## Tests

Unit tests cover URL resolution and connection state transitions. Integration
tests use a mock WebSocket server to verify that the client renders a received
frame without starting the worker.

## Footguns

Never run the worker and WS rendering paths simultaneously. Always treat binary
messages as `ArrayBuffer`, and avoid rendering on every WS message; rendering
should happen on the animation frame using the latest buffer.

## Acceptance criteria

The client renders server frames when the server is available, falls back to
the worker when the server is unavailable, and shows an accurate connection
status indicator.

## Execution checklist

- [x] Add `src/net/wsClient.ts`
- [x] Implement URL resolution
- [x] Integrate WS client into `main.ts`
- [x] Add connection status UI
- [x] Implement worker fallback logic
- [x] Add URL + state unit tests

## Function-by-function pseudocode

### `src/net/wsClient.ts`

```text
function connect(url):
  socket = new WebSocket(url)
  socket.onopen = () => send hello + join
  socket.onmessage = (evt) => handleMessage(evt)
  socket.onerror = () => callbacks.onError
  socket.onclose = () => callbacks.onDisconnected

function handleMessage(evt):
  if evt.data is ArrayBuffer:
    callbacks.onFrame(evt.data)
  else:
    msg = JSON.parse(evt.data)
    switch msg.type:
      case "welcome": callbacks.onConnected(msg)
      case "stats": callbacks.onStats(msg)
      case "assign": callbacks.onAssign(msg)
      case "sensors": callbacks.onSensors(msg)
      case "error": callbacks.onError(msg)

function sendJoin(mode, name):
  socket.send(JSON.stringify({ type:"join", mode, name }))

function disconnect():
  socket.close()
```

### `src/main.ts` integration

```text
function startClient():
  url = resolveServerUrl()
  ws = createWsClient(callbacks)
  ws.connect(url)
  setTimeout(() => if not connected then startWorker(), 2000)

function onFrame(buffer):
  currentFrameBuffer = buffer

function onDisconnected():
  if worker not running:
    startWorker()
```

## Error handling and edge cases

If the WS connection fails or times out, the client immediately starts the
worker to keep the UI alive. If a JSON message is malformed or missing `type`,
the client logs a warning and ignores it rather than crashing the render loop.
If binary frames arrive before `welcome`, the client still stores them but
continues to treat the server as connected only after `welcome` is received.

## Sample payloads and example WS session transcript

```json
{ "type": "welcome", "sessionId": "abc", "tickRate": 60, "worldSeed": 123, "cfgHash": "...", "sensorSpec": { "sensorCount": 128, "order": [] } }
```

```json
{ "type": "stats", "tick": 120, "gen": 3, "alive": 42, "fps": 60 }
```

Example WS session transcript:

```text
client -> server: {"type":"hello","clientType":"ui","version":1}
server -> client: {"type":"welcome",...}
client -> server: {"type":"join","mode":"spectator"}
server -> client: <binary frame>
server -> client: {"type":"stats",...}
```

## Test matrix

| Test name | Setup / input | Expected result | Failure cases to verify |
| --- | --- | --- | --- |
| resolve_url_query_param | `?server=ws://x:1234` in URL | URL resolves to query param | Query ignored and default used |
| ws_connect_timeout | Server not running | Worker starts after 2s | UI stays blank or stuck in connecting |
| ws_reconnect_switch | Worker running, server comes online | Worker stops, WS becomes primary | Both worker and WS render simultaneously |
| binary_frame_handling | Receive ArrayBuffer message | Buffer stored, no JSON parse | Attempt to JSON parse binary message |

# Phase 3 plan: Controller override + sensors to clients

## Purpose and scope

This phase enables external control of a snake via WebSocket actions. The
server sends raw sensor vectors to the owning client each tick, and the client
responds with control inputs (turn and boost). Uncontrolled snakes continue
using internal brains. This is the core multiplayer feature.

## Architecture narrative

The server already owns the World loop. This phase introduces a controller
registry that maps connections to snakes and tracks last actions with timeouts
and rate limits. Each tick, the server computes sensors for controlled snakes
and sends them to their owners, then applies the latest action (or neutral)
when calling `Snake.update`. The brain forward pass is skipped when control is
external to keep behavior deterministic and to avoid mixing AI and player
controls.

## Decisions locked for Phase 3

Each connection controls a single snake. The action message includes `turn`
and `boost` in a single payload. Raw sensors are sent along with schema
metadata in the welcome message. The last action is used for `N` ticks, then
the snake drops to neutral control, and after a sustained timeout the server
returns the snake to AI control.

## Low-level simulation changes

### `Snake.update` signature

Add an optional control override. The override path bypasses the brain forward
pass and directly applies the external inputs.

```ts
export type ControlInput = { turn: number; boost: number };

update(world: WorldLike, dt: number, control?: ControlInput): void;
```

Behavior rules: when `control` is provided, set `turnInput` and `boostInput`
directly and skip brain inference. When `control` is missing, run brain
inference as usual.

### `Snake.computeSensors`

Expose a dedicated method to compute sensors without changing snake state.
This keeps sensor publishing explicit and avoids side effects.

```ts
computeSensors(world: WorldLike, out: Float32Array): Float32Array;
```

Implementation uses the existing `buildSensors` helper and writes into the
preallocated buffer. No new allocations per tick.

### `World.update` integration

`World.update` accepts an optional controller registry. For each snake, if a
controller is assigned, compute sensors and publish them, fetch the action (or
neutral if timed out), and call `snake.update` with control. Otherwise, call
`snake.update` without control.

## Controller registry design

### Data structures

```ts
interface ControllerState {
  snakeId: number;
  connId: number;
  controllerType: "player" | "bot";
  lastTurn: number;
  lastBoost: number;
  lastTick: number;
  lastActionAtMs: number;
  droppedActions: number;
}
```

Storage maps are `bySnake: Map<number, ControllerState>` and
`byConn: Map<number, ControllerState>`, enabling lookup by snake id or
connection id.

### API surface

```ts
assignSnake(connId: number, type: "player" | "bot"): number;
releaseSnake(connId: number): void;
handleAction(connId: number, msg: ActionMsg): void;
getAction(snakeId: number, tickId: number): ControlInput;
publishSensors(snakeId: number, sensors: Float32Array, meta: SensorMeta): void;
```

### Action validation and rate limiting

The server clamps `turn` to `[-1, 1]` and `boost` to `[0, 1]`, rejects NaN and
Infinity, enforces `maxActionsPerTick = 1` per connection, and drops any extra
messages beyond `maxActionsPerSecond`.

### Timeout policy

If `tickId - lastTick <= actionTimeoutTicks`, reuse the last action. If the
timeout is exceeded, return neutral `{ turn: 0, boost: 0 }`. If the timeout is
exceeded for 2x the threshold, revoke controller ownership and return control
to AI.

## WebSocket routing updates

When a client joins in player mode, the server assigns a snake id and returns
`assign`. When the client sends `action`, the server validates that the snake id
matches the connection assignment. Sensor payloads are sent only to the owning
connection.

## Sensor payload format

```json
{
  "type": "sensors",
  "tick": 123,
  "snakeId": 42,
  "sensors": [0.1, -0.2, 0.3],
  "meta": { "x": 100.5, "y": -20.1, "dir": 1.57 }
}
```

Sensors are serialized as JSON arrays. This allocates per tick, but that is
acceptable for a hobby-scale deployment. If bandwidth becomes an issue, this
can be upgraded to a binary sensor channel later.

## Detailed design notes

Sensor delivery is intentionally one-to-one: only the owner of a snake receives
its sensors. This prevents information leaks and keeps bandwidth small. The
`sensorSpec` included in the welcome message acts as a contract for bots so they
can adapt if the sensor layout changes in future phases.

Action timing is tick-based rather than wall-clock-based. Each action message
includes the client tick it was produced for, but the server applies the last
known valid action at the start of each tick. This makes control deterministic
and avoids stalling the sim waiting for late packets. The timeout policy ensures
that a bot that stops sending actions does not freeze the snake forever.

Control overrides must never mix with brain outputs in the same tick. The
override path should fully bypass the brain forward pass to avoid subtle
behavior changes. This keeps external control semantics clear and preserves
evolutionary behavior for non-controlled snakes.

## Tests

Unit tests verify that `Snake.update` respects control overrides and bypasses
brain inference, that `computeSensors` returns the correct length into the
provided buffer, and that `ControllerRegistry` applies timeout and rate limit
rules. Integration tests connect a bot client, receive sensors, and confirm
that actions steer a snake.

## Footguns

Do not allocate new `Float32Array` instances inside the hot loop. Never send
sensors to non-owning clients, and never allow action messages to mutate World
state directly outside the controller path.

## Acceptance criteria

A connected client can control one snake, sensors are delivered each tick, and
uncontrolled snakes behave exactly as before.

## Execution checklist

- [ ] Add `ControlInput` type
- [ ] Extend `Snake.update`
- [ ] Add `Snake.computeSensors`
- [ ] Update `World.update` to accept controller registry
- [ ] Implement controller registry
- [ ] Wire WS routing to controller registry
- [ ] Add unit tests for control override + timeout

## Function-by-function pseudocode

### Pseudocode: `Snake.update`

```text
function update(world, dt, control):
  if control provided:
    turnInput = clamp(control.turn)
    boostInput = clamp(control.boost)
  else:
    sensors = buildSensors(world, this, sensorBuf)
    outputs = brain.forward(sensors)
    turnInput = outputs[0]
    boostInput = outputs[1]
  apply movement, collisions, growth as normal
```

### Pseudocode: `Snake.computeSensors`

```text
function computeSensors(world, out):
  return buildSensors(world, this, out)
```

### Pseudocode: `World.update`

```text
function update(dt, controllers):
  for each snake:
    if controllers has control for snake:
      sensors = snake.computeSensors(world, snake.sensorBuf)
      controllers.publishSensors(snake.id, sensors, meta)
      control = controllers.getAction(snake.id, tick)
      snake.update(world, dt, control)
    else:
      snake.update(world, dt)
  handle pellets, collisions, generation logic
```

### Pseudocode: `ControllerRegistry`

```text
function assignSnake(connId, type):
  snakeId = pickAvailableSnake()
  state = new ControllerState(snakeId, connId, type)
  bySnake[snakeId] = state
  byConn[connId] = state
  return snakeId

function handleAction(connId, msg):
  state = byConn[connId]
  if msg.snakeId != state.snakeId: drop
  if rateLimitExceeded: drop
  state.lastTurn = clamp(msg.turn)
  state.lastBoost = clamp(msg.boost)
  state.lastTick = msg.tick

function getAction(snakeId, tickId):
  state = bySnake[snakeId]
  if tickId - state.lastTick <= timeout:
    return { turn: state.lastTurn, boost: state.lastBoost }
  if timedOutLong:
    releaseSnake(state.connId)
  return neutral
```

## Error handling and edge cases

If a client sends actions for a snake it does not own, the server drops the
message without affecting simulation state. Actions with NaN or Infinity are
rejected immediately. If a connection disconnects, the server releases the
snake and returns it to AI control. If actions stop arriving, the timeout
policy ensures the snake does not freeze indefinitely.

## Sample payloads and example WS session transcript

```json
{ "type": "action", "tick": 120, "snakeId": 42, "turn": -0.3, "boost": 1 }
```

```json
{ "type": "sensors", "tick": 120, "snakeId": 42, "sensors": [0.1, -0.2, 0.3], "meta": { "x": 100.5, "y": -20.1, "dir": 1.57 } }
```

Example WS session transcript:

```text
client -> server: {"type":"hello","clientType":"bot","version":1}
server -> client: {"type":"welcome",...}
client -> server: {"type":"join","mode":"player","name":"botA"}
server -> client: {"type":"assign","snakeId":42,"controller":"bot"}
server -> client: {"type":"sensors",...}
client -> server: {"type":"action",...}
```

## Test matrix

| Test name | Setup / input | Expected result | Failure cases to verify |
| --- | --- | --- | --- |
| action_wrong_snake | Send action with mismatched snakeId | Action dropped, no effect | Action applied to wrong snake |
| action_timeout | Stop sending actions for N ticks | Neutral control applied | Snake freezes or keeps last action forever |
| sensor_length | computeSensors called | Length equals CFG.brain.inSize | Length mismatch causes out-of-bounds use |
| disconnect_release | Client disconnects | Snake reverts to AI | Snake remains locked to missing controller |

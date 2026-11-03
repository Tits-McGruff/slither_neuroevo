from pathlib import Path


def replace_exact(path: str, old: str, new: str) -> None:
    file_path = Path(path)
    text = file_path.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected exactly one match, found {count}")
    file_path.write_text(text.replace(old, new, 1), encoding="utf-8")


# Protocol 2 remains backwards compatible. Bot clients can opt into compact
# binary observation frames; old clients keep receiving JSON sensors.
replace_exact(
    "server/protocol.ts",
    """  /** Exact protocol version implemented by the client. */\n  version: number;\n}\n""",
    """  /** Exact protocol version implemented by the client. */\n  version: number;\n  /** Request compact binary sensor packets on bot connections. */\n  binarySensors?: boolean;\n}\n""",
)
replace_exact(
    "server/protocol.ts",
    """export function isHello(msg: unknown): msg is HelloMsg {\n  if (!isRecord(msg)) return false;\n  return (\n    hasKeys(msg, ['type', 'clientType', 'version'], ['type', 'clientType', 'version']) &&\n    msg['type'] === 'hello' &&\n    msg['version'] === PROTOCOL_VERSION &&\n    (msg['clientType'] === 'ui' || msg['clientType'] === 'bot')\n  );\n}\n""",
    """export function isHello(msg: unknown): msg is HelloMsg {\n  if (!isRecord(msg)) return false;\n  return (\n    hasKeys(msg, ['type', 'clientType', 'version', 'binarySensors'], ['type', 'clientType', 'version']) &&\n    msg['type'] === 'hello' &&\n    msg['version'] === PROTOCOL_VERSION &&\n    (msg['binarySensors'] === undefined || typeof msg['binarySensors'] === 'boolean') &&\n    (msg['clientType'] === 'ui' || msg['clientType'] === 'bot')\n  );\n}\n""",
)

# The external path must be literally free when no controller observations are
# present. The previous helper call did validation and buffer bookkeeping even
# for an empty slice on every fixed step.
phase_path = Path("native/src/engine/control_phase.rs")
phase = phase_path.read_text(encoding="utf-8")
old = """            {\n                let trace_request = self\n                    .inspector\n                    .as_mut()\n                    .filter(|capture| capture.delivered_enabled())\n                    .map(|capture| capture.trace_request());\n                let (deliveries, diagnostics) = self.neural.sample_independent_observations(\n                    &self.external_events,\n                    |event| event.snake_index,\n                    indexed,\n                    inputs.generation,\n                    prepared_v4,\n                    &mut self.external_observations,\n                    &mut self.sensor_scratch,\n                    trace_request,\n                )?;\n                for (ordinal, event) in self.external_events.iter_mut().enumerate() {\n                    event.delivery =\n                        deliveries[ordinal].ok_or(ControlPhaseError::InternalShapeMismatch {\n                            field: \"external sensor delivery\",\n                        })?;\n                    event.diagnostics = diagnostics[ordinal];\n                }\n            }\n"""
new = """            if !self.external_events.is_empty() {\n                let trace_request = self\n                    .inspector\n                    .as_mut()\n                    .filter(|capture| capture.delivered_enabled())\n                    .map(|capture| capture.trace_request());\n                let (deliveries, diagnostics) = self.neural.sample_independent_observations(\n                    &self.external_events,\n                    |event| event.snake_index,\n                    indexed,\n                    inputs.generation,\n                    prepared_v4,\n                    &mut self.external_observations,\n                    &mut self.sensor_scratch,\n                    trace_request,\n                )?;\n                for (ordinal, event) in self.external_events.iter_mut().enumerate() {\n                    event.delivery =\n                        deliveries[ordinal].ok_or(ControlPhaseError::InternalShapeMismatch {\n                            field: \"external sensor delivery\",\n                        })?;\n                    event.diagnostics = diagnostics[ordinal];\n                }\n            }\n"""
if phase.count(old) != 1:
    raise SystemExit(f"control_phase.rs external batch count was {phase.count(old)}, expected 1")
phase_path.write_text(phase.replace(old, new, 1), encoding="utf-8")

# Compact binary observation codec. Header is deliberately fixed width and
# little-endian because production native targets are x86_64 Windows/Linux.
Path("server/binarySensors.ts").write_text(
    """import type { SensorsMsg } from './protocol.ts';\n\n/** Four-byte discriminator for compact Protocol 2 bot observations. */\nexport const BINARY_SENSORS_MAGIC = 'SNS2';\n/** Version of the compact sensor payload layout. */\nexport const BINARY_SENSORS_VERSION = 1;\n/** Fixed bytes before packed float32 sensor values. */\nexport const BINARY_SENSORS_HEADER_BYTES = 44;\n\n/** Encode one already-validated sensor message without decimal JSON conversion. */\nexport function encodeBinarySensors(message: SensorsMsg): Buffer {\n  if (!Number.isSafeInteger(message.tick) || message.tick < 0) {\n    throw new RangeError('binary sensor tick must be a non-negative safe integer');\n  }\n  if (!Number.isSafeInteger(message.snakeId) || message.snakeId <= 0 || message.snakeId > 0xffffffff) {\n    throw new RangeError('binary sensor snakeId must fit uint32');\n  }\n  if (message.sensors.length > 0xffff || !message.sensors.every(Number.isFinite)) {\n    throw new RangeError('binary sensor vector is invalid');\n  }\n  const packed = Float32Array.from(message.sensors);\n  const output = Buffer.allocUnsafe(BINARY_SENSORS_HEADER_BYTES + packed.byteLength);\n  output.write(BINARY_SENSORS_MAGIC, 0, 4, 'ascii');\n  output.writeUInt8(BINARY_SENSORS_VERSION, 4);\n  output.writeUInt8(message.meta === undefined ? 0 : 1, 5);\n  output.writeUInt16LE(message.sensors.length, 6);\n  output.writeBigUInt64LE(BigInt(message.tick), 8);\n  output.writeUInt32LE(message.snakeId, 16);\n  output.writeDoubleLE(message.meta?.x ?? 0, 20);\n  output.writeDoubleLE(message.meta?.y ?? 0, 28);\n  output.writeDoubleLE(message.meta?.dir ?? 0, 36);\n  Buffer.from(packed.buffer, packed.byteOffset, packed.byteLength).copy(\n    output, BINARY_SENSORS_HEADER_BYTES\n  );\n  return output;\n}\n""",
    encoding="utf-8",
)

# Route negotiated bot observations through the existing reliable queue as
# binary buffers. Lifecycle/control traffic and non-opted-in clients stay JSON.
replace_exact(
    "server/wsHub.ts",
    """import { DEFAULT_CONFIG } from './config.ts';\n""",
    """import { DEFAULT_CONFIG } from './config.ts';\nimport { encodeBinarySensors } from './binarySensors.ts';\n""",
)
replace_exact(
    "server/wsHub.ts",
    """  SensorSpec,\n  ServerMessage,\n""",
    """  SensorSpec,\n  ServerMessage,\n""",
)
replace_exact(
    "server/wsHub.ts",
    """  clientType: 'unknown' | ClientType;\n  joined: boolean;\n""",
    """  clientType: 'unknown' | ClientType;\n  /** Whether this bot opted into compact binary observation packets. */\n  binarySensors: boolean;\n  joined: boolean;\n""",
)
replace_exact(
    "server/wsHub.ts",
    """  /** Priority JSON payloads waiting behind the current WebSocket send. */\n  reliableQueue: string[];\n  /** UTF-8 byte total represented by `reliableQueue`. */\n""",
    """  /** Priority text or binary payloads waiting behind the current WebSocket send. */\n  reliableQueue: Array<string | Buffer>;\n  /** Byte total represented by `reliableQueue`. */\n""",
)
replace_exact(
    "server/wsHub.ts",
    """  sendJsonTo(connId: number, payload: ServerMessage): boolean {\n    const state = this.connections.get(connId);\n    if (!state || !state.joined) return false;\n    return this.enqueueReliable(state, JSON.stringify(payload));\n  }\n""",
    """  sendJsonTo(connId: number, payload: ServerMessage): boolean {\n    const state = this.connections.get(connId);\n    if (!state || !state.joined) return false;\n    if (state.clientType === 'bot' && state.binarySensors && payload.type === 'sensors') {\n      return this.enqueueReliable(state, encodeBinarySensors(payload));\n    }\n    return this.enqueueReliable(state, JSON.stringify(payload));\n  }\n""",
)
replace_exact(
    "server/wsHub.ts",
    """   * Queue one priority JSON payload or report/close on bounded-queue failure.\n   * @param state - Target connection.\n   * @param payload - Pre-serialized JSON text.\n   * @returns True when the payload entered the reliable path.\n   */\n  private enqueueReliable(state: ConnectionState, payload: string): boolean {\n""",
    """   * Queue one priority payload or report/close on bounded-queue failure.\n   * @param state - Target connection.\n   * @param payload - Pre-serialized JSON text or compact binary observation.\n   * @returns True when the payload entered the reliable path.\n   */\n  private enqueueReliable(state: ConnectionState, payload: string | Buffer): boolean {\n""",
)
replace_exact(
    "server/wsHub.ts",
    """    const bytes = Buffer.byteLength(payload);\n""",
    """    const bytes = typeof payload === 'string' ? Buffer.byteLength(payload) : payload.byteLength;\n""",
)
replace_exact(
    "server/wsHub.ts",
    """    if (queuedReliable !== undefined) {\n      payload = queuedReliable;\n      state.reliableQueueBytes -= Buffer.byteLength(queuedReliable);\n      reliable = true;\n""",
    """    if (queuedReliable !== undefined) {\n      payload = queuedReliable;\n      state.reliableQueueBytes -= typeof queuedReliable === 'string'\n        ? Buffer.byteLength(queuedReliable)\n        : queuedReliable.byteLength;\n      binary = Buffer.isBuffer(queuedReliable);\n      reliable = true;\n""",
)
replace_exact(
    "server/wsHub.ts",
    """      clientType: 'unknown',\n      joined: false,\n""",
    """      clientType: 'unknown',\n      binarySensors: false,\n      joined: false,\n""",
)
replace_exact(
    "server/wsHub.ts",
    """        state.clientType = msg.clientType;\n        this.enqueueReliable(state, this.welcomeJson);\n""",
    """        state.clientType = msg.clientType;\n        state.binarySensors = msg.clientType === 'bot' && msg.binarySensors === true;\n        this.enqueueReliable(state, this.welcomeJson);\n""",
)

# Test fixture owns a ConnectionState literal and must state the new negotiated flag.
replace_exact(
    "server/wsHub.priority.integration.test.ts",
    """    clientType: 'ui',\n    joined: true,\n""",
    """    clientType: 'ui',\n    binarySensors: false,\n    joined: true,\n""",
)
replace_exact(
    "server/wsHub.priority.integration.test.ts",
    """describe('WsHub lifecycle priority', () => {\n""",
    """describe('WsHub lifecycle priority', () => {\n  it('sends negotiated bot observations as compact binary reliable payloads', () => {\n    const { hub, state, socket } = buildFakeHub();\n    state.clientType = 'bot';\n    state.binarySensors = true;\n    expect(hub.sendJsonTo(1, {\n      type: 'sensors', tick: 42, snakeId: 7, sensors: [1.25, -2.5],\n      meta: { x: 3, y: 4, dir: 0.5 }\n    })).toBe(true);\n    expect(socket.sent[0]!.binary).toBe(true);\n    expect(Buffer.isBuffer(socket.sent[0]!.payload)).toBe(true);\n    expect((socket.sent[0]!.payload as Buffer).subarray(0, 4).toString('ascii')).toBe('SNS2');\n    socket.sent[0]!.complete();\n    state.binarySensors = false;\n    expect(hub.sendJsonTo(1, { type: 'sensors', tick: 43, snakeId: 7, sensors: [0] })).toBe(true);\n    expect(socket.sent[1]!.binary).toBe(false);\n    expect(JSON.parse(String(socket.sent[1]!.payload)).type).toBe('sensors');\n  });\n\n""",
)

Path("server/binarySensors.test.ts").write_text(
    """import { describe, expect, it } from 'vitest';\nimport { BINARY_SENSORS_HEADER_BYTES, encodeBinarySensors } from './binarySensors.ts';\n\ndescribe('binary sensor codec', () => {\n  it('packs the fixed header and float32 body in little-endian order', () => {\n    const encoded = encodeBinarySensors({\n      type: 'sensors', tick: 0x12345678, snakeId: 9, sensors: [1.25, -2.5, 0.125],\n      meta: { x: 3.5, y: -4.25, dir: 0.75 }\n    });\n    expect(encoded.subarray(0, 4).toString('ascii')).toBe('SNS2');\n    expect(encoded.readUInt8(4)).toBe(1);\n    expect(encoded.readUInt8(5)).toBe(1);\n    expect(encoded.readUInt16LE(6)).toBe(3);\n    expect(encoded.readBigUInt64LE(8)).toBe(0x12345678n);\n    expect(encoded.readUInt32LE(16)).toBe(9);\n    expect(encoded.readDoubleLE(20)).toBe(3.5);\n    expect(encoded.readDoubleLE(28)).toBe(-4.25);\n    expect(encoded.readDoubleLE(36)).toBe(0.75);\n    expect(encoded.readFloatLE(BINARY_SENSORS_HEADER_BYTES)).toBeCloseTo(1.25);\n    expect(encoded.readFloatLE(BINARY_SENSORS_HEADER_BYTES + 4)).toBeCloseTo(-2.5);\n    expect(encoded.readFloatLE(BINARY_SENSORS_HEADER_BYTES + 8)).toBeCloseTo(0.125);\n  });\n});\n""",
    encoding="utf-8",
)

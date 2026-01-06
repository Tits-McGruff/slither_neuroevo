import type { FitnessData, FitnessHistoryEntry, VizData } from '../src/protocol/messages.ts';

/** Current protocol version for handshake compatibility. */
export const PROTOCOL_VERSION = 1;
/** Serializer version for binary frame layout compatibility. */
export const SERIALIZER_VERSION = 1;
/** Max player name length accepted during join. */
const MAX_NAME_LENGTH = 24;

/** Client identity types that can connect to the server. */
export type ClientType = 'ui' | 'bot';
/** Join mode for client registration. */
export type JoinMode = 'spectator' | 'player';

/** Initial handshake payload from the client. */
export interface HelloMsg {
  type: 'hello';
  clientType: ClientType;
  version: number;
}

/** Join request from client to register as spectator or player. */
export interface JoinMsg {
  type: 'join';
  mode: JoinMode;
  name?: string;
}

/** Client heartbeat message. */
export interface PingMsg {
  type: 'ping';
  t?: number;
}

/** Player action message aligned to a specific tick. */
export interface ActionMsg {
  type: 'action';
  tick: number;
  snakeId: number;
  turn: number;
  boost: number;
}

/** Viewport update from the UI client. */
export interface ViewMsg {
  type: 'view';
  viewW?: number;
  viewH?: number;
  mode?: 'overview' | 'follow' | 'toggle';
}

/** Toggle visualization streaming. */
export interface VizMsg {
  type: 'viz';
  enabled: boolean;
}

/** Union of all client-to-server message shapes. */
export type ClientMessage = HelloMsg | JoinMsg | PingMsg | ActionMsg | ViewMsg | VizMsg;

/** Sensor metadata describing the array order and size. */
export interface SensorSpec {
  sensorCount: number;
  order: string[];
}

/** Initial server welcome payload. */
export interface WelcomeMsg {
  type: 'welcome';
  sessionId: string;
  tickRate: number;
  worldSeed: number;
  cfgHash: string;
  sensorSpec: SensorSpec;
  serializerVersion: number;
  frameByteLength: number;
}

/** Periodic stats payload from the server. */
export interface StatsMsg {
  type: 'stats';
  tick: number;
  gen: number;
  alive: number;
  fps: number;
  fitnessData?: FitnessData;
  fitnessHistory?: FitnessHistoryEntry[];
  viz?: VizData;
}

/** Server assignment for a newly controlled snake. */
export interface AssignMsg {
  type: 'assign';
  snakeId: number;
  controller: 'player' | 'bot';
}

/** Sensor packet for a controlled snake. */
export interface SensorsMsg {
  type: 'sensors';
  tick: number;
  snakeId: number;
  sensors: number[];
  meta?: { x: number; y: number; dir: number };
}

/** Error payload sent by the server. */
export interface ErrorMsg {
  type: 'error';
  message: string;
}

/** Union of all server-to-client messages. */
export type ServerMessage = WelcomeMsg | StatsMsg | AssignMsg | SensorsMsg | ErrorMsg;

/**
 * Narrow a value to a plain record for message validation.
 * @param value - Unknown value to inspect.
 * @returns True when value is a non-null object.
 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

/**
 * Check for a finite number payload.
 * @param value - Value to test.
 * @returns True when value is a finite number.
 */
function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

/**
 * Validate a hello message.
 * @param msg - Raw message to validate.
 * @returns True when the payload is a valid hello message.
 */
export function isHello(msg: unknown): msg is HelloMsg {
  if (!isRecord(msg)) return false;
  return (
    msg['type'] === 'hello' &&
    msg['version'] === PROTOCOL_VERSION &&
    (msg['clientType'] === 'ui' || msg['clientType'] === 'bot')
  );
}

/**
 * Validate a join message.
 * @param msg - Raw message to validate.
 * @returns True when the payload is a valid join message.
 */
export function isJoin(msg: unknown): msg is JoinMsg {
  if (!isRecord(msg)) return false;
  if (msg['type'] !== 'join') return false;
  if (msg['mode'] !== 'spectator' && msg['mode'] !== 'player') return false;
  if ('name' in msg) {
    if (typeof msg['name'] !== 'string') return false;
    if (msg['name'].length > MAX_NAME_LENGTH) return false;
  }
  return true;
}

/**
 * Validate a ping message.
 * @param msg - Raw message to validate.
 * @returns True when the payload is a valid ping message.
 */
export function isPing(msg: unknown): msg is PingMsg {
  if (!isRecord(msg)) return false;
  if (msg['type'] !== 'ping') return false;
  if ('t' in msg && !isFiniteNumber(msg['t'])) return false;
  return true;
}

/**
 * Validate an action message.
 * @param msg - Raw message to validate.
 * @returns True when the payload is a valid action message.
 */
export function isAction(msg: unknown): msg is ActionMsg {
  if (!isRecord(msg)) return false;
  if (msg['type'] !== 'action') return false;
  if (!isFiniteNumber(msg['tick'])) return false;
  if (!isFiniteNumber(msg['snakeId'])) return false;
  if (!isFiniteNumber(msg['turn'])) return false;
  if (!isFiniteNumber(msg['boost'])) return false;
  return true;
}

/**
 * Validate a view message.
 * @param msg - Raw message to validate.
 * @returns True when the payload is a valid view message.
 */
export function isView(msg: unknown): msg is ViewMsg {
  if (!isRecord(msg)) return false;
  if (msg['type'] !== 'view') return false;
  if ('viewW' in msg && !isFiniteNumber(msg['viewW'])) return false;
  if ('viewH' in msg && !isFiniteNumber(msg['viewH'])) return false;
  if ('mode' in msg) {
    const mode = msg['mode'];
    if (mode !== 'overview' && mode !== 'follow' && mode !== 'toggle') return false;
  }
  return true;
}

/**
 * Validate a viz message.
 * @param msg - Raw message to validate.
 * @returns True when the payload is a valid viz message.
 */
export function isViz(msg: unknown): msg is VizMsg {
  if (!isRecord(msg)) return false;
  if (msg['type'] !== 'viz') return false;
  if (typeof msg['enabled'] !== 'boolean') return false;
  return true;
}

/**
 * Parse and validate a raw client message into a typed shape.
 * @param raw - Raw message payload.
 * @returns Validated client message or null on failure.
 */
export function parseClientMessage(raw: unknown): ClientMessage | null {
  if (!isRecord(raw)) return null;
  if (typeof raw['type'] !== 'string') return null;
  switch (raw['type']) {
    case 'hello':
      return isHello(raw) ? raw : null;
    case 'join':
      return isJoin(raw) ? raw : null;
    case 'ping':
      return isPing(raw) ? raw : null;
    case 'action':
      return isAction(raw) ? raw : null;
    case 'view':
      return isView(raw) ? raw : null;
    case 'viz':
      return isViz(raw) ? raw : null;
    default:
      return null;
  }
}

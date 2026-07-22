import type { FitnessData, FitnessHistoryEntry, HallOfFameEntry, VizData } from '../src/protocol/messages.ts';
import type { SensorSpec as SensorSpecBase } from '../src/protocol/sensors.ts';
import type {
  CoreSettings,
  LiveSettingsUpdate,
  SettingsUpdate
} from '../src/protocol/settings.ts';
import { getLiveSettingDefinition, SETTINGS_PATHS } from '../src/protocol/settings.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import type { InferenceModeRecord } from './inferenceMode.ts';

/** Current protocol version for handshake compatibility. */
export const PROTOCOL_VERSION = 2;
/** Serializer version for binary frame layout compatibility. */
export const SERIALIZER_VERSION = 1;
/** Max player name length accepted during join. */
const MAX_NAME_LENGTH = 24;
/** Max request-id length accepted for correlated commands. */
const MAX_REQUEST_ID_LENGTH = 64;
/** Set of valid settings update paths for reset messages. */
const SETTINGS_PATH_SET = new Set(SETTINGS_PATHS);
/** Core settings keys accepted in reset messages. */
const CORE_SETTINGS_KEYS: Array<keyof CoreSettings> = [
  'snakeCount',
  'simSpeed',
  'hiddenLayers',
  'neurons1',
  'neurons2',
  'neurons3',
  'neurons4',
  'neurons5'
];

/** Client identity types that can connect to the server. */
export type ClientType = 'ui' | 'bot';
/** Join mode for client registration. */
export type JoinMode = 'spectator' | 'player';

/** Initial handshake payload from the client. */
export interface HelloMsg {
  /** Message discriminator. */
  type: 'hello';
  /** Client capability class. */
  clientType: ClientType;
  /** Exact protocol version implemented by the client. */
  version: number;
}

/** Join request from client to register as spectator or player. */
export interface JoinMsg {
  /** Message discriminator. */
  type: 'join';
  /** Requested connection role. */
  mode: JoinMode;
  /** Optional player nickname. */
  name?: string;
}

/** Client heartbeat message. */
export interface PingMsg {
  /** Message discriminator. */
  type: 'ping';
  /** Optional client timestamp. */
  t?: number;
}

/** Player action message aligned to a specific tick. */
export interface ActionMsg {
  /** Message discriminator. */
  type: 'action';
  /** Client-observed authoritative tick. */
  tick: number;
  /** Assigned snake id. */
  snakeId: number;
  /** Turn command. */
  turn: number;
  /** Boost command. */
  boost: number;
}

/** Viewport update from the UI client. */
export interface ViewMsg {
  /** Message discriminator. */
  type: 'view';
  /** Viewport width. */
  viewW?: number;
  /** Viewport height. */
  viewH?: number;
  /** Requested observer mode. */
  mode?: 'overview' | 'follow' | 'toggle';
}

/** Toggle visualization streaming. */
export interface VizMsg {
  /** Message discriminator. */
  type: 'viz';
  /** Whether visualization payloads are requested. */
  enabled: boolean;
}

/** Reset request to rebuild the server world using updated settings. */
export interface ResetMsg {
  /** Message discriminator. */
  type: 'reset';
  /** Optional core settings applied during reconstruction. */
  settings?: Partial<CoreSettings>;
  /** Optional CFG settings applied during reconstruction. */
  updates?: SettingsUpdate[];
  /** Optional graph override. */
  graphSpec?: GraphSpec | null;
}

/** Atomic authoritative live-settings request. */
export interface LiveSettingsMsg {
  /** Message discriminator. */
  type: 'settings';
  /** Client-generated correlation id. */
  requestId: string;
  /** One or more setting values applied atomically. */
  updates: LiveSettingsUpdate[];
}

/** God Mode kill request. */
export interface GodModeKillMsg {
  /** Message discriminator. */
  type: 'godMode';
  /** Client-generated correlation id. */
  requestId: string;
  /** Requested mutation. */
  action: 'kill';
  /** Target snake id. */
  snakeId: number;
}

/** God Mode move request. */
export interface GodModeMoveMsg {
  /** Message discriminator. */
  type: 'godMode';
  /** Client-generated correlation id. */
  requestId: string;
  /** Requested mutation. */
  action: 'move';
  /** Target snake id. */
  snakeId: number;
  /** Requested head X coordinate. */
  x: number;
  /** Requested head Y coordinate. */
  y: number;
}

/** Supported God Mode requests. */
export type GodModeMsg = GodModeKillMsg | GodModeMoveMsg;

/** Explicit request for a fresh run identity. */
export interface NewRunMsg {
  /** Message discriminator. */
  type: 'newRun';
  /** Client-generated correlation id. */
  requestId: string;
}

/** Union of all client-to-server message shapes. */
export type ClientMessage =
  | HelloMsg
  | JoinMsg
  | PingMsg
  | ActionMsg
  | ViewMsg
  | VizMsg
  | ResetMsg
  | LiveSettingsMsg
  | GodModeMsg
  | NewRunMsg;

/** Sensor metadata describing the array order and size. */
export type SensorSpec = SensorSpecBase;

/** Complete authoritative settings snapshot sent during handshake. */
export interface AuthoritativeSettingsState {
  /** Active core/world-construction settings. */
  core: CoreSettings;
  /** Active CFG path/value snapshot. */
  updates: SettingsUpdate[];
}

/** Initial server welcome payload. */
export interface WelcomeMsg {
  /** Message discriminator. */
  type: 'welcome';
  /** Exact protocol version selected by the server. */
  protocolVersion: typeof PROTOCOL_VERSION;
  /** Process-local session id. */
  sessionId: string;
  /** Server scheduler tick rate. */
  tickRate: number;
  /** Active world seed. */
  worldSeed: number;
  /** Active evolutionary-lineage id. */
  runId: string;
  /** Monotonic accepted configuration revision. */
  configRevision: number;
  /** Versioned canonical configuration content hash. */
  configHash: string;
  /** Current authoritative settings state. */
  settings: AuthoritativeSettingsState;
  /** Honest active inference-path diagnostics. */
  inferenceMode: InferenceModeRecord;
  /** Active sensor contract. */
  sensorSpec: SensorSpec;
  /** Binary serializer contract version. */
  serializerVersion: number;
  /** Example serialized frame byte length. */
  frameByteLength: number;
}

/** Periodic stats payload from the server. */
export interface StatsMsg {
  /** Message discriminator. */
  type: 'stats';
  /** Last committed fixed-step id. */
  tick: number;
  /** Active generation. */
  gen: number;
  /** Elapsed generation simulation seconds. */
  generationTime: number;
  /** Configured generation duration. */
  generationSeconds: number;
  /** Alive evolving population count. */
  alive: number;
  /** Alive count including non-population snakes. */
  aliveTotal: number;
  /** Alive baseline-bot count. */
  baselineBotsAlive: number;
  /** Configured baseline-bot count. */
  baselineBotsTotal: number;
  /** Latest server pump rate. */
  fps: number;
  /** Optional current generation fitness summary. */
  fitnessData?: FitnessData;
  /** Optional bounded fitness history. */
  fitnessHistory?: FitnessHistoryEntry[];
  /** Optional neural visualization data. */
  viz?: VizData;
  /** Optional newly completed Hall-of-Fame entry. */
  hofEntry?: HallOfFameEntry;
}

/** Server assignment for a newly controlled snake. */
export interface AssignMsg {
  /** Message discriminator. */
  type: 'assign';
  /** Assigned snake id. */
  snakeId: number;
  /** Controller class. */
  controller: 'player' | 'bot';
}

/** Sensor packet for a controlled snake. */
export interface SensorsMsg {
  /** Message discriminator. */
  type: 'sensors';
  /** Authoritative observation tick. */
  tick: number;
  /** Controlled snake id. */
  snakeId: number;
  /** Numeric sensor vector. */
  sensors: number[];
  /** Optional pose used for client input. */
  meta?: { x: number; y: number; dir: number };
}

/** Error payload sent by the server. */
export interface ErrorMsg {
  /** Message discriminator. */
  type: 'error';
  /** Human-readable error. */
  message: string;
}

/** Authoritative result for one live-settings request. */
export interface SettingsAppliedMsg {
  /** Message discriminator. */
  type: 'settingsApplied';
  /** Original client correlation id. */
  requestId: string;
  /** Whether the complete atomic request was applied. */
  applied: boolean;
  /** Authoritative normalized values, empty on rejection. */
  updates: LiveSettingsUpdate[];
  /** Current monotonic config revision. */
  configRevision: number;
  /** Current canonical config content hash. */
  configHash: string;
  /** Global accepted-command order, absent on rejection. */
  sequence?: number;
  /** Fixed step before which the command was applied. */
  step?: number;
  /** Stable rejection reason. */
  reason?: string;
}

/** Authoritative result for one God Mode request. */
export interface GodModeResultMsg {
  /** Message discriminator. */
  type: 'godModeResult';
  /** Original client correlation id. */
  requestId: string;
  /** Requested mutation. */
  action: 'kill' | 'move';
  /** Target snake id. */
  snakeId: number;
  /** Whether the mutation was applied. */
  applied: boolean;
  /** Global accepted-command order, absent on pre-queue rejection. */
  sequence?: number;
  /** Fixed step before which the command was applied. */
  step?: number;
  /** Stable rejection reason. */
  reason?: string;
  /** Actual authoritative X after a move. */
  x?: number;
  /** Actual authoritative Y after a move. */
  y?: number;
  /** Number of normal death pellets added by a kill. */
  pelletsDropped?: number;
}

/** Result for the Protocol 2 New Run surface. */
export interface NewRunResultMsg {
  /** Message discriminator. */
  type: 'newRunResult';
  /** Original client correlation id. */
  requestId: string;
  /** Whether a durably checkpointed new run was started. */
  applied: boolean;
  /** New seed when a future durable request succeeds. */
  worldSeed?: number;
  /** New run id when a future durable request succeeds. */
  runId?: string;
  /** Explicit rejection or unavailability reason. */
  reason?: string;
}

/** Union of all server-to-client JSON message shapes. */
export type ServerMessage =
  | WelcomeMsg
  | StatsMsg
  | AssignMsg
  | SensorsMsg
  | ErrorMsg
  | SettingsAppliedMsg
  | GodModeResultMsg
  | NewRunResultMsg;

/**
 * Narrow a value to a plain record for message validation.
 * @param value - Unknown value to inspect.
 * @returns True when value is a non-null non-array object.
 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
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
 * Check that a record contains no unknown keys and all required keys.
 * @param value - Record to inspect.
 * @param allowed - Complete allowed key list.
 * @param required - Required key list.
 * @returns True when the key set conforms exactly.
 */
function hasKeys(
  value: Record<string, unknown>,
  allowed: readonly string[],
  required: readonly string[]
): boolean {
  const allowedSet = new Set(allowed);
  if (Object.keys(value).some((key) => !allowedSet.has(key))) return false;
  return required.every((key) => Object.prototype.hasOwnProperty.call(value, key));
}

/**
 * Check a client-generated request correlation id.
 * @param value - Unknown id.
 * @returns True for a non-empty bounded string.
 */
function isRequestId(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= MAX_REQUEST_ID_LENGTH;
}

/**
 * Describe a hello-version mismatch before generic message validation.
 * @param raw - Parsed client payload.
 * @returns Explicit incompatibility message, or null when no mismatch exists.
 */
export function getProtocolVersionError(raw: unknown): string | null {
  if (!isRecord(raw) || raw['type'] !== 'hello') return null;
  const version = raw['version'];
  if (!Number.isSafeInteger(version) || version === PROTOCOL_VERSION) return null;
  return `protocol version ${String(version)} is incompatible; server requires ${PROTOCOL_VERSION}`;
}

/**
 * Validate a hello message.
 * @param msg - Raw message to validate.
 * @returns True when the payload is a valid hello message.
 */
export function isHello(msg: unknown): msg is HelloMsg {
  if (!isRecord(msg)) return false;
  return (
    hasKeys(msg, ['type', 'clientType', 'version'], ['type', 'clientType', 'version']) &&
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
  if (!hasKeys(msg, ['type', 'mode', 'name'], ['type', 'mode'])) return false;
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
  if (!hasKeys(msg, ['type', 't'], ['type'])) return false;
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
  if (!hasKeys(msg, ['type', 'tick', 'snakeId', 'turn', 'boost'], ['type', 'tick', 'snakeId', 'turn', 'boost'])) return false;
  return (
    msg['type'] === 'action' &&
    isFiniteNumber(msg['tick']) &&
    isFiniteNumber(msg['snakeId']) &&
    isFiniteNumber(msg['turn']) &&
    isFiniteNumber(msg['boost'])
  );
}

/**
 * Validate a view message.
 * @param msg - Raw message to validate.
 * @returns True when the payload is a valid view message.
 */
export function isView(msg: unknown): msg is ViewMsg {
  if (!isRecord(msg)) return false;
  if (!hasKeys(msg, ['type', 'viewW', 'viewH', 'mode'], ['type'])) return false;
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
  return (
    isRecord(msg) &&
    hasKeys(msg, ['type', 'enabled'], ['type', 'enabled']) &&
    msg['type'] === 'viz' &&
    typeof msg['enabled'] === 'boolean'
  );
}

/**
 * Validate a core settings payload for reset messages.
 * @param value - Raw settings payload.
 * @returns True when the payload shape is valid.
 */
function isCoreSettings(value: unknown): value is Partial<CoreSettings> {
  if (!isRecord(value)) return false;
  if (!hasKeys(value, CORE_SETTINGS_KEYS, [])) return false;
  for (const key of CORE_SETTINGS_KEYS) {
    if (key in value && !isFiniteNumber(value[key])) return false;
  }
  return true;
}

/**
 * Validate one reset/import settings update.
 * @param value - Raw update.
 * @returns True when the path and numeric value are supported.
 */
function isSettingsUpdate(value: unknown): value is SettingsUpdate {
  if (!isRecord(value)) return false;
  if (!hasKeys(value, ['path', 'value'], ['path', 'value'])) return false;
  if (typeof value['path'] !== 'string') return false;
  if (!SETTINGS_PATH_SET.has(value['path'] as SettingsUpdate['path'])) return false;
  return isFiniteNumber(value['value']);
}

/**
 * Validate one Protocol 2 live setting update structurally.
 * Semantic live/reset classification is performed at the queued boundary.
 * @param value - Raw update.
 * @returns True when path and value can reach semantic validation.
 */
function isLiveSettingsUpdate(value: unknown): value is LiveSettingsUpdate {
  if (!isRecord(value)) return false;
  if (!hasKeys(value, ['path', 'value'], ['path', 'value'])) return false;
  if (typeof value['path'] !== 'string' || !getLiveSettingDefinition(value['path'])) return false;
  return isFiniteNumber(value['value']);
}

/**
 * Validate a reset message.
 * @param msg - Raw message to validate.
 * @returns True when the payload is a valid reset message.
 */
export function isReset(msg: unknown): msg is ResetMsg {
  if (!isRecord(msg)) return false;
  if (!hasKeys(msg, ['type', 'settings', 'updates', 'graphSpec'], ['type'])) return false;
  if (msg['type'] !== 'reset') return false;
  if ('settings' in msg && msg['settings'] !== undefined && !isCoreSettings(msg['settings'])) return false;
  if ('updates' in msg && msg['updates'] !== undefined) {
    if (!Array.isArray(msg['updates']) || !msg['updates'].every(isSettingsUpdate)) return false;
  }
  if ('graphSpec' in msg) {
    const spec = msg['graphSpec'];
    if (spec !== null && spec !== undefined && !isRecord(spec)) return false;
  }
  return true;
}

/**
 * Validate a live-settings message.
 * @param msg - Raw message to validate.
 * @returns True when the payload has the strict Protocol 2 shape.
 */
export function isLiveSettings(msg: unknown): msg is LiveSettingsMsg {
  if (!isRecord(msg)) return false;
  if (!hasKeys(msg, ['type', 'requestId', 'updates'], ['type', 'requestId', 'updates'])) return false;
  return (
    msg['type'] === 'settings' &&
    isRequestId(msg['requestId']) &&
    Array.isArray(msg['updates']) &&
    msg['updates'].length > 0 &&
    msg['updates'].length <= 64 &&
    msg['updates'].every(isLiveSettingsUpdate)
  );
}

/**
 * Validate a God Mode request.
 * @param msg - Raw message to validate.
 * @returns True when the action-specific payload is strict and finite.
 */
export function isGodMode(msg: unknown): msg is GodModeMsg {
  if (!isRecord(msg) || msg['type'] !== 'godMode') return false;
  if (!isRequestId(msg['requestId']) || !isFiniteNumber(msg['snakeId'])) return false;
  if (msg['action'] === 'kill') {
    return hasKeys(msg, ['type', 'requestId', 'action', 'snakeId'], ['type', 'requestId', 'action', 'snakeId']);
  }
  if (msg['action'] === 'move') {
    return (
      hasKeys(msg, ['type', 'requestId', 'action', 'snakeId', 'x', 'y'], ['type', 'requestId', 'action', 'snakeId', 'x', 'y']) &&
      isFiniteNumber(msg['x']) &&
      isFiniteNumber(msg['y'])
    );
  }
  return false;
}

/**
 * Validate a New Run request.
 * @param msg - Raw message to validate.
 * @returns True when the payload has the strict Protocol 2 shape.
 */
export function isNewRun(msg: unknown): msg is NewRunMsg {
  return (
    isRecord(msg) &&
    hasKeys(msg, ['type', 'requestId'], ['type', 'requestId']) &&
    msg['type'] === 'newRun' &&
    isRequestId(msg['requestId'])
  );
}

/**
 * Parse and validate a raw client message into a typed shape.
 * @param raw - Raw message payload.
 * @returns Validated client message or null on failure.
 */
export function parseClientMessage(raw: unknown): ClientMessage | null {
  if (!isRecord(raw) || typeof raw['type'] !== 'string') return null;
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
    case 'reset':
      return isReset(raw) ? raw : null;
    case 'settings':
      return isLiveSettings(raw) ? raw : null;
    case 'godMode':
      return isGodMode(raw) ? raw : null;
    case 'newRun':
      return isNewRun(raw) ? raw : null;
    default:
      return null;
  }
}

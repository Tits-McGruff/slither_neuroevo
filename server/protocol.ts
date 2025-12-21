import type { FitnessData, FitnessHistoryEntry } from '../src/protocol/messages.ts';

export const PROTOCOL_VERSION = 1;
export const SERIALIZER_VERSION = 1;
const MAX_NAME_LENGTH = 24;

export type ClientType = 'ui' | 'bot';
export type JoinMode = 'spectator' | 'player';

export interface HelloMsg {
  type: 'hello';
  clientType: ClientType;
  version: number;
}

export interface JoinMsg {
  type: 'join';
  mode: JoinMode;
  name?: string;
}

export interface PingMsg {
  type: 'ping';
  t?: number;
}

export interface ActionMsg {
  type: 'action';
  tick: number;
  snakeId: number;
  turn: number;
  boost: number;
}

export type ClientMessage = HelloMsg | JoinMsg | PingMsg | ActionMsg;

export interface SensorSpec {
  sensorCount: number;
  order: string[];
}

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

export interface StatsMsg {
  type: 'stats';
  tick: number;
  gen: number;
  alive: number;
  fps: number;
  fitnessData?: FitnessData;
  fitnessHistory?: FitnessHistoryEntry[];
}

export interface AssignMsg {
  type: 'assign';
  snakeId: number;
  controller: 'player' | 'bot';
}

export interface SensorsMsg {
  type: 'sensors';
  tick: number;
  snakeId: number;
  sensors: number[];
  meta?: { x: number; y: number; dir: number };
}

export interface ErrorMsg {
  type: 'error';
  message: string;
}

export type ServerMessage = WelcomeMsg | StatsMsg | AssignMsg | SensorsMsg | ErrorMsg;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

export function isHello(msg: unknown): msg is HelloMsg {
  if (!isRecord(msg)) return false;
  return (
    msg['type'] === 'hello' &&
    msg['version'] === PROTOCOL_VERSION &&
    (msg['clientType'] === 'ui' || msg['clientType'] === 'bot')
  );
}

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

export function isPing(msg: unknown): msg is PingMsg {
  if (!isRecord(msg)) return false;
  if (msg['type'] !== 'ping') return false;
  if ('t' in msg && !isFiniteNumber(msg['t'])) return false;
  return true;
}

export function isAction(msg: unknown): msg is ActionMsg {
  if (!isRecord(msg)) return false;
  if (msg['type'] !== 'action') return false;
  if (!isFiniteNumber(msg['tick'])) return false;
  if (!isFiniteNumber(msg['snakeId'])) return false;
  if (!isFiniteNumber(msg['turn'])) return false;
  if (!isFiniteNumber(msg['boost'])) return false;
  return true;
}

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
    default:
      return null;
  }
}

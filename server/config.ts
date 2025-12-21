export type LogLevel = 'debug' | 'info' | 'warn' | 'error';

export interface ServerConfig {
  host: string;
  port: number;
  tickRateHz: number;
  uiFrameRateHz: number;
  actionTimeoutTicks: number;
  maxActionsPerTick: number;
  maxActionsPerSecond: number;
  dbPath: string;
  checkpointEveryGenerations: number;
  logLevel: LogLevel;
  seed?: number;
}

export const DEFAULT_CONFIG: ServerConfig = {
  host: '127.0.0.1',
  port: 5174,
  tickRateHz: 60,
  uiFrameRateHz: 30,
  actionTimeoutTicks: 10,
  maxActionsPerTick: 1,
  maxActionsPerSecond: 120,
  dbPath: './data/slither.db',
  checkpointEveryGenerations: 1,
  logLevel: 'info'
};

type Env = Record<string, string | undefined>;

const LOG_LEVELS: LogLevel[] = ['debug', 'info', 'warn', 'error'];

function clampInt(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function parseIntValue(raw: string | undefined): number | undefined {
  if (raw == null) return undefined;
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed)) return undefined;
  return parsed;
}

function getArgValue(argv: string[], flag: string): string | undefined {
  const prefix = `${flag}=`;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (!arg) continue;
    if (arg === flag) {
      return argv[i + 1];
    }
    if (arg.startsWith(prefix)) {
      return arg.slice(prefix.length);
    }
  }
  return undefined;
}

function coerceInt(
  name: string,
  value: unknown,
  fallback: number,
  min: number,
  max: number,
  warn?: (msg: string) => void
): number {
  if (value === undefined || value === null) {
    return fallback;
  }
  let parsed: number;
  if (typeof value === 'number') {
    parsed = value;
  } else if (typeof value === 'string') {
    parsed = Number.parseInt(value, 10);
  } else {
    parsed = Number.NaN;
  }
  if (!Number.isFinite(parsed)) {
    warn?.(`${name} is invalid; using ${fallback}.`);
    return fallback;
  }
  const clamped = clampInt(Math.floor(parsed), min, max);
  if (clamped !== parsed) {
    warn?.(`${name} was clamped to ${clamped}.`);
  }
  return clamped;
}

export function normalizeConfig(
  input: Partial<ServerConfig>,
  warn?: (msg: string) => void
): ServerConfig {
  const port = coerceInt('port', input.port, DEFAULT_CONFIG.port, 1, 65535, warn);
  const rawHost = input.host;
  const host = rawHost && rawHost.trim() ? rawHost : DEFAULT_CONFIG.host;
  if (rawHost !== undefined && (!rawHost || !rawHost.trim())) {
    warn?.(`host is invalid; using ${host}.`);
  }
  const tickRateHz = coerceInt(
    'tickRateHz',
    input.tickRateHz,
    DEFAULT_CONFIG.tickRateHz,
    1,
    240,
    warn
  );
  let uiFrameRateHz = coerceInt(
    'uiFrameRateHz',
    input.uiFrameRateHz,
    DEFAULT_CONFIG.uiFrameRateHz,
    1,
    240,
    warn
  );
  if (uiFrameRateHz > tickRateHz) {
    warn?.('uiFrameRateHz exceeded tickRateHz; clamping to tickRateHz.');
    uiFrameRateHz = tickRateHz;
  }
  const actionTimeoutTicks = coerceInt(
    'actionTimeoutTicks',
    input.actionTimeoutTicks,
    DEFAULT_CONFIG.actionTimeoutTicks,
    1,
    600,
    warn
  );
  const maxActionsPerTick = coerceInt(
    'maxActionsPerTick',
    input.maxActionsPerTick,
    DEFAULT_CONFIG.maxActionsPerTick,
    1,
    60,
    warn
  );
  const maxActionsPerSecond = coerceInt(
    'maxActionsPerSecond',
    input.maxActionsPerSecond,
    DEFAULT_CONFIG.maxActionsPerSecond,
    1,
    10000,
    warn
  );
  const checkpointEveryGenerations = coerceInt(
    'checkpointEveryGenerations',
    input.checkpointEveryGenerations,
    DEFAULT_CONFIG.checkpointEveryGenerations,
    0,
    100000,
    warn
  );
  const rawDbPath = input.dbPath;
  const dbPath = rawDbPath && rawDbPath.trim()
    ? rawDbPath
    : DEFAULT_CONFIG.dbPath;
  if (rawDbPath !== undefined && (!rawDbPath || !rawDbPath.trim())) {
    warn?.(`dbPath is invalid; using ${dbPath}.`);
  }
  let logLevel = DEFAULT_CONFIG.logLevel;
  if (input.logLevel && LOG_LEVELS.includes(input.logLevel)) {
    logLevel = input.logLevel;
  } else if (input.logLevel) {
    warn?.(`logLevel "${input.logLevel}" is invalid; using ${logLevel}.`);
  }

  let seed: number | undefined;
  if (input.seed !== undefined) {
    const parsedSeed =
      typeof input.seed === 'number'
        ? input.seed
        : Number.parseInt(String(input.seed), 10);
    if (Number.isFinite(parsedSeed)) {
      seed = Math.floor(parsedSeed);
    } else {
      warn?.('seed is invalid; ignoring.');
    }
  }

  const output: ServerConfig = {
    host,
    port,
    tickRateHz,
    uiFrameRateHz,
    actionTimeoutTicks,
    maxActionsPerTick,
    maxActionsPerSecond,
    dbPath,
    checkpointEveryGenerations,
    logLevel
  };
  if (seed !== undefined) output.seed = seed;
  return output;
}

export function parseConfig(argv: string[], env: Env): ServerConfig {
  const input: Partial<ServerConfig> = {};
  const host = getArgValue(argv, '--host') ?? env['HOST'];
  if (host) input.host = host;
  const port = parseIntValue(getArgValue(argv, '--port')) ?? parseIntValue(env['PORT']);
  if (port !== undefined) input.port = port;
  const tickRate =
    parseIntValue(getArgValue(argv, '--tick')) ?? parseIntValue(env['TICK_RATE']);
  if (tickRate !== undefined) input.tickRateHz = tickRate;
  const uiRate =
    parseIntValue(getArgValue(argv, '--ui-rate')) ?? parseIntValue(env['UI_RATE']);
  if (uiRate !== undefined) input.uiFrameRateHz = uiRate;
  const actionTimeout =
    parseIntValue(getArgValue(argv, '--action-timeout')) ??
    parseIntValue(env['ACTION_TIMEOUT_TICKS']);
  if (actionTimeout !== undefined) input.actionTimeoutTicks = actionTimeout;
  const maxActionsPerTick =
    parseIntValue(getArgValue(argv, '--actions-per-tick')) ??
    parseIntValue(env['ACTIONS_PER_TICK']);
  if (maxActionsPerTick !== undefined) input.maxActionsPerTick = maxActionsPerTick;
  const maxActionsPerSecond =
    parseIntValue(getArgValue(argv, '--actions-per-second')) ??
    parseIntValue(env['ACTIONS_PER_SECOND']);
  if (maxActionsPerSecond !== undefined) input.maxActionsPerSecond = maxActionsPerSecond;
  const dbPath = getArgValue(argv, '--db-path') ?? env['DB_PATH'];
  if (dbPath) input.dbPath = dbPath;
  const checkpointEvery =
    parseIntValue(getArgValue(argv, '--checkpoint-every')) ??
    parseIntValue(env['CHECKPOINT_EVERY']);
  if (checkpointEvery !== undefined) input.checkpointEveryGenerations = checkpointEvery;
  const logLevel = (getArgValue(argv, '--log') ?? env['LOG_LEVEL']) as
    | LogLevel
    | undefined;
  if (logLevel) input.logLevel = logLevel;
  const seed =
    parseIntValue(getArgValue(argv, '--seed')) ?? parseIntValue(env['WORLD_SEED']);
  if (seed !== undefined) input.seed = seed;
  return normalizeConfig(input, (msg) => console.warn(`[config] ${msg}`));
}

import fs from 'node:fs';
import path from 'node:path';
import { parse as parseToml, stringify as stringifyToml } from 'smol-toml';

/** Allowed log levels for server output. */
export type LogLevel = 'debug' | 'info' | 'warn' | 'error';

/** Server runtime configuration values derived from defaults, config, env, and CLI. */
export interface ServerConfig {
  host: string;
  port: number;
  /** Hostname or IP for the UI dev server bind. */
  uiHost: string;
  /** Port for the UI dev server bind. */
  uiPort: number;
  /** Optional default WebSocket URL for the UI when no override is provided. */
  publicWsUrl: string;
  tickRateHz: number;
  uiFrameRateHz: number;
  actionTimeoutTicks: number;
  maxActionsPerTick: number;
  maxActionsPerSecond: number;
  dbPath: string;
  checkpointEveryGenerations: number;
  logLevel: LogLevel;
  /** Enable server-side MT inference. */
  mtEnabled: boolean;
  /** Requested worker count (0 for auto). */
  mtWorkers: number;
  seed?: number;
}

/** Default server configuration used before overrides are applied. */
export const DEFAULT_CONFIG: ServerConfig = {
  host: '127.0.0.1',
  port: 5174,
  uiHost: '127.0.0.1',
  uiPort: 5173,
  publicWsUrl: '',
  tickRateHz: 60,
  uiFrameRateHz: 30,
  actionTimeoutTicks: 10,
  maxActionsPerTick: 1,
  maxActionsPerSecond: 120,
  dbPath: './data/slither.db',
  checkpointEveryGenerations: 0,
  logLevel: 'info',
  mtEnabled: false,
  mtWorkers: 0
};

/** Shape of a process environment map. */
type Env = Record<string, string | undefined>;
/** Partial raw config input before validation and coercion. */
type RawConfigInput = Partial<Record<keyof ServerConfig, unknown>>;

/** Supported log levels for validation. */
const LOG_LEVELS: LogLevel[] = ['debug', 'info', 'warn', 'error'];
/** Default TOML config file path relative to the repo root. */
const DEFAULT_CONFIG_PATH = 'server/config.toml';

/**
 * Clamp a numeric value to a fixed integer range.
 * @param value - Input value to clamp.
 * @param min - Inclusive minimum.
 * @param max - Inclusive maximum.
 * @returns The clamped integer.
 */
function clampInt(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

/**
 * Parse a decimal integer from a string, returning undefined for invalid input.
 * @param raw - Raw string to parse.
 * @returns Parsed integer or undefined when invalid.
 */
function parseIntValue(raw: string | undefined): number | undefined {
  if (raw == null) return undefined;
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed)) return undefined;
  return parsed;
}

/**
 * Parse a boolean from a string value.
 * @param raw - Raw string to parse.
 * @returns Parsed boolean or undefined when invalid.
 */
function parseBoolValue(raw: string | undefined): boolean | undefined {
  if (raw == null) return undefined;
  const normalized = raw.trim().toLowerCase();
  if (!normalized) return undefined;
  if (['1', 'true', 'yes', 'on'].includes(normalized)) return true;
  if (['0', 'false', 'no', 'off'].includes(normalized)) return false;
  return undefined;
}

/**
 * Resolve a CLI argument value for a flag, supporting `--flag value` and `--flag=value`.
 * @param argv - Argument vector to scan.
 * @param flag - Flag name to match (e.g. `--port`).
 * @returns The resolved value if present.
 */
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

/**
 * Check if a CLI flag is present in the argument list.
 * @param argv - Argument vector to scan.
 * @param flag - Flag name to match.
 * @returns True when the flag exists in argv.
 */
function hasArgFlag(argv: string[], flag: string): boolean {
  return argv.includes(flag);
}

/**
 * Coerce an arbitrary value into a bounded integer with warnings.
 * @param name - Name for warning messages.
 * @param value - Raw value to coerce.
 * @param fallback - Fallback value when coercion fails.
 * @param min - Inclusive minimum value.
 * @param max - Inclusive maximum value.
 * @param warn - Optional warning callback.
 * @returns The validated, clamped integer value.
 */
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

/**
 * Normalize raw config input into a validated server config object.
 * @param input - Raw config data to normalize.
 * @param warn - Optional warning callback.
 * @returns The normalized configuration object.
 */
export function normalizeConfig(
  input: RawConfigInput,
  warn?: (msg: string) => void
): ServerConfig {
  const port = coerceInt('port', input.port, DEFAULT_CONFIG.port, 1, 65535, warn);
  const rawHost = typeof input.host === 'string' ? input.host : '';
  const host = rawHost.trim() ? rawHost : DEFAULT_CONFIG.host;
  if (input.host !== undefined && !rawHost.trim()) {
    warn?.(`host is invalid; using ${host}.`);
  }
  const rawUiHost = typeof input.uiHost === 'string' ? input.uiHost : '';
  const uiHost = rawUiHost.trim() ? rawUiHost : DEFAULT_CONFIG.uiHost;
  if (input.uiHost !== undefined && !rawUiHost.trim()) {
    warn?.(`uiHost is invalid; using ${uiHost}.`);
  }
  const uiPort = coerceInt('uiPort', input.uiPort, DEFAULT_CONFIG.uiPort, 1, 65535, warn);
  const rawPublicWsUrl = typeof input.publicWsUrl === 'string' ? input.publicWsUrl : '';
  const publicWsUrl = rawPublicWsUrl.trim() || DEFAULT_CONFIG.publicWsUrl;
  if (input.publicWsUrl !== undefined && typeof input.publicWsUrl !== 'string') {
    warn?.('publicWsUrl is invalid; leaving unset.');
  }
  const tickRateHz = coerceInt(
    'tickRateHz',
    input.tickRateHz,
    DEFAULT_CONFIG.tickRateHz,
    1,
    240,
    warn
  );
  // Ensure the UI frame rate does not exceed the server tick rate.
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
  const rawDbPath = typeof input.dbPath === 'string' ? input.dbPath : '';
  const dbPath = rawDbPath.trim() ? rawDbPath : DEFAULT_CONFIG.dbPath;
  if (input.dbPath !== undefined && !rawDbPath.trim()) {
    warn?.(`dbPath is invalid; using ${dbPath}.`);
  }
  let logLevel = DEFAULT_CONFIG.logLevel;
  const rawLogLevel = typeof input.logLevel === 'string' ? input.logLevel : '';
  if (rawLogLevel && LOG_LEVELS.includes(rawLogLevel as LogLevel)) {
    logLevel = rawLogLevel as LogLevel;
  } else if (input.logLevel !== undefined) {
    warn?.(`logLevel "${String(input.logLevel)}" is invalid; using ${logLevel}.`);
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

  let mtEnabled = DEFAULT_CONFIG.mtEnabled;
  if (input.mtEnabled !== undefined) {
    if (typeof input.mtEnabled === 'boolean') {
      mtEnabled = input.mtEnabled;
    } else if (typeof input.mtEnabled === 'string') {
      const parsed = parseBoolValue(input.mtEnabled);
      if (parsed !== undefined) mtEnabled = parsed;
      else warn?.('mtEnabled is invalid; using false.');
    } else {
      warn?.('mtEnabled is invalid; using false.');
    }
  }
  const mtWorkers = coerceInt('mtWorkers', input.mtWorkers, DEFAULT_CONFIG.mtWorkers, 0, 128, warn);

  const output: ServerConfig = {
    host,
    port,
    uiHost,
    uiPort,
    publicWsUrl,
    tickRateHz,
    uiFrameRateHz,
    actionTimeoutTicks,
    maxActionsPerTick,
    maxActionsPerSecond,
    dbPath,
    checkpointEveryGenerations,
    logLevel,
    mtEnabled,
    mtWorkers
  };
  if (seed !== undefined) output.seed = seed;
  return output;
}

/**
 * Build the default TOML contents for the config file.
 * @returns TOML-formatted string with defaults and comments.
 */
function defaultConfigToml(): string {
  const base = stringifyToml(DEFAULT_CONFIG).trim();
  const seedHint = '# seed = 12345 # optional: fixed world seed\n';
  const publicWsHint =
    '# publicWsUrl overrides the UI default when no ?server= override is used.\n' +
    '# Leave it blank to use the UI hostname + server port.\n';
  const uiHint = '# uiHost/uiPort control the Vite dev server bind.\n';
  return `# Slither Neuroevo server configuration (TOML)\n${seedHint}${publicWsHint}${uiHint}${base}\n`;
}

/**
 * Resolve the config path from argv/env, falling back to the default file location.
 * @param argv - CLI arguments to inspect.
 * @param env - Environment variables.
 * @returns Config path to use.
 */
function resolveConfigPath(argv: string[], env: Env): string {
  return getArgValue(argv, '--config') ?? env['SERVER_CONFIG'] ?? DEFAULT_CONFIG_PATH;
}

/**
 * Ensure the config file exists by creating a default if missing.
 * @param filePath - Path to the TOML file.
 * @param warn - Optional warning callback.
 */
function ensureConfigFile(filePath: string, warn?: (msg: string) => void): void {
  if (fs.existsSync(filePath)) return;
  const dir = path.dirname(filePath);
  if (dir && dir !== '.') fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(filePath, defaultConfigToml(), 'utf8');
  warn?.(`config file missing; created default at ${filePath}.`);
}

/**
 * Parse a TOML object into raw config input.
 * @param raw - Parsed TOML data.
 * @param warn - Optional warning callback.
 * @returns Partial raw config input.
 */
function parseConfigFile(raw: unknown, warn?: (msg: string) => void): RawConfigInput {
  if (!raw || typeof raw !== 'object') {
    warn?.('config file must be a TOML table; ignoring.');
    return {};
  }
  const data = raw as Record<string, unknown>;
  return {
    host: data['host'],
    port: data['port'],
    uiHost: data['uiHost'],
    uiPort: data['uiPort'],
    publicWsUrl: data['publicWsUrl'],
    tickRateHz: data['tickRateHz'],
    uiFrameRateHz: data['uiFrameRateHz'],
    actionTimeoutTicks: data['actionTimeoutTicks'],
    maxActionsPerTick: data['maxActionsPerTick'],
    maxActionsPerSecond: data['maxActionsPerSecond'],
    dbPath: data['dbPath'],
    checkpointEveryGenerations: data['checkpointEveryGenerations'],
    logLevel: data['logLevel'],
    seed: data['seed'],
    mtEnabled: data['mtEnabled'],
    mtWorkers: data['mtWorkers']
  };
}

/**
 * Load the config file, creating it when missing.
 * @param filePath - Path to the TOML file.
 * @param warn - Optional warning callback.
 * @returns Partial raw config input from file.
 */
function loadConfigFile(filePath: string, warn?: (msg: string) => void): RawConfigInput {
  ensureConfigFile(filePath, warn);
  if (!fs.existsSync(filePath)) return {};
  const raw = fs.readFileSync(filePath, 'utf8');
  if (!raw.trim()) return {};
  try {
    const parsed = parseToml(raw) as unknown;
    return parseConfigFile(parsed, warn);
  } catch (err) {
    warn?.(`failed to parse TOML config: ${(err as Error).message}`);
    return {};
  }
}

/**
 * Parse config overrides from config file, env vars, and CLI args.
 * @param argv - CLI arguments array.
 * @param env - Environment variables map.
 * @returns Normalized server config ready for runtime use.
 */
export function parseConfig(argv: string[], env: Env): ServerConfig {
  const warn = (msg: string) => console.warn(`[config] ${msg}`);
  const configPath = resolveConfigPath(argv, env);
  const input: RawConfigInput = {
    ...loadConfigFile(configPath, warn)
  };
  const host = getArgValue(argv, '--host') ?? env['HOST'];
  if (host) input.host = host;
  const port = parseIntValue(getArgValue(argv, '--port')) ?? parseIntValue(env['PORT']);
  if (port !== undefined) input.port = port;
  const uiHost = getArgValue(argv, '--ui-host') ?? env['UI_HOST'];
  if (uiHost) input.uiHost = uiHost;
  const uiPort = parseIntValue(getArgValue(argv, '--ui-port')) ?? parseIntValue(env['UI_PORT']);
  if (uiPort !== undefined) input.uiPort = uiPort;
  const publicWsUrl = getArgValue(argv, '--public-ws-url') ?? env['PUBLIC_WS_URL'];
  if (publicWsUrl) input.publicWsUrl = publicWsUrl;
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
  const mtRaw = getArgValue(argv, '--mt') ?? env['MT_ENABLED'];
  const mtFlag = mtRaw ?? (hasArgFlag(argv, '--mt') ? 'true' : undefined);
  const mtEnabled = parseBoolValue(mtFlag);
  if (mtEnabled !== undefined) input.mtEnabled = mtEnabled;
  const mtWorkers =
    parseIntValue(getArgValue(argv, '--mt-workers')) ?? parseIntValue(env['MT_WORKERS']);
  if (mtWorkers !== undefined) input.mtWorkers = mtWorkers;
  return normalizeConfig(input, warn);
}

import fs from 'node:fs';
import path from 'node:path';
import { parse as parseToml, stringify as stringifyToml } from 'smol-toml';
import { RUST_CALCULATION_WORKER_MAX, validateRustCalculationWorkers } from './rustWorkers.ts';
import { DEFAULT_DISTILLATION_SPOOL_GIB, validateDistillationSpoolGiB } from './rustEngine/distillationSpoolLimits.ts';

/** Allowed log levels for server output. */
export type LogLevel = 'debug' | 'info' | 'warn' | 'error';
/** Existing per-authority native allocation admission budget, in MiB. */
export const DEFAULT_NATIVE_MEMORY_MIB = 4096;

/** Validate an explicit resource budget without silently clamping or changing game settings. */
export function validateNativeMemoryMiB(value: unknown = DEFAULT_NATIVE_MEMORY_MIB): number {
  const parsed = typeof value === 'number' ? value :
    typeof value === 'string' && /^[1-9][0-9]*$/u.test(value) ? Number(value) : NaN;
  if (!Number.isSafeInteger(parsed) || parsed < 256 || parsed > 65536) {
    throw new RangeError('nativeMemoryMiB requires an integer from 256 to 65536 MiB');
  }
  return parsed;
}
/** Stable arithmetic identities accepted by normal startup and checkpoint admission. */
export const INFERENCE_MATH_BACKENDS = [
  'rust-scalar-v1', 'rust-sse2-v1', 'rust-i16-matrix-fma128-v1',
  'rust-i16-affine-fma128-sse2-v1'
] as const;
/** Explicit native inference arithmetic; omission preserves fresh SSE2 / recorded restore. */
export type InferenceMathBackend = typeof INFERENCE_MATH_BACKENDS[number];
/** Existing native CLI spellings also accepted by the production selector. */
const MATH_BACKEND_ALIASES = {
  scalar: 'rust-scalar-v1', sse2: 'rust-sse2-v1',
  'i16-matrix-fma128': 'rust-i16-matrix-fma128-v1',
  'i16-affine-fma128-sse2': 'rust-i16-affine-fma128-sse2-v1'
} as const;
/** Complete CLI choice set; resolved configuration always carries a stable identity. */
export const INFERENCE_MATH_BACKEND_CHOICES = [...INFERENCE_MATH_BACKENDS, ...Object.keys(MATH_BACKEND_ALIASES)];

/** Validate an explicit request without silently substituting another arithmetic backend. */
export function parseInferenceMathBackend(value: unknown): InferenceMathBackend {
  if (typeof value === 'string') {
    if ((INFERENCE_MATH_BACKENDS as readonly string[]).includes(value)) return value as InferenceMathBackend;
    if (Object.hasOwn(MATH_BACKEND_ALIASES, value)) return MATH_BACKEND_ALIASES[value as keyof typeof MATH_BACKEND_ALIASES];
  }
  throw new RangeError(`mathBackend requires one of: ${INFERENCE_MATH_BACKEND_CHOICES.join(', ')}`);
}
/** Startup population-selection policy. */
export type ResumeSelection = 'auto' | 'latest' | 'fresh' | `sha256:${string}`;

/** Server runtime configuration values derived from defaults, config, env, and CLI. */
export interface ServerConfig {
  host: string;
  port: number;
  /** Hostname or IP for the UI dev server bind. */
  uiHost: string;
  /** Port for the UI dev server bind. */
  uiPort: number;
  /** Optional default WebSocket URL for UI clients on a different host. */
  publicWsUrl: string;
  uiFrameRateHz: number;
  maxActionsPerTick: number;
  maxActionsPerSecond: number;
  /** Wall milliseconds to hold the newest accepted external action. */
  controllerInputHoldMs: number;
  /** Wall milliseconds to reserve disconnected controller ownership. */
  controllerDisconnectGraceMs: number;
  dbPath: string;
  checkpointEveryGenerations: number;
  /** Managed-store physical budget in MiB, excluding owner-pinned checkpoints. */
  checkpointBudgetMiB: number;
  /** Encoded dataset disk quota in GiB, outside world checkpoints and native scratch. */
  distillationSpoolGiB: number;
  logLevel: LogLevel;
  /** Rust calculation threads within the process-available logical CPU count. */
  rustCalculationWorkers: number;
  /** Explicit native arithmetic; absent leaves fresh SSE2 and recorded checkpoint arithmetic intact. */
  mathBackend?: InferenceMathBackend;
  /** Optional native source/successor/workspace admission budget; omission retains 4096 MiB. */
  nativeMemoryMiB?: number;
  /** Automatic first startup, explicit fresh/latest startup, or one managed checkpoint identity. */
  resume: ResumeSelection;
  /** One-shot population-only source for an explicit fresh lineage, never a resume override. */
  populationFrom?: string;
  seed?: number;
}

/** Default server configuration used before overrides are applied. */
export const DEFAULT_CONFIG: ServerConfig = {
  host: '127.0.0.1',
  port: 5174,
  uiHost: '127.0.0.1',
  uiPort: 5173,
  publicWsUrl: '',
  uiFrameRateHz: 36,
  maxActionsPerTick: 1,
  maxActionsPerSecond: 120,
  controllerInputHoldMs: 500,
  controllerDisconnectGraceMs: 30_000,
  dbPath: './data/slither.db',
  checkpointEveryGenerations: 1,
  checkpointBudgetMiB: 4096,
  distillationSpoolGiB: DEFAULT_DISTILLATION_SPOOL_GIB,
  logLevel: 'info',
  rustCalculationWorkers: 5,
  resume: 'auto'
};

/** Shape of a process environment map. */
type Env = Record<string, string | undefined>;
/** Partial raw config input before validation and coercion. */
type RawConfigInput = Partial<Record<keyof ServerConfig, unknown>>;

/** Immutable native fixed-step frequency; display publication cannot exceed it. */
export const NATIVE_STEP_RATE_HZ = 60;

/** Validate retired configuration keys without exposing them in the active runtime. */
function validateLegacyKeys(input: Record<string, unknown>, warn?: (message: string) => void): void {
  const neutral: Record<string, unknown> = { inferenceBackend: 'native', mtEnabled: false, mtWorkers: 0, tickRateHz: NATIVE_STEP_RATE_HZ };
  for (const [key, expected] of Object.entries(neutral)) {
    const raw = input[key];
    if (raw === undefined) continue;
    const value = typeof expected === 'boolean' ? (typeof raw === 'boolean' ? raw : typeof raw === 'string' ? parseBoolValue(raw) : undefined) :
      typeof expected === 'number' ? (typeof raw === 'number' ? raw : typeof raw === 'string' && /^[0-9]+$/u.test(raw) ? Number(raw) : undefined) :
      typeof raw === 'string' ? raw.trim().toLowerCase() : undefined;
    if (value !== expected) throw new Error(`${key} was removed; only its neutral legacy value ${String(expected)} is accepted. Use --rust-workers for native workers.`);
    warn?.(`${key} is obsolete and ignored; remove it from configuration.`);
  }
}

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
 * Normalize a startup resume selector from TOML, environment, or CLI text.
 * @param value - Raw selector value.
 * @param warn - Optional warning callback for invalid non-CLI input.
 * @returns Automatic, fresh, latest, or one managed SHA-256 checkpoint identity.
 */
function normalizeResumeSelection(
  value: unknown,
  warn?: (msg: string) => void
): ResumeSelection {
  if (value === undefined || value === null || value === '') return DEFAULT_CONFIG.resume;
  if (value === 'auto' || value === 'latest' || value === 'fresh') return value;
  const text = String(value).trim();
  if (/^(?:sha256:)?[0-9a-f]{64}$/u.test(text)) return `sha256:${text.replace(/^sha256:/u, '')}`;
  if (typeof value === 'number' || /^[0-9]+$/u.test(text)) throw new Error('numeric reference snapshot IDs were removed; use latest for legacy conversion or sha256:ID for native checkpoints');
  warn?.(`resume selector "${String(value)}" is invalid; using ${DEFAULT_CONFIG.resume}.`);
  return DEFAULT_CONFIG.resume;
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
      const next = argv[i + 1];
      return next && !next.startsWith('--') ? next : undefined;
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

/** Require an exact configured storage budget rather than silently clamping it. */
function checkpointBudgetMiB(value: unknown): number {
  if (value === undefined) return DEFAULT_CONFIG.checkpointBudgetMiB;
  const parsed = typeof value === 'number' ? value :
    typeof value === 'string' && /^(?:0|[1-9][0-9]*)$/u.test(value)
      ? Number(value) : Number.NaN;
  if (!Number.isSafeInteger(parsed) || parsed < 1_280 || parsed > 65_536) {
    throw new RangeError('checkpointBudgetMiB must be an integer from 1280 to 65536');
  }
  return parsed;
}

/**
 * Normalize raw config input into a validated server config object.
 * @param input - Raw config data to normalize.
 * @param warn - Optional warning callback.
 * @param rustWorkerMaximum - Detected Rust worker ceiling; injectable for tests.
 * @returns The normalized configuration object.
 */
export function normalizeConfig(
  input: RawConfigInput,
  warn?: (msg: string) => void,
  rustWorkerMaximum = RUST_CALCULATION_WORKER_MAX
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
  validateLegacyKeys(input as Record<string, unknown>, warn);
  const uiFrameRateHz = coerceInt('uiFrameRateHz', input.uiFrameRateHz, DEFAULT_CONFIG.uiFrameRateHz, 1, NATIVE_STEP_RATE_HZ, warn);
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
  const controllerInputHoldMs = coerceInt(
    'controllerInputHoldMs',
    input.controllerInputHoldMs,
    DEFAULT_CONFIG.controllerInputHoldMs,
    0,
    60_000,
    warn
  );
  const controllerDisconnectGraceMs = coerceInt(
    'controllerDisconnectGraceMs',
    input.controllerDisconnectGraceMs,
    DEFAULT_CONFIG.controllerDisconnectGraceMs,
    0,
    10 * 60_000,
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
  const selectedCheckpointBudgetMiB = checkpointBudgetMiB(input.checkpointBudgetMiB);
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

  const rustWorkerDefault = validateRustCalculationWorkers(
    Math.min(DEFAULT_CONFIG.rustCalculationWorkers, rustWorkerMaximum), rustWorkerMaximum);
  const rustCalculationWorkers = coerceInt('rustCalculationWorkers', input.rustCalculationWorkers,
    rustWorkerDefault,
    1, rustWorkerMaximum,
    warn ? message => warn(`${message} Valid Rust worker range: 1..${rustWorkerMaximum} (available logical CPUs).`) : undefined);
  const resume = normalizeResumeSelection(input.resume, warn);
  if (input.populationFrom !== undefined &&
      (typeof input.populationFrom !== 'string' || !/^sha256:[0-9a-f]{64}$/u.test(input.populationFrom) ||
       resume !== 'fresh' || input.mathBackend === undefined || seed !== undefined)) {
    throw new Error('--population-from requires sha256:ID, --fresh, an explicit math backend, and no seed override');
  }

  const output: ServerConfig = {
    host,
    port,
    uiHost,
    uiPort,
    publicWsUrl,
    uiFrameRateHz,
    maxActionsPerTick,
    maxActionsPerSecond,
    controllerInputHoldMs,
    controllerDisconnectGraceMs,
    dbPath,
    checkpointEveryGenerations,
    checkpointBudgetMiB: selectedCheckpointBudgetMiB,
    distillationSpoolGiB: validateDistillationSpoolGiB(input.distillationSpoolGiB),
    logLevel,
    rustCalculationWorkers,
    ...(input.mathBackend === undefined ? {} : { mathBackend: parseInferenceMathBackend(input.mathBackend) }),
    ...(input.nativeMemoryMiB === undefined ? {} : { nativeMemoryMiB: validateNativeMemoryMiB(input.nativeMemoryMiB) }),
    ...(input.populationFrom === undefined ? {} : { populationFrom: input.populationFrom as string }),
    resume
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
    '# publicWsUrl tells browsers where to find the simulation server when it differs from the UI host.\n' +
    '# Leave it blank to use the UI hostname plus the configured server port.\n';
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
  validateLegacyKeys(data, warn);
  return {
    host: data['host'],
    port: data['port'],
    uiHost: data['uiHost'],
    uiPort: data['uiPort'],
    publicWsUrl: data['publicWsUrl'],
    uiFrameRateHz: data['uiFrameRateHz'],
    maxActionsPerTick: data['maxActionsPerTick'],
    maxActionsPerSecond: data['maxActionsPerSecond'],
    controllerInputHoldMs: data['controllerInputHoldMs'],
    controllerDisconnectGraceMs: data['controllerDisconnectGraceMs'],
    dbPath: data['dbPath'],
    checkpointEveryGenerations: data['checkpointEveryGenerations'],
    checkpointBudgetMiB: data['checkpointBudgetMiB'],
    distillationSpoolGiB: data['distillationSpoolGiB'],
    logLevel: data['logLevel'],
    seed: data['seed'],
    rustCalculationWorkers: data['rustCalculationWorkers'],
    mathBackend: data['mathBackend'],
    nativeMemoryMiB: data['nativeMemoryMiB'],
    resume: data['resume']
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
    return parsed as RawConfigInput;
  } catch (err) {
    warn?.(`failed to parse TOML config: ${(err as Error).message}`);
    return {};
  }
}

/**
 * Parse config overrides from config file, env vars, and CLI args.
 * @param argv - CLI arguments array.
 * @param env - Environment variables map.
 * @param rustWorkerMaximum - Detected Rust worker ceiling shared by all override paths.
 * @returns Normalized server config ready for runtime use.
 */
export function parseConfig(argv: string[], env: Env, rustWorkerMaximum = RUST_CALCULATION_WORKER_MAX): ServerConfig {
  for (const arg of argv) {
    if (['--backend', '--mt', '--mt-workers', '--tick'].includes(arg.split('=')[0]!)) {
      throw new Error(`${arg.split('=')[0]} was removed; use --rust-workers for native workers. The simulation step rate is fixed at 60 Hz.`);
    }
  }
  const warn = (msg: string) => console.warn(`[config] ${msg}`);
  validateLegacyKeys({ inferenceBackend: env['INFERENCE_BACKEND'], mtEnabled: env['MT_ENABLED'], mtWorkers: env['MT_WORKERS'], tickRateHz: env['TICK_RATE'] }, warn);
  const cliMath = getArgValue(argv, '--math-backend');
  if (argv.some(arg => arg === '--math-backend' || arg.startsWith('--math-backend=')) && cliMath === undefined) {
    throw new Error('--math-backend requires a value');
  }
  const selectedMath = cliMath ?? env['MATH_BACKEND'];
  if (selectedMath !== undefined) parseInferenceMathBackend(selectedMath);
  const cliNativeMemory = getArgValue(argv, '--native-memory-mib');
  if (argv.some(arg => arg === '--native-memory-mib' || arg.startsWith('--native-memory-mib=')) && cliNativeMemory === undefined) {
    throw new Error('--native-memory-mib requires a value');
  }
  const nativeMemoryMiB = cliNativeMemory ?? env['NATIVE_MEMORY_MIB'];
  if (nativeMemoryMiB !== undefined) validateNativeMemoryMiB(nativeMemoryMiB);
  const populationFrom = getArgValue(argv, '--population-from') ?? env['SERVER_POPULATION_FROM'];
  if (populationFrom !== undefined && !/^sha256:[0-9a-f]{64}$/u.test(populationFrom)) {
    throw new Error('--population-from requires sha256:ID with a lowercase managed checkpoint digest');
  }
  const cliSpoolGiB = getArgValue(argv, '--distillation-spool-gib');
  if (argv.some(arg => arg === '--distillation-spool-gib' || arg.startsWith('--distillation-spool-gib=')) &&
      cliSpoolGiB === undefined) throw new Error('--distillation-spool-gib requires a value');
  const selectedSpoolGiB = cliSpoolGiB ?? env['DISTILLATION_SPOOL_GIB'];
  // Resource overrides reject before a missing owner config file could be created.
  if (selectedSpoolGiB !== undefined) validateDistillationSpoolGiB(selectedSpoolGiB);
  const configPath = resolveConfigPath(argv, env);
  const input: RawConfigInput = {
    ...parseConfigFile(loadConfigFile(configPath, warn), warn)
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
  const uiRate =
    parseIntValue(getArgValue(argv, '--ui-rate')) ?? parseIntValue(env['UI_RATE']);
  if (uiRate !== undefined) input.uiFrameRateHz = uiRate;
  const maxActionsPerTick =
    parseIntValue(getArgValue(argv, '--actions-per-tick')) ??
    parseIntValue(env['ACTIONS_PER_TICK']);
  if (maxActionsPerTick !== undefined) input.maxActionsPerTick = maxActionsPerTick;
  const maxActionsPerSecond =
    parseIntValue(getArgValue(argv, '--actions-per-second')) ??
    parseIntValue(env['ACTIONS_PER_SECOND']);
  if (maxActionsPerSecond !== undefined) input.maxActionsPerSecond = maxActionsPerSecond;
  const controllerInputHoldMs =
    parseIntValue(getArgValue(argv, '--input-hold-ms')) ??
    parseIntValue(env['CONTROLLER_INPUT_HOLD_MS']);
  if (controllerInputHoldMs !== undefined) input.controllerInputHoldMs = controllerInputHoldMs;
  const controllerDisconnectGraceMs =
    parseIntValue(getArgValue(argv, '--disconnect-grace-ms')) ??
    parseIntValue(env['CONTROLLER_DISCONNECT_GRACE_MS']);
  if (controllerDisconnectGraceMs !== undefined) {
    input.controllerDisconnectGraceMs = controllerDisconnectGraceMs;
  }
  const dbPath = getArgValue(argv, '--db-path') ?? env['DB_PATH'];
  if (dbPath) input.dbPath = dbPath;
  const checkpointEvery =
    parseIntValue(getArgValue(argv, '--checkpoint-every')) ??
    parseIntValue(env['CHECKPOINT_EVERY']);
  if (checkpointEvery !== undefined) input.checkpointEveryGenerations = checkpointEvery;
  const cliCheckpointBudget = getArgValue(argv, '--checkpoint-budget-mib');
  if (argv.some(arg => arg === '--checkpoint-budget-mib' || arg.startsWith('--checkpoint-budget-mib=')) &&
      cliCheckpointBudget === undefined) throw new Error('--checkpoint-budget-mib requires a value');
  const checkpointBudget = cliCheckpointBudget ?? env['CHECKPOINT_BUDGET_MIB'];
  if (checkpointBudget !== undefined) input.checkpointBudgetMiB = checkpointBudget;
  const logLevel = (getArgValue(argv, '--log') ?? env['LOG_LEVEL']) as
    | LogLevel
    | undefined;
  if (logLevel) input.logLevel = logLevel;
  const seed =
    parseIntValue(getArgValue(argv, '--seed')) ?? parseIntValue(env['WORLD_SEED']);
  if (seed !== undefined) input.seed = seed;
  const rustWorkers = parseIntValue(getArgValue(argv, '--rust-workers')) ?? parseIntValue(env['RUST_WORKERS']);
  if (rustWorkers !== undefined) input.rustCalculationWorkers = rustWorkers;
  const hasFresh = hasArgFlag(argv, '--fresh');
  const hasResumeFlag = argv.some((arg) => arg === '--resume' || arg.startsWith('--resume='));
  if (hasFresh && hasResumeFlag) {
    throw new Error('--fresh and --resume are mutually exclusive');
  }
  const resumeRaw = getArgValue(argv, '--resume') ?? env['SERVER_RESUME'];
  if (hasFresh) {
    input.resume = 'fresh';
  } else if (hasResumeFlag) {
    if (!resumeRaw || resumeRaw.startsWith('--')) {
      throw new Error('--resume requires latest or sha256:ID with a managed SHA-256 checkpoint id');
    }
    const selection = normalizeResumeSelection(resumeRaw);
    if (selection === DEFAULT_CONFIG.resume && resumeRaw !== 'latest') {
      throw new Error(`invalid --resume selection: ${resumeRaw}`);
    }
    input.resume = selection;
  } else if (resumeRaw !== undefined) {
    input.resume = resumeRaw;
  }
  if (selectedSpoolGiB !== undefined) input.distillationSpoolGiB = selectedSpoolGiB;
  if (selectedMath !== undefined) input.mathBackend = parseInferenceMathBackend(selectedMath);
  if (nativeMemoryMiB !== undefined) input.nativeMemoryMiB = nativeMemoryMiB;
  if (populationFrom !== undefined) input.populationFrom = populationFrom;
  return normalizeConfig(input, warn, rustWorkerMaximum);
}

import { parseConfig, type ServerConfig } from './config.ts';

/** One production CLI option accepted before any configuration-file or game startup work. */
interface ProductionOption {
  /** Placeholder for a required value; absent for a boolean switch. */
  value?: string;
  /** Whether the value must be a complete safe integer. */
  integer?: boolean;
  /** Fixed production value required by the current Rust runtime. */
  fixed?: number;
  /** Finite set of accepted values when appropriate. */
  choices?: readonly string[];
  /** User-facing purpose and corresponding environment override. */
  description: string;
}

/** Explicit production interface; reference-only math/pool flags are handled separately below. */
const OPTIONS: Readonly<Record<string, ProductionOption>> = {
  '--config': { value: 'PATH', description: 'TOML configuration file (SERVER_CONFIG).' },
  '--host': { value: 'HOST', description: 'HTTP/WebSocket bind, default 127.0.0.1 (HOST).' },
  '--port': { value: 'N', integer: true, description: 'HTTP/WebSocket port, default 5174 (PORT).' },
  '--ui-host': { value: 'HOST', description: 'Trusted UI host (UI_HOST).' },
  '--ui-port': { value: 'N', integer: true, description: 'Trusted Vite UI port (UI_PORT).' },
  '--public-ws-url': { value: 'URL', description: 'Browser WebSocket route (PUBLIC_WS_URL).' },
  '--ui-rate': { value: 'N', integer: true, description: 'Display frame rate (UI_RATE).' },
  '--actions-per-tick': { value: 'N', integer: true, description: 'Accepted actions per controller/tick (ACTIONS_PER_TICK).' },
  '--actions-per-second': { value: 'N', integer: true, description: 'Controller attempt rate limit (ACTIONS_PER_SECOND).' },
  '--input-hold-ms': { value: 'N', integer: true, fixed: 500, description: 'Production action hold is fixed at 500 ms.' },
  '--disconnect-grace-ms': { value: 'N', integer: true, fixed: 30000, description: 'Production disconnect grace is fixed at 30000 ms.' },
  '--db-path': { value: 'PATH', description: 'Exact SQLite database path (DB_PATH).' },
  '--checkpoint-every': { value: 'N', integer: true, fixed: 1, description: 'Production checkpoints every generation; fixed at 1.' },
  '--checkpoint-budget-mib': { value: 'N', integer: true, description: 'Managed storage budget, 1280..65536 MiB (CHECKPOINT_BUDGET_MIB).' },
  '--log': { value: 'LEVEL', choices: ['debug', 'info', 'warn', 'error'], description: 'Logging level (LOG_LEVEL).' },
  '--seed': { value: 'N', integer: true, description: 'Run seed; requires --fresh when a database already exists (WORLD_SEED).' },
  '--rust-workers': { value: 'N', integer: true, description: 'Rust calculation workers, 1..7 (RUST_WORKERS).' },
  '--fresh': { description: 'Start a new generation-one run; conflicts with --resume.' },
  '--resume': { value: 'latest|sha256:ID', description: 'Select a retained managed checkpoint (SERVER_RESUME).' },
  '--help': { description: 'Print help and exit without creating configuration or starting the game.' }
};

/** Flags retained solely by the explicitly selected reference runtime. */
const REFERENCE_FLAGS = new Set(['--backend', '--mt', '--mt-workers', '--tick']);

/** Help generated from the same option surface used for complete-vector validation. */
export const PRODUCTION_CLI_HELP = [
  'Usage: npm run server -- [options]',
  '',
  'Rust-authoritative server. Loopback is the default; use --host 0.0.0.0 for deliberate trusted-LAN access.',
  'Values accept --option value or --option=value. The fixed-step rate is 60 Hz.',
  '',
  ...Object.entries(OPTIONS).map(([flag, option]) =>
    `  ${(flag + (option.value ? ' ' + option.value : '')).padEnd(38)}${option.description}`),
  '  -h                                    Alias for --help.',
  '',
  'Backend, Node pool and tick overrides belong to npm run server:reference.'
].join('\n');

/**
 * Validate every production argument before config loading, then resolve existing configuration precedence.
 * @param argv - Complete production argument vector.
 * @param env - Environment overrides accepted by the existing configuration parser.
 * @returns Configuration for startup, or null for a side-effect-free help request.
 */
export function parseProductionCli(argv: string[], env: NodeJS.ProcessEnv): ServerConfig | null {
  const seen = new Set<string>();
  for (let index = 0; index < argv.length; index++) {
    const argument = argv[index]!;
    const equals = argument.indexOf('=');
    const rawFlag = equals < 0 ? argument : argument.slice(0, equals);
    const flag = rawFlag === '-h' ? '--help' : rawFlag;
    if (REFERENCE_FLAGS.has(flag)) throw new Error(`${flag} is reference-only; use npm run server:reference`);
    const option = Object.hasOwn(OPTIONS, flag) ? OPTIONS[flag] : undefined;
    if (!option) throw new Error(`unknown production argument: ${argument}; use --help for supported options`);
    if (seen.has(flag)) throw new Error(`duplicate production option: ${flag}`);
    seen.add(flag);
    if (!option.value) {
      if (equals >= 0) throw new Error(`${flag} does not accept a value`);
      continue;
    }
    const value = equals < 0 ? argv[++index] : argument.slice(equals + 1);
    if (!value?.trim() || (equals < 0 && value.startsWith('-') && !/^-\d+$/u.test(value))) {
      throw new Error(`${flag} requires ${option.value}`);
    }
    if (option.integer && (!/^-?\d+$/u.test(value) || !Number.isSafeInteger(Number(value)))) {
      throw new Error(`${flag} requires a safe integer`);
    }
    if (option.fixed !== undefined && Number(value) !== option.fixed) {
      throw new Error(`${flag} is fixed at ${option.fixed} in production; other values are reference-only`);
    }
    if (option.choices && !option.choices.includes(value)) {
      throw new Error(`${flag} requires one of: ${option.choices.join(', ')}`);
    }
    if (flag === '--resume' && value !== 'latest' && !/^sha256:[0-9a-f]{64}$/u.test(value)) {
      throw new Error('--resume requires latest or sha256:ID with a lowercase SHA-256 digest');
    }
  }
  if (seen.has('--fresh') && seen.has('--resume')) {
    throw new Error('--fresh and --resume are mutually exclusive');
  }
  if (seen.has('--help')) return null;
  return parseConfig(argv, env);
}
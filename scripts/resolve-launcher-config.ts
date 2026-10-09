import { existsSync } from 'node:fs';
import { parseProductionCli } from '../server/productionCli.ts';

/** Quote one value for safe evaluation by a POSIX shell. */
function shellQuote(value: string): string {
  return `'${value.replaceAll("'", `'"'"'`)}'`;
}

/** Build only explicitly requested legacy launcher overrides. */
function launcherArgs(env: NodeJS.ProcessEnv): string[] {
  const args: string[] = [];
  const addValue = (flag: string, value: string | undefined): void => {
    if (value !== undefined && value !== '') args.push(flag, value);
  };

  addValue('--host', env['SLITHER_HOST']);
  addValue('--port', env['SLITHER_PORT']);
  addValue('--db-path', env['SLITHER_DB_PATH']);

  const startMode = env['SLITHER_START_MODE'];
  if (startMode === undefined || startMode === '') return args;
  if (startMode === 'fresh') {
    args.push('--fresh');
    return args;
  }
  if (startMode === 'resume') {
    args.push('--resume', env['SLITHER_RESUME_TARGET'] || 'latest');
    return args;
  }
  if (startMode !== 'auto') {
    throw new Error('SLITHER_START_MODE must be auto, fresh, or resume.');
  }

  // Explicit legacy "auto" remains an override. Resolve the configured database
  // path first, then reproduce the launcher's old fresh/existing-store choice.
  const baseConfig = parseProductionCli(args, env);
  if (baseConfig === null) throw new Error('launcher config resolution cannot request help');
  if (existsSync(baseConfig.dbPath)) {
    args.push('--resume', env['SLITHER_RESUME_TARGET'] || 'latest');
  } else {
    args.push('--fresh');
  }
  return args;
}

const args = launcherArgs(process.env);
const config = parseProductionCli(args, process.env);
if (config === null) throw new Error('launcher config resolution cannot request help');
const configPath = process.env['SERVER_CONFIG'] || 'server/config.toml';
const resume = String(config.resume);

process.stdout.write([
  `HOST=${shellQuote(config.host)}`,
  `PORT=${shellQuote(String(config.port))}`,
  `DB_PATH=${shellQuote(config.dbPath)}`,
  `RESOLVED_RESUME=${shellQuote(resume)}`,
  `RESOLVED_PUBLIC_WS_URL=${shellQuote(config.publicWsUrl)}`,
  `CONFIG_PATH=${shellQuote(configPath)}`,
  `set --${args.map(argument => ` ${shellQuote(argument)}`).join('')}`
].join('\n') + '\n');
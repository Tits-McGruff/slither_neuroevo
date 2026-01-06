import type { LogLevel } from './config.ts';

/** Numeric severity order for log filtering. */
const LEVELS: Record<LogLevel, number> = {
  debug: 10,
  info: 20,
  warn: 30,
  error: 40
};

/** Logger interface used by the server modules. */
export type Logger = {
  debug: (module: string, message: string) => void;
  info: (module: string, message: string) => void;
  warn: (module: string, message: string) => void;
  error: (module: string, message: string) => void;
};

/**
 * Build a console-backed logger that filters messages below a threshold.
 * @param level - Minimum log level to emit.
 * @returns Logger instance with module-aware helpers.
 */
export function createLogger(level: LogLevel): Logger {
  const threshold = LEVELS[level] ?? LEVELS.info;
  // Use a closure to format and filter logs consistently.
  const log = (lvl: LogLevel, module: string, message: string) => {
    if (LEVELS[lvl] < threshold) return;
    const stamp = new Date().toISOString();
    console.log(`${stamp} | ${lvl} | ${module} | ${message}`);
  };
  return {
    debug: (module, message) => log('debug', module, message),
    info: (module, message) => log('info', module, message),
    warn: (module, message) => log('warn', module, message),
    error: (module, message) => log('error', module, message)
  };
}

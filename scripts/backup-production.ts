import { resolve } from 'node:path';
import { createManagedBackup } from './managedBackup.ts';

/** Read one required CLI option in either `--name value` or `--name=value` form. */
function option(args: string[], name: string): string | undefined {
  const equals = args.find(value => value.startsWith(`${name}=`));
  if (equals) return equals.slice(name.length + 1);
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
}

/** Command-line arguments supplied to the backup command. */
const args = process.argv.slice(2);

/** Source SQLite path, matching the production default when omitted. */
const databasePath = resolve(option(args, '--db-path') ?? process.env['SLITHER_DB_PATH'] ?? './data/rust-authority.db');

/** Destination backup-set directory; it must not already exist. */
const outputDirectory = resolve(option(args, '--output') ??
  `./backups/slither-${new Date().toISOString().replaceAll(':', '').replaceAll('.', '-')}`);

void createManagedBackup({ databasePath, outputDirectory }).then(manifest => {
  console.info(`[OK] Backup created: ${outputDirectory}`);
  console.info(`[OK] ${manifest.managedFiles.length} managed files plus one SQLite snapshot verified.`);
}).catch(error => {
  console.error(`[ERROR] ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});

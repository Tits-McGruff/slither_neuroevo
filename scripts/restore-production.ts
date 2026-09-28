import { resolve } from 'node:path';
import { restoreManagedBackup } from './managedBackup.ts';

/** Read one required CLI option in either `--name value` or `--name=value` form. */
function option(args: string[], name: string): string | undefined {
  const equals = args.find(value => value.startsWith(`${name}=`));
  if (equals) return equals.slice(name.length + 1);
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
}

/** Command-line arguments supplied to the restore command. */
const args = process.argv.slice(2);

/** Backup directory selected explicitly by the operator. */
const backupDirectory = option(args, '--backup');

/** New SQLite target path; existing databases are never overwritten. */
const databasePath = option(args, '--db-path') ?? process.env['SLITHER_DB_PATH'];

/** Optional retained prior-run current boundary selected only in the new restored copy. */
const checkpointId = option(args, '--checkpoint-id');

if (!backupDirectory || !databasePath) {
  console.error('[ERROR] Usage: npm run restore:production -- --backup DIR --db-path NEW_DB_PATH');
  process.exitCode = 1;
} else {
  void restoreManagedBackup({
    backupDirectory: resolve(backupDirectory),
    databasePath: resolve(databasePath),
    ...(checkpointId ? { checkpointId } : {})
  }).then(manifest => {
    console.info(`[OK] Restored ${manifest.managedFiles.length} managed files and SQLite to ${resolve(databasePath)}`);
  }).catch(error => {
    console.error(`[ERROR] ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  });
}

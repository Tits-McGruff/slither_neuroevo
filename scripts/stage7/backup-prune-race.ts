import { resolve } from 'node:path';
import Database from 'better-sqlite3';
import { CheckpointPersistenceClient } from '../../server/rustEngine/checkpointPersistenceClient.ts';
import { createManagedBackup, restoreManagedBackup, validateManagedBackup } from '../managedBackup.ts';

/** Required command-line option supplied as a separate value. */
function option(args: string[], name: string): string {
  const index = args.indexOf(name);
  const value = index < 0 ? undefined : args[index + 1];
  if (!value || value.startsWith('--')) throw new Error(`missing ${name} value`);
  return value;
}

/** Exercise backup retry after the real persistence worker prunes the first snapshot's files. */
async function run(): Promise<void> {
  const args = process.argv.slice(2);
  const databasePath = resolve(option(args, '--db-path'));
  const outputDirectory = resolve(option(args, '--output'));
  const restoredPath = resolve(option(args, '--restore'));
  const budgetMiB = Number(option(args, '--checkpoint-budget-mib'));
  if (!Number.isSafeInteger(budgetMiB) || budgetMiB < 1_280 || budgetMiB > 65_536) {
    throw new RangeError('checkpoint budget must be from 1280 to 65536 MiB');
  }
  const persistence = new CheckpointPersistenceClient({ databasePath,
    managedRootPath: `${databasePath}.checkpoints`, existingOnly: true,
    automaticByteCapBytes: BigInt(budgetMiB) * 1024n * 1024n });
  const originalBackup = Database.prototype.backup;
  let snapshotCount = 0;
  let prunedCheckpoints = 0;
  try {
    Database.prototype.backup = async function(destinationFile, options) {
      const result = await originalBackup.call(this, destinationFile, options);
      snapshotCount++;
      if (snapshotCount === 1) {
        const pruned = await persistence.applyRetention();
        prunedCheckpoints = pruned.deletedCheckpointCount;
        if (prunedCheckpoints < 1) throw new Error('fixture did not prune a checkpoint after its first snapshot');
      }
      return result;
    };
    const manifest = await createManagedBackup({ databasePath, outputDirectory });
    if (snapshotCount !== 2) throw new Error(`backup took ${snapshotCount} snapshots instead of retrying once`);
    await validateManagedBackup(outputDirectory);
    await restoreManagedBackup({ backupDirectory: outputDirectory, databasePath: restoredPath });
    process.stdout.write(`${JSON.stringify({ snapshotCount, prunedCheckpoints,
      managedFiles: manifest.managedFiles.length, outputDirectory, restoredPath })}\n`);
  } finally {
    Database.prototype.backup = originalBackup;
    await persistence.close();
  }
}

void run().catch(error => {
  console.error(error);
  process.exitCode = 1;
});

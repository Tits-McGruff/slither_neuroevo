/** Bounded lineage queries and SQL projections; no lease or commit ownership. */
import { u64HexToBigInt } from './managed_files.ts';
import type Database from 'better-sqlite3';
import {
  parseManagedImportBranchResult,
  type ManagedImportBranchResult,
  type U64Hex
} from './checkpointPersistenceProtocol.ts';
import { parseRecoveryBranchResult, type RecoveryBranchResult } from './recoveryProtocol.ts';
/** One bounded ancestor and the inherited checkpoint prefix still eligible for recovery. */
interface RecoveryLineage {
  /** Run that physically owns immutable descriptor rows. */
  runId: string;
  /** Inclusive inherited boundary; null only for the failed active lineage. */
  maximumGeneration: U64Hex | null;
}

/** Read one strictly bounded branch record used to authorize an aliased source checkpoint. */
export function readRecoveryBranch(db: ReturnType<typeof Database>, runId: string): RecoveryBranchResult | undefined {
  const row = db.prepare(`SELECT CASE WHEN length(CAST(provenance_json AS BLOB)) <= 32768
    THEN provenance_json END AS provenance_json FROM rust_recovery_branches_v1 WHERE branch_run_id = ?`)
    .get(runId) as { provenance_json: string | null } | undefined;
  if (!row) return undefined;
  if (row.provenance_json === null) throw new Error('recovery provenance exceeds bounded metadata limits');
  const result = parseRecoveryBranchResult(JSON.parse(row.provenance_json));
  const historyThrough = (BigInt(`0x${result.recoveredDescriptor.generation}`) - 1n).toString(16).padStart(16, '0');
  const matching = db.prepare(`SELECT 1 FROM rust_recovery_branches_v1 WHERE branch_run_id = ?
    AND operation_id = ? AND source_run_id = ? AND recovered_checkpoint_id = ? AND history_through_generation_hex = ?`)
    .get(runId, result.operationId, result.sourceRunId, result.recoveredDescriptor.logicalRootSha256, historyThrough);
  if (result.branchRunId !== runId || !matching) throw new Error('recovery branch identity mismatch');
  return result;
}

/** Read one durable owner-selected import branch without treating it as crash recovery. */
export function readImportBranch(db: ReturnType<typeof Database>, runId: string): ManagedImportBranchResult | undefined {
  const row = db.prepare(`SELECT CASE WHEN length(CAST(provenance_json AS BLOB)) <= 32768
    THEN provenance_json END AS provenance_json FROM rust_import_branches_v1 WHERE branch_run_id = ?`)
    .get(runId) as { provenance_json: string | null } | undefined;
  if (!row) return undefined;
  if (row.provenance_json === null) throw new Error('import branch provenance exceeds bounded metadata limits');
  const result = parseManagedImportBranchResult(JSON.parse(row.provenance_json));
  const historyThrough = (BigInt(`0x${result.sourceGeneration}`) - 1n).toString(16).padStart(16, '0');
  const matching = db.prepare(`SELECT 1 FROM rust_import_branches_v1 WHERE branch_run_id = ?
    AND operation_id = ? AND source_run_id = ? AND recovered_checkpoint_id = ? AND history_through_generation_hex = ?`)
    .get(runId, result.operationId, result.sourceRunId, result.sourceCheckpointId, historyThrough);
  if (result.branchRunId !== runId || !matching) throw new Error('import branch identity mismatch');
  return result;
}

/** Resolve either durable branch kind while rejecting an impossible double identity. */
export function readLineageBranch(db: ReturnType<typeof Database>, runId: string): RecoveryBranchResult | ManagedImportBranchResult | undefined {
  const recovery = readRecoveryBranch(db, runId);
  const imported = readImportBranch(db, runId);
  if (recovery && imported) throw new Error('run identity belongs to multiple branch kinds');
  return recovery ?? imported;
}

/** Resolve inherited history with decreasing cutoffs and reject corrupt cycles. */
export function recoveryLineage(db: ReturnType<typeof Database>, sourceRunId: string): RecoveryLineage[] {
  const lineage: RecoveryLineage[] = [];
  const seen = new Set<string>();
  let runId = sourceRunId;
  let maximumGeneration: U64Hex | null = null;
  for (;;) {
    if (seen.has(runId) || lineage.length === 64) throw new Error('recovery ancestry is cyclic or exceeds 64 retained branches');
    seen.add(runId);
    lineage.push({ runId, maximumGeneration });
    const branch = readLineageBranch(db, runId);
    if (!branch) return lineage;
    const boundary = branch.recoveredDescriptor.generation;
    const previous = lineage[lineage.length - 1]!.maximumGeneration;
    maximumGeneration = previous === null || boundary < previous ? boundary : previous;
    runId = branch.sourceRunId;
  }
}

/** Build a parameterized bounded prefix filter; run names never become SQL text. */
export function recoveryLineageFilter(lineage: RecoveryLineage[]): { sql: string; parameters: Array<string | null> } {
  return { sql: lineage.map(() => '(run_id = ? AND (? IS NULL OR generation_hex <= ?))').join(' OR '),
    parameters: lineage.flatMap(item => [item.runId, item.maximumGeneration, item.maximumGeneration]) };
}

/** Build the inherited completed-generation filter for one exact checkpoint boundary. */
export function exportLineageFilter(db: ReturnType<typeof Database>, 
  runId: string,
  checkpointGeneration: U64Hex,
  tableAlias: string
): { sql: string; parameters: string[]; completedGenerationCount: bigint } {
  const completedGenerationCount = u64HexToBigInt(checkpointGeneration) - 1n;
  const lineage = recoveryLineage(db, runId).map(item => {
    const boundary = item.maximumGeneration === null
      ? completedGenerationCount
      : u64HexToBigInt(item.maximumGeneration) - 1n;
    return { runId: item.runId, maximumCompletedGeneration: boundary };
  });
  return {
    sql: lineage.map(() => `(${tableAlias}.run_id = ? AND ${tableAlias}.generation_hex <= ?)`)
      .join(' OR '),
    parameters: lineage.flatMap(item => [
      item.runId,
      item.maximumCompletedGeneration.toString(16).padStart(16, '0')
    ]),
    completedGenerationCount
  };
}

/**
 * Project retained genomes onto this boundary's completed history. A later, better
 * assessment can supersede an earlier row for the same genome without removing
 * its packed object. Recovery must use the best assessment inside its prefix,
 * rather than losing that genome because its currently selected row is later.
 * Only representatives outside the prefix need projection; selected entries
 * inside it, including pins, keep their existing retention semantics. This does
 * not resurrect pruned objects or rewrite source records.
 */
export function boundaryHallOfFameSelection(db: ReturnType<typeof Database>, runId: string, checkpointGeneration: U64Hex): {
  sql: string; parameters: string[];
} {
  const filter = exportLineageFilter(db, runId, checkpointGeneration, 'candidate');
  const retainedFilter = exportLineageFilter(db, runId, checkpointGeneration, 'retained');
  return {
    sql: `WITH eligible_hall AS (
      SELECT candidate.*
      FROM rust_hall_of_fame_v1 AS candidate
      WHERE candidate.weight_state = 'selected' AND (${filter.sql})
      UNION ALL
      SELECT DISTINCT candidate.*
      FROM rust_hall_of_fame_v1 AS retained
      JOIN rust_hall_of_fame_v1 AS candidate
        ON candidate.run_id = retained.run_id AND candidate.genome_sha256 = retained.genome_sha256
      WHERE retained.weight_state = 'selected' AND NOT (${retainedFilter.sql})
        AND candidate.weight_state = 'unselected' AND (${filter.sql})
    ), ranked_hall AS (
      SELECT eligible_hall.*, row_number() OVER (
        PARTITION BY run_id, genome_sha256 ORDER BY fitness_value DESC, generation_hex ASC
      ) AS genome_rank FROM eligible_hall
    ), boundary_hall AS (
      SELECT run_id, generation_hex, record_version, record_blob, fitness_value, pinned,
        CASE WHEN weight_state = 'unselected' THEN genome_sha256 ELSE weights_sha256 END AS weights_sha256
      FROM ranked_hall WHERE genome_rank = 1 OR pinned = 1
    )`,
    parameters: [...filter.parameters, ...retainedFilter.parameters, ...filter.parameters]
  };
}

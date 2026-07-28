/** Stage 2 graph-ordering and persisted-layout inventory. */

import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import Database from 'better-sqlite3';
import { compileGraph, graphKey } from '../../src/brains/graph/compiler.ts';
import type { GraphSpec } from '../../src/brains/graph/schema.ts';

/** Options for the graph inventory. */
interface GraphOptions {
  /** Existing SQLite database. */
  databasePath: string;
  /** Optional output artifact. */
  outputPath: string | null;
}

/** Candidate graph with provenance. */
interface GraphCandidate {
  /** Source table and row. */
  source: string;
  /** Graph definition. */
  spec: GraphSpec;
  /** Stored architecture key when available. */
  storedKey: string | null;
}

/**
 * Parse graph inventory options.
 * @param argv - Arguments after script path.
 * @returns Validated options.
 */
function parseOptions(argv: readonly string[]): GraphOptions {
  let databasePath = path.resolve('data', 'slither.db');
  let outputPath: string | null = null;
  for (let index = 0; index < argv.length; index++) {
    const option = argv[index];
    const value = argv[index + 1];
    if (!value) throw new Error(`${option ?? '<missing>'} requires a value`);
    if (option === '--db') databasePath = path.resolve(value);
    else if (option === '--output') outputPath = path.resolve(value);
    else throw new Error(`Unknown option ${option}`);
    index++;
  }
  return { databasePath, outputPath };
}

/**
 * Compare strings by JavaScript UTF-16 code units.
 * @param left - Left value.
 * @param right - Right value.
 * @returns Ordering.
 */
function compareCodeUnits(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

/**
 * Read source identity without repository mutation.
 * @returns Commit and dirty flag.
 */
function sourceIdentity(): { commit: string; dirty: boolean } {
  const commit = spawnSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' });
  const status = spawnSync('git', ['status', '--porcelain'], { encoding: 'utf8' });
  return {
    commit: commit.status === 0 ? commit.stdout.trim() : 'unavailable',
    dirty: status.status !== 0 || status.stdout.trim().length > 0
  };
}

/**
 * Parse current snapshot and preset graphs from one database.
 * @param databasePath - Existing database file.
 * @returns Graphs with row provenance.
 */
function loadGraphs(databasePath: string): GraphCandidate[] {
  const db = new Database(databasePath, { readonly: true, fileMustExist: true });
  try {
    const candidates: GraphCandidate[] = [];
    const snapshotTable = db.prepare(
      `SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'population_snapshots'`
    ).get();
    if (snapshotTable) {
      const rows = db.prepare(
        `SELECT id, payload_json FROM population_snapshots ORDER BY id`
      ).all() as Array<{ id: number; payload_json: string | null }>;
      for (const row of rows) {
        if (!row.payload_json) continue;
        const payload = JSON.parse(row.payload_json) as {
          graphSpec?: GraphSpec;
          archKey?: string;
        };
        if (payload.graphSpec) {
          candidates.push({
            source: `population_snapshots.id=${row.id}`,
            spec: payload.graphSpec,
            storedKey: payload.archKey ?? null
          });
        }
      }
    }
    const presetTable = db.prepare(
      `SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'graph_presets'`
    ).get();
    if (presetTable) {
      const rows = db.prepare(
        `SELECT id, name, spec_json FROM graph_presets ORDER BY id`
      ).all() as Array<{ id: number; name: string; spec_json: string }>;
      for (const row of rows) {
        candidates.push({
          source: `graph_presets.id=${row.id},name=${row.name}`,
          spec: JSON.parse(row.spec_json) as GraphSpec,
          storedKey: null
        });
      }
    }
    return candidates;
  } finally {
    db.close();
  }
}

/**
 * Capture graph layout and locale-sensitive ordering evidence.
 * @param options - Database and output paths.
 * @returns Evidence object.
 */
function captureGraphBaseline(options: GraphOptions): Record<string, unknown> {
  const databaseBytes = fs.readFileSync(options.databasePath);
  const candidates = loadGraphs(options.databasePath);
  const unique = new Map<string, GraphCandidate & { sources: string[] }>();
  for (const candidate of candidates) {
    const digest = createHash('sha256').update(JSON.stringify(candidate.spec)).digest('hex');
    const prior = unique.get(digest);
    if (prior) prior.sources.push(candidate.source);
    else unique.set(digest, { ...candidate, sources: [candidate.source] });
  }
  const probe = ['a', 'A', 'ä', 'z', 'Z', '10', '2', 'é', 'e\u0301', '_', '-'];
  return {
    schema: 'slither-stage2-graph-baseline',
    version: 1,
    evidenceClass: 'new reproducible fixture',
    source: sourceIdentity(),
    environment: {
      capturedAt: new Date().toISOString(),
      platform: process.platform,
      architecture: process.arch,
      osType: os.type(),
      osRelease: os.release(),
      locale: Intl.DateTimeFormat().resolvedOptions().locale,
      node: process.version,
      icu: process.versions.icu
    },
    database: {
      path: options.databasePath,
      sha256: createHash('sha256').update(databaseBytes).digest('hex'),
      graphBearingRows: candidates.length,
      uniqueGraphSpecs: unique.size
    },
    orderingProbe: {
      input: probe,
      localeCompare: [...probe].sort((left, right) => left.localeCompare(right)),
      codeUnit: [...probe].sort(compareCodeUnits),
      note: 'The current compiler uses localeCompare; code-unit order is the proposed portable replacement.'
    },
    graphs: Array.from(unique.entries()).map(([specSha256, candidate]) => {
      const compiled = compileGraph(candidate.spec);
      return {
        sources: candidate.sources,
        specSha256,
        spec: candidate.spec,
        storedKey: candidate.storedKey,
        computedKey: graphKey(candidate.spec),
        storedKeyMatches: candidate.storedKey === null || candidate.storedKey === graphKey(candidate.spec),
        order: compiled.order,
        totalParams: compiled.totalParams,
        totalStateSize: compiled.totalStateSize,
        outputs: compiled.outputs,
        nodes: compiled.nodes,
        concatInputs: compiled.nodes
          .filter(node => node.type === 'Concat')
          .map(node => ({ nodeId: node.id, inputs: node.inputs }))
      };
    })
  };
}

/** Execute the CLI. */
function main(): void {
  const options = parseOptions(process.argv.slice(2));
  const result = captureGraphBaseline(options);
  const json = `${JSON.stringify(result, null, 2)}\n`;
  if (options.outputPath) {
    fs.mkdirSync(path.dirname(options.outputPath), { recursive: true });
    fs.writeFileSync(options.outputPath, json, 'utf8');
    console.info(`[stage2.graph] wrote ${options.outputPath}`);
  } else {
    process.stdout.write(json);
  }
}

try {
  main();
} catch (error) {
  console.error(error);
  process.exitCode = 1;
}

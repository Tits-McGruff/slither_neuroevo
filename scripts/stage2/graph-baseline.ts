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
  databasePath: string | null;
  /** Retained JSON graph fixture, mutually exclusive with `databasePath`. */
  fixturePath: string | null;
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

/** Immutable, text-based source for a reproducible graph inventory. */
interface GraphFixture {
  /** Fixture format identifier. */
  schema: 'slither-stage2-graph-fixture';
  /** Fixture format version. */
  version: 1;
  /** Honest provenance for either extracted database rows or a synthetic case. */
  source:
    | {
      /** Fixture content was extracted from a retained database. */
      kind: 'database';
      /** SHA-256 of the original database bytes. */
      sha256: string;
    }
    | {
      /** Fixture was constructed to exercise a named compatibility risk. */
      kind: 'synthetic';
      /** Stable human-readable fixture identity. */
      fixtureId: string;
      /** Short statement of the behavior exercised by the fixture. */
      purpose: string;
    };
  /** Graphs and their original row provenance. */
  graphs: Array<{
    /** Original rows containing this exact graph spec. */
    sources: string[];
    /** Graph definition in its retained JSON form. */
    spec: GraphSpec;
    /** Architecture key stored with the graph, when present. */
    storedKey?: string | null;
  }>;
}

/** Fully loaded graph input with enough metadata for the output artifact. */
interface GraphInput {
  /** Parsed graph candidates. */
  candidates: GraphCandidate[];
  /** Tagged source identity retained in the output schema. */
  source:
    | {
      /** Live read-only SQLite input. */
      kind: 'database';
      /** Resolved database path. */
      path: string;
      /** SHA-256 of the exact database bytes. */
      sha256: string;
    }
    | {
      /** Immutable JSON fixture input. */
      kind: 'fixture';
      /** Resolved fixture path. */
      path: string;
      /** SHA-256 of the exact fixture bytes. */
      sha256: string;
      /** Exact fixture byte count. */
      byteLength: number;
      /** Retained fixture format identifier. */
      schema: GraphFixture['schema'];
      /** Retained fixture format version. */
      version: GraphFixture['version'];
      /** Provenance declared inside the hashed fixture. */
      provenance: GraphFixture['source'];
    };
}

/** Lowercase or uppercase hexadecimal SHA-256 text. */
const SHA256_PATTERN = /^[0-9a-f]{64}$/iu;

/**
 * Parse graph inventory options.
 * @param argv - Arguments after script path.
 * @returns Validated options.
 */
function parseOptions(argv: readonly string[]): GraphOptions {
  let databasePath: string | null = null;
  let fixturePath: string | null = null;
  let outputPath: string | null = null;
  let databaseSupplied = false;
  let fixtureSupplied = false;
  let outputSupplied = false;
  for (let index = 0; index < argv.length; index++) {
    const option = argv[index];
    const value = argv[index + 1];
    if (!value) throw new Error(`${option ?? '<missing>'} requires a value`);
    if (option === '--db') {
      if (databaseSupplied) throw new Error('--db may be supplied only once');
      if (fixtureSupplied) throw new Error('--db and --fixture cannot be used together');
      databaseSupplied = true;
      databasePath = path.resolve(value);
    } else if (option === '--fixture') {
      if (fixtureSupplied) throw new Error('--fixture may be supplied only once');
      if (databaseSupplied) throw new Error('--db and --fixture cannot be used together');
      fixtureSupplied = true;
      fixturePath = path.resolve(value);
    } else if (option === '--output') {
      if (outputSupplied) throw new Error('--output may be supplied only once');
      outputSupplied = true;
      outputPath = path.resolve(value);
    }
    else throw new Error(`Unknown option ${option}`);
    index++;
  }
  if (!databaseSupplied && !fixtureSupplied) databasePath = path.resolve('data', 'slither.db');
  return { databasePath, fixturePath, outputPath };
}

/**
 * Determine whether a parsed JSON value is a non-null object.
 * @param value - Parsed JSON value.
 * @returns True for record-like objects.
 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
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
  const env = { ...process.env, GIT_OPTIONAL_LOCKS: '0' };
  const commit = spawnSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8', env });
  const status = spawnSync('git', ['status', '--porcelain'], { encoding: 'utf8', env });
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
 * Load a retained text fixture without depending on a local SQLite database.
 * @param fixturePath - UTF-8 JSON fixture path.
 * @returns Candidates and hashes needed to identify the exact fixture bytes.
 */
function loadFixture(fixturePath: string): GraphInput {
  const bytes = fs.readFileSync(fixturePath);
  const parsedValue: unknown = JSON.parse(bytes.toString('utf8'));
  if (!isRecord(parsedValue)) throw new Error(`Graph fixture root must be an object: ${fixturePath}`);
  const parsed = parsedValue as Partial<GraphFixture>;
  if (parsed.schema !== 'slither-stage2-graph-fixture' || parsed.version !== 1) {
    throw new Error(`Unsupported graph fixture: ${fixturePath}`);
  }
  if (!isRecord(parsed.source)) {
    throw new Error(`Graph fixture is missing tagged source provenance: ${fixturePath}`);
  }
  if (parsed.source.kind === 'database') {
    if (typeof parsed.source.sha256 !== 'string' || !SHA256_PATTERN.test(parsed.source.sha256)) {
      throw new Error(`Graph fixture database source requires a 64-hex SHA-256: ${fixturePath}`);
    }
  } else if (parsed.source.kind === 'synthetic') {
    if (
      typeof parsed.source.fixtureId !== 'string' ||
      parsed.source.fixtureId.trim().length === 0 ||
      typeof parsed.source.purpose !== 'string' ||
      parsed.source.purpose.trim().length === 0
    ) {
      throw new Error(`Graph fixture synthetic source requires fixtureId and purpose: ${fixturePath}`);
    }
  } else {
    throw new Error(`Graph fixture has unknown source provenance: ${fixturePath}`);
  }
  if (!Array.isArray(parsed.graphs) || parsed.graphs.length === 0) {
    throw new Error(`Graph fixture must contain at least one graph: ${fixturePath}`);
  }
  const candidates: GraphCandidate[] = [];
  for (const [graphIndex, graph] of parsed.graphs.entries()) {
    if (
      !isRecord(graph) ||
      !Array.isArray(graph.sources) ||
      graph.sources.length === 0 ||
      !graph.sources.every(source => typeof source === 'string' && source.trim().length > 0)
    ) {
      throw new Error(`Graph fixture has invalid sources: ${fixturePath}`);
    }
    if (
      !isRecord(graph.spec) ||
      graph.spec.type !== 'graph' ||
      !Array.isArray(graph.spec.nodes) ||
      !Array.isArray(graph.spec.edges) ||
      !Array.isArray(graph.spec.outputs) ||
      typeof graph.spec.outputSize !== 'number'
    ) {
      throw new Error(`Graph fixture has invalid graph spec: ${fixturePath}`);
    }
    if (graph.storedKey != null && typeof graph.storedKey !== 'string') {
      throw new Error(`Graph fixture has invalid storedKey: ${fixturePath}`);
    }
    try {
      compileGraph(graph.spec as GraphSpec);
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error);
      throw new Error(`Graph fixture graph ${graphIndex} does not compile: ${reason}`);
    }
    for (const source of graph.sources) {
      candidates.push({ source, spec: graph.spec as GraphSpec, storedKey: graph.storedKey ?? null });
    }
  }
  return {
    candidates,
    source: {
      kind: 'fixture',
      path: fixturePath,
      sha256: createHash('sha256').update(bytes).digest('hex'),
      byteLength: bytes.length,
      schema: parsed.schema,
      version: parsed.version,
      provenance: parsed.source
    }
  };
}

/**
 * Load either the historical SQLite source or a retained graph fixture.
 * @param options - Parsed command-line options.
 * @returns Candidates and source identity.
 */
function loadGraphInput(options: GraphOptions): GraphInput {
  if (options.fixturePath) return loadFixture(options.fixturePath);
  if (!options.databasePath) throw new Error('Either --db or --fixture is required');
  const databaseBytes = fs.readFileSync(options.databasePath);
  return {
    candidates: loadGraphs(options.databasePath),
    source: {
      kind: 'database',
      path: options.databasePath,
      sha256: createHash('sha256').update(databaseBytes).digest('hex')
    }
  };
}

/**
 * Capture graph layout and locale-sensitive ordering evidence.
 * @param options - Database and output paths.
 * @returns Evidence object.
 */
function captureGraphBaseline(options: GraphOptions): Record<string, unknown> {
  const input = loadGraphInput(options);
  const { candidates } = input;
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
    version: 2,
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
    input: {
      ...input.source,
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

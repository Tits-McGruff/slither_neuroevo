/** Contract tests for the retained Stage 2 graph fixtures. */

import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { describe, expect, it } from 'vitest';

/** Directory that contains this script and its retained fixture inputs. */
const stage2Directory = path.dirname(fileURLToPath(import.meta.url));
/** Node executable used to invoke the TypeScript CLI through the local tsx loader. */
const nodeExecutable = process.execPath;
/** Local TypeScript runner, avoiding a shell-dependent npm wrapper. */
const tsxExecutable = path.resolve(stage2Directory, '..', '..', 'node_modules', 'tsx', 'dist', 'cli.mjs');
/** Graph inventory CLI under test. */
const graphBaseline = path.join(stage2Directory, 'graph-baseline.ts');

/** One compiled graph node used by the focused assertions. */
interface GraphNodeArtifact {
  /** Starting float offset in the graph parameter vector. */
  paramOffset: number;
  /** Number of float parameters owned by the node. */
  paramLength: number;
}

/** One graph entry returned by the inventory CLI. */
interface GraphArtifact {
  /** Original rows that supplied this graph. */
  sources: string[];
  /** Hash of the retained graph specification. */
  specSha256: string;
  /** Whether a stored architecture key agreed with the compiled key. */
  storedKeyMatches: boolean;
  /** Current locale-dependent architecture key. */
  computedKey: string;
  /** Topological compiler order. */
  order: string[];
  /** Total float parameter count. */
  totalParams: number;
  /** Total recurrent-state float count. */
  totalStateSize: number;
  /** Resolved per-node parameter ranges. */
  nodes: GraphNodeArtifact[];
  /** Explicit Concat input order. */
  concatInputs: Array<{
    /** Concat node identifier. */
    nodeId: string;
    /** Resolved source node and output port. */
    inputs: Array<{ fromId: string; fromPort: number }>;
  }>;
}

/** Parsed Stage 2 graph-baseline CLI output used by this test. */
interface GraphBaselineArtifact {
  /** Tagged database or retained-fixture identity and graph row totals. */
  input: {
    /** Source kind. */
    kind: 'database' | 'fixture';
    /** Resolved input path. */
    path: string;
    /** Exact input byte SHA-256. */
    sha256: string;
    /** Exact fixture byte count. */
    byteLength?: number;
    /** Retained fixture format identifier. */
    schema?: string;
    /** Retained fixture format version. */
    version?: number;
    /** Provenance declared by the hashed fixture. */
    provenance?:
      | { kind: 'database'; sha256: string }
      | { kind: 'synthetic'; fixtureId: string; purpose: string };
    /** Graph-bearing source row count. */
    graphBearingRows: number;
    /** De-duplicated graph specification count. */
    uniqueGraphSpecs: number;
  };
  /** Compiled graph evidence. */
  graphs: GraphArtifact[];
}

/**
 * Run the graph inventory against one retained fixture.
 * @param fixtureName - File name inside `graph-fixtures`.
 * @returns Parsed CLI artifact and exact fixture bytes.
 */
function runFixture(fixtureName: string): { artifact: GraphBaselineArtifact; bytes: Buffer; path: string } {
  const fixturePath = path.join(stage2Directory, 'graph-fixtures', fixtureName);
  const result = spawnSync(nodeExecutable, [tsxExecutable, graphBaseline, '--fixture', fixturePath], {
    cwd: path.resolve(stage2Directory, '..', '..'),
    encoding: 'utf8'
  });
  expect(result.status, result.stderr).toBe(0);
  return {
    artifact: JSON.parse(result.stdout) as GraphBaselineArtifact,
    bytes: fs.readFileSync(fixturePath),
    path: fixturePath
  };
}

/**
 * Run the graph inventory with arbitrary arguments.
 * @param args - CLI arguments following the script name.
 * @returns Synchronous child-process result.
 */
function runArguments(args: string[]) {
  return spawnSync(nodeExecutable, [tsxExecutable, graphBaseline, ...args], {
    cwd: path.resolve(stage2Directory, '..', '..'),
    encoding: 'utf8'
  });
}

describe('Stage 2 graph baseline fixtures', () => {
  it('replays the graph rows formerly available only in the owner database', () => {
    const result = runFixture('current-snapshot-graphs.v1.json');
    expect(result.artifact.input).toEqual({
      kind: 'fixture',
      path: result.path,
      sha256: '3588820b9787204bec0f80df9d5b9a1faa368ac1d26e89b86c35604a3999894e',
      byteLength: result.bytes.length,
      schema: 'slither-stage2-graph-fixture',
      version: 1,
      provenance: {
        kind: 'database',
        sha256: '9b8774387cff7aa82e64dbf75f4807d06ada9072814d24b71b8c76c4fe4bd8a4'
      },
      graphBearingRows: 2,
      uniqueGraphSpecs: 1
    });
    expect(result.artifact.input.sha256).toBe(createHash('sha256').update(result.bytes).digest('hex'));
    expect(result.artifact.graphs).toHaveLength(1);
    expect(result.artifact.graphs[0]).toMatchObject({
      sources: ['population_snapshots.id=1', 'population_snapshots.id=2'],
      specSha256: 'cd9e0457e7c0628043e644e43f7a69514a5bb5236cc7633a300cae78e18a8e74',
      storedKeyMatches: true,
      order: ['input', 'mlp', 'gru', 'head'],
      totalParams: 13_458,
      totalStateSize: 16
    });
    expect(result.artifact.graphs[0].nodes.map(node => node.paramOffset))
      .toEqual([0, 0, 9_536, 13_424]);
  });

  it('captures locale-sensitive topology, key, offsets, and implicit Concat order', () => {
    const result = runFixture('legacy-locale-concat-order.v1.json');
    expect(result.artifact.input).toMatchObject({
      kind: 'fixture',
      sha256: '22b85250d08de5d15d6d46989e6fb64f7a98b1c3470ff40707d4bcfa7395b922',
      provenance: {
        kind: 'synthetic',
        fixtureId: 'legacy-locale-concat-order.v1'
      },
      graphBearingRows: 1,
      uniqueGraphSpecs: 1
    });
    expect(result.artifact.graphs[0]).toMatchObject({
      order: ['source', '_', '-', '10', '2', 'a', 'A', 'ä', 'é', 'é', 'z', 'Z', 'merge', 'head'],
      totalParams: 46,
      concatInputs: [{
        nodeId: 'merge',
        inputs: ['_', '-', '10', '2', 'a', 'A', 'ä', 'é', 'é', 'z', 'Z']
          .map(fromId => ({ fromId, fromPort: 0 }))
      }]
    });
    expect(result.artifact.graphs[0].nodes.map(node => [
      node.paramOffset,
      node.paramLength
    ])).toEqual([
      [0, 0], [0, 2], [2, 2], [4, 2], [6, 2], [8, 2], [10, 2],
      [12, 2], [14, 2], [16, 2], [18, 2], [20, 2], [22, 0], [22, 24]
    ]);
    expect(createHash('sha256').update(result.artifact.graphs[0].computedKey).digest('hex'))
      .toBe('1cfe387bbd9eb9d86db39d0d2cf2ea671ce7d6fbe9654111059b02aa104d0774');
  });

  it('rejects repeated or conflicting source options', () => {
    const fixturePath = path.join(
      stage2Directory,
      'graph-fixtures',
      'current-snapshot-graphs.v1.json'
    );
    const defaultDatabase = path.join('data', 'slither.db');
    const conflict = runArguments(['--db', defaultDatabase, '--fixture', fixturePath]);
    expect(conflict.status).not.toBe(0);
    expect(conflict.stderr).toContain('--db and --fixture cannot be used together');
    const repeated = runArguments(['--fixture', fixturePath, '--fixture', fixturePath]);
    expect(repeated.status).not.toBe(0);
    expect(repeated.stderr).toContain('--fixture may be supplied only once');
  });

  it('rejects dishonest, empty, and non-compiling fixtures', () => {
    const temporaryDirectory = fs.mkdtempSync(path.join(os.tmpdir(), 'slither-graph-fixture-test-'));
    const cases: Array<{ name: string; fixture: unknown; message: string }> = [
      {
        name: 'bad-database-sha.json',
        fixture: {
          schema: 'slither-stage2-graph-fixture',
          version: 1,
          source: { kind: 'database', sha256: 'not-a-sha' },
          graphs: []
        },
        message: 'requires a 64-hex SHA-256'
      },
      {
        name: 'empty-graphs.json',
        fixture: {
          schema: 'slither-stage2-graph-fixture',
          version: 1,
          source: { kind: 'synthetic', fixtureId: 'empty', purpose: 'negative test' },
          graphs: []
        },
        message: 'must contain at least one graph'
      },
      {
        name: 'empty-sources.json',
        fixture: {
          schema: 'slither-stage2-graph-fixture',
          version: 1,
          source: { kind: 'synthetic', fixtureId: 'empty-source', purpose: 'negative test' },
          graphs: [{ sources: [], spec: { type: 'graph', nodes: [], edges: [], outputs: [], outputSize: 0 } }]
        },
        message: 'invalid sources'
      },
      {
        name: 'non-compiling.json',
        fixture: {
          schema: 'slither-stage2-graph-fixture',
          version: 1,
          source: { kind: 'synthetic', fixtureId: 'bad-graph', purpose: 'negative test' },
          graphs: [{
            sources: ['synthetic:bad-graph'],
            spec: { type: 'graph', nodes: [], edges: [], outputs: [], outputSize: 0 }
          }]
        },
        message: 'does not compile: Graph: no nodes defined'
      }
    ];
    try {
      for (const testCase of cases) {
        const fixturePath = path.join(temporaryDirectory, testCase.name);
        fs.writeFileSync(fixturePath, JSON.stringify(testCase.fixture), 'utf8');
        const result = runArguments(['--fixture', fixturePath]);
        expect(result.status, testCase.name).not.toBe(0);
        expect(result.stderr, testCase.name).toContain(testCase.message);
      }
    } finally {
      fs.rmSync(temporaryDirectory, { recursive: true, force: true });
    }
  });
});

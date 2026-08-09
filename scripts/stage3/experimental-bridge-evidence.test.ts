/** Focused short-duration regression coverage for the Stage 3 evidence runner. */

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  parseExperimentalBridgeEvidenceOptions,
  runExperimentalBridgeEvidence,
  writeEvidenceAtomically,
  type ExperimentalBridgeEvidenceOptions
} from './experimental-bridge-evidence.ts';

/** Small real-addon run suitable for CI; the CLI default remains the required ten-minute idle measurement. */
const SHORT_OPTIONS: ExperimentalBridgeEvidenceOptions = {
  idleMs: 40,
  sustainedMs: 120,
  batchesPerSecond: 20,
  batchSize: 2,
  payloadBytes: 32,
  sampleIntervalMs: 20,
  outputPath: null,
  sourceCommit: null,
  sourceDirty: null
};

describe('Stage 3 experimental bridge evidence runner', () => {
  it('keeps its ten-minute idle default while allowing explicitly bounded short test runs', async () => {
    const defaults = parseExperimentalBridgeEvidenceOptions([]);
    expect(defaults.idleMs).toBe(600_000);
    expect(defaults.sustainedMs).toBe(60_000);
    expect(defaults.sourceCommit).toBeNull();
    expect(defaults.sourceDirty).toBeNull();
    const archive = parseExperimentalBridgeEvidenceOptions([
      '--source-commit', 'a'.repeat(40), '--source-dirty', 'false'
    ]);
    expect(archive.sourceCommit).toBe('a'.repeat(40));
    expect(archive.sourceDirty).toBe(false);
    expect(() => parseExperimentalBridgeEvidenceOptions([
      '--sustained-ms', '900000', '--batches-per-second', '240', '--batch-size', '64'
    ])).toThrow(/65536/i);
    expect(() => parseExperimentalBridgeEvidenceOptions(['--idle-ms', '10milliseconds'])).toThrow(
      /positive integer/
    );
    await expect(runExperimentalBridgeEvidence({ ...SHORT_OPTIONS, idleMs: 0 })).rejects.toThrow(
      /idleMs must be a positive integer/
    );
    await expect(runExperimentalBridgeEvidence({
      ...SHORT_OPTIONS,
      sourceCommit: 'a'.repeat(40),
      sourceDirty: null
    })).rejects.toThrow(/must either both be supplied/);
  });

  it('uses the real addon, proves an idle wake plateau, and records bounded coarse probe evidence', async () => {
    const result = await runExperimentalBridgeEvidence(SHORT_OPTIONS);
    expect(result).toMatchObject({
      schema: 'slither-stage3-experimental-bridge-evidence',
      version: 1,
      native: { buildProfile: 'release', buildClass: 'production' },
      result: {
        assertions: {
          realAddonSourceMatchesCurrentTree: true,
          idleHadNoWakeOrQueueGrowth: true,
          allSubmittedCommandsCompleted: true,
          noNativeWakeFailures: true,
          noNativeTerminalFault: true
        }
      }
    });
    const typed = result as {
      source: { commit: string; dirty: boolean; method: string };
      result: {
        submittedCommands: number;
        completedCommands: number;
        commandLatencyMs: { count: number };
        idle: {
          wallMs: number;
          cpu: { totalMicros: number; averageCpuCoreEquivalents: number };
          healthBefore: { wakeAttempts: string };
          healthAfter: { wakeAttempts: string };
        };
        sustained: {
          submittedBatches: number;
          submissionWallMs: number;
          completionWallMs: number;
          achievedSubmittedCommandsPerSecond: number;
          cpu: { totalMicros: number };
        };
      };
    };
    expect(typed.source.commit).not.toBe('');
    expect(typeof typed.source.dirty).toBe('boolean');
    expect(typed.source.method).toBe('git');
    expect(typed.result.submittedCommands).toBeGreaterThan(0);
    expect(typed.result.completedCommands).toBe(typed.result.submittedCommands);
    expect(typed.result.commandLatencyMs.count).toBe(typed.result.submittedCommands);
    expect(typed.result.idle.wallMs).toBeGreaterThanOrEqual(SHORT_OPTIONS.idleMs);
    expect(typed.result.idle.cpu.totalMicros).toBeGreaterThanOrEqual(0);
    expect(typed.result.idle.healthAfter.wakeAttempts).toBe(typed.result.idle.healthBefore.wakeAttempts);
    expect(typed.result.sustained.submittedBatches).toBeGreaterThan(0);
    expect(typed.result.sustained.completionWallMs).toBeGreaterThanOrEqual(typed.result.sustained.submissionWallMs);
    expect(typed.result.sustained.achievedSubmittedCommandsPerSecond).toBeGreaterThan(0);
    expect(typed.result.sustained.cpu.totalMicros).toBeGreaterThanOrEqual(0);
  });

  it('parses an output path without creating it during option validation', () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'slither-stage3-bridge-options-'));
    try {
      const output = path.join(root, 'evidence.json');
      const parsed = parseExperimentalBridgeEvidenceOptions(['--output', output]);
      expect(parsed.outputPath).toBe(output);
      expect(fs.existsSync(output)).toBe(false);
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });

  it('publishes a complete evidence file without replacing an existing result', () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'slither-stage3-bridge-output-'));
    try {
      const output = path.join(root, 'evidence.json');
      writeEvidenceAtomically(output, { complete: true, count: 1 });
      expect(JSON.parse(fs.readFileSync(output, 'utf8'))).toEqual({ complete: true, count: 1 });
      expect(() => writeEvidenceAtomically(output, { complete: false })).toThrow(/Refusing to overwrite/);
      expect(JSON.parse(fs.readFileSync(output, 'utf8'))).toEqual({ complete: true, count: 1 });
      expect(fs.readdirSync(root)).toEqual(['evidence.json']);
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });
});

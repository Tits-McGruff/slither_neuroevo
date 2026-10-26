/** Focused short-duration regression coverage for the Stage 3 evidence runner. */

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  evaluateOwnerTargetVmFacts,
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
  sourceDirty: null,
  environmentClass: 'development-machine'
};

describe('Stage 3 experimental bridge evidence runner', () => {
  it('accepts only the recorded Oxygen VM facts as owner-target evidence', () => {
    const oxygenFacts = {
      platform: 'linux',
      architecture: 'x64',
      hostname: 'oxygen.home.arpa',
      distributionId: 'debian',
      cpuModel: 'AMD Ryzen 7 2700 Eight-Core Processor',
      logicalCpuCount: 8,
      totalMemoryBytes: 16_775_352_320
    };
    expect(Object.values(evaluateOwnerTargetVmFacts(oxygenFacts)).every(Boolean)).toBe(true);
    expect(evaluateOwnerTargetVmFacts({ ...oxygenFacts, platform: 'win32' }).platformIsLinux).toBe(false);
    expect(evaluateOwnerTargetVmFacts({ ...oxygenFacts, architecture: 'arm64' }).architectureIsX64).toBe(false);
    expect(evaluateOwnerTargetVmFacts({ ...oxygenFacts, distributionId: 'ubuntu' }).distributionIsDebian).toBe(false);
    expect(evaluateOwnerTargetVmFacts({ ...oxygenFacts, hostname: 'not-oxygen' }).hostnameIsOxygen).toBe(false);
    expect(evaluateOwnerTargetVmFacts({
      ...oxygenFacts,
      cpuModel: 'AMD Ryzen 7 2700X Eight-Core Processor'
    }).cpuIsRyzen7_2700).toBe(false);
    expect(evaluateOwnerTargetVmFacts({ ...oxygenFacts, logicalCpuCount: 16 }).logicalCpuCountIsEight).toBe(false);
    expect(evaluateOwnerTargetVmFacts({
      ...oxygenFacts,
      totalMemoryBytes: 15 * 1024 ** 3 - 1
    }).memoryMatches16GiBAllocation).toBe(false);
  });

  it('keeps its ten-minute idle default while allowing explicitly bounded short test runs', async () => {
    expect(() => parseExperimentalBridgeEvidenceOptions([])).toThrow(/environmentClass/);
    const defaults = parseExperimentalBridgeEvidenceOptions(['--environment', 'development-machine']);
    expect(defaults.idleMs).toBe(600_000);
    expect(defaults.sustainedMs).toBe(60_000);
    expect(defaults.sourceCommit).toBeNull();
    expect(defaults.sourceDirty).toBeNull();
    const archive = parseExperimentalBridgeEvidenceOptions([
      '--environment', 'owner-target-vm',
      '--source-commit', 'a'.repeat(40), '--source-dirty', 'false'
    ]);
    expect(archive.sourceCommit).toBe('a'.repeat(40));
    expect(archive.sourceDirty).toBe(false);
    expect(archive.environmentClass).toBe('owner-target-vm');
    expect(() => parseExperimentalBridgeEvidenceOptions([
      '--environment', 'development-machine',
      '--sustained-ms', '900000', '--batches-per-second', '240', '--batch-size', '64'
    ])).toThrow(/65536/i);
    expect(() => parseExperimentalBridgeEvidenceOptions([
      '--environment', 'development-machine', '--idle-ms', '10milliseconds'
    ])).toThrow(/positive integer/);
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
      evidenceClass: 'short-validation coarse-bridge result',
      environmentClass: 'development-machine',
      environment: { ownerTargetVmValidated: expect.any(Boolean) },
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
          processedBatches: string;
          submissionWallMs: number;
          completionWallMs: number;
          achievedSubmittedCommandsPerSecond: number;
          achievedProcessedBatchesPerSecond: number;
          cpu: { totalMicros: number };
        };
        stage3IdleGate: {
          idleDurationSatisfied: boolean;
          stage3IdleEvidenceSatisfied: boolean;
        };
      };
    };
    expect(typed.source.commit).toMatch(/^[0-9a-f]{40}$/);
    expect(typeof typed.source.dirty).toBe('boolean');
    expect(typed.source.method).toBe('git');
    expect(typed.result.submittedCommands).toBeGreaterThan(0);
    expect(typed.result.completedCommands).toBe(typed.result.submittedCommands);
    expect(typed.result.commandLatencyMs.count).toBe(typed.result.submittedCommands);
    expect(typed.result.idle.wallMs).toBeGreaterThanOrEqual(SHORT_OPTIONS.idleMs);
    expect(typed.result.idle.cpu.totalMicros).toBeGreaterThanOrEqual(0);
    expect(typed.result.idle.healthAfter.wakeAttempts).toBe(typed.result.idle.healthBefore.wakeAttempts);
    expect(typed.result.sustained.submittedBatches).toBeGreaterThan(0);
    expect(typed.result.sustained.processedBatches).toBe(typed.result.sustained.submittedBatches.toString(10));
    expect(typed.result.sustained.completionWallMs).toBeGreaterThanOrEqual(typed.result.sustained.submissionWallMs);
    expect(typed.result.sustained.achievedSubmittedCommandsPerSecond).toBeGreaterThan(0);
    expect(typed.result.sustained.cpu.totalMicros).toBeGreaterThanOrEqual(0);
    expect(typed.result.stage3IdleGate.idleDurationSatisfied).toBe(false);
    expect(typed.result.stage3IdleGate.stage3IdleEvidenceSatisfied).toBe(false);
  });

  it('parses an output path without creating it during option validation', () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'slither-stage3-bridge-options-'));
    try {
      const output = path.join(root, 'evidence.json');
      const parsed = parseExperimentalBridgeEvidenceOptions([
        '--environment', 'development-machine', '--output', output
      ]);
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

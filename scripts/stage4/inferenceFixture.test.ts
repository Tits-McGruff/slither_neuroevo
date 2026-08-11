/** Unit contracts for Stage 4 cross-language inference evidence fixtures. */

import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { compileGraph } from '../../src/brains/graph/compiler.ts';
import {
  buildStage4CurrentBrains,
  buildStage4InferenceFixture,
  stage4InferenceScenario,
  stage4OneStepComparisonData,
  stage4ResultDigests
} from './inferenceFixture.ts';
import { parseStage4InferenceBenchmarkOptions } from './inference-benchmark.ts';
import { compareFloat32Buffers, runComparison } from './compare-inference-reports.ts';

/** Test-suite label retained in complete-suite output. */
const SUITE = 'Stage 4 inference evidence fixture';

describe(SUITE, () => {
  it('matches the approved P0-P3 population and graph shapes', () => {
    const expected = {
      P0: { population: 55, input: 83, parameters: 13_458, recurrent: 16 },
      P1: { population: 300, input: 83, parameters: 13_458, recurrent: 16 },
      P2: { population: 55, input: 147, parameters: 402_914, recurrent: 96 },
      P3: { population: 300, input: 147, parameters: 402_914, recurrent: 96 }
    } as const;
    for (const name of ['P0', 'P1', 'P2', 'P3'] as const) {
      const scenario = stage4InferenceScenario(name);
      const compiled = compileGraph(scenario.graphSpec);
      expect(scenario.populationCount).toBe(expected[name].population);
      expect(scenario.inputSize).toBe(expected[name].input);
      expect(compiled.totalParams).toBe(expected[name].parameters);
      expect(compiled.totalStateSize).toBe(expected[name].recurrent);
    }
  });

  it('creates distinct bit-stable P0 weights, observations, and nonzero state', () => {
    const fixture = buildStage4InferenceFixture('P0');
    expect(fixture.weights).toHaveLength(55);
    expect(fixture.observations).toHaveLength(55);
    expect(fixture.weights[0]).not.toEqual(fixture.weights[1]);
    expect(fixture.observations[0]).not.toEqual(fixture.observations[1]);
    expect(fixture.weightsSha256).toBe(
      '5d08fee2550b4c438e96608fac993845967fb5c5edc2e5ac09a40ea23cd18d69'
    );
    expect(fixture.observationsSha256).toBe(
      'c10e2266960c4ba346855f4114d80c27f25970eb46a30cc948de0d6f886aeb5b'
    );
    expect(fixture.initialRecurrentSha256).toBe(
      '686f728d7c1057e652ae0f69b359ddc2d4318885cfcedde08477b775049fecd1'
    );
  });

  it('advances 55 differently weighted JS brains from nonzero synthetic recurrent state', () => {
    const fixture = buildStage4InferenceFixture('P0');
    const brains = buildStage4CurrentBrains(fixture, 'js');
    for (let slot = 0; slot < brains.length; slot++) {
      brains[slot]!.forward(fixture.observations[slot]!);
    }
    const result = stage4ResultDigests(brains);
    const comparison = stage4OneStepComparisonData(brains);
    expect(result.distinctOutputPairs).toBeGreaterThan(1);
    expect(result.recurrentSha256).not.toBe(fixture.initialRecurrentSha256);
    expect(comparison.outputFloats).toBe(110);
    expect(comparison.recurrentFloats).toBe(880);
    expect(comparison.outputsF32LeHex).toHaveLength(110 * 8);
    expect(comparison.recurrentF32LeHex).toHaveLength(880 * 8);
    expect(comparison.outputsSha256).toBe(result.outputsSha256);
    expect(comparison.recurrentSha256).toBe(result.recurrentSha256);
  });

  it('compares raw Float32 results element by element instead of rounded hashes', () => {
    const left = Buffer.alloc(8);
    const right = Buffer.alloc(8);
    left.writeFloatLE(0.123449, 0);
    right.writeFloatLE(0.123451, 0);
    left.writeFloatLE(-0.25, 4);
    right.writeFloatLE(-0.2502, 4);
    const comparison = compareFloat32Buffers(left, right, 1e-4);
    expect(comparison.maxDifferenceIndex).toBe(1);
    expect(comparison.outsideTolerance).toBe(1);
    expect(comparison.firstFailureIndex).toBe(1);
    expect(comparison.maxAbsoluteDifference).toBeGreaterThan(1e-4);
  });

  it('rejects a JavaScript report substituted for the count-one native role', () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'slither-stage4-report-role-'));
    try {
      const values = Buffer.alloc(8);
      values.writeFloatLE(0.125, 0);
      values.writeFloatLE(-0.25, 4);
      const digest = createHash('sha256').update(values).digest('hex');
      /** Build one minimal structurally valid comparator input for a named execution role. */
      const report = (
        schema: string,
        name: string,
        nativeCallsPerWholePass: number,
        nativeIdentity?: Record<string, string>
      ) => ({
        schema,
        source: schema === 'slither-stage4-rust-inference-benchmark'
          ? {
              buildProfile: 'release',
              buildClass: 'test-hooks',
              targetTriple: 'x86_64-pc-windows-msvc'
            }
          : {},
        workload: {
          scenario: 'P0',
          weightsSha256: 'weights',
          observationsSha256: 'observations',
          initialRecurrentSha256: 'recurrent'
        },
        path: { name, nativeCallsPerWholePass, nativeIdentity },
        result: {
          oneStepComparisonProbe: {
            absoluteTolerance: 1e-4,
            outputsF32LeHex: values.toString('hex'),
            recurrentF32LeHex: values.toString('hex'),
            outputFloats: 2,
            recurrentFloats: 2,
            outputsSha256: digest,
            recurrentSha256: digest
          }
        }
      });
      const rustPath = path.join(directory, 'rust.json');
      const jsPath = path.join(directory, 'js.json');
      const nativePath = path.join(directory, 'native.json');
      const outputPath = path.join(directory, 'comparison.json');
      const rust = report(
        'slither-stage4-rust-inference-benchmark',
        'rust-scalar-coarse-heterogeneous',
        0
      );
      const js = report(
        'slither-stage4-current-inference-benchmark',
        'current-typescript-js-graph',
        0
      );
      const native = report(
        'slither-stage4-current-inference-benchmark',
        'current-typescript-count-one-native',
        165,
        {
          nativeAddonSourceSha256: 'source',
          nativeAddonBuildProfile: 'release',
          nativeAddonBuildClass: 'production',
          nativeAddonBuildTarget: 'x86_64-pc-windows-msvc'
        }
      );
      fs.writeFileSync(rustPath, JSON.stringify(rust));
      fs.writeFileSync(jsPath, JSON.stringify(js));
      fs.writeFileSync(nativePath, JSON.stringify(native));
      expect(() => runComparison({ rustPath, jsPath, nativePath, outputPath })).not.toThrow();
      fs.writeFileSync(nativePath, JSON.stringify(js));
      expect(() => runComparison({ rustPath, jsPath, nativePath, outputPath })).toThrow(
        'does not prove the current count-one native path'
      );
    } finally {
      fs.rmSync(directory, { recursive: true, force: true });
    }
  });

  it('requires an explicit scenario, path, and output destination', () => {
    expect(parseStage4InferenceBenchmarkOptions([
      '--scenario', 'P2',
      '--path', 'native',
      '--warmup-passes', '3',
      '--passes', '7',
      '--environment', 'owner-target-vm',
      '--output', 'result.json'
    ])).toMatchObject({
      scenario: 'P2',
      path: 'native',
      warmupPasses: 3,
      measuredPasses: 7,
      evidenceEnvironment: 'owner-target-vm'
    });
    expect(() => parseStage4InferenceBenchmarkOptions(['--scenario', 'P0'])).toThrow(
      '--scenario, --path, and --output are required'
    );
  });
});

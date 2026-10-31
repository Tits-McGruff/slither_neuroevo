import { describe, expect, it } from 'vitest';
import { parseGenerationWorkJson } from './generationWork.ts';

const fixture = {
  "version": 1,
  "generation": "0000000000000001",
  "firstStep": "0000000000000001",
  "lastStep": "0000000000000002",
  "steps": "0000000000000002",
  "partial": false,
  "neuralDeliveries": "0000000000000000",
  "baselineDeliveries": "0000000000000000",
  "rayPreparations": "0000000000000000",
  "timingsNs": {
    "authorityBegin": "0000000000000000",
    "prefix": "0000000000000000",
    "controlTotal": "0000000000000000",
    "controlCommit": "0000000000000000",
    "worldTotal": "0000000000000000",
    "publication": "0000000000000000",

    "movement": "0000000000000000",
    "food": "0000000000000000",
    "collision": "0000000000000000",
    "collisionIndex": "0000000000000000",
    "collisionHeadHead": "0000000000000000",
    "collisionHeadBody": "0000000000000000",
    "physicsPelletIndex": "0000000000000000",
    "effects": "0000000000000000",
    "controlSpatialIndex": "0000000000000000",
    "bodyRayPrepare": null,
    "baselineSensing": null,
    "baselinePolicy": null,
    "neuralSensing": "0000000000000000",
    "neuralInference": "0000000000000000",
    "neuralBatchPreparation": "0000000000000000",
    "controlBodyIndex": "0000000000000000",
    "controlPelletIndex": "0000000000000000",
    "sensorHeadPrepare": null,
    "bodyQueryWorker": null,
    "bodyRayQueryWorker": null
  },
  "workload": {
    "aliveEvolved": {
      "samples": "0000000000000002",
      "total": "0000000000000000",
      "maximum": "0000000000000000"
    },
    "aliveTotal": {
      "samples": "0000000000000002",
      "total": "0000000000000000",
      "maximum": "0000000000000000"
    },
    "bodyPoints": {
      "samples": "0000000000000002",
      "total": "0000000000000000",
      "maximum": "0000000000000000"
    },
    "bodySegments": {
      "samples": "0000000000000002",
      "total": "0000000000000000",
      "maximum": "0000000000000000"
    },
    "evolvedBodyLength": null,
    "pellets": {
      "samples": "0000000000000002",
      "total": "0000000000000000",
      "maximum": "0000000000000000"
    },
    "bodyQueryCandidates": null,
    "rayQueryCandidates": null
  }
};

describe('generation operational history', () => {
  it('keeps exact integer fields and absent measurements separate from measured zero', () => {
    const parsed = parseGenerationWorkJson(JSON.stringify(fixture), fixture.generation, fixture.lastStep);
    expect(parsed).toEqual(fixture);
    expect(parsed.workload.aliveTotal?.total).toBe('0000000000000000');
    expect(parsed.workload.rayQueryCandidates).toBeNull();
    expect(parsed.timingsNs.bodyRayQueryWorker).toBeNull();
  });
  it('rejects a different generation, terminal step, holes and malformed counters', () => {
    const json = JSON.stringify(fixture);
    expect(() => parseGenerationWorkJson(json, '0000000000000003')).toThrow(/coverage/u);
    expect(() => parseGenerationWorkJson(json, undefined, '0000000000000003')).toThrow(/coverage/u);
    expect(() => parseGenerationWorkJson(JSON.stringify({...fixture, steps:'0000000000000003'}))).toThrow(/coverage/u);
    expect(() => parseGenerationWorkJson(JSON.stringify({...fixture, steps:2}))).toThrow(/counter/u);
    expect(() => parseGenerationWorkJson(JSON.stringify({...fixture, extra:0}))).toThrow(/fields/u);
    expect(() => parseGenerationWorkJson(' '.repeat(8193))).toThrow(/size/u);
  });
  it('rejects impossible distributions and incorrect workload sample denominators', () => {
    const bad = structuredClone(fixture);
    bad.workload.aliveTotal!.maximum = '0000000000000001';
    expect(() => parseGenerationWorkJson(JSON.stringify(bad))).toThrow(/distribution/u);
    bad.workload.aliveTotal!.maximum = '0000000000000000';
    bad.workload.aliveTotal!.samples = '0000000000000001';
    expect(() => parseGenerationWorkJson(JSON.stringify(bad))).toThrow(/sample count/u);
  });
});

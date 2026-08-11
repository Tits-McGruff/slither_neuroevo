import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import {
  buildStage4SensorReferenceDocument,
  type Stage4SensorReferenceDocument
} from './test/stage4SensorReference.ts';

/** Shared TypeScript-to-Rust fixture committed beside the native crate. */
const FIXTURE_PATH = fileURLToPath(
  new URL('../native/fixtures/sensor-v3-reference.json', import.meta.url)
);

describe('Stage 4 sensor-v3 cross-language reference', () => {
  it('matches the retained corrected TypeScript formula fixture exactly', () => {
    const retained = JSON.parse(
      readFileSync(FIXTURE_PATH, 'utf8')
    ) as Stage4SensorReferenceDocument;
    expect(buildStage4SensorReferenceDocument()).toEqual(retained);
  });
});

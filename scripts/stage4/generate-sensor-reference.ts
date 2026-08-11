import { buildStage4SensorReferenceDocument } from '../../src/test/stage4SensorReference.ts';

/** Emit the deterministic fixture for review and `apply_patch` retention. */
const originalInfo = console.info;
try {
  console.info = () => undefined;
  process.stdout.write(`${JSON.stringify(buildStage4SensorReferenceDocument(), null, 2)}\n`);
} finally {
  console.info = originalInfo;
}

import type { ShadowMetrics, ShadowMetricTotals, ShadowReceipt, ShadowSnapshot } from '../../src/protocol/shadows.ts';
import { distillationHex, distillationInteger } from './distillationProtocol.ts';

/** Require an exact bounded metadata record before interpreting fields. */
function record(value: unknown, keys: string): Record<string, unknown> {
  const expected = keys.split(' ');
  if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).length !== expected.length ||
      expected.some(key => !Object.hasOwn(value, key))) throw new TypeError('invalid shadow metadata fields');
  const raw = value as Record<string, unknown>;
  return Object.fromEntries(expected.map(key => [key, raw[key]]));
}
/** Preserve only finite nonnegative aggregate/rate values. */
function nonnegative(value: unknown, label: string): number {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) throw new TypeError('invalid shadow ' + label);
  return value;
}
/** Raw native errors are bounded and internally consistent without recomputing inference. */
function totals(value: unknown): ShadowMetricTotals {
  const raw = record(value, 'samples turnAbsoluteError turnSquaredError boostDisagreements turnMae turnMse boostDisagreementRate');
  const samples = distillationHex(raw['samples'], 16, 'shadow samples');
  const disagreements = distillationHex(raw['boostDisagreements'], 16, 'shadow disagreements');
  if (disagreements > samples) throw new TypeError('shadow disagreement count exceeds samples');
  for (const name of ['turnAbsoluteError', 'turnSquaredError', 'turnMae', 'turnMse', 'boostDisagreementRate']) nonnegative(raw[name], name);
  if ((raw['turnMae'] as number) > 2 || (raw['turnMse'] as number) > 4 || (raw['boostDisagreementRate'] as number) > 1) throw new TypeError('shadow action metric range');
  return raw as unknown as ShadowMetricTotals;
}
/** Validate fixed stratum arrays without accepting lifetime-sized histories. */
function strata(value: unknown, count: number): ShadowMetricTotals[] {
  if (!Array.isArray(value) || value.length !== count) throw new TypeError('invalid shadow strata count');
  return value.map(totals);
}
/** Read one fixed native slot and keep step/sample identities distinct. */
function snapshot(value: unknown, slot: number): ShadowSnapshot | null {
  if (value === null) return null;
  const raw = record(value, 'shadowSlot artifactSha256 captureEpoch baselineSlot activationSequence activationStep completedStep lastSampleStep generation physicalSnakeId enabled error rawOutputs totals strategies lifeStages strata cruising preDeath deaths');
  if (distillationInteger(raw['shadowSlot'], 0, 3, 'shadow slot') !== slot) throw new TypeError('shadow slot ordering mismatch');
  distillationHex(raw['artifactSha256'], 64, 'shadow artifact');
  distillationHex(raw['captureEpoch'], 32, 'shadow capture', true);
  distillationInteger(raw['baselineSlot'], 0, 119, 'shadow teacher');
  for (const name of ['activationSequence', 'activationStep', 'completedStep', 'lastSampleStep', 'generation', 'physicalSnakeId', 'deaths']) distillationHex(raw[name], 16, name, name === 'activationSequence');
  if ((raw['activationStep'] as string) > (raw['completedStep'] as string) || (raw['lastSampleStep'] as string) > (raw['completedStep'] as string)) throw new TypeError('shadow sample chronology');
  if (typeof raw['enabled'] !== 'boolean' || (raw['error'] !== null && (typeof raw['error'] !== 'string' || Buffer.byteLength(raw['error']) > 1024))) throw new TypeError('invalid shadow state');
  if (!Array.isArray(raw['rawOutputs']) || raw['rawOutputs'].length !== 2 || raw['rawOutputs'].some(value => typeof value !== 'number' || !Number.isFinite(value))) throw new TypeError('invalid shadow raw outputs');
  totals(raw['totals']); totals(raw['cruising']); totals(raw['preDeath']);
  strata(raw['strategies'], 4); strata(raw['lifeStages'], 3); strata(raw['strata'], 6);
  return raw as unknown as ShadowSnapshot;
}
/** Parse only the bounded Rust publication, with no sensor/model execution in Node. */
export function parseShadowMetrics(value: string): ShadowMetrics {
  if (typeof value !== 'string' || Buffer.byteLength(value) > 64 * 1024) throw new TypeError('shadow metrics byte admission');
  const raw = record(JSON.parse(value), 'schemaVersion slots');
  distillationInteger(raw['schemaVersion'], 1, 1, 'shadow metrics version');
  if (!Array.isArray(raw['slots']) || raw['slots'].length !== 4) throw new TypeError('shadow metrics slot count');
  return { schemaVersion: 1, slots: raw['slots'].map(snapshot) };
}
/** Strict compact chronology records can be retried without revising admitted results. */
export function parseShadowReceipt(value: unknown): ShadowReceipt {
  const raw = record(value, 'schemaVersion processEpoch requestId commandSequence activationSequence previousActivationSequence runId configRevision captureEpoch artifactSha256 shadowSlot baselineSlot effectiveStep enabled acknowledgedAt');
  distillationInteger(raw['schemaVersion'], 1, 1, 'shadow receipt version');
  distillationHex(raw['processEpoch'], 32, 'shadow process', true);
  distillationHex(raw['requestId'], 32, 'shadow request', true);
  distillationHex(raw['captureEpoch'], 32, 'shadow capture', true);
  distillationHex(raw['artifactSha256'], 64, 'shadow artifact');
  for (const name of ['commandSequence', 'activationSequence', 'configRevision', 'effectiveStep']) distillationHex(raw[name], 16, name, name !== 'effectiveStep');
  distillationHex(raw['previousActivationSequence'], 16, 'shadow previous activation');
  distillationInteger(raw['shadowSlot'], 0, 3, 'shadow slot');
  distillationInteger(raw['baselineSlot'], 0, 119, 'shadow teacher');
  distillationInteger(raw['acknowledgedAt'], 1, Number.MAX_SAFE_INTEGER, 'shadow receipt time');
  if (typeof raw['enabled'] !== 'boolean' || typeof raw['runId'] !== 'string' || !raw['runId'] || Buffer.byteLength(raw['runId']) > 128 || raw['runId'].includes('\0') ||
      (raw['enabled'] && (raw['activationSequence'] !== raw['commandSequence'] || (raw['previousActivationSequence'] as string) >= (raw['activationSequence'] as string))) ||
      (!raw['enabled'] && raw['previousActivationSequence'] !== raw['activationSequence']) || (raw['activationSequence'] as string) > (raw['commandSequence'] as string)) throw new TypeError('invalid shadow receipt scope');
  return raw as unknown as ShadowReceipt;
}

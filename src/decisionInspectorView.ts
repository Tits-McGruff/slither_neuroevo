import type { InspectorSample, InspectorSnapshot } from './protocol/inspector.ts';
import { getSensorLayout } from './protocol/sensors.ts';

/** Concise values retain exact identities and explicit absent measurements. */
function display(value: unknown): string {
  if (value === null || value === undefined) return '—';
  if (typeof value === 'number') return Number.isInteger(value) ? String(value) : value.toFixed(4);
  return String(value);
}
/** Human-readable metadata labels, never markup. */
function label(value: string): string {
  const text = value.replace(/([a-z])([A-Z])/gu, '$1 $2').replaceAll('_', ' ').toLowerCase();
  return text.charAt(0).toUpperCase() + text.slice(1);
}
/** Format actual native values for accessible text beside the angular plot. */
export function describeInspector(snapshot: InspectorSnapshot): {summary: string; scalars: string; sectors: string; inputs: string; policy: string; lifecycle: string; shadows: string} {
  const sample = snapshot.sample;
  if (!sample) return {summary: 'Waiting for a delivered sample from this life.', scalars: '', sectors: '', inputs: '', policy: '', lifecycle: '', shadows: ''};
  const layout = getSensorLayout(sample.bins, sample.profile === 'v3' ? 'v3' : 'v4', sample.profile === 'v3' ? undefined : sample.profile);
  const trace = sample.trace;
  const scalars = Object.entries(sample.raw).map(([key, value]) => label(key) + ': ' + display(value));
  if (trace) for (const [key, value] of Object.entries(trace)) {
    if (Array.isArray(value)) continue;
    scalars.push(label(key) + ': ' + (key === 'kills' ? String(BigInt('0x' + String(value))) : display(value)));
  }
  const sectors = ['bin | food | far food | body sector | body ray | wall | head | heads | TTC | forward | lateral | urgency | head ID'];
  if (trace) for (let bin = 0; bin < sample.bins; bin++) {
    const ray = trace.bodyRays[bin]!;
    sectors.push([bin, trace.foodStrength[bin], sample.profile === 'v3' ? 'not-sampled' : trace.farFoodStrength[bin], trace.bodyClearance[bin],
      ray.kind === 'hit' ? ray.distance : ray.kind, trace.wallClearance[bin], trace.headClearance[bin],
      trace.headCount[bin], ...(sample.profile === 'motion' ? [trace.encounterSeconds[bin], trace.relativeForwardSpeed[bin], trace.relativeSideSpeed[bin], trace.urgency[bin]]
        : ['not-sampled', 'not-sampled', 'not-sampled', 'not-sampled']), trace.headIds[bin]].map(display).join(' | '));
  }
  const action = sample.baselineV1Action ?? sample.baselineAction;
  const policy = action ? Object.entries(action).map(([key, value]) => label(key) + ': ' + display(value)).join('\n')
    : sample.neuralOutputs ? 'Graph turn output: ' + display(sample.neuralOutputs[0]) + '\nGraph boost output: ' + display(sample.neuralOutputs[1])
      : sample.source.kind === 'baseline-v1' ? 'Baseline v1 action details are unavailable for this sample.'
        : sample.source.kind === 'baseline-v2' ? 'Baseline v2 action details are unavailable for this sample.' : 'External controller observation; no internal graph action.';
  const life = sample.lifecycle;
  const lifecycle = life ? ['At delivered sample boundary · provisional fitness is not the final generation outcome.',
    ...(['lifeOrdinal', 'livesStarted', 'deaths', 'successfulRespawns', 'failedPlacements'] as const).map(key =>
      label(key) + ': ' + BigInt('0x' + life[key])),
    'Pending seconds: ' + display(life.pendingSeconds), 'Point assessment: ' + life.pointMode,
    'Nonpoint aggregation: ' + life.nonpointMode, 'Provisional fitness: ' + display(life.provisionalFitness),
    'Population maximum assessment points: ' + display(life.maximumAssessmentPoints),
    ...['Survival', 'Food', 'Growth', 'Kills', 'Points', 'Top bonus', 'Life penalty (subtracted)'].map((key, index) =>
      key + ': ' + display(life.fitnessComponents[index])),
    ...Object.entries(life.assessment).map(([key, value]) => 'Selected ' + label(key).toLowerCase() + ': ' +
      (key === 'kills' ? String(BigInt('0x' + String(value))) : display(value))),
    ...Object.entries(life.ledger).map(([key, value]) => label(key) + ': ' +
      (key === 'operations' ? String(BigInt('0x' + String(value))) : display(value))),
    ...(['current', 'bestPointLife'] as const).flatMap(tuple => [label(tuple) + ':', ...Object.entries(life[tuple]).map(([key, value]) =>
      '  ' + label(key) + ': ' + (key === 'ordinal' || key === 'kills' ? String(BigInt('0x' + String(value))) : display(value)))])
    ].join('\n') : '';
  const shadows = (sample.shadowOutputs ?? []).map(output => `Shadow slot ${output.shadowSlot} · at delivered sample step ${BigInt('0x' + output.sampleStep)}\n` +
    `Candidate SHA256 ${output.artifactSha256}\nCapture epoch ${output.captureEpoch} · activation ${BigInt('0x' + output.activationSequence)}\n` +
    `Graph turn output: ${display(output.rawOutputs[0])}\nGraph boost output: ${display(output.rawOutputs[1])}`).join('\n\n');
  return {summary: `Snake #${sample.snakeId} · ${sample.source.kind} · ${sample.profile} B${sample.bins}\n` +
    `Sample step ${BigInt('0x' + sample.sampleStep)} · age ${BigInt('0x' + sample.ageSteps)} completed steps\n` +
    `Physical ID ${sample.physicalId} · generation ${BigInt('0x' + sample.generation)}\nSensor SHA256 ${sample.sensorSha256}`,
    scalars: scalars.join('\n'), sectors: sectors.join('\n'), policy, lifecycle, shadows,
    inputs: sample.normalizedInputs.map((value, index) => `${index}  ${layout.order[index]}: ${display(value)}`).join('\n')};
}

/** Draw only the delivered raw directions/distances; no geometry queries or inferred observations. */
function draw(canvas: HTMLCanvasElement, sample: InspectorSample | null): void {
  const ctx = canvas.getContext('2d'); if (!ctx) return;
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  if (!sample?.trace) return;
  const trace = sample.trace; const center = canvas.width / 2; const reach = center - 30;
  ctx.save(); ctx.translate(center, canvas.height / 2);
  ctx.strokeStyle = '#596577'; ctx.lineWidth = 1;
  for (const fraction of [0.5, 1]) { ctx.beginPath(); ctx.arc(0, 0, reach * fraction, 0, Math.PI * 2); ctx.stroke(); }
  for (let bin = 0; bin < sample.bins; bin++) {
    const angle = trace.rayHeadings[bin]! - sample.raw.heading - Math.PI / 2;
    const ray = trace.bodyRays[bin]!;
    const length = ray.kind === 'hit' ? Math.min(1, ray.distance / trace.nearRadius) * reach : reach;
    if (ray.kind !== 'not-sampled') {
      ctx.setLineDash(ray.kind === 'incomplete' ? [4, 4] : []);
      ctx.strokeStyle = ray.kind === 'incomplete' ? '#f3b648' : ray.kind === 'hit' ? '#ff6b6b' : '#597565';
      ctx.beginPath(); ctx.moveTo(0, 0); ctx.lineTo(Math.cos(angle) * length, Math.sin(angle) * length); ctx.stroke();
    }
    ctx.setLineDash([]);
    const wall = Math.min(1, Math.max(0, trace.wallClearance[bin]! / trace.nearRadius)) * reach;
    ctx.fillStyle = '#70b8ff'; ctx.beginPath(); ctx.arc(Math.cos(angle) * wall, Math.sin(angle) * wall, 2.5, 0, Math.PI * 2); ctx.fill();
    const food = trace.foodStrength[bin]!;
    if (food > 0) {
      ctx.fillStyle = `rgba(93,221,139,${Math.min(0.85, 0.15 + food / 10)})`;
      ctx.beginPath(); ctx.arc(Math.cos(angle) * reach * 0.55, Math.sin(angle) * reach * 0.55, 4, 0, Math.PI * 2); ctx.fill();
    }
  }
  ctx.fillStyle = '#ecf0f8'; ctx.beginPath(); ctx.arc(0, 0, 5, 0, Math.PI * 2); ctx.fill();
  ctx.fillText('Forward', -22, -reach - 10); ctx.restore();
}

/** On-demand DOM projection. Closed or absent samples clear all diagnostic content. */
export class DecisionInspectorView {
  /** Surface capabilities without enabling capture. */
  supported(supported: boolean): void { const root = document.getElementById('decisionInspector'); if (root) root.hidden = !supported; }
  /** Small status does not rebuild the diagnostic tree. */
  status(message: string): void { const node = document.getElementById('decisionInspectorStatus'); if (node) node.textContent = message; }
  /** Replace text and plot only on a validated native snapshot, at most 10 Hz. */
  snapshot(snapshot: InspectorSnapshot | null): void {
    const content = document.getElementById('decisionInspectorData'); if (content) content.hidden = !snapshot?.sample;
    const result = snapshot ? describeInspector(snapshot) : null;
    for (const [id, value] of Object.entries({decisionInspectorSummary: result?.summary, decisionInspectorScalars: result?.scalars,
      decisionInspectorSectors: result?.sectors, decisionInspectorInputs: result?.inputs, decisionInspectorPolicy: result?.policy,
      decisionInspectorLifecycle: result?.lifecycle, decisionInspectorShadows: result?.shadows})) {
      const node = document.getElementById(id); if (node) node.textContent = value ?? '';
    }
    for (const [id, value] of [['decisionInspectorLifecycleSection', result?.lifecycle], ['decisionInspectorShadowsSection', result?.shadows]]) {
      const section = document.getElementById(id!); if (section) section.hidden = !value;
    }
    const legend = document.getElementById('decisionInspectorLegend');
    if (legend) legend.textContent = snapshot?.sample?.profile === 'v3'
      ? "Blue: wall clearance · green: full-range food strength. V3 body clearances are reported by sector below. Plot distances stop at near reach."
      : "Red: body hits · dashed amber: unknown rays · blue: wall clearance · green: near-food strength. Plot distances stop at near reach; the table retains the sampler's values.";
    const canvas = document.getElementById('decisionInspectorCanvas') as HTMLCanvasElement | null;
    if (canvas) draw(canvas, snapshot?.sample ?? null);
  }
}

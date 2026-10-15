import type { InspectorSnapshot } from '../protocol/inspector.ts';
import type { SensorProfile } from '../protocol/sensors.ts';
import { getSensorLayout } from '../protocol/sensors.ts';

/** Synthetic bounded native metadata for presentation/transport tests, never a simulator. */
export function inspectorFixture(profile: SensorProfile = 'v3', bins = 16): InspectorSnapshot {
  const layout = getSensorLayout(bins, profile === 'v3' ? 'v3' : 'v4', profile === 'v3' ? undefined : profile);
  return {schemaVersion: 1, sequence: '0000000000000001', subscriptionSequence: '0000000000000001',
    worldEpoch: '0000000000000001', completedStep: '0000000000000002', sample: {
      generation: '0000000000000001', physicalId: 'ffffffffffffffff', snakeId: 3, populationSlot: 0,
      sampleStep: '0000000000000001', ageSteps: '0000000000000001', source: {kind: 'neural'}, profile, bins,
      sensorSha256: 'a'.repeat(64), normalizedInputs: Array<number>(layout.inputSize).fill(0), neuralOutputs: [2, -3], baselineAction: null,
      raw: {kind: 'evolved', points: 10, previouslyDeliveredPoints: 5, firstDelivery: true,
        x: 0, y: 0, heading: 0, radius: 9, speed: 165, controlAccumulatorSeconds: 0.06},
      trace: {nearRadius: 100, farRadius: 200, bodyLength: 5, ageSeconds: 1, food: 0, fitness: -2, sizeFactor: 0,
        heldTurn: 0, previousAppliedTurn: 0, headingDelta: 0, speedDelta: 0, boostCost: 1, boostHeadroom: 0,
        generationSeconds: 240, foodSaturation: 1, boostSpeed: 500,
        assessmentPoints: 10, lifePoints: profile === 'v3' ? null : 10, generationBestPoints: profile === 'v3' ? 10 : null,
        generationElapsedSeconds: profile === 'v3' ? null : 1, nearestFoodDistance: null, nearestFoodBearing: null,
        nearestBodyClearance: null, nearestHeadClearance: null,
        foodStrength: Array<number>(bins).fill(0), farFoodStrength: Array<number>(bins).fill(0), bodyClearance: Array<number>(bins).fill(100),
        rayHeadings: Array<number>(bins).fill(0), wallClearance: Array<number>(bins).fill(100), headClearance: Array<number>(bins).fill(100),
        headBearingOffset: Array<number>(bins).fill(0), headLengthRatio: Array<number>(bins).fill(0), relativeHeading: Array<number>(bins).fill(0),
        relativeForwardSpeed: Array<number>(bins).fill(0), relativeSideSpeed: Array<number>(bins).fill(0), urgency: Array<number>(bins).fill(0),
        kills: 'ffffffffffffffff', actualBoost: false, heldBoostRequest: false, boostEligible: null,
        pelletSaturated: false, bodySaturated: false, bodyRays: Array.from({length: bins}, () => ({kind: profile === 'v3' ? 'not-sampled' : 'incomplete'})),
        headCount: Array<number>(bins).fill(0), headIds: Array<null>(bins).fill(null), encounterSeconds: Array<null>(bins).fill(null)},
      diagnostics: {pelletChecks: 0, segmentChecks: 0, headChecks: 0, rayWork: 0, incompleteRays: profile === 'v3' ? 0 : bins}}};
}

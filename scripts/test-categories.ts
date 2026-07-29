/** Primary, non-overlapping test layers used for complete-suite accounting. */
export const PRIMARY_TEST_CATEGORIES = {
  unit: [
    'scripts/ci-contract.test.ts',
    'scripts/recoveryPhase10.lan.test.ts',
    'scripts/stage2/fixtures.test.ts',
    'scripts/stage2/retentionPolicy.test.ts',
    'scripts/test-categories.test.ts',
    'server/authoritativeWorldDigest.test.ts',
    'server/hash.test.ts',
    'server/stage1.controllerConfig.test.ts',
    'server/test/networkSuites.test.ts',
    'src/BrainViz.test.ts',
    'src/FitnessChart.test.ts',
    'src/bots/baselineBots.test.ts',
    'src/brains/graph.test.ts',
    'src/brains/graph.unit.test.ts',
    'src/brains/graph/editor.test.ts',
    'src/brains/nativeBridge.missing.test.ts',
    'src/brains/nativeBridge.test.ts',
    'src/brains/nullBrain.test.ts',
    'src/brains/ops.test.ts',
    'src/brains/stackBuilder.unit.test.ts',
    'src/chartUtils.test.ts',
    'src/config.test.ts',
    'src/hallOfFame.test.ts',
    'src/mlp.test.ts',
    'src/particles.test.ts',
    'src/protocol/settings.test.ts',
    'src/recoveryPhase5.sensors.test.ts',
    'src/render.test.ts',
    'src/rng.test.ts',
    'src/sensors.test.ts',
    'src/serializer.test.ts',
    'src/snake.test.ts',
    'src/spatialHash.test.ts',
    'src/stage2.killCredit.characterization.test.ts',
    'src/storage.test.ts',
    'src/theme.test.ts',
    'src/utils.test.ts'
  ],
  component: [
    'server/brainPool.test.ts',
    'server/controllerRegistry.test.ts',
    'server/inferenceMode.test.ts',
    'server/persistence.test.ts',
    'server/protocol.test.ts',
    'server/recoveryPhase0.characterization.test.ts',
    'server/recoveryPhase4.brainPool.test.ts',
    'src/net/authoritativeControls.test.ts',
    'src/net/playerActionPump.test.ts',
    'src/net/wsClient.test.ts',
    'src/recoveryPhase1.world.test.ts',
    'src/settings.test.ts',
    'src/sim/SimCore.test.ts',
    'src/world.test.ts'
  ],
  integration: [
    'server/integration.test.ts',
    'server/recoveryPhase0Startup.characterization.test.ts',
    'server/recoveryPhase1.lifecycle.test.ts',
    'server/recoveryPhase2.determinism.test.ts',
    'server/recoveryPhase3.native.test.ts',
    'server/recoveryPhase4.simServer.test.ts',
    'server/recoveryPhase6.controls.test.ts',
    'server/recoveryPhase7.persistence.test.ts',
    'server/stage1.browserControl.integration.test.ts',
    'server/stage1.controllerGrace.test.ts',
    'server/stage1.schedulerYield.test.ts',
    'server/wsHub.priority.test.ts',
    'src/brains/graph.integration.test.ts',
    'src/main.test.ts'
  ],
  system: ['server/system.test.ts'],
  acceptance: ['server/acceptance.test.ts'],
  regression: [
    'src/stack.regression.test.ts',
    'src/stage1.correctionFixtures.test.ts'
  ],
  performance: ['server/performance.test.ts'],
  security: ['server/security.test.ts']
} as const;

/** Primary category name used by completeness checks. */
export type PrimaryTestCategory = keyof typeof PRIMARY_TEST_CATEGORIES;

/** Ordered primary layers used to assemble the complete JavaScript suite. */
export const PRIMARY_TEST_CATEGORY_ORDER = [
  'unit',
  'component',
  'integration',
  'system',
  'acceptance',
  'regression',
  'performance',
  'security'
] as const satisfies readonly PrimaryTestCategory[];

/** Required-native overlay that must fail when the addon or native MT path is unavailable. */
export const NATIVE_REQUIRED_TEST_FILES = [
  'src/brains/nativeBridge.test.ts',
  'server/brainPool.test.ts',
  'server/inferenceMode.test.ts',
  'server/recoveryPhase3.native.test.ts',
  'server/recoveryPhase4.brainPool.test.ts',
  'server/recoveryPhase4.simServer.test.ts',
  'server/system.test.ts'
] as const;

/** Complete category registry, including aggregate and required-native overlays. */
export const TEST_CATEGORIES = {
  ...PRIMARY_TEST_CATEGORIES,
  all: PRIMARY_TEST_CATEGORY_ORDER.flatMap(category => PRIMARY_TEST_CATEGORIES[category]),
  'native-required': NATIVE_REQUIRED_TEST_FILES
} as const;

/** Valid category accepted by the command-line runner. */
export type TestCategory = keyof typeof TEST_CATEGORIES;

/**
 * Check whether a string names one registered test category.
 * @param value - Candidate category.
 * @returns True when the category exists.
 */
export function isTestCategory(value: string): value is TestCategory {
  return Object.hasOwn(TEST_CATEGORIES, value);
}

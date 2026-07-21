/** Phase 0 canonical authoritative-World digest and divergence test helper. */

import { createHash } from 'node:crypto';
import type { Genome } from '../../src/mlp.ts';
import type { Snake } from '../../src/snake.ts';
import type { World } from '../../src/world.ts';

/** Canonical digest schema version. */
const DIGEST_VERSION = 1;
/** Raw Float32 hexadecimal width. */
const FLOAT32_HEX_WIDTH = 8;

/** Encoding used by one canonical state entry. */
export type StateEncoding =
  | 'absent'
  | 'boolean'
  | 'float32-array-bits'
  | 'float32-bits'
  | 'integer'
  | 'string';

/** Diagnostic ownership for a canonical state entry. */
export interface StateContext {
  /** Durable population slot, when known. */
  populationSlot: number | null;
  /** Brain family, when applicable. */
  brainType: string | null;
  /** Recurrent graph node, when applicable. */
  brainNode: string | null;
}

/** One canonically encoded authoritative value. */
export interface StateEntry {
  /** Stable comparison path. */
  path: string;
  /** Value encoding. */
  encoding: StateEncoding;
  /** Encoded scalar or raw Float32 words. */
  encoded: string | Uint32Array;
  /** Mismatch context. */
  context: StateContext;
}

/** Canonical state plus a compact equality digest. */
export interface AuthoritativeWorldDigest {
  /** Capture schema version. */
  version: number;
  /** Hash algorithm. */
  algorithm: 'sha256';
  /** Lowercase hexadecimal digest. */
  digest: string;
  /** Simulation tick at capture. */
  tick: number;
  /** Detailed entries retained for diagnostics. */
  entries: readonly StateEntry[];
}

/** Details for the first canonical mismatch. */
export interface AuthoritativeWorldDivergence {
  /** Stable mismatching path. */
  path: string;
  /** Expected encoded value, or null when missing. */
  expected: string | null;
  /** Actual encoded value, or null when missing. */
  actual: string | null;
  /** Expected capture tick. */
  expectedTick: number;
  /** Actual capture tick. */
  actualTick: number;
  /** Durable population slot, when known. */
  populationSlot: number | null;
  /** Brain family, when known. */
  brainType: string | null;
  /** Brain node, when known. */
  brainNode: string | null;
  /** One-line diagnostic. */
  message: string;
}

/** Error raised when invalid state cannot be represented honestly. */
export class AuthoritativeStateCaptureError extends Error {
  /** Invalid canonical path. */
  readonly path: string;

  /**
   * Create a path-specific capture error.
   * @param path - Invalid state path.
   * @param reason - Validation failure.
   */
  constructor(path: string, reason: string) {
    super(`Cannot capture authoritative state at ${path}: ${reason}`);
    this.name = 'AuthoritativeStateCaptureError';
    this.path = path;
  }
}

/** Compatibility view for hand fixtures created before durable slots existed. */
interface OptionalPopulationSlot {
  /** Durable population identity when supplied by the fixture. */
  populationSlot?: number | null;
}

/** Recurrent graph-node state visible on current GraphBrain instances. */
interface RecurrentNodeView {
  /** Stable graph node id. */
  id: string;
  /** Graph node family. */
  type: string;
  /** GRU state, when present. */
  gru?: { h: Float32Array };
  /** LSTM state, when present. */
  lstm?: { h: Float32Array; c: Float32Array };
  /** RRU state, when present. */
  rru?: { h: Float32Array };
}

/** Structural view used without binding the helper to one backend. */
interface GraphBrainView {
  /** Runtime graph nodes. */
  nodes?: RecurrentNodeView[];
}

/** Current baseline-manager fields that affect future control. */
interface BotManagerView {
  /** Stable bot seeds. */
  botSeeds?: number[];
  /** State-machine labels. */
  botStates?: string[];
  /** State-machine timers. */
  botStateTimers?: number[];
  /** Wander angles. */
  botWanderAngles?: number[];
  /** Wander timers. */
  botWanderTimers?: number[];
  /** Pending actions. */
  botActions?: Array<{ turn: number; boost: number }>;
  /** Current snake ids. */
  botSnakeIds?: number[];
  /** Respawn timers. */
  respawnTimers?: number[];
  /** Global controller-disable flag. */
  controllerDisabled?: boolean;
}

/** Canonical identity assigned to one snake. */
interface CanonicalSnake {
  /** Snake to capture. */
  snake: Snake;
  /** Sortable durable identity. */
  identity: string;
  /** Population slot, when owned by the population. */
  populationSlot: number | null;
}

/** Canonically sortable pellet fields. */
interface CanonicalPellet {
  /** Raw Float32 X bits. */
  x: string;
  /** Raw Float32 Y bits. */
  y: string;
  /** Raw Float32 value bits. */
  value: string;
  /** Gameplay/source kind. */
  kind: string;
}

/** Context for non-neural state. */
const WORLD_CONTEXT: StateContext = {
  populationSlot: null,
  brainType: null,
  brainNode: null
};

/**
 * Compare text with locale-independent code-unit ordering.
 * @param left - Left value.
 * @param right - Right value.
 * @returns Ordering result.
 */
function compareText(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

/**
 * Encode a continuous scalar as exact raw Float32 bits.
 * @param path - Error-reporting path.
 * @param value - Scalar value.
 * @returns Canonical hexadecimal bits.
 */
function float32Bits(path: string, value: number): string {
  if (!Number.isFinite(value)) {
    throw new AuthoritativeStateCaptureError(path, `non-finite value ${String(value)}`);
  }
  const rounded = Math.fround(value);
  if (!Number.isFinite(rounded)) {
    throw new AuthoritativeStateCaptureError(path, `Float32 overflow from ${String(value)}`);
  }
  const bytes = new ArrayBuffer(4);
  const view = new DataView(bytes);
  view.setFloat32(0, rounded, false);
  return `0x${view.getUint32(0, false).toString(16).padStart(FLOAT32_HEX_WIDTH, '0')}`;
}

/**
 * Encode Float32Array elements as raw words.
 * @param path - Buffer path.
 * @param values - Values to encode.
 * @returns Raw words in element order.
 */
function float32ArrayBits(path: string, values: Float32Array): Uint32Array {
  const words = new Uint32Array(values.length);
  for (let index = 0; index < values.length; index++) {
    const encoded = float32Bits(`${path}[${index}]`, values[index]!);
    words[index] = Number.parseInt(encoded.slice(2), 16);
  }
  return words;
}

/**
 * Encode a safe integer exactly.
 * @param path - Error-reporting path.
 * @param value - Integer value.
 * @returns Decimal representation.
 */
function integer(path: string, value: number): string {
  if (!Number.isSafeInteger(value)) {
    throw new AuthoritativeStateCaptureError(path, `expected safe integer, received ${String(value)}`);
  }
  return String(value);
}

/**
 * Format a non-negative identity for lexical sorting.
 * @param path - Error-reporting path.
 * @param value - Identity value.
 * @returns Padded identity.
 */
function identity(path: string, value: number): string {
  const encoded = integer(path, value);
  if (value < 0) throw new AuthoritativeStateCaptureError(path, `negative identity ${encoded}`);
  return encoded.padStart(12, '0');
}

/**
 * Add a scalar entry.
 * @param entries - Entry accumulator.
 * @param path - Stable path.
 * @param encoding - Scalar encoding.
 * @param encoded - Encoded value.
 * @param context - Diagnostic context.
 */
function addScalar(
  entries: StateEntry[],
  path: string,
  encoding: Exclude<StateEncoding, 'float32-array-bits'>,
  encoded: string,
  context: StateContext = WORLD_CONTEXT
): void {
  entries.push({ path, encoding, encoded, context });
}

/** Add a continuous Float32 scalar entry. */
function addNumber(entries: StateEntry[], path: string, value: number, context: StateContext = WORLD_CONTEXT): void {
  addScalar(entries, path, 'float32-bits', float32Bits(path, value), context);
}

/** Add an exact integer entry. */
function addInteger(entries: StateEntry[], path: string, value: number, context: StateContext = WORLD_CONTEXT): void {
  addScalar(entries, path, 'integer', integer(path, value), context);
}

/** Add a Boolean entry. */
function addBoolean(entries: StateEntry[], path: string, value: boolean, context: StateContext = WORLD_CONTEXT): void {
  addScalar(entries, path, 'boolean', value ? 'true' : 'false', context);
}

/** Add a string entry. */
function addString(entries: StateEntry[], path: string, value: string, context: StateContext = WORLD_CONTEXT): void {
  addScalar(entries, path, 'string', value, context);
}

/** Add an absent marker. */
function addAbsent(entries: StateEntry[], path: string, context: StateContext = WORLD_CONTEXT): void {
  addScalar(entries, path, 'absent', '', context);
}

/** Add a raw Float32 buffer entry. */
function addFloat32Array(
  entries: StateEntry[],
  path: string,
  values: Float32Array,
  context: StateContext = WORLD_CONTEXT
): void {
  entries.push({ path, encoding: 'float32-array-bits', encoded: float32ArrayBits(path, values), context });
}

/** Add an optional continuous scalar. */
function addOptionalNumber(entries: StateEntry[], path: string, value: number | undefined, context: StateContext): void {
  if (value == null) addAbsent(entries, path, context);
  else addNumber(entries, path, value, context);
}

/** Add an optional exact integer. */
function addOptionalInteger(entries: StateEntry[], path: string, value: number | undefined, context: StateContext): void {
  if (value == null) addAbsent(entries, path, context);
  else addInteger(entries, path, value, context);
}

/** Add an optional Boolean. */
function addOptionalBoolean(entries: StateEntry[], path: string, value: boolean | undefined, context: StateContext): void {
  if (value == null) addAbsent(entries, path, context);
  else addBoolean(entries, path, value, context);
}

/** Capture one genome. */
function captureGenome(entries: StateEntry[], path: string, genome: Genome, context: StateContext): void {
  addString(entries, `${path}.archKey`, genome.archKey, context);
  addString(entries, `${path}.brainType`, genome.brainType, context);
  addNumber(entries, `${path}.fitness`, genome.fitness, context);
  addFloat32Array(entries, `${path}.weights`, genome.weights, context);
}

/** Infer explicit future slot or current id-based population ownership. */
function populationSlot(world: World, snake: Snake): number | null {
  const explicit = (snake as unknown as OptionalPopulationSlot).populationSlot;
  if (explicit != null) {
    if (explicit < 0) throw new AuthoritativeStateCaptureError(`snake[id=${snake.id}].populationSlot`, 'negative slot');
    integer(`snake[id=${snake.id}].populationSlot`, explicit);
    return explicit;
  }
  if (snake.baselineBotIndex == null && snake.id >= 1 && snake.id <= world.population.length) return snake.id - 1;
  return null;
}

/** Build canonical snake identities independent of array order. */
function canonicalSnakes(world: World): CanonicalSnake[] {
  const result = world.snakes.map((snake): CanonicalSnake => {
    const slot = populationSlot(world, snake);
    if (slot != null) return { snake, populationSlot: slot, identity: `population=${identity('populationSlot', slot)}` };
    if (snake.baselineBotIndex != null) {
      return {
        snake,
        populationSlot: null,
        identity: `baseline=${identity('baselineBotIndex', snake.baselineBotIndex)}`
      };
    }
    return { snake, populationSlot: null, identity: `snake-id=${identity('snake.id', snake.id)}` };
  });
  result.sort((left, right) => compareText(left.identity, right.identity));
  for (let index = 1; index < result.length; index++) {
    if (result[index - 1]!.identity === result[index]!.identity) {
      throw new AuthoritativeStateCaptureError(`20.snakes.${result[index]!.identity}`, 'duplicate durable identity');
    }
  }
  return result;
}

/** Capture recurrent state in graph-node-id order. */
function captureRecurrent(entries: StateEntry[], path: string, snake: Snake, context: StateContext): void {
  const brain = snake.brain as unknown as GraphBrainView;
  if (!Array.isArray(brain.nodes)) return;
  const nodes = [...brain.nodes].sort((left, right) => compareText(left.id, right.id));
  for (let index = 1; index < nodes.length; index++) {
    if (nodes[index - 1]!.id === nodes[index]!.id) {
      throw new AuthoritativeStateCaptureError(`${path}.brain.node=${nodes[index]!.id}`, 'duplicate node id');
    }
  }
  for (const node of nodes) {
    const nodePath = `${path}.brain.node=${node.id}`;
    const nodeContext: StateContext = { ...context, brainNode: node.id };
    if (node.gru) addFloat32Array(entries, `${nodePath}.h`, node.gru.h, nodeContext);
    if (node.lstm) {
      addFloat32Array(entries, `${nodePath}.c`, node.lstm.c, nodeContext);
      addFloat32Array(entries, `${nodePath}.h`, node.lstm.h, nodeContext);
    }
    if (node.rru) addFloat32Array(entries, `${nodePath}.h`, node.rru.h, nodeContext);
  }
}

/** Capture one snake's authoritative gameplay and neural state. */
function captureSnake(entries: StateEntry[], item: CanonicalSnake): void {
  const { snake, populationSlot: slot } = item;
  const path = `20.snakes.${item.identity}`;
  const context: StateContext = { populationSlot: slot, brainType: snake.genome.brainType, brainNode: null };
  addInteger(entries, `${path}.id`, snake.id, context);
  if (slot == null) addAbsent(entries, `${path}.populationSlot`, context);
  else addInteger(entries, `${path}.populationSlot`, slot, context);
  if (snake.baselineBotIndex == null) addAbsent(entries, `${path}.baselineBotIndex`, context);
  else addInteger(entries, `${path}.baselineBotIndex`, snake.baselineBotIndex, context);
  addString(entries, `${path}.controlMode`, snake.controlMode, context);
  addBoolean(entries, `${path}.alive`, snake.alive, context);
  addNumber(entries, `${path}.x`, snake.x, context);
  addNumber(entries, `${path}.y`, snake.y, context);
  addNumber(entries, `${path}.dir`, snake.dir, context);
  addNumber(entries, `${path}.radius`, snake.radius, context);
  addNumber(entries, `${path}.speed`, snake.speed, context);
  addNumber(entries, `${path}.boost`, snake.boost, context);
  addInteger(entries, `${path}.foodEaten`, snake.foodEaten, context);
  addNumber(entries, `${path}.age`, snake.age, context);
  addNumber(entries, `${path}.killScore`, snake.killScore, context);
  addNumber(entries, `${path}.pointsScore`, snake.pointsScore, context);
  addNumber(entries, `${path}.prevPointsScore`, snake.prevPointsScore, context);
  addNumber(entries, `${path}.targetLen`, snake.targetLen, context);
  addNumber(entries, `${path}.turnInput`, snake.turnInput, context);
  addNumber(entries, `${path}.boostInput`, snake.boostInput, context);
  addOptionalNumber(entries, `${path}.controlAccumulator`, snake._ctrlAcc, context);
  addOptionalInteger(entries, `${path}.hasAction`, snake._hasAct, context);
  addOptionalBoolean(entries, `${path}.lastControlExternal`, snake._lastControlExternal, context);
  addInteger(entries, `${path}.points.count`, snake.points.length, context);
  for (let index = 0; index < snake.points.length; index++) {
    const pointPath = `${path}.points.${identity('point.index', index)}`;
    addNumber(entries, `${pointPath}.x`, snake.points[index]!.x, context);
    addNumber(entries, `${pointPath}.y`, snake.points[index]!.y, context);
  }
  captureGenome(entries, `${path}.genome`, snake.genome, context);
  captureRecurrent(entries, path, snake, context);
}

/** Canonicalize pellets independently of backing-array order. */
function canonicalPellets(world: World): CanonicalPellet[] {
  const result = world.pellets.map((pellet, index): CanonicalPellet => ({
    x: float32Bits(`30.pellets.source[${index}].x`, pellet.x),
    y: float32Bits(`30.pellets.source[${index}].y`, pellet.y),
    value: float32Bits(`30.pellets.source[${index}].value`, pellet.v),
    kind: pellet.kind
  }));
  result.sort((left, right) => compareText(
    `${left.x}\u0000${left.y}\u0000${left.value}\u0000${left.kind}`,
    `${right.x}\u0000${right.y}\u0000${right.value}\u0000${right.kind}`
  ));
  return result;
}

/**
 * Capture inspectable baseline-manager state. Opaque closure RNG continuation
 * remains deferred until Phase 2 provides exportable production RNG state.
 */
function captureBotManager(entries: StateEntry[], world: World): void {
  const manager = world.botManager as unknown as BotManagerView;
  const count = Math.max(
    manager.botSeeds?.length ?? 0,
    manager.botStates?.length ?? 0,
    manager.botStateTimers?.length ?? 0,
    manager.botWanderAngles?.length ?? 0,
    manager.botWanderTimers?.length ?? 0,
    manager.botActions?.length ?? 0,
    manager.botSnakeIds?.length ?? 0,
    manager.respawnTimers?.length ?? 0
  );
  addInteger(entries, '40.baselineBots.count', count);
  addBoolean(entries, '40.baselineBots.controllerDisabled', manager.controllerDisabled === true);
  for (let slot = 0; slot < count; slot++) {
    const path = `40.baselineBots.slot=${identity('baselineBots.slot', slot)}`;
    const seed = manager.botSeeds?.[slot];
    const state = manager.botStates?.[slot];
    const action = manager.botActions?.[slot];
    if (seed == null) addAbsent(entries, `${path}.seed`); else addInteger(entries, `${path}.seed`, seed);
    if (state == null) addAbsent(entries, `${path}.state`); else addString(entries, `${path}.state`, state);
    addOptionalNumber(entries, `${path}.stateTimer`, manager.botStateTimers?.[slot], WORLD_CONTEXT);
    addOptionalNumber(entries, `${path}.wanderAngle`, manager.botWanderAngles?.[slot], WORLD_CONTEXT);
    addOptionalNumber(entries, `${path}.wanderTimer`, manager.botWanderTimers?.[slot], WORLD_CONTEXT);
    if (action == null) {
      addAbsent(entries, `${path}.action.turn`);
      addAbsent(entries, `${path}.action.boost`);
    } else {
      addNumber(entries, `${path}.action.turn`, action.turn);
      addNumber(entries, `${path}.action.boost`, action.boost);
    }
    addOptionalInteger(entries, `${path}.snakeId`, manager.botSnakeIds?.[slot], WORLD_CONTEXT);
    addOptionalNumber(entries, `${path}.respawnTimer`, manager.respawnTimers?.[slot], WORLD_CONTEXT);
  }
}

/** Build the sorted canonical entry list for one completed World boundary. */
function captureEntries(world: World): StateEntry[] {
  const entries: StateEntry[] = [];
  addString(entries, '00.world.archKey', world.archKey);
  addInteger(entries, '00.world.generation', world.generation);
  addNumber(entries, '00.world.generationTime', world.generationTime);
  addInteger(entries, '00.world.tick', world.tickId);
  addNumber(entries, '00.world.pelletSpawnAccumulator', world._pelletSpawnAcc);
  addNumber(entries, '00.world.simSpeed', world.simSpeed);
  addNumber(entries, '00.world.bestPointsThisGeneration', world.bestPointsThisGen);
  addInteger(entries, '00.world.bestPointsSnakeId', world.bestPointsSnakeId);
  addInteger(entries, '00.world.nextExternalSnakeId', world._nextExternalSnakeId);
  addInteger(entries, '00.world.nextBaselineBotId', world._nextBaselineBotId);
  addInteger(entries, '01.settings.snakeCount', world.settings.snakeCount);
  addNumber(entries, '01.settings.simSpeed', world.settings.simSpeed);
  addInteger(entries, '01.settings.hiddenLayers', world.settings.hiddenLayers);
  addInteger(entries, '01.settings.neurons1', world.settings.neurons1);
  addInteger(entries, '01.settings.neurons2', world.settings.neurons2);
  addInteger(entries, '01.settings.neurons3', world.settings.neurons3);
  addInteger(entries, '01.settings.neurons4', world.settings.neurons4);
  addInteger(entries, '01.settings.neurons5', world.settings.neurons5);
  addNumber(entries, '01.settings.worldRadius', world.settings.worldRadius);
  addNumber(entries, '01.settings.collision.substepMaxDt', world.settings.collision.substepMaxDt);
  addInteger(entries, '01.settings.collision.skipSegments', world.settings.collision.skipSegments);
  addNumber(entries, '01.settings.collision.hitScale', world.settings.collision.hitScale);
  addNumber(entries, '01.settings.collision.cellSize', world.settings.collision.cellSize);
  addInteger(entries, '01.settings.collision.neighborRange', world.settings.collision.neighborRange);

  addInteger(entries, '10.population.count', world.population.length);
  for (let slot = 0; slot < world.population.length; slot++) {
    const genome = world.population[slot];
    if (!genome) throw new AuthoritativeStateCaptureError(`10.population.slot=${slot}`, 'missing dense slot');
    const path = `10.population.slot=${identity('population.slot', slot)}`;
    const context: StateContext = { populationSlot: slot, brainType: genome.brainType, brainNode: null };
    captureGenome(entries, path, genome, context);
  }

  const snakes = canonicalSnakes(world);
  addInteger(entries, '20.snakes.count', snakes.length);
  for (const snake of snakes) captureSnake(entries, snake);

  const pellets = canonicalPellets(world);
  addInteger(entries, '30.pellets.count', pellets.length);
  for (let index = 0; index < pellets.length; index++) {
    const pellet = pellets[index]!;
    const path = `30.pellets.${identity('pellets.index', index)}`;
    addScalar(entries, `${path}.x`, 'float32-bits', pellet.x);
    addScalar(entries, `${path}.y`, 'float32-bits', pellet.y);
    addScalar(entries, `${path}.value`, 'float32-bits', pellet.value);
    addString(entries, `${path}.kind`, pellet.kind);
  }
  captureBotManager(entries, world);

  entries.sort((left, right) => compareText(left.path, right.path));
  for (let index = 1; index < entries.length; index++) {
    if (entries[index - 1]!.path === entries[index]!.path) {
      throw new AuthoritativeStateCaptureError(entries[index]!.path, 'duplicate canonical path');
    }
  }
  return entries;
}

/** Add one unambiguous length-prefixed UTF-8 value to a hash. */
function hashString(hash: ReturnType<typeof createHash>, value: string): void {
  const bytes = Buffer.from(value, 'utf8');
  const length = Buffer.allocUnsafe(4);
  length.writeUInt32BE(bytes.length, 0);
  hash.update(length);
  hash.update(bytes);
}

/** Hash canonical entries without JSON numeric formatting. */
function digestEntries(entries: readonly StateEntry[]): string {
  const hash = createHash('sha256');
  hashString(hash, `slither-authoritative-world-v${DIGEST_VERSION}`);
  for (const entry of entries) {
    hashString(hash, entry.path);
    hashString(hash, entry.encoding);
    if (typeof entry.encoded === 'string') {
      hashString(hash, entry.encoded);
    } else {
      const length = Buffer.allocUnsafe(4);
      length.writeUInt32BE(entry.encoded.length, 0);
      hash.update(length);
      const word = Buffer.allocUnsafe(4);
      for (const bits of entry.encoded) {
        word.writeUInt32BE(bits, 0);
        hash.update(word);
      }
    }
  }
  return hash.digest('hex');
}

/**
 * Capture a stable digest from any supplied World instance.
 *
 * Observer camera/focus/view state, render particles/colors, debug sensor and
 * output caches, run/session ids, timestamps, and chart/HoF history are excluded.
 * @param world - World captured at a completed authoritative boundary.
 * @returns Canonical digest and detailed entries.
 */
export function captureAuthoritativeWorldDigest(world: World): AuthoritativeWorldDigest {
  const entries = captureEntries(world);
  integer('00.world.tick', world.tickId);
  return {
    version: DIGEST_VERSION,
    algorithm: 'sha256',
    digest: digestEntries(entries),
    tick: world.tickId,
    entries
  };
}

/** Render one encoded element for a diagnostic. */
function display(entry: StateEntry, index?: number): string | null {
  if (typeof entry.encoded === 'string') return `${entry.encoding}:${JSON.stringify(entry.encoded)}`;
  if (index == null) return `${entry.encoding}[length=${entry.encoded.length}]`;
  const bits = entry.encoded[index];
  return bits == null ? null : `float32-bits:0x${bits.toString(16).padStart(FLOAT32_HEX_WIDTH, '0')}`;
}

/** Build a consistently contextualized divergence. */
function divergence(
  expectedDigest: AuthoritativeWorldDigest,
  actualDigest: AuthoritativeWorldDigest,
  path: string,
  expected: string | null,
  actual: string | null,
  expectedEntry?: StateEntry,
  actualEntry?: StateEntry
): AuthoritativeWorldDivergence {
  const context = expectedEntry?.context ?? actualEntry?.context ?? WORLD_CONTEXT;
  const slot = context.populationSlot == null ? 'n/a' : String(context.populationSlot);
  const brain = context.brainType ?? 'n/a';
  const node = context.brainNode ?? 'n/a';
  return {
    path,
    expected,
    actual,
    expectedTick: expectedDigest.tick,
    actualTick: actualDigest.tick,
    populationSlot: context.populationSlot,
    brainType: context.brainType,
    brainNode: context.brainNode,
    message: `authoritative divergence tick=${expectedDigest.tick}/${actualDigest.tick} path=${path} `
      + `slot=${slot} brain=${brain} node=${node} expected=${expected ?? '<missing>'} actual=${actual ?? '<missing>'}`
  };
}

/**
 * Find the first path/value mismatch between two canonical captures.
 * @param expectedDigest - Expected capture.
 * @param actualDigest - Actual capture.
 * @returns First mismatch, or null for equality.
 */
export function findFirstAuthoritativeWorldDivergence(
  expectedDigest: AuthoritativeWorldDigest,
  actualDigest: AuthoritativeWorldDigest
): AuthoritativeWorldDivergence | null {
  let expectedIndex = 0;
  let actualIndex = 0;
  while (expectedIndex < expectedDigest.entries.length || actualIndex < actualDigest.entries.length) {
    const expectedEntry = expectedDigest.entries[expectedIndex];
    const actualEntry = actualDigest.entries[actualIndex];
    if (!expectedEntry && actualEntry) {
      return divergence(expectedDigest, actualDigest, actualEntry.path, null, display(actualEntry), undefined, actualEntry);
    }
    if (expectedEntry && !actualEntry) {
      return divergence(expectedDigest, actualDigest, expectedEntry.path, display(expectedEntry), null, expectedEntry);
    }
    if (!expectedEntry || !actualEntry) break;
    const order = compareText(expectedEntry.path, actualEntry.path);
    if (order < 0) return divergence(expectedDigest, actualDigest, expectedEntry.path, display(expectedEntry), null, expectedEntry);
    if (order > 0) return divergence(expectedDigest, actualDigest, actualEntry.path, null, display(actualEntry), undefined, actualEntry);
    if (expectedEntry.encoding !== actualEntry.encoding) {
      return divergence(expectedDigest, actualDigest, expectedEntry.path, display(expectedEntry), display(actualEntry), expectedEntry, actualEntry);
    }
    if (typeof expectedEntry.encoded === 'string' || typeof actualEntry.encoded === 'string') {
      if (typeof expectedEntry.encoded !== 'string' || typeof actualEntry.encoded !== 'string'
        || expectedEntry.encoded !== actualEntry.encoded) {
        return divergence(expectedDigest, actualDigest, expectedEntry.path, display(expectedEntry), display(actualEntry), expectedEntry, actualEntry);
      }
    } else {
      const count = Math.max(expectedEntry.encoded.length, actualEntry.encoded.length);
      for (let element = 0; element < count; element++) {
        if (expectedEntry.encoded[element] !== actualEntry.encoded[element]) {
          return divergence(
            expectedDigest,
            actualDigest,
            `${expectedEntry.path}[${element}]`,
            display(expectedEntry, element),
            display(actualEntry, element),
            expectedEntry,
            actualEntry
          );
        }
      }
    }
    expectedIndex++;
    actualIndex++;
  }
  return null;
}

/** Constant-sized placement work from one committed newborn boundary. */
export interface EvolvedPlacementDiagnostics {
  /** Exact completed step, or zero before any replacement boundary. */
  step: string;
  /** Owners eligible before the eight-slot selection limit. */
  eligible: number;
  /** Owners actually attempted within the shared work budget. */
  attempted: number;
  /** Successful physical replacements. */
  placed: number;
  /** Completed searches with no safe placement. */
  pendingNoPlacement: number;
  /** Searches stopped by admitted work limits. */
  pendingWorkLimit: number;
  /** Eligible owners deferred without an attempt. */
  deferred: number;
  /** Candidates examined, bounded to 2048. */
  candidates: number;
  /** Geometry checks, bounded to 250000. */
  geometryChecks: number;
}
/** Native facts sampled with a committed frame; no life log or world copy. */
export interface EvolvedLifecycleTelemetry {
  /** Initial compact life-diagnostics schema. */
  version: 1;
  /** Number of persistent population owners. */
  population: number;
  /** Alive physical lives. */
  alive: number;
  /** Dead owners awaiting their delay/deadline. */
  waiting: number;
  /** Owners pending a safe placement after failed work. */
  pendingPlacement: number;
  /** Owners whose finite life allowance is exhausted. */
  exhausted: number;
  /** Dead owners with replacement disabled. */
  deadNoRespawn: number;
  /** Exact total lives started as lowercase hexadecimal u64. */
  lives: string;
  /** Exact finalized life count. */
  deaths: string;
  /** Exact successful replacements. */
  respawns: string;
  /** Exact unsuccessful placement attempts; these do not consume lives. */
  failedPlacements: string;
  /** Accumulated individual pending time in simulated seconds. */
  pendingSeconds: number;
  /** Number of retained ephemeral life-end events, at most 256. */
  recentEventCount: number;
  /** Last generation-scoped life-end ordinal, including evicted events. */
  lifeEndCursor: string;
  /** Last committed placement boundary and actual bounded work. */
  placement: EvolvedPlacementDiagnostics;
}
/** Small-count fields are always schema-bounded safe integers. */
const LIFE_COUNT_FIELDS = ['population', 'alive', 'waiting', 'pendingPlacement', 'exhausted', 'deadNoRespawn'] as const;
/** Lifetime counters remain exact across the JSON boundary. */
const LIFE_WORD_FIELDS = ['lives', 'deaths', 'respawns', 'failedPlacements', 'lifeEndCursor'] as const;
/** Work fields have fixed caps; traversal never depends on a life history. */
const PLACEMENT_COUNT_FIELDS = ['eligible', 'attempted', 'placed', 'pendingNoPlacement', 'pendingWorkLimit', 'deferred', 'candidates', 'geometryChecks'] as const;
/** Require an own finite bounded integer property without allocating copied metadata. */
function count(record: Record<string, unknown>, key: string, maximum: number): boolean {
  const value = record[key];
  return Object.hasOwn(record, key) && typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 && value <= maximum;
}
/** Admit optional version-one diagnostics; invalid/stale payloads clear the UI projection. */
export function parseEvolvedLifecycleTelemetry(value: unknown): EvolvedLifecycleTelemetry | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined;
  const record = value as Record<string, unknown>;
  if (record['version'] !== 1) return undefined;
  for (const key of LIFE_COUNT_FIELDS) if (!count(record, key, 300)) return undefined;
  for (const key of LIFE_WORD_FIELDS) if (!Object.hasOwn(record, key) || typeof record[key] !== 'string' || !/^[0-9a-f]{16}$/u.test(record[key])) return undefined;
  if (!count(record, 'recentEventCount', 256) || typeof record['pendingSeconds'] !== 'number' || !Number.isFinite(record['pendingSeconds']) || record['pendingSeconds'] < 0 ||
      !record['placement'] || typeof record['placement'] !== 'object' || Array.isArray(record['placement'])) return undefined;
  const placement = record['placement'] as Record<string, unknown>;
  if (typeof placement['step'] !== 'string' || !/^[0-9a-f]{16}$/u.test(placement['step'])) return undefined;
  for (const key of PLACEMENT_COUNT_FIELDS) if (!count(placement, key, key === 'geometryChecks' ? 250000 : key === 'candidates' ? 2048 : key === 'eligible' || key === 'deferred' ? 300 : 8)) return undefined;
  const result = record as unknown as EvolvedLifecycleTelemetry;
  const work = result.placement;
  if (result.population < 1 || result.alive + result.waiting + result.pendingPlacement + result.exhausted + result.deadNoRespawn !== result.population ||
      BigInt('0x' + result.lives) !== BigInt(result.population) + BigInt('0x' + result.respawns) || BigInt('0x' + result.deaths) > BigInt('0x' + result.lives) ||
      BigInt(result.recentEventCount) > BigInt('0x' + result.lifeEndCursor) || work.eligible > result.population || work.attempted > work.eligible || work.deferred !== work.eligible - work.attempted || work.placed + work.pendingNoPlacement + work.pendingWorkLimit !== work.attempted) return undefined;
  return result;
}

import { isEntitySelectionToken, isRetainedEntityOrigin, type SelectEntityMsg, type EntitySelectedMsg, type SelectedEntity } from '../protocol/entitySelection.ts';

/** Presentation-only scope observed from welcome and packed frames. */
export interface SelectionScope {
  /** Current server process identity. */
  sessionId: string;
  /** Current run identity. */
  runId: string;
  /** Current generation. */
  generation: number;
}

/** Bounded identity-resolution transport and UI callbacks. */
export interface EntitySelectionTransport {
  /** Queue one native selector. */
  send: (request: SelectEntityMsg) => void;
  /** Send a mutation using the exact token that was resolved. */
  kill: (snakeId: number, token: string) => void;
  /** Publish scalar selection changes; null retires physical references. */
  changed: (selection: SelectedEntity | null) => void;
  /** Read the user's individual-follow presentation preference. */
  followIndividual: () => boolean;
}

/** Current bounded selection and at most one queued intent. */
export interface EntitySelectionClient {
  /** Replace observed scope, clearing selections and outstanding intent when it changes. */
  setScope: (scope: SelectionScope) => void;
  /** Resolve an explicitly selected displayed snake. */
  select: (snakeId: number) => void;
  /** Select a persistent population owner by its zero-based native slot. */
  selectIndividual: (populationSlot: number) => void;
  /** Resolve or reuse a current token for one right-click kill. */
  kill: (snakeId: number) => void;
  /** Consume only the matching current-scope native result. */
  receive: (message: EntitySelectedMsg) => void;
  /** Reacquire one followed owner at at most four requests per second. */
  observeFrame: (generation: number, contains: (id: number) => boolean, now: number) => void;
  /** Persistent selected owner, including a temporarily dead/pending life. */
  followedPopulationSlot: () => number | null;
  /** Clear physical and individual selection within the current scope. */
  clear: () => void;
  /** Clear process/run scope on disconnect or replacement. */
  reset: () => void;
}

/** Latest user/follow intent; frame IDs and slots remain distinct. */
interface SelectionIntent {
  /** Requested presentation action. */
  action: 'select' | 'kill' | 'follow' | 'validate';
  /** Existing physical token that a validation must preserve. */
  token?: string;
  /** Selector domain. */
  kind: 'frame' | 'slot';
  /** Exact domain index. */
  index: number;
}

/** Follow polling interval; only one selected owner incurs native work. */
const FOLLOW_INTERVAL_MS = 250;

/** Validate a scalar native selection before accepting it into UI state. */
function validEntity(entity: SelectedEntity): boolean {
  return isEntitySelectionToken(entity.token) && Number.isSafeInteger(entity.snakeId)
    && entity.snakeId > 0 && entity.snakeId <= 16_777_216
    && Number.isFinite(entity.x) && Number.isFinite(entity.y)
    && Number.isFinite(entity.radius) && entity.radius > 0 && Number.isSafeInteger(entity.skin) && entity.skin >= 0 && entity.skin <= 0xffffffff
    && (entity.populationSlot == null || (Number.isSafeInteger(entity.populationSlot)
      && entity.populationSlot >= 0 && entity.populationSlot < 300))
    && (entity.lifeOrdinal == null || (/^[0-9a-f]{16}$/u.test(entity.lifeOrdinal)
      && entity.lifeOrdinal !== '0000000000000000'))
    && (entity.retainedOrigin == null || (entity.populationSlot == null && entity.lifeOrdinal == null && isRetainedEntityOrigin(entity.retainedOrigin)));
}

/** Describe only the current native selection; names and model bytes never require a catalog lookup. */
export function describeSelectedEntity(entity: SelectedEntity | null): string {
  if (!entity) return 'Select a snake to inspect its live origin.';
  const lines = [`Snake #${entity.snakeId}`];
  if (entity.populationSlot != null) lines.push(`Population slot ${entity.populationSlot}` +
    (entity.lifeOrdinal == null ? '' : ` · life ${BigInt('0x' + entity.lifeOrdinal)}`));
  const origin = entity.retainedOrigin;
  if (origin?.kind === 'hall-of-fame') lines.push(`Hall of Fame entry ${BigInt('0x' + origin.entryId)}`, `Weights: ${origin.weightsSha256}`);
  if (origin?.kind === 'saved-brain') lines.push(`Saved brain · ${origin.sourceKind}`, `Artifact: ${origin.brainId}`, `Weights: ${origin.weightsSha256}`);
  return lines.join('\n');
}

/** Resolve native identities while retaining no world, bodies, or neural execution. */
export function createEntitySelectionClient(transport: EntitySelectionTransport): EntitySelectionClient {
  let scope: SelectionScope | null = null;
  let selected: SelectedEntity | null = null;
  let followedSlot: number | null = null;
  let ordinal = 0;
  let pending: { requestId: string; intent: SelectionIntent } | null = null;
  let queued: SelectionIntent | null = null;
  let lastFollow = Number.NEGATIVE_INFINITY;

  /** Discard physical references and every old response correlation. */
  const clear = (): void => {
    selected = null; followedSlot = null; pending = null; queued = null;
    lastFollow = Number.NEGATIVE_INFINITY;
    transport.changed(null);
  };
  /** Observe a scope replacement without silently reusing its old physical token. */
  const setScope = (next: SelectionScope): void => {
    if (!scope || scope.sessionId !== next.sessionId || scope.runId !== next.runId || scope.generation !== next.generation) clear();
    scope = { ...next };
  };
  /** Keep one admitted request and only the latest waiting intent. */
  const request = (intent: SelectionIntent): void => {
    if (!scope) return;
    if (pending) { queued = intent; return; }
    const requestId = 'selection-' + ++ordinal;
    pending = { requestId, intent };
    transport.send({ type: 'selectEntity', requestId, expectedSessionId: scope.sessionId,
      expectedRunId: scope.runId, expectedGeneration: scope.generation,
      ...(intent.kind === 'frame' ? { snakeId: intent.index } : { populationSlot: intent.index }) });
  };
  /** Retire the old physical selection before resolving a new explicit click. */
  const select = (snakeId: number): void => {
    selected = null; followedSlot = null; transport.changed(null);
    request({ action: 'select', kind: 'frame', index: snakeId });
  };
  /** Use a known physical token, otherwise resolve the right-click target first. */
  const kill = (snakeId: number): void => {
    if (selected?.snakeId === snakeId) transport.kill(snakeId, selected.token);
    else request({ action: 'kill', kind: 'frame', index: snakeId });
  };
  /** Ignore old/superseded replies and reject malformed or mismatched scalar identity. */
  const receive = (message: EntitySelectedMsg): void => {
    if (!pending || message.requestId !== pending.requestId) return;
    const intent = pending.intent; pending = null;
    if (queued) { const next = queued; queued = null; request(next); return; }
    const entity = message.entity;
    if (message.found === true && entity && validEntity(entity)
        && (intent.kind === 'frame' ? entity.snakeId === intent.index : entity.populationSlot === intent.index)) {
      if (intent.action === 'kill') { transport.kill(entity.snakeId, entity.token); return; }
      if (intent.action === 'validate' && entity.token !== intent.token) { clear(); return; }
      selected = entity; followedSlot = entity.populationSlot ?? null;
      transport.changed(entity);
    } else if (intent.action !== 'kill') {
      selected = null;
      if (intent.action === 'select' || intent.action === 'validate') followedSlot = null;
      transport.changed(null);
    }
  };
  /** Follow by persistent slot even if a fast replacement reuses the same frame ID. */
  const observeFrame = (generation: number, contains: (id: number) => boolean, now: number): void => {
    if (!scope) return;
    if (generation !== scope.generation) { setScope({ ...scope, generation }); return; }
    if (followedSlot !== null && transport.followIndividual()) {
      if (!pending && now - lastFollow >= FOLLOW_INTERVAL_MS) {
        lastFollow = now;
        request({ action: 'follow', kind: 'slot', index: followedSlot });
      }
    } else if (selected) {
      if (!contains(selected.snakeId)) clear();
      else if (!pending && now - lastFollow >= FOLLOW_INTERVAL_MS) {
        lastFollow = now;
        request({ action: 'validate', kind: 'frame', index: selected.snakeId, token: selected.token });
      }
    }
  };
  /** Keyboard-accessible selection also retains a temporarily pending individual. */
  const selectIndividual = (populationSlot: number): void => {
    if (!scope || !Number.isSafeInteger(populationSlot) || populationSlot < 0 || populationSlot >= 300) return;
    selected = null; followedSlot = populationSlot; transport.changed(null);
    request({action: 'follow', kind: 'slot', index: populationSlot});
  };
  /** Disconnect invalidates all observed world ownership. */
  const reset = (): void => { clear(); scope = null; };
  return { setScope, select, selectIndividual, kill, receive, observeFrame, followedPopulationSlot: () => followedSlot, clear, reset };
}

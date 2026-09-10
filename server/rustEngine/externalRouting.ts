import type { ActionMsg, ClientType, JoinMsg, ServerMessage } from '../protocol.ts';
import type { RustBackgroundControllerAction, RustBackgroundEvent, RustBackgroundIdentity, RustBackgroundReclaimRequest } from '../../src/protocol/rustBackground.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';
import type { BackgroundCommandAdmission } from './commandAdmission.ts';

/** Transport-only ownership tags for one socket; no snake state is stored here. */
interface Route {
  /** Numeric WsHub identity, exact within JavaScript's supported range. */
  connection: number;
  /** Native connection encoding. */
  id: RustBackgroundIdentity;
  /** Original join/reclaim scalars, retained only while its command is pending. */
  request: RustBackgroundReclaimRequest;
  /** Next bounded control to admit. */
  pending?: 'reclaim' | 'join' | 'close';
  /** Correlation for the admitted join/reclaim request. */
  requestSequence?: RustBackgroundIdentity;
  /** Lifecycle operation awaiting its Rust transport-receipt resolution. */
  lifecycleOperation?: 'join' | 'reclaim';
  /** Last Rust-issued lease, used only to route actions and closes. */
  lease?: RustBackgroundIdentity;
  /** Public ID from the assignment envelope, never used as a lease ID. */
  snakeId?: number;
  /** Latest unsent steering; new input replaces it under backpressure. */
  action?: RustBackgroundControllerAction;
  /** Monotonic receipt of the latest unsent steering value. */
  actionReceivedAt?: number;
  /** Monotonic start of the current join/reclaim lifecycle. */
  lifecycleStartedAt: number;
  /** Socket already closed; pending native results must still be consumed. */
  closed: boolean;
  /** Transport abuse limit window. */
  secondStart: number;
  /** Accepted input messages during this transport window. */
  actions: number;
  /** Last native completed-step boundary used for the RL transport limit. */
  actionStep?: string;
  /** RL requests accepted at that boundary. */
  stepActions: number;
}

/** Bounded routing configuration for existing Protocol 2 sockets. */
export interface ExternalRoutingOptions {
  /** Sole native command queue. */
  native: ExperimentalRunningAuthorityNativeHandle;
  /** Shared with the reliable output pump. */
  admission: BackgroundCommandAdmission;
  /** Maximum live or unresolved socket routes. */
  maxControllers: number;
  /** Transport flood bound; gameplay timing remains native. */
  maxActionsPerSecond: number;
  /** Maximum observation-driven RL actions per native boundary. */
  maxActionsPerTick: number;
  /** Existing reliable WebSocket send. */
  send(connection: number, message: ServerMessage): boolean;
  /** Observe accepted transport input through Rust application. */
  observeActionLatency?(kind: RustBackgroundReclaimRequest['controllerKind'], durationMs: number): void;
  /** Observe join/reclaim through a successful delivered assignment. */
  observeLifecycleLatency?(
    kind: RustBackgroundReclaimRequest['controllerKind'],
    operation: 'freshAssignment' | 'reclaim',
    durationMs: number
  ): void;
  /** Observe a disconnect only after Rust applies it to the current lease. */
  observeDisconnect?(kind: RustBackgroundReclaimRequest['controllerKind']): void;
}

/** Exact unsigned encoding for transport IDs and diagnostic ticks. */
function hex(value: number): RustBackgroundIdentity {
  if (!Number.isSafeInteger(value) || value < 0) throw new RangeError('inexact Protocol 2 identity');
  return BigInt(value).toString(16).padStart(16, '0');
}

/** Keep only bounded socket tags, unadmitted lifecycle commands and newest input. */
export class ExternalControllerRouting {
  /** One route per active or unresolved socket. */
  private readonly routes = new Map<number, Route>();
  /** Admitted action sequences awaiting their Rust application result. */
  private readonly pendingActions = new Map<RustBackgroundIdentity, {
    /** Monotonic transport receipt boundary. */
    startedAt: number;
    /** Distinguishes browser-player and observation-driven trainer latency. */
    kind: RustBackgroundReclaimRequest['controllerKind'];
  }>();
  /** Admitted disconnect identities awaiting their authoritative Rust result. */
  private readonly pendingDisconnects = new Map<
    RustBackgroundIdentity,
    RustBackgroundReclaimRequest['controllerKind']
  >();

  /** Bind the existing socket hub to native ownership commands. */
  constructor(private readonly options: ExternalRoutingOptions) {
    if (!Number.isSafeInteger(options.maxControllers) || options.maxControllers <= 0 ||
        !Number.isSafeInteger(options.maxActionsPerSecond) || options.maxActionsPerSecond <= 0 ||
        !Number.isSafeInteger(options.maxActionsPerTick) || options.maxActionsPerTick <= 0) {
      throw new RangeError('invalid external routing capacity');
    }
  }

  /** Request legacy or explicit-token reclaim before considering a fresh snake. */
  join(connection: number, message: JoinMsg, client: ClientType): void {
    if (message.mode !== 'player') { this.disconnect(connection); return; }
    if (this.routes.has(connection)) return;
    const name = message.name?.trim();
    if (!name) { this.options.send(connection, { type: 'error', message: 'name required for player mode' }); return; }
    if (this.routes.size >= this.options.maxControllers) {
      this.options.send(connection, { type: 'error', message: 'experimental controller capacity reached' }); return;
    }
    const id = hex(connection);
    if (connection === 0) throw new RangeError('connection identity must be positive');
    const request: RustBackgroundReclaimRequest = {
      connectionId: id, controllerKind: client === 'bot' ? 'reinforcementLearning' : 'player',
      identityKey: `${client === 'bot' ? 'bot' : 'player'}:${name}`,
      ...(message.resumeToken ? { resumeToken: message.resumeToken } : {})
    };
    const now = performance.now();
    this.routes.set(connection, { connection, id, request, pending: 'reclaim', closed: false,
      lifecycleStartedAt: now, secondStart: now, actions: 0, stepActions: 0 });
    this.flush();
  }

  /** Coalesce unsent input while retaining the exact Rust-issued routing tags. */
  action(connection: number, message: ActionMsg): void {
    const route = this.routes.get(connection);
    if (!route?.lease || route.closed || route.snakeId !== message.snakeId || !Number.isSafeInteger(message.tick) || message.tick < 0 ||
        !Number.isFinite(message.turn) || !Number.isFinite(message.boost)) return;
    const now = performance.now();
    if (now - route.secondStart >= 1000) { route.secondStart = now; route.actions = 0; }
    if (++route.actions > this.options.maxActionsPerSecond) return;
    const step = this.options.native.health().completedStep;
    if (route.actionStep !== step) { route.actionStep = step; route.stepActions = 0; }
    if (++route.stepActions > this.options.maxActionsPerTick && route.request.controllerKind === 'reinforcementLearning') return;
    route.action = { connectionId: route.id, leaseId: route.lease, clientTick: hex(message.tick), turn: Math.max(-1, Math.min(1, message.turn)), boost: message.boost > 0 };
    route.actionReceivedAt = now;
    this.flush();
  }

  /** Retain a close until Rust admits it, including closes racing assignment sends. */
  disconnect(connection: number): void {
    const route = this.routes.get(connection);
    if (!route) return;
    route.closed = true;
    delete route.action;
    if (route.lease) route.pending = 'close';
    else if (!route.requestSequence) { this.routes.delete(connection); return; }
    this.flush();
  }

  /** Update socket routing tags solely from native envelopes and correlated results. */
  event(event: RustBackgroundEvent): void {
    const assignment = event.controllerJoinAssignment ?? event.controllerReclaimAssignment;
    if (assignment) this.assign(assignment.connectionId, assignment.leaseId, assignment.snakeId);
    for (const message of event.controllerMessages ?? []) {
      if (message.kind === 'replacementAssignment') this.assign(message.connectionId, message.leaseId, message.snakeId);
    }
    if (event.kind === 'generationReassignmentsPrepared') {
      const result = event.reassignments as { assignments?: Array<{ connectionId: string; leaseId: string; frameV1Id: string }> } | undefined;
      for (const item of result?.assignments ?? []) this.assign(item.connectionId, item.leaseId, Number(BigInt(`0x${item.frameV1Id}`)));
    }
    const resolution = event.controllerJoinResolution ?? event.controllerReclaimResolution;
    const sequence = resolution?.requestSequence ?? event.commandSequence;
    let route: Route | undefined;
    if (sequence) for (const candidate of this.routes.values()) {
      if (candidate.requestSequence === sequence) { route = candidate; break; }
    }
    if (route && resolution?.matched) {
      delete route.requestSequence;
      const operation = route.lifecycleOperation;
      delete route.lifecycleOperation;
      if (!resolution.accepted) this.routes.delete(route.connection);
      else {
        if (!operation) throw new Error('successful controller lifecycle omitted its operation');
        this.options.observeLifecycleLatency?.(route.request.controllerKind,
          operation === 'join' ? 'freshAssignment' : 'reclaim',
          performance.now() - route.lifecycleStartedAt);
        if (route.closed) route.pending = 'close';
      }
    } else if (route && event.kind === 'commandRejected') {
      delete route.requestSequence;
      delete route.lifecycleOperation;
      if (!route.closed && !route.request.resumeToken && event.rejectionCode === 'InvalidCommand' &&
          event.rejectionDetail === 'InvalidCommand: no reserved legacy identity match') route.pending = 'join';
      else {
        this.routes.delete(route.connection);
        if (!route.closed) this.options.send(route.connection, { type: 'reclaimResult', reclaimed: false,
          reason: event.rejectionDetail?.includes('ambiguous') ? 'ambiguous' : 'invalid' });
      }
    }
    if (event.commandSequence) {
      const disconnectKind = this.pendingDisconnects.get(event.commandSequence);
      if (disconnectKind &&
          (event.kind === 'controllerDisconnected' || event.kind === 'commandRejected')) {
        this.pendingDisconnects.delete(event.commandSequence);
        if (event.kind === 'controllerDisconnected' && event.controllerDisconnect?.applied) {
          this.options.observeDisconnect?.(disconnectKind);
        }
      }
      const action = this.pendingActions.get(event.commandSequence);
      if (action &&
          (event.kind === 'controllerActionApplied' || event.kind === 'commandRejected')) {
        this.pendingActions.delete(event.commandSequence);
        if (event.kind === 'controllerActionApplied') {
          this.options.observeActionLatency?.(action.kind, performance.now() - action.startedAt);
        }
      }
    }
    this.flush();
  }

  /** Store native routing tags before the transport sends their assignment. */
  private assign(connectionId: string, leaseId: string, snakeId: number): void {
    const route = this.routes.get(Number(BigInt(`0x${connectionId}`)));
    if (!route) return;
    route.lease = leaseId;
    route.snakeId = snakeId;
    delete route.action;
  }

  /** Admit lifecycle controls first, then at most one newest action per socket. */
  flush(): void {
    const { native, admission } = this.options;
    for (const route of this.routes.values()) {
      const pending = route.pending;
      if (!pending) continue;
      if (!admission.trySubmitControl(sequence => {
        if (pending === 'close') {
          native.submitControllerDisconnect(sequence, { connectionId: route.id, leaseId: route.lease! });
          this.pendingDisconnects.set(sequence, route.request.controllerKind);
        }
        else if (pending === 'join') native.submitControllerJoin(sequence, { ...route.request, identityKey: route.request.identityKey! });
        else native.submitControllerReclaim(sequence, route.request);
        if (pending !== 'close') {
          route.requestSequence = sequence;
          route.lifecycleOperation = pending;
        }
      })) return;
      delete route.pending;
      if (pending === 'close') this.routes.delete(route.connection);
    }
    for (const route of this.routes.values()) {
      const action = route.action;
      if (!action || route.closed) continue;
      if (this.pendingActions.size >= this.options.maxControllers * 4) return;
      const receivedAt = route.actionReceivedAt ?? performance.now();
      if (!admission.trySubmitControl(sequence => {
        native.submitControllerAction(sequence, action);
        this.pendingActions.set(sequence, { startedAt: receivedAt, kind: route.request.controllerKind });
      })) return;
      delete route.action;
      delete route.actionReceivedAt;
    }
  }
}

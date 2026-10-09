/** Correlate the actual Protocol 2 result/assignment pair for one reconnect. */
import { createHash } from 'node:crypto';

/** Compact retained lifecycle packet; ownership tokens are fingerprinted. */
export interface LifecyclePacket {
  /** Actual server packet discriminator. */
  type: 'assign' | 'reclaimResult';
  /** Snake identity advertised by this packet, when present. */
  snakeId?: number;
  /** Actual server reclaim flag. */
  reclaimed: boolean;
  /** Explicit result reason for a reconnect response. */
  reason?: string;
  /** Fingerprint of the assignment token, never the reusable token itself. */
  tokenHash?: string;
  /** Whether the assignment token differs from the token sent on this connection. */
  tokenChanged?: boolean;
  /** Client receipt boundary relative to the measured window, when supplied. */
  wallSeconds?: number;
  /** Latest generation advertised to this client at receipt time. */
  generation?: number;
  /** Latest observation or stats tick advertised at receipt time. */
  tick?: number;
}

/** Exact evidence for the initial request and any explicitly requested fresh join. */
export interface ReconnectExchangeRecord {
  /** Public identity held before reconnect, when one existed. */
  requestedSnakeId?: number;
  /** Fingerprint of the exact token sent in the initial join. */
  requestedTokenHash?: string;
  /** Chronological actual server results and assignments during this exchange. */
  packets: LifecyclePacket[];
  /** Whether a failed token reply caused an explicit token-free join. */
  freshJoinRequested: boolean;
  /** Completed, correlated operation; a fresh fallback is never counted as same-snake reclaim. */
  outcome?: 'fresh' | 'sameSnakeReclaim' | 'freshAfterRejectedReclaim' | 'legacyReclaim';
  /** Exact failed invariant, retained with the packets that caused it. */
  failure?: string;
}

/** Fingerprint an opaque token for correlation without retaining its ownership capability. */
function fingerprint(token: string): string {
  return createHash('sha256').update(token).digest('hex');
}

/** One bounded exchange with an immutable initial reply and assignment. */
export class PlayerReconnectExchange {
  /** Durable correlation record populated before any assertion can fail. */
  readonly record: ReconnectExchangeRecord;
  /** Result of the initial token request, preserved separately from a fallback reply. */
  private initialReply: LifecyclePacket | undefined;
  /** Optional legacy result from the explicitly requested token-free join. */
  private freshReply: LifecyclePacket | undefined;
  /** First assignment delivered for the operation currently awaiting completion. */
  private assignment: LifecyclePacket | undefined;

  /** Require either a complete prior identity or an ordinary fresh join. */
  constructor(requestedSnakeId?: number, requestedToken?: string) {
    if ((requestedSnakeId === undefined) !== (requestedToken === undefined) ||
        (requestedSnakeId !== undefined && (!Number.isSafeInteger(requestedSnakeId) || requestedSnakeId <= 0)) ||
        requestedToken === '') throw new Error('reconnect requires a complete prior identity');
    this.record = { packets: [], freshJoinRequested: false,
      ...(requestedSnakeId === undefined ? {} : { requestedSnakeId }),
      ...(requestedToken === undefined ? {} : { requestedTokenHash: fingerprint(requestedToken) }) };
  }

  /** Completion requires the exact correlated result and assignment, never a later replacement. */
  get ready(): boolean { return this.record.outcome !== undefined; }

  /** Retain an invariant failure before surfacing it to the bounded socket wait. */
  private reject(reason: string): never {
    this.record.failure = reason;
    throw new Error(reason);
  }

  /** Consume one actual lifecycle packet; true asks the caller to send one explicit fresh join. */
  consume(message: Record<string, unknown>, observation?: { wallSeconds: number; generation: number; tick: number }): boolean {
    if (message['type'] !== 'assign' && message['type'] !== 'reclaimResult') return false;
    if (this.record.failure) throw new Error(this.record.failure);
    if (this.ready) {
      if (message['type'] === 'reclaimResult') this.reject('unsolicited reclaim result after completed exchange');
      return false;
    }
    if (this.record.packets.length >= 8) this.reject('too many lifecycle packets in one exchange');
    const snakeId = message['snakeId'];
    if (snakeId !== undefined && (!Number.isSafeInteger(snakeId) || Number(snakeId) <= 0)) {
      this.reject('lifecycle packet has an invalid snake identity');
    }
    let packet: LifecyclePacket;
    if (message['type'] === 'assign') {
      const token = message['resumeToken'];
      if (snakeId === undefined || typeof token !== 'string' || token.length === 0 ||
          (message['controller'] !== 'player' && message['controller'] !== 'bot')) {
        this.reject('assignment omitted a valid identity or token');
      }
      const tokenHash = fingerprint(token);
      packet = { type: 'assign', snakeId: Number(snakeId), reclaimed: message['reclaimed'] === true,
        tokenHash, tokenChanged: tokenHash !== this.record.requestedTokenHash };
      this.record.packets.push(Object.assign(packet, observation));
      if (this.assignment) this.reject('multiple initial assignments before lifecycle completion');
      this.assignment = packet;
    } else {
      if (typeof message['reclaimed'] !== 'boolean' || typeof message['reason'] !== 'string') {
        this.reject('reclaim result omitted its explicit outcome');
      }
      packet = { type: 'reclaimResult', reclaimed: message['reclaimed'], reason: message['reason'],
        ...(snakeId === undefined ? {} : { snakeId: Number(snakeId) }) };
      this.record.packets.push(Object.assign(packet, observation));
      if (this.record.freshJoinRequested) {
        if (this.freshReply) this.reject('duplicate result for explicitly requested fresh join');
        if (!packet.reclaimed) this.reject('token-free fallback was explicitly rejected');
        this.freshReply = packet;
      } else {
        if (this.initialReply) this.reject('duplicate initial reclaim result');
        this.initialReply = packet;
        if (!packet.reclaimed) {
          if (this.record.requestedTokenHash === undefined) this.reject('fresh join was explicitly rejected');
          if (this.assignment) this.reject('assignment preceded rejected reclaim result');
          this.record.freshJoinRequested = true;
          return true;
        }
      }
    }
    this.completeIfCorrelated();
    return false;
  }

  /** Check same-snake identity and rotation against the original request, not mutable client state. */
  private completeIfCorrelated(): void {
    const assignment = this.assignment;
    if (!assignment) return;
    if (this.record.freshJoinRequested) {
      if (assignment.reclaimed && !this.freshReply) return;
      if (this.freshReply && this.freshReply.snakeId !== assignment.snakeId) {
        this.reject('fallback result and assignment identify different snakes');
      }
      if (!assignment.tokenChanged) this.reject('fresh fallback reused rejected token');
      this.record.outcome = 'freshAfterRejectedReclaim';
      return;
    }
    if (this.record.requestedTokenHash === undefined && !assignment.reclaimed) {
      if (this.initialReply) this.reject('ordinary fresh assignment had an unexpected reclaim reply');
      this.record.outcome = 'fresh';
      return;
    }
    const reply = this.initialReply;
    if (!reply) return;
    if (!reply.reclaimed || !assignment.reclaimed || reply.reason !== 'reclaimed') {
      this.reject('successful reclaim result and assignment disagree');
    }
    if (reply.snakeId !== assignment.snakeId) this.reject('reclaim result and assignment identify different snakes');
    if (!assignment.tokenChanged) this.reject('reclaim did not rotate the requested token');
    if (this.record.requestedTokenHash !== undefined && assignment.snakeId !== this.record.requestedSnakeId) {
      this.reject('successful reclaim changed the requested snake');
    }
    this.record.outcome = this.record.requestedTokenHash === undefined ? 'legacyReclaim' : 'sameSnakeReclaim';
  }
}

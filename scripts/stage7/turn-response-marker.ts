/** Correlate a client steering reversal with an observed authoritative heading change. */

/** One delivered Protocol 2 observation, stamped on the observing client's clock. */
export interface HeadingObservation {
  /** Assigned public snake identity; replacement cannot finish the previous trial. */
  snakeId: number;
  /** Completed observation boundary advertised by Rust. */
  tick: number;
  /** Heading reconstructed from the v3 sine/cosine pair. */
  direction: number;
  /** Monotonic client receipt time in milliseconds. */
  receivedAtMs: number;
}

/** Signed angular motion, excluding ambiguous large gaps between observations. */
export function observedTurn(from: number, to: number): -1 | 0 | 1 {
  if (!Number.isFinite(from) || !Number.isFinite(to)) throw new Error('invalid observed heading');
  const delta = Math.atan2(Math.sin(to - from), Math.cos(to - from));
  if (Math.abs(delta) < 0.001 || Math.abs(delta) > Math.PI / 2) return 0;
  return delta > 0 ? 1 : -1;
}

/** Keep every attempted command in the percentile; unknown responses rank beyond every finite bound. */
export function summarizeTurnResponseBounds(trials: readonly { latencyUpperBoundMs?: number }[], requestedCount: number): {
  completedTrials: number; unknownTrials: number; p95Ms: number | undefined; maximumKnownMs: number | undefined;
  meetsP95Gate: boolean;
} {
  if (!Number.isSafeInteger(requestedCount) || requestedCount < 1 || trials.some(trial =>
    trial.latencyUpperBoundMs !== undefined && (!Number.isFinite(trial.latencyUpperBoundMs) || trial.latencyUpperBoundMs < 0))) {
    throw new Error('invalid turn latency summary');
  }
  const ranked = trials.map(trial => trial.latencyUpperBoundMs ?? Infinity).sort((left, right) => left - right);
  const finite = ranked.filter(Number.isFinite);
  const percentile = ranked[Math.ceil(ranked.length * 0.95) - 1];
  const p95Ms = percentile !== undefined && Number.isFinite(percentile) ? percentile : undefined;
  return { completedTrials: finite.length, unknownTrials: ranked.length - finite.length, p95Ms,
    maximumKnownMs: finite.at(-1), meetsP95Gate: trials.length === requestedCount && p95Ms !== undefined && p95Ms < 100 };
}

/** One reversal after the caller has observed motion under the opposite held command. */
export class TurnResponseMarker {
  /** Latest increasing observation from this exact assigned snake. */
  private previous: HeadingObservation;
  /** Client timestamp immediately before its actual WebSocket send. */
  private readonly sentAtMs: number;
  /** Reversal sign, opposite the previously established held turn. */
  private readonly requestedTurn: -1 | 1;
  /** Prevent a later observation from being counted as another action response. */
  private finished = false;

  /** Establish the pre-send observation and require an explicit opposite turn. */
  constructor(baseline: HeadingObservation, sentAtMs: number, priorTurn: -1 | 1, requestedTurn: -1 | 1) {
    if (requestedTurn !== -priorTurn || !Number.isFinite(sentAtMs) || sentAtMs < baseline.receivedAtMs ||
        !Number.isSafeInteger(baseline.snakeId) || !Number.isSafeInteger(baseline.tick) || baseline.tick < 0 ||
        !Number.isFinite(baseline.direction) || !Number.isFinite(baseline.receivedAtMs)) {
      throw new Error('turn marker requires a valid pre-send boundary and reversal');
    }
    this.previous = baseline;
    this.sentAtMs = sentAtMs;
    this.requestedTurn = requestedTurn;
  }

  /** Return an upper latency bound only after increasing same-snake observations reverse. */
  observe(sample: HeadingObservation): { latencyUpperBoundMs: number; observedTick: number } | undefined {
    if (this.finished) return undefined;
    if (sample.snakeId !== this.previous.snakeId) throw new Error('assignment changed during turn trial');
    if (!Number.isSafeInteger(sample.tick) || !Number.isFinite(sample.receivedAtMs) ||
        !Number.isFinite(sample.direction)) throw new Error('invalid turn observation');
    if (sample.tick <= this.previous.tick || sample.receivedAtMs < this.sentAtMs ||
        sample.receivedAtMs < this.previous.receivedAtMs) return undefined;
    const turn = observedTurn(this.previous.direction, sample.direction);
    this.previous = sample;
    if (turn !== this.requestedTurn) return undefined;
    this.finished = true;
    return { latencyUpperBoundMs: sample.receivedAtMs - this.sentAtMs, observedTick: sample.tick };
  }
}

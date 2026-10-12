/** Generation-scoped manual completion request from a joined UI. */
export interface FinishGenerationMsg {
  /** Message discriminator. */
  type: 'finishGeneration';
  /** Idempotent generation-scoped correlation. */
  requestId: string;
  /** Active run expected by the requester. */
  expectedRunId: string;
  /** Active generation expected by the requester. */
  expectedGeneration: number;
}

/** Manual completion admission or durable publication result. */
export interface FinishGenerationResultMsg {
  /** Message discriminator. */
  type: 'finishGenerationResult';
  /** Original request correlation. */
  requestId: string;
  /** Run to which the request belongs. */
  runId: string;
  /** Generation to which the request belongs. */
  generation: number;
  /** Pending until checkpoint, metadata and successor publication complete. */
  status: 'pending' | 'completed' | 'rejected';
  /** First eligible or completed authoritative step. */
  step?: number;
  /** Rejection detail. */
  reason?: string;
}

/** Bytes in one binary gibibyte; all admitted spool sizes remain safe integers. */
export const GIB_BYTES = 1024 ** 3;
/** Initial encoded dataset-disk quota, independent of native scratch and replay RAM. */
export const DEFAULT_DISTILLATION_SPOOL_GIB = 4;
/** Largest explicitly selected disk quota supported by the storage protocol. */
export const MAX_DISTILLATION_SPOOL_GIB = 64;

/** Require an exact disk quota before configuration or diagnostic storage is opened. */
export function validateDistillationSpoolGiB(value: unknown): number {
  if (value === undefined) return DEFAULT_DISTILLATION_SPOOL_GIB;
  const parsed = typeof value === 'number' ? value :
    typeof value === 'string' && /^[1-9][0-9]*$/u.test(value) ? Number(value) : Number.NaN;
  if (!Number.isSafeInteger(parsed) || parsed < 1 || parsed > MAX_DISTILLATION_SPOOL_GIB) {
    throw new RangeError(`distillationSpoolGiB must be an integer from 1 to ${MAX_DISTILLATION_SPOOL_GIB}`);
  }
  return parsed;
}

/** Flat spatial hash grid used by collision detection and body sensing. */

/** Estimated bytes used by one segment entry across typed arrays and one object reference. */
const ESTIMATED_BYTES_PER_ENTRY = 16;
/** Temporary TypeScript-reference ceiling; the Rust migration will replace this implementation. */
const DEFAULT_MAX_CAPACITY = 32 * 1024 * 1024;

/** Operational collision-grid measurements that never affect simulation decisions. */
export interface SpatialHashDiagnostics {
  /** Number of entries currently stored. */
  currentEntries: number;
  /** Largest completed entry count observed since construction. */
  peakEntries: number;
  /** Number of entries available without another allocation. */
  capacity: number;
  /** Configured hard admission ceiling. */
  maxCapacity: number;
  /** Estimated bytes represented by the current capacity. */
  estimatedCapacityBytes: number;
  /** Number of complete grid rebuilds. */
  rebuilds: number;
  /** Number of successful capacity increases. */
  growths: number;
  /** Number of entries rejected because their coordinates were outside the grid. */
  outOfBoundsEntries: number;
  /** Last successful admission or growth reason. */
  admissionReason: string;
  /** Last capacity/allocation failure, or null when none has occurred. */
  faultReason: string | null;
}

/** Spatial hash grid using typed arrays to avoid per-query allocation. */
export class FlatSpatialHash<T extends { alive: boolean; points: Array<{ x: number; y: number }> }> {
  /** Approximate world width covered by the grid. */
  private readonly width: number;
  /** Approximate world height covered by the grid. */
  private readonly height: number;
  /** Size of each spatial hash cell in world units. */
  cellSize: number;
  /** Column count in the grid. */
  cols: number;
  /** Row count in the grid. */
  rows: number;
  /** Half column count used for coordinate offsetting. */
  halfCols: number;
  /** Half row count used for coordinate offsetting. */
  halfRows: number;
  /** Head indices for linked lists stored per cell. */
  head: Int32Array;
  /** Next pointers for linked list nodes. */
  next: Int32Array;
  /** Segment indices corresponding to each node. */
  indices: Int32Array;
  /** Object references corresponding to each node. */
  objects: Array<T | undefined>;
  /** Current number of stored nodes. */
  count: number;
  /** Maximum number of nodes available without another allocation. */
  capacity: number;
  /** Hard admission ceiling used to turn unsafe configurations into clear faults. */
  readonly maxCapacity: number;
  /** Largest completed entry count observed since construction. */
  private peakCount: number;
  /** Number of complete rebuilds. */
  private rebuildCount: number;
  /** Number of successful capacity increases. */
  private growthCount: number;
  /** Number of out-of-grid entries encountered since construction. */
  private outOfBoundsCount: number;
  /** Last successful admission or growth reason. */
  private lastAdmissionReason: string;
  /** Last capacity/allocation failure. */
  private lastFaultReason: string | null;

  /**
   * @param width - Approximate world width.
   * @param height - Approximate world height.
   * @param cellSize - Spatial hash cell size.
   * @param capacity - Initial number of segment entries.
   * @param maxCapacity - Hard segment-entry ceiling for this temporary implementation.
   */
  constructor(
    width: number,
    height: number,
    cellSize: number,
    capacity: number,
    maxCapacity = DEFAULT_MAX_CAPACITY
  ) {
    if (!Number.isFinite(width) || width <= 0 || !Number.isFinite(height) || height <= 0) {
      throw new RangeError(`collision grid dimensions must be positive; received ${width}x${height}`);
    }
    if (!Number.isFinite(cellSize) || cellSize <= 0) {
      throw new RangeError(`collision grid cell size must be positive; received ${cellSize}`);
    }
    const initialCapacity = Math.floor(capacity);
    const normalizedMaximum = Math.floor(maxCapacity);
    if (initialCapacity < 1 || normalizedMaximum < initialCapacity) {
      throw new RangeError(
        `collision grid capacity range is invalid; initial=${capacity}, maximum=${maxCapacity}`
      );
    }

    this.width = width;
    this.height = height;
    this.cellSize = cellSize;
    this.cols = Math.ceil(width / cellSize);
    this.rows = Math.ceil(height / cellSize);
    this.halfCols = Math.floor(this.cols / 2);
    this.halfRows = Math.floor(this.rows / 2);
    this.head = new Int32Array(this.cols * this.rows);
    this.head.fill(-1);
    this.next = new Int32Array(initialCapacity);
    this.indices = new Int32Array(initialCapacity);
    this.objects = new Array<T | undefined>(initialCapacity);
    this.count = 0;
    this.capacity = initialCapacity;
    this.maxCapacity = normalizedMaximum;
    this.peakCount = 0;
    this.rebuildCount = 0;
    this.growthCount = 0;
    this.outOfBoundsCount = 0;
    this.lastAdmissionReason = `initial capacity ${initialCapacity} entries`;
    this.lastFaultReason = null;
  }

  /**
   * Return a stable snapshot of operational collision-grid diagnostics.
   * @returns Current capacity, load, rebuild, growth, and fault measurements.
   */
  getDiagnostics(): SpatialHashDiagnostics {
    return {
      currentEntries: this.count,
      peakEntries: this.peakCount,
      capacity: this.capacity,
      maxCapacity: this.maxCapacity,
      estimatedCapacityBytes: this.capacity * ESTIMATED_BYTES_PER_ENTRY,
      rebuilds: this.rebuildCount,
      growths: this.growthCount,
      outOfBoundsEntries: this.outOfBoundsCount,
      admissionReason: this.lastAdmissionReason,
      faultReason: this.lastFaultReason
    };
  }

  /**
   * Ensure a complete rebuild can fit before clearing the current index.
   * @param required - Exact or conservative number of entries required.
   * @param reason - Human-readable operation requesting admission.
   */
  ensureCapacity(required: number, reason = 'collision-grid rebuild'): void {
    if (!Number.isSafeInteger(required) || required < 0) {
      this.failAdmission(`${reason} requested invalid entry count ${required}`);
    }
    if (required > this.maxCapacity) {
      this.failAdmission(
        `${reason} requires ${required} entries, exceeding the configured maximum ${this.maxCapacity}`
      );
    }
    if (required <= this.capacity) return;

    const grown = Math.ceil(this.capacity * 1.5);
    const nextCapacity = Math.min(this.maxCapacity, Math.max(required, grown));
    try {
      const next = new Int32Array(nextCapacity);
      const indices = new Int32Array(nextCapacity);
      const objects = new Array<T | undefined>(nextCapacity);
      next.set(this.next.subarray(0, this.count));
      indices.set(this.indices.subarray(0, this.count));
      for (let index = 0; index < this.count; index++) objects[index] = this.objects[index];
      this.next = next;
      this.indices = indices;
      this.objects = objects;
      const previous = this.capacity;
      this.capacity = nextCapacity;
      this.growthCount++;
      this.lastAdmissionReason = `${reason} grew capacity from ${previous} to ${nextCapacity} entries`;
      this.lastFaultReason = null;
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error);
      this.failAdmission(
        `${reason} could not allocate ${nextCapacity} entries (estimated ${
          nextCapacity * ESTIMATED_BYTES_PER_ENTRY
        } bytes): ${detail}`
      );
    }
  }

  /**
   * Reset the grid and optionally update the cell size.
   * @param cellSize - Optional new cell size.
   */
  reset(cellSize = this.cellSize): void {
    if (!Number.isFinite(cellSize) || cellSize <= 0) {
      throw new RangeError(`collision grid cell size must be positive; received ${cellSize}`);
    }
    if (cellSize !== this.cellSize) {
      const cols = Math.ceil(this.width / cellSize);
      const rows = Math.ceil(this.height / cellSize);
      const nextHead = new Int32Array(cols * rows);
      nextHead.fill(-1);
      this.cellSize = cellSize;
      this.cols = cols;
      this.rows = rows;
      this.halfCols = Math.floor(cols / 2);
      this.halfRows = Math.floor(rows / 2);
      this.head = nextHead;
    } else {
      this.head.fill(-1);
    }
    for (let index = 0; index < this.count; index++) this.objects[index] = undefined;
    this.count = 0;
  }

  /**
   * Populate the grid with segments from all alive snakes.
   * Capacity admission completes before the prior grid is cleared.
   * @param snakes - Snakes to insert into the grid.
   * @param skipSegments - Number of head segments to skip.
   * @param cellSize - Cell size for this complete rebuild.
   */
  build(snakes: T[], skipSegments = 0, cellSize = this.cellSize): void {
    const skip = Math.max(0, Math.floor(skipSegments));
    let required = 0;
    for (const snake of snakes) {
      if (snake.alive) required += Math.max(0, snake.points.length - Math.max(1, skip));
    }
    const growthsBeforeAdmission = this.growthCount;
    this.ensureCapacity(required, 'collision-grid rebuild');
    this.reset(cellSize);
    for (const snake of snakes) {
      if (!snake.alive) continue;
      const points = snake.points;
      for (let index = Math.max(1, skip); index < points.length; index++) {
        const previous = points[index - 1];
        const current = points[index];
        if (!previous || !current) continue;
        this.add(
          (previous.x + current.x) * 0.5,
          (previous.y + current.y) * 0.5,
          snake,
          index
        );
      }
    }
    this.rebuildCount++;
    this.peakCount = Math.max(this.peakCount, this.count);
    this.lastFaultReason = null;
    if (this.growthCount === growthsBeforeAdmission) {
      this.lastAdmissionReason = `collision-grid rebuild admitted ${required} entries`;
    }
  }

  /**
   * Add a segment midpoint to an already admitted spatial-hash rebuild.
   * @param x - Segment midpoint x coordinate.
   * @param y - Segment midpoint y coordinate.
   * @param snake - Snake reference for the segment.
   * @param segIdx - Segment index on the snake.
   */
  add(x: number, y: number, snake: T, segIdx: number): void {
    const cx = Math.floor(x / this.cellSize) + this.halfCols;
    const cy = Math.floor(y / this.cellSize) + this.halfRows;
    if (cx < 0 || cx >= this.cols || cy < 0 || cy >= this.rows) {
      this.outOfBoundsCount++;
      return;
    }
    if (this.count >= this.capacity) {
      this.failAdmission(
        `collision-grid insertion exceeded admitted capacity ${this.capacity}; rebuild was not preflighted`
      );
    }

    const cellIndex = cy * this.cols + cx;
    const index = this.count++;
    this.objects[index] = snake;
    this.indices[index] = segIdx;
    this.next[index] = this.head[cellIndex] ?? -1;
    this.head[cellIndex] = index;
    this.peakCount = Math.max(this.peakCount, this.count);
  }

  /**
   * Query the cell containing a world-space position.
   * @param x - World X coordinate.
   * @param y - World Y coordinate.
   * @param callback - Callback invoked with each segment in the cell.
   */
  query(x: number, y: number, callback: (snake: T, segIdx: number) => void): void {
    const cx = Math.floor(x / this.cellSize) + this.halfCols;
    const cy = Math.floor(y / this.cellSize) + this.halfRows;
    this.queryIndexedCell(cx, cy, callback);
  }

  /**
   * Query by raw cell coordinates (integers relative to the world origin).
   * @param rawCx - Cell X coordinate relative to origin.
   * @param rawCy - Cell Y coordinate relative to origin.
   * @param callback - Callback invoked with each segment in the cell.
   */
  queryCell(rawCx: number, rawCy: number, callback: (snake: T, segIdx: number) => void): void {
    this.queryIndexedCell(rawCx + this.halfCols, rawCy + this.halfRows, callback);
  }

  /**
   * Walk one already-normalized grid cell.
   * @param cx - Zero-based grid column.
   * @param cy - Zero-based grid row.
   * @param callback - Callback invoked for every valid linked-list entry.
   */
  private queryIndexedCell(
    cx: number,
    cy: number,
    callback: (snake: T, segIdx: number) => void
  ): void {
    if (cx < 0 || cx >= this.cols || cy < 0 || cy >= this.rows) return;
    const cellIndex = cy * this.cols + cx;
    let index = this.head[cellIndex] ?? -1;
    const max = this.count;
    let steps = 0;
    while (index !== -1 && index >= 0 && index < max && steps < max) {
      const object = this.objects[index];
      const segmentIndex = this.indices[index];
      if (object !== undefined && segmentIndex !== undefined) callback(object, segmentIndex);
      index = this.next[index] ?? -1;
      steps++;
    }
  }

  /**
   * Record and throw one clear admission failure.
   * @param reason - Human-readable failure reason surfaced through diagnostics.
   */
  private failAdmission(reason: string): never {
    this.lastFaultReason = reason;
    throw new Error(reason);
  }
}

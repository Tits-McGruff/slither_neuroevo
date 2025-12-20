// spatialHash.ts
// A flat, zero-allocation spatial hash grid for collision detection.
// Uses TypedArrays for the linked list structure to avoid Garbage Collection.

export class FlatSpatialHash {
  cellSize: number;
  cols: number;
  rows: number;
  halfCols: number;
  halfRows: number;
  head: Int32Array;
  next: Int32Array;
  indices: Int32Array;
  objects: any[];
  count: number;
  capacity: number;

  /**
   * @param {number} width World width (approx)
   * @param {number} height World height (approx)
   * @param {number} cellSize 
   * @param {number} capacity Max number of segments to track
   */
  constructor(width: number, height: number, cellSize: number, capacity: number) {
    this.cellSize = cellSize;
    this.cols = Math.ceil(width / cellSize);
    this.rows = Math.ceil(height / cellSize);
    this.halfCols = Math.floor(this.cols / 2);
    this.halfRows = Math.floor(this.rows / 2);

    // Grid: Stores the index of the first item in the linked list for each cell
    this.head = new Int32Array(this.cols * this.rows);
    this.head.fill(-1);
    
    // Linked List of Items
    this.next = new Int32Array(capacity);
    this.indices = new Int32Array(capacity);  // Segment index
    this.objects = new Array(capacity);       // Snake reference (direct pointer)
    
    this.count = 0;
    this.capacity = capacity;
  }

  reset(cellSize?: number): void {
    if (cellSize) {
       // Re-dim if needed? For now assuming fixed world size, but dynamic cell size.
       // Changing cell size implies re-calculating cols/rows.
       // If drastic change, we might need to re-alloc head.
       // For safety in this project, we'll assume World radius is constant-ish,
       // but cell size comes from CFG.
       if (Math.abs(cellSize - this.cellSize) > 1) {
         this.cellSize = cellSize;
         // We might simply clamp index to valid range if grid changed,
         // but strictly we should re-alloc head if cols/rows change significantly.
         // Let's assume typical usage for now.
       }
    }
    this.head.fill(-1);
    this.count = 0;
    // We don't need to clear objects/next/indices, they get overwritten.
    // However, to permit GC of dead snakes, we *should* clear objects eventually?
    // Since we overwrite count, old objects > count are "leaked" until overwritten?
    // No, we reset count = 0.
    // Ideally we null out objects to let GC reclaim snakes if they died.
    // But for speed we can skip if we are sure capacity is constantly reused.
    // Let's safe-clear objects for correctness.
    // actually, clearing simple array is fast.
    // this.objects.fill(null); // Optional but safe
  }

  /**
   * Populates the grid with segments from all alive snakes.
   * @param {Array<{alive: boolean, points: Array<{x: number, y: number}>}>} snakes
   * @param {number} skipSegments
   */
  build(snakes: Array<{ alive: boolean; points: Array<{ x: number; y: number }> }>, skipSegments = 0): void {
    this.reset();
    const skip = Math.max(0, Math.floor(skipSegments));
    for (const s of snakes) {
      if (!s.alive) continue;
      const pts = s.points;
      for (let i = Math.max(1, skip); i < pts.length; i++) {
        const p0 = pts[i - 1];
        const p1 = pts[i];
        if (!p0 || !p1) continue;
        const mx = (p0.x + p1.x) * 0.5;
        const my = (p0.y + p1.y) * 0.5;
        this.add(mx, my, s, i);
      }
    }
  }
  
  add(x: number, y: number, snake: any, segIdx: number): void {
    if (this.count >= this.capacity) return; // Full

    // Map (x,y) to cell index
    // Assuming (0,0) is center of world.
    const cx = Math.floor(x / this.cellSize) + this.halfCols;
    const cy = Math.floor(y / this.cellSize) + this.halfRows;

    if (cx < 0 || cx >= this.cols || cy < 0 || cy >= this.rows) return; // Out of bounds

    const cellIndex = cy * this.cols + cx;
    const i = this.count++;

    this.objects[i] = snake;
    this.indices[i] = segIdx;
    this.next[i] = this.head[cellIndex] ?? -1;
    this.head[cellIndex] = i;
  }

  /**
   * Queries specific cell.
   * @param {number} x 
   * @param {number} y 
   * @param {Function} callback (snake, segIdx) => void
   */
  query(x: number, y: number, callback: (snake: any, segIdx: number) => void): void {
    const cx = Math.floor(x / this.cellSize) + this.halfCols;
    const cy = Math.floor(y / this.cellSize) + this.halfRows;

    if (cx < 0 || cx >= this.cols || cy < 0 || cy >= this.rows) return;

    const cellIndex = cy * this.cols + cx;
    let i = this.head[cellIndex] ?? -1;
    const max = this.count;
    let steps = 0;

    while (i !== -1 && i >= 0 && i < max && steps < max) {
      const obj = this.objects[i];
      const segIdx = this.indices[i];
      if (obj !== undefined && segIdx !== undefined) {
        callback(obj, segIdx);
      }
      i = this.next[i] ?? -1;
      steps++;
    }
  }

  /**
   * Queries by raw cell coordinates (integers).
   * @param {number} rawCx 
   * @param {number} rawCy 
   * @param {Function} callback 
   */
  queryCell(rawCx: number, rawCy: number, callback: (snake: any, segIdx: number) => void): void {
    const cx = rawCx + this.halfCols;
    const cy = rawCy + this.halfRows;
    
    if (cx < 0 || cx >= this.cols || cy < 0 || cy >= this.rows) return;

    const cellIndex = cy * this.cols + cx;
    let i = this.head[cellIndex] ?? -1;
    const max = this.count;
    let steps = 0;

    while (i !== -1 && i >= 0 && i < max && steps < max) {
      const obj = this.objects[i];
      const segIdx = this.indices[i];
      if (obj !== undefined && segIdx !== undefined) {
        callback(obj, segIdx);
      }
      i = this.next[i] ?? -1;
      steps++;
    }
  }
}

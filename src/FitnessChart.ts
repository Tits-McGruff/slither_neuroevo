/** Renders a fitness history chart. */

/** Simple chart renderer for fitness history. */
export class FitnessChart {
  /** Left position of the chart region. */
  x: number;
  /** Top position of the chart region. */
  y: number;
  /** Width of the chart region. */
  w: number;
  /** Height of the chart region. */
  h: number;

  /**
   * Create a chart renderer with a fixed drawing rectangle.
   * @param x - Left position of the chart.
   * @param y - Top position of the chart.
   * @param w - Chart width in pixels.
   * @param h - Chart height in pixels.
   */
  constructor(x: number, y: number, w: number, h: number) {
    this.x = x;
    this.y = y;
    this.w = w;
    this.h = h;
  }

  /**
   * Render the fitness chart into the given canvas context.
   * @param ctx - Canvas 2D context to draw into.
   * @param history - Fitness history records to plot.
   */
  render(
    ctx: CanvasRenderingContext2D,
    history: Array<{ gen: number; best: number; avg: number }>
  ): void {
    if (!history || history.length < 2) return;
    
    const maxGen = history[history.length - 1]!.gen;
    const minGen = history[0]!.gen;
    let maxFit = 0;
    for (const h of history) maxFit = Math.max(maxFit, h.best);
    if (maxFit <= 0) maxFit = 1;
    
    ctx.save();
    ctx.translate(this.x, this.y);
    
    // Background
    ctx.fillStyle = 'rgba(0,0,0,0.3)';
    ctx.fillRect(0, 0, this.w, this.h);
    
    // Draw Best
    ctx.strokeStyle = '#4f4';
    ctx.lineWidth = 2;
    ctx.beginPath();
    let idx = 0;
    for (const h of history) {
      const px = ((h.gen - minGen) / (maxGen - minGen || 1)) * this.w;
      const py = this.h - (h.best / maxFit) * this.h;
      if (idx === 0) ctx.moveTo(px, py);
      else ctx.lineTo(px, py);
      idx++;
    }
    ctx.stroke();
    
    // Draw Avg
    ctx.strokeStyle = '#ff4';
    ctx.lineWidth = 1;
    ctx.beginPath();
    idx = 0;
    for (const h of history) {
      const px = ((h.gen - minGen) / (maxGen - minGen || 1)) * this.w;
      const py = this.h - (h.avg / maxFit) * this.h;
      if (idx === 0) ctx.moveTo(px, py);
      else ctx.lineTo(px, py);
      idx++;
    }
    ctx.stroke();
    
    // Labels
    ctx.fillStyle = '#fff';
    ctx.font = '10px monospace';
    ctx.fillText(`Max: ${maxFit.toFixed(1)}`, 2, 10);
    ctx.fillText(`Gen: ${maxGen}`, this.w - 40, this.h - 2);

    ctx.restore();
  }
}

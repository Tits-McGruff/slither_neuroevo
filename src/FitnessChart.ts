// FitnessChart.ts
// Renders a fitness history chart.

export class FitnessChart {
  x: number;
  y: number;
  w: number;
  h: number;

  constructor(x: number, y: number, w: number, h: number) {
    this.x = x;
    this.y = y;
    this.w = w;
    this.h = h;
  }

  render(
    ctx: CanvasRenderingContext2D,
    history: Array<{ gen: number; best: number; avg: number }>
  ): void {
    if (!history || history.length < 2) return;
    
    const maxGen = history[history.length - 1].gen;
    const minGen = history[0].gen;
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
    for (let i = 0; i < history.length; i++) {
        const h = history[i];
        const px = ((h.gen - minGen) / (maxGen - minGen || 1)) * this.w;
        const py = this.h - (h.best / maxFit) * this.h;
        if (i === 0) ctx.moveTo(px, py);
        else ctx.lineTo(px, py);
    }
    ctx.stroke();
    
    // Draw Avg
    ctx.strokeStyle = '#ff4';
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (let i = 0; i < history.length; i++) {
        const h = history[i];
        const px = ((h.gen - minGen) / (maxGen - minGen || 1)) * this.w;
        const py = this.h - (h.avg / maxFit) * this.h;
        if (i === 0) ctx.moveTo(px, py);
        else ctx.lineTo(px, py);
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

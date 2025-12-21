// BrainViz.ts
// Visualizes the neural network structure and activity.

import { clamp } from './utils.ts';
import type { VizData } from './protocol/messages.ts';

export class BrainViz {
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

  render(ctx: CanvasRenderingContext2D, brain: VizData | null): void {
    if (!brain) return;
    
    // Determine layers to draw
    const layers = brain.layers ?? [];
    if (!layers.length) return;

    // Draw
    const maxCols = layers.length;
    const colStep = this.w / Math.max(1, maxCols - 1);
    
    ctx.save();
    ctx.translate(this.x, this.y);
    
    const heatW = Math.min(12, colStep * 0.4);
    for (let c = 0; c < maxCols; c++) {
      const layer = layers[c];
      if (!layer) continue;
      const cx = c * colStep;
      const count = layer.count;
      const rowStep = Math.min(15, this.h / Math.max(1, count));
      const startY = (this.h - (count - 1) * rowStep) / 2;
      
      for (let r = 0; r < count; r++) {
        const cy = startY + r * rowStep;
        const rectH = Math.max(3, rowStep * 0.8);
        
        // Connections to prev layer
        if (c > 0) {
          const prevLayer = layers[c - 1];
          if (!prevLayer) continue;
          const prevCount = prevLayer.count;
          const prevRowStep = Math.min(15, this.h / prevCount);
          const prevStartY = (this.h - (prevCount - 1) * prevRowStep) / 2;
          
          // Draw a few simplified connections instead of all N*M
          // For viz performance, just draw faint lines if < 5000 total lines.
          if (count * prevCount < 5000) {
             ctx.strokeStyle = 'rgba(255,255,255,0.03)';
             ctx.lineWidth = 1;
             ctx.beginPath();
             for (let pr = 0; pr < prevCount; pr++) {
               const py = prevStartY + pr * prevRowStep;
               ctx.moveTo(cx - colStep, py);
               ctx.lineTo(cx, cy);
             }
             ctx.stroke();
          }
        }
        
        // Activation heat strip
        if (layer.activations && r < layer.activations.length) {
          const val = layer.activations[r] ?? 0;
          const intensity = clamp(Math.abs(val), 0, 1);
          const alpha = 0.15 + intensity * 0.65;
          ctx.fillStyle = val >= 0
            ? `rgba(80,220,140,${alpha})`
            : `rgba(240,90,90,${alpha})`;
          ctx.fillRect(cx - heatW * 0.5, cy - rectH * 0.5, heatW, rectH);
        }

        // Neuron circle
        let alpha = 0.2;
        let val = 0;
        if (layer.activations && r < layer.activations.length) {
          val = layer.activations[r] ?? 0;
          // Tanh -1 to 1.
          alpha = 0.3 + Math.abs(val) * 0.7;
        }
        
        ctx.fillStyle = (val > 0) ? `rgba(100,255,100,${alpha})` : `rgba(255,100,100,${alpha})`;
        if (layer.isRecurrent) ctx.fillStyle = `rgba(100,200,255,${alpha})`; // Blue for memory
        
        ctx.beginPath();
        ctx.arc(cx, cy, 3, 0, Math.PI * 2);
        ctx.fill();
      }
    }
    
    ctx.restore();
  }
}

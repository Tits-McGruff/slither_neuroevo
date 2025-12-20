// chartUtils.ts
// Advanced chart rendering utilities for visualizing evolution statistics

interface FitnessHistoryEntry {
  gen: number;
  avgFitness?: number;
  maxFitness?: number;
  minFitness?: number;
  speciesCount?: number;
  topSpeciesSize?: number;
  avgWeight?: number;
  weightVariance?: number;
}

export class AdvancedCharts {
  /**
   * Renders average fitness over generations
   * @param {CanvasRenderingContext2D} ctx 
   * @param {Array} history Array of {gen, avgFitness, maxFitness, minFitness}
   * @param {number} w 
   * @param {number} h 
   */
  static renderAverageFitness(
    ctx: CanvasRenderingContext2D,
    history: FitnessHistoryEntry[],
    w: number,
    h: number
  ): void {
    if (!history || history.length === 0) return;
    
    ctx.clearRect(0, 0, w, h);
    
    const padding = 40;
    const graphW = w - padding * 2;
    const graphH = h - padding * 2;
    
    // Find data range
    let maxFit = 0;
    let minFit = Infinity;
    history.forEach(h => {
      maxFit = Math.max(maxFit, h.maxFitness || 0, h.avgFitness || 0);
      minFit = Math.min(minFit, h.minFitness || Infinity, h.avgFitness || 0);
    });
    
    if (minFit === Infinity) minFit = 0;
    const range = maxFit - minFit || 1;
    
    // Draw axes
    ctx.strokeStyle = '#444';
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(padding, padding);
    ctx.lineTo(padding, h - padding);
    ctx.lineTo(w - padding, h - padding);
    ctx.stroke();
    
    // Draw grid lines
    ctx.strokeStyle = '#333';
    ctx.lineWidth = 0.5;
    for (let i = 0; i <= 5; i++) {
      const y = padding + (graphH * i / 5);
      ctx.beginPath();
      ctx.moveTo(padding, y);
      ctx.lineTo(w - padding, y);
      ctx.stroke();
      
      // Y-axis labels
      const val = maxFit - (range * i / 5);
      ctx.fillStyle = '#aaa';
      ctx.font = '10px monospace';
      ctx.textAlign = 'right';
      ctx.fillText(val.toFixed(1), padding - 5, y + 3);
    }
    
    // Draw data lines
    if (history.length > 1) {
      const xStep = graphW / (history.length - 1);
      
      // Max fitness line (green)
      ctx.strokeStyle = '#0f0';
      ctx.lineWidth = 2;
      ctx.beginPath();
      history.forEach((h, i) => {
        const x = padding + i * xStep;
        const y = padding + graphH - (((h.maxFitness ?? 0) - minFit) / range) * graphH;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });
      ctx.stroke();
      
      // Average fitness line (yellow)
      ctx.strokeStyle = '#ff0';
      ctx.lineWidth = 2;
      ctx.beginPath();
      history.forEach((h, i) => {
        const x = padding + i * xStep;
        const y = padding + graphH - (((h.avgFitness ?? 0) - minFit) / range) * graphH;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });
      ctx.stroke();
      
      // Min fitness line (red)
      ctx.strokeStyle = '#f00';
      ctx.lineWidth = 1;
      ctx.beginPath();
      history.forEach((h, i) => {
        const x = padding + i * xStep;
        const y = padding + graphH - (((h.minFitness || 0) - minFit) / range) * graphH;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });
      ctx.stroke();
    }
    
    // Title and legend
    ctx.fillStyle = '#fff';
    ctx.font = 'bold 14px sans-serif';
    ctx.textAlign = 'left';
    ctx.fillText('Average Fitness Over Time', padding, 20);
    
    ctx.font = '11px sans-serif';
    ctx.fillStyle = '#0f0';
    ctx.fillText('■ Max', padding + 200, 20);
    ctx.fillStyle = '#ff0';
    ctx.fillText('■ Avg', padding + 250, 20);
    ctx.fillStyle = '#f00';
    ctx.fillText('■ Min', padding + 300, 20);
  }
  
  /**
   * Renders species diversity as a bar chart
   * @param {CanvasRenderingContext2D} ctx 
   * @param {Array} history Array of {gen, speciesCount, topSpeciesSize}
   * @param {number} w 
   * @param {number} h 
   */
  static renderSpeciesDiversity(
    ctx: CanvasRenderingContext2D,
    history: FitnessHistoryEntry[],
    w: number,
    h: number
  ): void {
    if (!history || history.length === 0) return;
    
    ctx.clearRect(0, 0, w, h);
    
    const padding = 40;
    const graphW = w - padding * 2;
    const graphH = h - padding * 2;

    let maxVal = 0;
    history.forEach(h => {
      const species = h.speciesCount || 0;
      const topSize = h.topSpeciesSize || 0;
      maxVal = Math.max(maxVal, species, topSize);
    });
    if (maxVal <= 0) maxVal = 1;

    // Axes
    ctx.strokeStyle = '#444';
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(padding, padding);
    ctx.lineTo(padding, h - padding);
    ctx.lineTo(w - padding, h - padding);
    ctx.stroke();

    // Grid + labels
    ctx.strokeStyle = '#333';
    ctx.lineWidth = 0.5;
    for (let i = 0; i <= 5; i++) {
      const y = padding + (graphH * i / 5);
      ctx.beginPath();
      ctx.moveTo(padding, y);
      ctx.lineTo(w - padding, y);
      ctx.stroke();
      const val = maxVal - (maxVal * i / 5);
      ctx.fillStyle = '#aaa';
      ctx.font = '10px monospace';
      ctx.textAlign = 'right';
      ctx.fillText(val.toFixed(0), padding - 5, y + 3);
    }

    const xStep = graphW / Math.max(1, history.length - 1);
    const barW = Math.max(4, xStep * 0.55);

    // Bars for species count
    history.forEach((h, i) => {
      const x = padding + i * xStep;
      const species = h.speciesCount || 0;
      const barH = (species / maxVal) * graphH;
      ctx.fillStyle = 'rgba(90,170,255,0.35)';
      ctx.fillRect(x - barW * 0.5, padding + graphH - barH, barW, barH);
    });

    // Line for top species size
    if (history.length > 1) {
      ctx.strokeStyle = '#ffb347';
      ctx.lineWidth = 2;
      ctx.beginPath();
      history.forEach((h, i) => {
        const x = padding + i * xStep;
        const topSize = h.topSpeciesSize || 0;
        const y = padding + graphH - (topSize / maxVal) * graphH;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });
      ctx.stroke();
    }

    // Title + legend
    ctx.fillStyle = '#fff';
    ctx.font = 'bold 14px sans-serif';
    ctx.textAlign = 'left';
    ctx.fillText('Species Diversity', padding, 20);
    ctx.font = '11px sans-serif';
    ctx.fillStyle = 'rgba(90,170,255,0.9)';
    ctx.fillText('■ Species count', padding + 180, 20);
    ctx.fillStyle = '#ffb347';
    ctx.fillText('— Top species size', padding + 300, 20);
  }
  
  /**
   * Renders network complexity metrics
   * @param {CanvasRenderingContext2D} ctx 
   * @param {Array} history Array of {gen, avgWeights, weightVariance}
   * @param {number} w 
   * @param {number} h 
   */
  static renderNetworkComplexity(
    ctx: CanvasRenderingContext2D,
    history: FitnessHistoryEntry[],
    w: number,
    h: number
  ): void {
    if (!history || history.length === 0) return;
    
    ctx.clearRect(0, 0, w, h);
    
    const padding = 40;
    const graphW = w - padding * 2;
    const graphH = h - padding * 2;

    let maxVal = 0;
    let minVal = Infinity;
    history.forEach(h => {
      const avgW = h.avgWeight || 0;
      const varW = h.weightVariance || 0;
      maxVal = Math.max(maxVal, avgW, varW);
      minVal = Math.min(minVal, avgW, varW);
    });
    if (minVal === Infinity) minVal = 0;
    const range = maxVal - minVal || 1;

    // Axes
    ctx.strokeStyle = '#444';
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(padding, padding);
    ctx.lineTo(padding, h - padding);
    ctx.lineTo(w - padding, h - padding);
    ctx.stroke();

    // Grid + labels
    ctx.strokeStyle = '#333';
    ctx.lineWidth = 0.5;
    for (let i = 0; i <= 5; i++) {
      const y = padding + (graphH * i / 5);
      ctx.beginPath();
      ctx.moveTo(padding, y);
      ctx.lineTo(w - padding, y);
      ctx.stroke();
      const val = maxVal - (range * i / 5);
      ctx.fillStyle = '#aaa';
      ctx.font = '10px monospace';
      ctx.textAlign = 'right';
      ctx.fillText(val.toFixed(2), padding - 5, y + 3);
    }

    if (history.length > 1) {
      const xStep = graphW / (history.length - 1);

      // Avg absolute weight line
      ctx.strokeStyle = '#5ee1ff';
      ctx.lineWidth = 2;
      ctx.beginPath();
      history.forEach((h, i) => {
        const x = padding + i * xStep;
        const y = padding + graphH - ((h.avgWeight || 0) - minVal) / range * graphH;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });
      ctx.stroke();

      // Weight variance line
      ctx.strokeStyle = '#d58bff';
      ctx.lineWidth = 2;
      ctx.beginPath();
      history.forEach((h, i) => {
        const x = padding + i * xStep;
        const y = padding + graphH - ((h.weightVariance || 0) - minVal) / range * graphH;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });
      ctx.stroke();
    }

    // Title + legend
    ctx.fillStyle = '#fff';
    ctx.font = 'bold 14px sans-serif';
    ctx.textAlign = 'left';
    ctx.fillText('Network Complexity', padding, 20);
    ctx.font = '11px sans-serif';
    ctx.fillStyle = '#5ee1ff';
    ctx.fillText('— Avg |w|', padding + 200, 20);
    ctx.fillStyle = '#d58bff';
    ctx.fillText('— Var(|w|)', padding + 280, 20);
  }
}

// chartUtils.js
// Advanced chart rendering utilities for visualizing evolution statistics

export class AdvancedCharts {
  /**
   * Renders average fitness over generations
   * @param {CanvasRenderingContext2D} ctx 
   * @param {Array} history Array of {gen, avgFitness, maxFitness, minFitness}
   * @param {number} w 
   * @param {number} h 
   */
  static renderAverageFitness(ctx, history, w, h) {
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
        const y = padding + graphH - ((h.maxFitness - minFit) / range) * graphH;
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
        const y = padding + graphH - ((h.avgFitness - minFit) / range) * graphH;
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
  static renderSpeciesDiversity(ctx, history, w, h) {
    if (!history || history.length === 0) return;
    
    ctx.clearRect(0, 0, w, h);
    
    const padding = 40;
    const graphW = w - padding * 2;
    const graphH = h - padding * 2;
    
    // For now, just show a placeholder
    ctx.fillStyle = '#888';
    ctx.font = '14px sans-serif';
    ctx.textAlign = 'center';
    ctx.fillText('Species Diversity (Coming Soon)', w/2, h/2);
    ctx.font = '11px sans-serif';
    ctx.fillText('Clustering algorithm needed for species detection', w/2, h/2 + 20);
  }
  
  /**
   * Renders network complexity metrics
   * @param {CanvasRenderingContext2D} ctx 
   * @param {Array} history Array of {gen, avgWeights, weightVariance}
   * @param {number} w 
   * @param {number} h 
   */
  static renderNetworkComplexity(ctx, history, w, h) {
    if (!history || history.length === 0) return;
    
    ctx.clearRect(0, 0, w, h);
    
    const padding = 40;
    const graphW = w - padding * 2;
    const graphH = h - padding * 2;
    
    // Placeholder
    ctx.fillStyle = '#888';
    ctx.font = '14px sans-serif';
    ctx.textAlign = 'center';
    ctx.fillText('Network Complexity (Coming Soon)', w/2, h/2);
    ctx.font = '11px sans-serif';
    ctx.fillText('Weight statistics and layer analysis', w/2, h/2 + 20);
  }
}

// BrainViz.js
// Visualizes the neural network structure and activity.

export class BrainViz {
  constructor(x, y, w, h) {
    this.x = x;
    this.y = y;
    this.w = w;
    this.h = h;
  }

  render(ctx, brain) {
    if (!brain) return;
    
    // Determine layers to draw
    const layers = [];
    
    if (brain.kind === 'mlp') {
      // Just MLP layers
      const mlp = brain.mlp;
      if (mlp.layerSizes) {
        for (let i = 0; i < mlp.layerSizes.length; i++) {
          const val = (i === 0) ? null : // Inputs not easily avail here unless passed? 
                      (i < mlp.layerSizes.length) ? mlp._bufs[i-1] : null;
          // Actually _bufs[0] is output of layer 0 (which is input to layer 1).
          // Wait, MLP.forward: cur = input. Loop l=0..N-1. next = _bufs[l]. cur processed to next.
          // So _bufs[l] stores output of layer l+1?
          // layerSizes: [in, h1, h2, out]
          // l=0: in->h1. next=_bufs[0]. Stores h1 activations.
          // l=1: h1->h2. next=_bufs[1]. Stores h2 activations.
          // l=2: h2->out. next=_bufs[2]. Stores out activations.
          
          layers.push({ 
            count: mlp.layerSizes[i], 
            activations: (i === 0) ? null : mlp._bufs[i-1] 
          });
        }
      }
    } else {
      // MLP feature extractor -> GRU -> Head
      // 1. MLP inputs
      if (brain.mlp) {
        for (let i = 0; i < brain.mlp.layerSizes.length; i++) {
           // Skip last layer of MLP if it feeds into GRU? 
           // Usually MLP output is GRU input.
           layers.push({
             count: brain.mlp.layerSizes[i],
             activations: (i === 0) ? null : brain.mlp._bufs[i-1]
           });
        }
      }
      // 2. GRU
      if (brain.gru) {
        layers.push({
          count: brain.gru.hiddenSize,
          activations: brain.gru.h,
          isRecurrent: true
        });
      }
      // 3. Head
      if (brain.head) {
         // Head is just linear layer from GRU to Out.
         // Head doesn't store state usually? DenseHead.forward returns result.
         // But BrainController stores final output.
         layers.push({
           count: brain.head.outSize,
           activations: null // We don't have internal buffer for head easily, but distinct from output
         });
      }
    }

    // Draw
    const maxCols = layers.length;
    const colStep = this.w / Math.max(1, maxCols - 1);
    
    ctx.save();
    ctx.translate(this.x, this.y);
    
    for (let c = 0; c < maxCols; c++) {
      const layer = layers[c];
      const cx = c * colStep;
      const count = layer.count;
      const rowStep = Math.min(15, this.h / count);
      const startY = (this.h - (count - 1) * rowStep) / 2;
      
      for (let r = 0; r < count; r++) {
        const cy = startY + r * rowStep;
        
        // Connections to prev layer
        if (c > 0) {
          const prevLayer = layers[c - 1];
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
        
        // Neuron circle
        let alpha = 0.2;
        let val = 0;
        if (layer.activations && r < layer.activations.length) {
          val = layer.activations[r];
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

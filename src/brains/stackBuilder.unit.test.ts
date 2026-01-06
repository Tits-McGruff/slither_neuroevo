import { describe, it, expect } from 'vitest';
import { buildStackGraphSpec } from './stackBuilder.ts';

describe('stackBuilder (unit)', () => {
  it('builds MLP-only spec when stack disabled', () => {
    const spec = buildStackGraphSpec(
      {
        hiddenLayers: 2,
        neurons1: 8,
        neurons2: 6,
        neurons3: 4,
        neurons4: 4,
        neurons5: 4
      },
      {
        brain: {
          inSize: 5,
          outSize: 2,
          useMlp: true,
          stack: { gru: 0, lstm: 0, rru: 0 }
        }
      }
    );
    const types = spec.nodes.map(node => node.type);
    expect(types).toEqual(['Input', 'MLP']);
    expect(spec.outputs[0]?.nodeId).toBe('mlp');
  });

  it('builds Dense-only spec when MLP and stack disabled', () => {
    const spec = buildStackGraphSpec(
      {
        hiddenLayers: 2,
        neurons1: 8,
        neurons2: 6,
        neurons3: 4,
        neurons4: 4,
        neurons5: 4
      },
      {
        brain: {
          inSize: 5,
          outSize: 2,
          useMlp: false,
          stack: { gru: 0, lstm: 0, rru: 0 }
        }
      }
    );
    const types = spec.nodes.map(node => node.type);
    expect(types).toEqual(['Input', 'Dense']);
    expect(spec.outputs[0]?.nodeId).toBe('head');
  });

  it('adds a head when any recurrent is enabled', () => {
    const spec = buildStackGraphSpec(
      {
        hiddenLayers: 1,
        neurons1: 6,
        neurons2: 4,
        neurons3: 4,
        neurons4: 4,
        neurons5: 4
      },
      {
        brain: {
          inSize: 6,
          outSize: 2,
          gruHidden: 5,
          stack: { gru: 1, lstm: 0, rru: 0 }
        }
      }
    );
    const types = spec.nodes.map(node => node.type);
    expect(types).toEqual(['Input', 'MLP', 'GRU', 'Dense']);
    expect(spec.outputs[0]?.nodeId).toBe('head');
  });
});

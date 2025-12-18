import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import {
  buildSettingsUI,
  applyValuesToSlidersFromCFG,
  hookSliderEvents,
  updateCFGFromUI
} from './settings.js';
import { CFG } from './config.js';
import { fmtNumber } from './utils.js';

class FakeElement {
  constructor(tag) {
    this.tagName = tag;
    this.children = [];
    this.dataset = {};
    this.className = '';
    this.id = '';
    this.textContent = '';
    this.innerHTML = '';
    this.value = '';
    this.type = '';
    this.style = {};
  }
  appendChild(child) {
    this.children.push(child);
    return child;
  }
  querySelectorAll() {
    return [];
  }
  addEventListener() {}
}

describe('settings.js', () => {
  let originalDocument;

  beforeEach(() => {
    originalDocument = globalThis.document;
    globalThis.document = {
      createElement: (tag) => new FakeElement(tag),
      getElementById: () => null
    };
  });

  afterEach(() => {
    globalThis.document = originalDocument;
  });

  it('buildSettingsUI populates the container', () => {
    const container = new FakeElement('div');
    buildSettingsUI(container);
    expect(container.children.length).toBeGreaterThan(0);
  });

  it('applyValuesToSlidersFromCFG updates slider values and labels', () => {
    const original = CFG.boost.minPointsToBoost;
    CFG.boost.minPointsToBoost = 7.5;

    const slider = {
      dataset: { path: 'boost.minPointsToBoost', decimals: '1' },
      value: '0'
    };
    const output = { textContent: '' };
    globalThis.document.getElementById = () => output;

    const root = { querySelectorAll: () => [slider] };
    applyValuesToSlidersFromCFG(root);

    expect(slider.value).toBe(String(CFG.boost.minPointsToBoost));
    expect(output.textContent).toBe(fmtNumber(CFG.boost.minPointsToBoost, 1));

    CFG.boost.minPointsToBoost = original;
  });

  it('updateCFGFromUI writes slider values back into CFG', () => {
    const slider = {
      dataset: { path: 'collision.cellSize' },
      value: '123'
    };
    const root = { querySelectorAll: () => [slider] };

    updateCFGFromUI(root);

    expect(CFG.collision.cellSize).toBe(123);
  });

  it('hookSliderEvents triggers live updates for live sliders', () => {
    const handler = vi.fn();
    let stored;
    const slider = {
      dataset: { path: 'collision.cellSize', decimals: '0', requiresReset: '0' },
      value: '80',
      addEventListener: (evt, cb) => {
        if (evt === 'input') stored = cb;
      }
    };
    const root = { querySelectorAll: () => [slider] };

    hookSliderEvents(root, handler);
    stored();

    expect(handler).toHaveBeenCalledWith(slider);
  });
});

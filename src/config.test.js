import { describe, it, expect } from 'vitest';
import { CFG, CFG_DEFAULT, resetCFGToDefaults } from './config.js';

describe('config.js', () => {
  it('resetCFGToDefaults restores default values', () => {
    const original = CFG.snakeBaseSpeed;
    CFG.snakeBaseSpeed = original + 10;
    resetCFGToDefaults();
    expect(CFG.snakeBaseSpeed).toBe(CFG_DEFAULT.snakeBaseSpeed);
  });

  it('mutating CFG does not mutate CFG_DEFAULT', () => {
    const before = CFG_DEFAULT.snakeBoostSpeed;
    CFG.snakeBoostSpeed = before + 5;
    expect(CFG_DEFAULT.snakeBoostSpeed).toBe(before);
    resetCFGToDefaults();
  });
});

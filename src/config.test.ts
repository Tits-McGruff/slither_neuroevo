import { describe, it, expect } from 'vitest';
import { CFG, CFG_DEFAULT, resetCFGToDefaults, applyAuthoritativeSensorSpec } from './config.ts';
import { readFileSync } from 'node:fs';
import { getSensorLayout,getSensorSpec,sensorLayoutFromContract,type SensorContractEnvelope } from './protocol/sensors.ts';

/** Shared Rust canonical fixtures cover both v4 profiles and the 504-input ceiling. */
const CONTRACTS = JSON.parse(readFileSync(new URL('../native/fixtures/sensor-contract-shapes.json',import.meta.url),'utf8')) as SensorContractEnvelope[];

/** Test suite label for config defaults. */
const SUITE = 'config.ts';

describe(SUITE, () => {
  it('sizes stack/editor drafts from exact authoritative v4 profiles and resets to v3',()=>{
    try {
      for (const contract of CONTRACTS) {
        const layout=sensorLayoutFromContract(contract);
        if (layout.layoutVersion==='v3') continue;
        const spec=getSensorSpec(layout,contract);
        applyAuthoritativeSensorSpec(spec);
        expect(CFG.brain.inSize).toBe(layout.inputSize);
        expect(CFG.sense.layoutVersion).toBe('v4');
        expect(CFG.sense.profile).toBe(layout.profile);
        expect(()=>applyAuthoritativeSensorSpec({...spec,order:spec.order.slice().reverse()})).toThrow();
        const missing = {...spec};
        delete missing.contract;
        expect(()=>applyAuthoritativeSensorSpec(missing)).toThrow();
      }
      expect(getSensorLayout(16,'v3').inputSize).toBe(83);
      expect(getSensorLayout(16,'v4','basic').inputSize).toBe(120);
      expect(getSensorLayout(32,'v4','motion').inputSize).toBe(504);
      expect(()=>getSensorLayout(16,'v4')).toThrow('explicit');
      expect(()=>getSensorLayout(16,'v2')).toThrow('unsupported');
      CFG.sense.bubbleBins = 8;
      applyAuthoritativeSensorSpec(getSensorSpec(getSensorLayout(8, 'v3')));
      expect(CFG.sense.layoutVersion).toBe('v3'); expect(CFG.brain.inSize).toBe(51);
    } finally { resetCFGToDefaults(); }
    expect(CFG.brain.inSize).toBe(264);
    expect(CFG.sense.layoutVersion).toBe('v4');
    expect(CFG.baselineBots.algorithm).toBe(2);
    expect(CFG.baselineBots.behaviorMode).toBe(1);
    expect(CFG.evolvedRespawn.enabled).toBe(false);
  });
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

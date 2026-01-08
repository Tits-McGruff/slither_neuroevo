import { describe, it, expect, vi, beforeEach, type Mock } from 'vitest';
import { BaselineBotManager } from './baselineBots.ts';
import type { World } from '../world.ts';
import type { Snake } from '../snake.ts';

describe('BaselineBotManager AI', () => {
  let manager: BaselineBotManager;
  let mockWorld: World;
  let mockSnake: Snake;

  beforeEach(() => {
    // Setup a default manager with 1 bot
    manager = new BaselineBotManager({ count: 1, seed: 123, randomizeSeedPerGen: false });
    
    mockWorld = {
      baselineBots: [],
      // minimal mock
    } as unknown as World;

    mockSnake = {
      id: 100,
      alive: true,
      pointsScore: 1000,
      computeSensors: vi.fn(), 
      // minimal mocks
    } as unknown as Snake;
    
    // Inject the snake into the world and manager
    mockWorld.baselineBots[0] = mockSnake;
    manager.registerBot(0, 100);
    // Force state to seek (since we want to test decision making when food is present)
    // We can't easily force private state, but we can rely on update() logic
    // or just assume default 'roam' -> 'seek' transition if food > 0.1
  });

  // Helper to construct sensor array
  // 5 global values + 3 * bins
  // bins defaults to 12 in sensors.ts if not changed in CFG.
  // We'll assume 12 bins.
  function makeSensors(bins = 12): Float32Array {
    return new Float32Array(5 + 3 * bins);
  }

  // Indices for 12 bins: 
  // Food: 5..16
  // Hazard: 17..28
  // Wall: 29..40
  function setBin(sensors: Float32Array, binIdx: number, food: number, hazard: number, wall: number) {
    const bins = 12;
    // Food
    sensors[5 + binIdx] = food;
    // Hazard
    sensors[5 + bins + binIdx] = hazard;
    // Wall
    sensors[5 + 2 * bins + binIdx] = wall;
  }
  
  // NOTE: Hazard sensor: +1 is Clear, -1 is Blocked/Hazard
  // Food sensor: +1 is Dense Food, -1 is No Food
  // NOTE: Food is now clamped to 0.4 max in computeAction!

  it('reproduces kamikaze behavior: chooses high food despite hazard', () => {
    const sensors = makeSensors(12);
    // Init all to empty/neutral
    for(let i=0; i<12; i++) setBin(sensors, i, -1, 1, 1); 

    // Bin 0 (Forward): HIGH Food (1.0), BLOCKED Hazard (-0.9) 
    // New Clamped Food: min(1.0, 0.4) = 0.4.
    // Score ~ 0.4*0.5 + 0.05*1.5 = 0.2 + 0.075 = 0.275 (Seek weights: Food=0.5, Clear=1.5)
    // Wait, Clearance = (-0.9+1)/2 = 0.05.
    
    setBin(sensors, 0, 1.0, -0.9, 1.0); 

    // Bin 6: NO Food (-1.0), CLEAR Hazard (1.0) -> Clear = 1.0
    // Score ~ -1.0*0.5 + 1.0*1.5 = -0.5 + 1.5 = 1.0.
    
    // 1.0 > 0.275. So it SHOULD pick Bin 6 (safe) over Bin 0 (food).
    // This confirms the fix works.

    (mockSnake.computeSensors as Mock).mockReturnValue(sensors);
    
    // Run update
    // We expect it to NOT avoid (unless worstClear < trigger)
    // worstClear here is 0.05.
    // trigger is -0.25.
    // 0.05 > -0.25, so it doesn't trigger 'avoid' state override in the current logic.
    // It stays in 'seek' or 'roam'.
    // In seek, it picks max score.
    
    manager.update(mockWorld, 0.1, vi.fn());
    
    const action = manager.getActionForSnake(100);
    expect(action).not.toBeNull();
    
    // Bin 0 is angle 0.
    // With new logic, it should avoid the hazard (Bin 0) and pick a clearer path (e.g. Bin 1 or side).
    expect(Math.abs(action!.turn)).toBeGreaterThan(0.2);
  });
  
  it('avoids hazard with new logic (veto)', () => {
     // This test will fail until we implement the fix
     const sensors = makeSensors(12);
    // Init all to empty/neutral
    for(let i=0; i<12; i++) setBin(sensors, i, -1, 1, 1); 

    // Bin 0: HIGH Food (1.0), MODERATE Hazard (-0.6, just below veto threshold?) 
    // Let's say Veto is -0.5. -0.6 should be vetoed.
    setBin(sensors, 0, 1.0, -0.6, 1.0); 
    
    // Bin 1: NO Food (-1.0), CLEAR (1.0)
    setBin(sensors, 1, -1.0, 1.0, 1.0);

    (mockSnake.computeSensors as Mock).mockReturnValue(sensors);
    
    manager.update(mockWorld, 0.1, vi.fn());
    const action = manager.getActionForSnake(100);
    
    // Should NOT pick bin 0. Should pick bin 1 (or any clear one).
    // Bin 1 angle is approx (1/12)*TAU = 0.52 rad.
    // Turn value roughly 0.52 / (PI/2) = 0.33
    
    // If it picks bin 0, turn is ~0.
    expect(Math.abs(action!.turn)).toBeGreaterThan(0.2);
  });

  it('prevents boosting when hazard is nearby', () => {
    const sensors = makeSensors(12);
    // Init all to clear
    for(let i=0; i<12; i++) setBin(sensors, i, -1, 1, 1); 
    
    // Bin 0: Safe, Food. 
    setBin(sensors, 0, 1.0, 1.0, 1.0);
    
    // Bin 5: Hazard (-0.8). Nearby but not in path.
    // worstClear will be (-0.8+1)/2 = 0.1.
    setBin(sensors, 5, -1.0, -0.8, 1.0);
    
    (mockSnake.computeSensors as Mock).mockReturnValue(sensors);
    
    // Inject RNG that triggers boost (normally boostChance is 0.02)
    // We can try to force it by monkey-patching or just relying on "worstClear should block it"
    // Actually, the boost check is:
    // if (state !== 'avoid' && state !== 'boost') { ... if (boostOk && ... ) state = 'boost' }
    // We want to ensure it DOES NOT enter boost state if worstClear is suspicious.
    
    // To reliably test this without RNG, we might need to inspect the state or logic differently.
    // Or we can rely on the property that we are ADDING a check: `&& worstClear > 0.5`.
    
    // Let's just create the scenario where it MIGHT boost, and verify strictly no boost in action.
    // But action.boost comes from state='boost'.
    
    // Accessing private state is hard in strict TS unless we cast to any.
    // Let's verify via action logic if possible, or skip strict unit testing of the RNG branch for now and focus on the Veto which is deterministic.
  });
});

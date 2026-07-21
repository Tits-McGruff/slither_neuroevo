# Slither-Native: The Exhaustive Refactor & Parity Documentation

> [!CAUTION]
> This plan is superseded and must not be implemented. On 2026-07-21 the
> repository owner chose Rust N-API neural kernels, used by both single-thread
> and MT inference, instead of a second full world/physics implementation.
> The authoritative current plan is
> [project-recovery-plan.md](./project-recovery-plan.md). This file remains
> unchanged below this notice as historical context outside the archive.

## Revision Notes (Phase 1 Final)

- **Modular Rebuild**: Gutting monolithic `lib.rs` into `math`, `kernels`, `brain`, `spatial_hash`, `sensors`, and `physics`.
- **Kernel Parity (Deterministic)**: Port "OG WASM" logic with explicit ordering and explicit f32 truncation points to match JS `Float32Array` behavior (no hidden reassociation), including bias init defaults (GRU -0.7, LSTM forget 0.6, RRU gate 0.1).
- **Zero-Allocation Policy**: Eager buffer allocation during backend construction to prevent per-tick heap churn.
- **N-API ABI Reality**: Use `TypedArray`/`Buffer` for zero-copy data paths; `#[napi(object)]` is for small config/diagnostics only and is not zero-copy.
- **Source Mapping**: Section 15 maps Rust submodules to TS/WASM *functions* (not brittle line numbers).
- **Formal Verification**: Mirror existing Vitest numeric expectations (with matching tolerances) into Rust tests; add new parity tests only where JS lacks coverage.
- **Sensor Layout Alignment**: Sensor bin counts and offsets follow `CFG.sense.bubbleBins` + `layoutVersion` (`v2` default).

## 1. Revision History & Design Philosophy

This document serves as the authoritative blueprint for the total refactor of the `slither-native` Rust crate. Following a series of regression errors and structural failures, we are abandoning the high-level "port-as-you-go" approach in favor of a rigorous, specification-first methodology.

The primary philosophy of this refactor is **Parity-First Behavior**. We assume the existing TypeScript/WASM implementation is the "Gold Standard" of truth. Every logic branch, every floating-point constant, and every buffer stride in the Rust engine must be traceable back to a specific function in the TypeScript reference. We are building a "Digital Twin" of the simulation, optimized for the native CPU instruction set but numerically tethered to the original implementation within the same test tolerances.

We are also moving to a **Modular Multi-File Architecture**. The monolithic `lib.rs` currently in place is a maintenance nightmare and an architectural "footgun". By splitting the crate into discrete modules (`math`, `kernels`, `brain`, `spatial_hash`, `sensors`, `physics`), we enable unit testing at the functional level, allowing us to verify the mathematics of a GRU gate or a ray-circle intersection independently of the world state.

## 2. Global Constraints and System Invariants

To ensure compatibility with the existing Node.js parent process and the subsequent rendering pipeline, the following invariants are non-negotiable:

- **INV-001 (Precision + Storage)**: Match JS semantics: weights/state/IO buffers are `Float32Array` in TS, so native storage MUST be `f32`. Computation uses `f64` only where JS uses `Number`, then results are explicitly truncated to `f32` at the same write points as the TS implementation (layer outputs, recurrent state, and batch outputs). Do not silently upgrade buffers to `f64`.
- **INV-002 (Memory Strategy)**: The high-frequency simulation loop (backend `step`) must satisfy a "Zero-Allocation Guarantee". All vectors, scratch buffers, and hash maps must be pre-allocated during the constructor phase. Per-tick heap allocations (`Box`, `Vec::new`) are strictly prohibited to prevent GC-like pauses and ensure consistent frame times.
  - **Allowed allocation boundary**: allocations are permitted only in constructor, explicit reset paths, and explicit config-change reconfigure paths that are never called from the per-substep loop.
- **INV-003 (Byte Stride Alignment)**: The weight buffer passed from Node.js is a flat `Float32Array`. The Rust engine must interpret this buffer using offset-based slicing that matches the `enrichArchInfo` layout in `mlp.ts` exactly. Any off-by-one error in weight indexing will result in "brain-dead" agents.
- **INV-004 (Coordinate Space)**: The world origin `(0,0)` is the center of the circular arena. Positive Y is "Down" and positive X is "Right". Heading `0` is eastward, with positive rotations going clockwise (following the standard DOM/Canvas coordinate model).
- **INV-005 (Sensor Layout)**: Sensor input sizing and offsets must match `getSensorLayout(bins, layoutVersion)` with `bins = max(8, floor(CFG.sense.bubbleBins ?? 16))` and `layoutVersion = CFG.sense.layoutVersion ?? 'v2'`.
- **INV-006 (FP Flags)**: Disable fast-math and contraction for parity builds. Do not allow reassociation or FMA unless the JS reference does.
- **INV-007 (Grid Epoch Ordering)**: Sensors that query the collision grid must use the grid built at the end of the previous substep; collision resolution must use the grid rebuilt after physics advance for the current substep. Backend init and any hard reset must build an initial grid from current positions before the first control evaluation.
- **INV-008 (Iteration Ordering)**: Snake update order, pellet scan order, and segment insertion order into the grid must match TS iteration order. Do not introduce sorting, hashing iteration, or parallelism in any step that affects control, sensing, spawning, feeding, or collisions.

```toml
# .cargo/config.toml (parity profile guidance)
[build]
rustflags = [
  "-C", "target-feature=-fma",
  "-C", "llvm-args=-fp-contract=off",
]
```

## 3. Module Specification: `native/src/math.rs`

This module provides the primitive mathematical operations. Although simple, these functions are the most frequently called blocks in the entire codebase.

- **`sigmoid(x: f64) -> f64`**: Must implement the standard logistic function `1 / (1 + exp(-x))`. We will use `f64::exp()` for parity with `Math.exp`.
- **`tanh(x: f64) -> f64`**: Must use `f64::tanh()` for parity with `Math.tanh`.
- **`ang_norm(a: f64) -> f64`**: A critical utility that wraps an angle into the range `[-PI, PI]`. It must handle multiple rotations and negative angles correctly to prevent "spinning" artifacts in the AI's heading sensors. Port the exact TS function body (`src/utils.ts`), do not substitute a mathematically equivalent form unless tests confirm identical outputs across representative inputs.
- **`lerp(a: f64, b: f64, t: f64) -> f64`**: Standard linear interpolation `a + t * (b - a)`. Used for speed transitions and camera smoothing.
- **`clamp(x: f64, min: f64, max: f64) -> f64`**: Standard clamp.

```rs
#[inline(always)]
fn sigmoid(x: f64) -> f64 {
  1.0 / (1.0 + (-x).exp())
}

#[inline(always)]
fn ang_norm(a: f64) -> f64 {
  let mut x = a;
  while x > std::f64::consts::PI {
    x -= std::f64::consts::TAU;
  }
  while x < -std::f64::consts::PI {
    x += std::f64::consts::TAU;
  }
  x
}
```

```rs
#[test]
fn ang_norm_large_values_match_ts() {
  let cases = [
    0.0,
    std::f64::consts::PI,
    -std::f64::consts::PI,
    std::f64::consts::PI + 1e-6,
    -std::f64::consts::PI - 1e-6,
    3.0 * std::f64::consts::PI,
    -3.0 * std::f64::consts::PI,
    1.0e9,
    -1.0e9,
  ];
  for &a in &cases {
    let got = ang_norm(a);
    assert!(got >= -std::f64::consts::PI && got <= std::f64::consts::PI);
  }
}
```

## 4. Module Specification: `native/src/kernels.rs`

This module is the direct port of the "OG WASM" kernels. We are migrating the C-style pointer logic from `legacy_wasm_reference.rs` into idiomatic (but performance-identical) Rust.

- **`dense_dot(input: &[f32], weights: &[f32]) -> f64`**:
    This function computes the sum of `inputs[i] * weights[i]` with a *fixed* iteration order. Use an explicit `for` loop and a `f64` accumulator, then cast to `f32` only when writing to an output buffer (to mirror `Float32Array` writes in JS). Avoid `.zip().map().sum()` or auto-vectorization if it changes reassociation or ordering.
- **`gru_step_kernel(...)`**:
    This kernel precisely replicates the GRU gating logic. It must handle nine distinct weight matrices (Wz, Wr, Wh, Uz, Ur, Uh, bz, br, bh) and preserve the default update-bias init (`initUpdateBias = -0.7`). The most critical part of this port is the state update: `h_new = (1 - z) * h_prev + z * h_tilde`.
- **`lstm_step_kernel(...)`**:
    Ported from WASM, this handles the Input, Forget, Output, and Cell gates. It must maintain both the hidden state `h` and the cell state `c`. We will carefully port the forget bias initialization logic (default 0.6) to ensure the recurrent memory persists as expected.
- **`rru_step_kernel(...)`**:
    A "Minimal Recurrent Unit" with only two gates. This is a custom node type in Slither-Neuroevo and must be ported with exact fidelity to `src/brains/ops.ts` (including `initGateBias = 0.1`).

```rs
fn dense_dot(inputs: &[f32], weights: &[f32]) -> f64 {
  let mut sum = 0.0_f64;
  for i in 0..inputs.len() {
    sum += inputs[i] as f64 * weights[i] as f64;
  }
  sum
}
```

## 5. Module Specification: `native/src/spatial_hash.rs`

The spatial hash is the "Broad Phase" of our collision system. The TypeScript implementation in `src/spatialHash.ts` uses a "Flat" architecture with three typed arrays to avoid object overhead. We will replicate this pattern in Rust.

- **`FlatSpatialHash` Struct**:
  - `head: Vec<i32>`: An array of size `cols * rows` initialized to `-1`. Each index corresponds to a grid cell.
  - `next: Vec<i32>`: An array of size `capacity` storing the index of the next segment in the linked list.
  - `ent: Vec<i32>`: Stores the snake index/id for each node.
  - `seg: Vec<i32>`: Stores the segment index for each node.
- **Occupancy**:
  - Nodes are considered live when `i < count`. Do not clear `ent/seg/next` on reset; only reset `head` and `count`.
- **Construction**:
  - Use `width = worldRadius * 2.5` and `height = worldRadius * 2.5` with `cellSize = CFG.collision.cellSize` to match `World` initialization.
- **`insert(x, y, entity, segment_idx)`**:
  - Maps world `(x,y)` to cell `(cx, cy)`.
  - Handles the offset mapping: `cx = floor(x / cellSize) + halfCols`, `cy = floor(y / cellSize) + halfRows`.
  - Checks bounds and capacity.
  - Performs the linked-list insertion: `next[new_idx] = head[cell_idx]; head[cell_idx] = new_idx;`.
- **`query_cell(cx, cy, callback)`**:
  - Iterates through the linked list starting at `head[cell_idx]`.
  - Invokes the logic for each segment found.

```rs
fn add(&mut self, x: f64, y: f64, ent: i32, seg_idx: i32) {
  if self.count >= self.capacity { return; }
  let cx = (x / self.cell_size).floor() as i32 + self.half_cols;
  let cy = (y / self.cell_size).floor() as i32 + self.half_rows;
  if cx < 0 || cx >= self.cols || cy < 0 || cy >= self.rows { return; }
  let cell = (cy * self.cols + cx) as usize;
  let i = self.count as usize;
  self.ent[i] = ent;
  self.seg[i] = seg_idx;
  self.next[i] = self.head[cell];
  self.head[cell] = i as i32;
  self.count += 1;
}
```

## 6. Module Specification: `native/src/sensors.rs`

This is the "Brain's Interface" to the world. A sensor mismatch of even 1% can cause the neural network to miscalculate its path.

- **The Polar Bubble Algorithm**:
    The engine computes `bins = max(8, floor(CFG.sense.bubbleBins ?? 16))` angular bins (360 degrees) around the head. Layout offsets come from `getSensorLayout(bins, layoutVersion)` with `layoutVersion = CFG.sense.layoutVersion ?? 'v2'`.
  - **`angle_to_centered_bin(rel_angle, total_bins)`**: Precisely ports `angleToCenteredBin` from `sensors.ts`. This ensures the AI's "forward" view is centered on Bin 0 for the active layout.
  - **`layout.inputSize`**: Always use the layout-provided input size and offsets; do not hardcode `5 + 3 * bins` for v2.
- **`Wall Bins`**:
    Uses ray-circle intersection math. For each bin angle `theta`, solve for distance `d` to the arena boundary. Normalize with the active `rNear` from `computeSensorRadii`: `ratio = clamp(d / rNear, 0, 1)`, output `ratio * 2.0 - 1.0` (range `[-1, 1]`).
- **`Food Bins`**:
    For each bin, sum weighted pellet values within `rFar`. Use the same `wDist = 1 - d / r` weighting and `bubbleFoodK` normalization logic as `sensors.ts`, with per-bin accumulation in scratch buffers sized to the current bin count.
- **`Scan Caps`**:
    Honor `CFG.sense.maxPelletChecks` and `CFG.sense.maxSegmentChecks` caps when iterating pellets/segments (same defaults and guards as `sensors.ts`).
- **`Hazard Bins`**:
    Query the collision grid for segments within `rNear`. Compute the minimum clearance per bin and normalize with `ratio = clamp(clearance / rNear, 0, 1)` as in `sensors.ts`. This uses segment distance, snake radii, and `CFG.collision.hitScale`.
    - Exclude segments belonging to the same snake id (self), mirroring `sensors.ts`.
- **`Head Pressure (v2)`**:
    When `layout.offsets.head` exists, compute head-only clearance bins using `rNear` and the head-vs-head logic from `sensors.ts`.
- **`Speed + Boost Scalars (v2)`**:
    When `layoutVersion === 'v2'`, write speed and boost ratios using the layout contract. For the current v2 layout, these are indices 5 and 6; add a construction-time assertion that the layout matches those indices, or use layout-provided offsets if the layout surface changes.
- **`Legacy Layout`**:
    If `layoutVersion !== 'v2'`, use the legacy bubble radius (`_bubbleRadiusForSnake`) and the legacy food/hazard/wall bin fillers from `sensors.ts`.

```rs
// v2 scalar slots (indices 5, 6 in current layout)
let speed_ratio = if snake.speed.is_finite() {
  snake.speed / cfg.snake_boost_speed.max(1e-6)
} else {
  0.0
};
ins[5] = ratio_to_bipolar(speed_ratio);
let boost_ratio = snake.boost.clamp(0.0, 1.0);
ins[6] = ratio_to_bipolar(boost_ratio);
```

```rs
// Construction-time layout contract assertion (v2 scalars).
assert_eq!(layout.layout_version, LayoutVersion::V2);
assert_eq!(layout.scalar_speed_index, 5);
assert_eq!(layout.scalar_boost_index, 6);
```

```rs
// Sensor layout resolution + radii (no allocations).
let bins = (cfg.sense.bubble_bins.floor() as i32).max(8);
let layout = get_sensor_layout(bins, cfg.sense.layout_version);
let size_norm = snake.size_norm();
let (r_near, r_far) = compute_sensor_radii(size_norm, cfg);
let ins = scratch_inputs.as_mut_slice();
assert!(ins.len() == layout.input_size as usize);
```

```rs
// Food bin accumulation with cap.
scratch_food.fill(0.0);
let mut checks = 0;
for p in pellets.iter() {
  if checks >= max_pellet_checks { break; }
  checks += 1;
  let dx = p.x - sx;
  let dy = p.y - sy;
  let d = (dx * dx + dy * dy).sqrt();
  if d > r_far || d <= 1e-6 { continue; }
  let rel = ang_norm(dy.atan2(dx) - snake.dir);
  let bi = angle_to_centered_bin(rel, bins);
  let w_dist = 1.0 - d / r_far;
  let w_val = (p.v / cfg.food_value).clamp(0.0, 6.0);
  scratch_food[bi] += w_dist * w_val;
}
for i in 0..bins {
  let s = scratch_food[i];
  let frac = s / (s + bubble_food_k);
  ins[food_off + i] = (frac * 2.0 - 1.0).clamp(-1.0, 1.0);
}
```

```rs
// Hazard bins (v2) – centered binning + clearance normalization.
scratch_haz.fill(r_near);
for seg in nearby_segments.iter().take(max_segment_checks) {
  if seg.ent_id == snake.id { continue; }
  let (qx, qy, d2) = closest_point_on_segment(sx, sy, seg.ax, seg.ay, seg.bx, seg.by);
  let thr = (snake.radius + seg.other_radius) * cfg.collision.hit_scale;
  let max_dist = r_near + thr;
  if d2 > max_dist * max_dist { continue; }
  let d = d2.sqrt();
  let clear = (d - thr).max(0.0);
  let rel = ang_norm((qy - sy).atan2(qx - sx) - snake.dir);
  let bi = angle_to_centered_bin(rel, bins);
  if clear < scratch_haz[bi] { scratch_haz[bi] = clear; }
}
for i in 0..bins {
  ins[haz_off + i] = ratio_to_bipolar(scratch_haz[i] / r_near);
}
```

## 7. Module Specification: `native/src/physics.rs`

This module implements the Newtonian integrator.

- **Movement Integration**:
    1. **Turn Rate**: Use `computeSnakeTurnRateByLen(length)` from `snake.ts` (base rate scaled by `CFG.snakeTurnPenalty`).
    2. **Heading**: `dir = ang_norm(dir + turn_input * turn_rate * dt)`.
    3. **Velocity**: Compute `baseNow` and `boostNow` exactly as `snake.ts` using `CFG.snakeBaseSpeed`, size penalties, and boost multipliers, then smooth speed with `lerp(speed, target, 1 - exp(-dt * 6.5))`.
    4. **Position**: `x += cos(dir) * speed * dt; y += sin(dir) * speed * dt`.
    5. **Boundary**: If `hypot(x, y) + radius >= CFG.worldRadius`, the snake dies (no wrap/clamp).
- **Boost Burn + Drops**:
    Apply `_applyBoostMassBurn` to reduce `targetLen` and emit boost pellets when boosting (matching `snake.ts`).
- **Tail Maintenance**:
    We iterate through segments `1..N`. For each segment `i`, we calculate the distance to segment `i-1`. If the distance deviates from `snakeSpacing`, we move segment `i` along the vector towards `i-1` until the spacing is exactly `snakeSpacing`.
- **Feeding + Growth**:
    Consume pellets within the eat radius, update `pointsScore`, and adjust `targetLen` according to `CFG.growPerFood` and clamps.

```rs
// Speed smoothing
let t = 1.0 - (-dt * 6.5).exp();
snake.speed = lerp(snake.speed, target_speed, t);

// Boundary death
let d = (snake.x * snake.x + snake.y * snake.y).sqrt();
if d + snake.radius >= cfg.world_radius {
  snake.die(world, false);
  return;
}
```

```rs
// Collision resolve (head vs segments)
let cx = (snake.x / cell_size).floor() as i32;
let cy = (snake.y / cell_size).floor() as i32;
for oy in -1..=1 {
  for ox in -1..=1 {
    grid.query_cell(cx + ox, cy + oy, |other, seg_idx| {
      if other.id == snake.id { return; }
      let (p0, p1) = other.segment(seg_idx);
      let d2 = point_segment_dist2(snake.x, snake.y, p0, p1);
      let thr = (snake.radius + other.radius) * cfg.collision.hit_scale;
      if d2 <= thr * thr {
        snake.die(world, true);
      }
    });
  }
}
```

## 8. Module Specification: `native/src/lib.rs` (The Integration)

Export a backend object that implements the `PhysicsBackend` interface (`step(dt)` + `syncTo(world)`), and keep JS `World.update` as the orchestrator for timing, camera, and generation lifecycle.

- **`step()` Implementation (Backend Substep)**:
    1. **Substep Input**: Accept `dt` from `World._stepPhysics`; JS already handles `simSpeed`, `dtClamp`, and substep subdivision.
    2. **Pre-Step Bookkeeping**: Call `prepareForStep(dt)` to update age/score before control evaluation.
    3. **Pellet Spawn**: Accumulate `pelletSpawnPerSecond`, clamp to `pelletCountTarget`, and add ambient pellets via `_spawnAmbientPellet` (fractal food) before control/physics.
    4. **Control Dispatch**: For each alive snake:
        - Respect bot actions (`BaselineBotManager`) and controller inputs (`ControllerRegistry`) before running the brain.
        - Only recompute sensors and brain outputs when `needsControlUpdate(dt)` is true; otherwise keep the previous control inputs.
    5. **Advance Physics**: Apply movement, boost burn, tail maintenance, and pellet eating exactly as `snake.ts`.
    6. **Grid Rebuild**: Reset collision grid and insert segment midpoints, honoring `CFG.collision.skipSegments`.
       - Hazard sensing uses the grid from the previous substep in the JS reference (grid rebuild happens after `advance`). Match that ordering unless you intentionally change parity.
    7. **Collision Resolution**:
        - Query head cell + 8 neighbors and run `pointSegmentDist2` checks.
        - Apply `hitScale`, kill snake, and award points as in `world.ts`.
    8. **Sync**: Provide `syncTo(world)` data for JS-side camera, stats, and rendering.
    9. **Serialization**: Prefer a packed `Float32Array` frame buffer compatible with `src/serializer.ts` (header + snakes + pellets) for render/UI parity; object getters are for debug only.

```rs
pub trait PhysicsBackend {
  fn step(&mut self, dt: f64);
  fn sync_to(&self, world: &mut World);
}
```

- **Backend Integration Constraints**:
  - When `backend` is active, JS still runs `World.update` bookkeeping (bot manager update, controller sensor publishing, generation timers). Either keep backend state in sync with those mutations or gate them off when backend is active.
  - If backend owns authoritative state, add a `syncFrom(world)` or equivalent input path to ingest resets, spawns, and config changes.
  - Frame buffers must match the serialized layout used by `src/serializer.ts`, `src/render.ts`, and `src/main.ts` (header fields and per-snake/pellet ordering).

```ts
export interface PhysicsBackend {
  step: (dt: number) => void;
  syncTo: (world: World) => void;
}
```

```rs
// Frame serialization (header + snakes + pellets).
// Header: generation, totalSnakes, aliveCount, worldRadius, cameraX, cameraY, zoom.
buf[0] = generation as f32;
buf[1] = total_snakes as f32;
buf[2] = alive_count as f32;
buf[3] = world_radius as f32;
buf[4] = camera_x as f32;
buf[5] = camera_y as f32;
buf[6] = zoom as f32;
// Then alive snakes: [id, radius, skin, x, y, dir, boost, pointCount, ...points]
// Then pellets: [count, x, y, value, type, colorId] repeated.
```

```rs
// Control batch buffers (stride math).
let input_stride = cfg.brain.in_size as usize;
let output_stride = cfg.brain.out_size as usize;
let max_batch = snakes.len();
batch.inputs.resize(max_batch * input_stride, 0.0);
batch.outputs.resize(max_batch * output_stride, 0.0);
batch.indices.resize(max_batch, 0);
```

## 9. Verification Matrix (The Parity Targets)

We will implement `native/src/tests.rs` containing the following mirrored test cases. Completion is defined as $100\%$ green on `cargo test`.

| Test Area | Reference Target | Numeric Requirement |
| --- | --- | --- |
| **Centered Bin Mapping** | `sensors.test.ts` (`uses centered bin mapping for v2 food bins`) | Forward pellet lands in `angleToCenteredBin(0, bins)` for configured `bins`. |
| **Wall Sensing** | `sensors.test.ts` (`normalizes v2 wall clearance by rNear`) | Wall bin near boundary is `≈ -0.8` for the test setup. |
| **Hazard Clearance** | `sensors.test.ts` (`matches hitScale when computing v2 hazard clearance`) | Hazard bin value matches JS within the test tolerance. |
| **Spatial Query** | `spatialHash.test.ts` (`queryCell uses raw cell coordinates`) | Raw cell coordinates return the inserted segment. |
| **Batch Parity** | `brains/ops.test.ts` | `DenseHead` and `MLP` batch outputs match per-sample forward within tolerance. |

```rs
#[test]
fn batch_matches_single_sample() {
  // Build deterministic weights and compare batch vs single outputs.
}
```

## 10. Execution Plan

1. **Step 1: Skeleton & Compatibility**. Add the new module layout and keep the existing `lib.rs` as a thin adapter until tests pass.
2. **Step 2: Primitives**. Implement `math.rs` and `kernels.rs` with explicit ordering and f32 truncation points. Verify with `cargo test`.
3. **Step 3: Geometry**. Implement `spatial_hash.rs`. Verify query correctness.
4. **Step 4: AI Bridge**. Implement `brain.rs` using the pre-allocated weight offsets.
5. **Step 5: Physics & Perception**. Implement `physics.rs` and `sensors.rs`. This is the core domain logic port.
6. **Step 6: Integration**. Initialize the new `lib.rs` and backend struct. Expose `step` plus a packed frame buffer API compatible with `src/serializer.ts` (object getters are optional for tooling).
7. **Step 7: Final Parity Run**. Execute the complete verification matrix. Verify in the Browser UI that snakes move and think correctly.

```bash
cd native
cargo test --all-features
```

## 11. N-API Bridge & Type Mapping Strategy

This refactor goes beyond logic; it defines a stable ABI between the JS V8 engine and the Rust runtime. We will use `napi-rs` macros to expose the following transformations:

### 11.1 Data Structure Mapping

| Rust Struct | JS Equivalent | Mapping Strategy |
| --- | --- | --- |
| `Vector2` | `{x: number, y: number}` | `#[napi(object)]` for config or debug only (allocates). |
| `WorldSettings` | `WorldSettingsInput` | Must mirror `src/world.ts` settings keys (snakeCount, simSpeed, hidden layer sizes, worldRadius, observer/collision overrides). |
| `Snake`/`Pellet` | Render data | Prefer packed `Float32Array` buffers for hot paths; only expose objects for diagnostics or tooling. |

### 11.2 The World Constructor

The backend constructor must perform the following eager allocations:

1. Initialize the `SpatialHash` with `capacity = 200,000` segments.
2. Reset pellet storage (including pellet grid) and fill to `CFG.pelletCountTarget` using ambient spawns (mirrors `World._initPellets`).
3. Allocate recurrent state and any batch buffers needed for control evaluation.

```rs
let mut world = Backend::new(settings);
world.init_pellets(cfg.pellet_count_target);
world.alloc_control_buffers(population_size);
```

```rs
// Pellet grid reset (mirrors JS pelletGrid.resetForCFG()).
pellet_grid.reset(cfg.pellet_cell_size, cfg.world_radius);
```

## 12. Memory Management & "Zero-Allocation" Policy

To keep frame times stable, avoid per-step heap allocations and reuse scratch buffers aggressively.

### 12.1 The Pointer Strategy

For the `Brain` implementation, we will NOT store weights as separate `Vec<f32>` per layer. Instead:

- The `Brain` struct will hold a single `Vec<f32>` containing the entire population's weights.
- Each layer instance will hold a `slice` into this master buffer.
- **Safety**: Use `napi::bindgen_prelude::Float32Array` for input/output transfers to align with JS `Float32Array` memory.

### 12.2 Reuse of Buffers

In the `step()` loop:

- The `Sensor` vector will be a stack-allocated buffer (or a pre-allocated fixed `Vec`) that is cleared and reused for every snake.
- The `SpatialHash` will use `.clear()` which resets the "count" and "head" pointers to `-1` but keeps the underlying memory allocated for the next tick's insertions.

```rs
scratch_food.fill(0.0);
scratch_haz.fill(r_near);
scratch_head.fill(r_near);
```

```rs
// Batch buffers reused per step.
pending_source.fill(0);
pending_turn.fill(0.0);
pending_boost.fill(0.0);
```

## 13. Advanced Physics: Integration and Boundary Conditions

### 13.1 Newtonian Step Logic

We will implement the same semi-implicit Euler step used in `snake.ts` with the exact speed lerp, turn rate scaling, and boundary death behavior (no wrap/clamp).

```rs
let turn_rate = compute_snake_turn_rate_by_len(snake.length(), cfg);
snake.dir = ang_norm(snake.dir + snake.turn_input * turn_rate * dt);
```

### 13.2 Collision Resolve

Collision occurs if `point_to_segment_dist(head, segment) < radius_sum * hit_scale`.

- **Math**: We calculate the projection of the head point onto the segment line. If the projection falls between the two endpoints, we use the perpendicular distance; otherwise, we use the distance to the nearest endpoint.

## 14. Error Handling & Stability

The Rust engine must NEVER crash (panic!) the Node.js process.

- **Bound Checks**: Validate external inputs once, then allow unchecked indexing inside hot loops after invariants are established.
- **NaN Handling**: Mirror JS guardrails (`Math.max(1e-6, ...)`, `Number.isFinite` checks, and clamps). If a value is still `NaN`, zero it before writing to output buffers to avoid cascading corruption.
- **N-API Errors**: We will return `napi::Result` and use `napi::Error` to propagate meaningful messages back to the `SimServer` for logging.

```rs
if !dt.is_finite() {
  return Err(napi::Error::from_reason("dt must be finite"));
}
```

## 15. Source-of-Truth Mapping Table

To prevent logic drift, every Rust submodule will reference these specific TypeScript/WASM source symbols.

| Rust Target | Source File | Source Symbols | Logic Summary |
| --- | --- | --- | --- |
| `kernels::dense_dot` | `legacy_wasm_reference.rs` | `dense_dot` | SIMD-unrolled dot product reference. |
| `kernels::gru_step` | `src/brains/ops.ts` | `GRU.stepReference` | GRU gating and bias logic. |
| `sensors::angle_bin` | `src/sensors.ts` | `angleToCenteredBin` | Angular bin centering logic. |
| `sensors::food_bins` | `src/sensors.ts` | `_fillFoodBinsV2` / `_fillFoodBubbleBins` | Radial pellet summation (v2/legacy). |
| `sensors::hazard_bins` | `src/sensors.ts` | `_fillHazardBinsV2` / `_fillHazardBubbleBins` | Hazard clearance (v2/legacy). |
| `sensors::wall_bins` | `src/sensors.ts` | `_fillWallBinsV2` / `_fillWallBubbleBins` | Wall clearance (v2/legacy). |
| `sensors::head_bins` | `src/sensors.ts` | `_fillHeadBinsV2` | Head-only clearance (v2). |
| `physics::integrate` | `src/snake.ts` | `advance` | Semi-implicit Euler movement. |
| `physics::turn_rate` | `src/snake.ts` | `computeSnakeTurnRateByLen` | Length-scaled turn suppression. |
| `collision::broad` | `src/spatialHash.ts` | `FlatSpatialHash` | Flat linked-list grid query. |
| `collision::narrow` | `src/world.ts` | `_resolveCollisionsGrid` | Point-segment distance check. |

## 16. Verification Procedures & Acceptance Tests

Before Phase 6 (Integration), the following command must pass with zero failures:

```bash
cd native && cargo test --all-features
```

### 16.1 Critical Verification Scenarios

- **Scenario A (The Wall Check)**: With `CFG.worldRadius = 100`, a snake at `(x=80, y=0, radius=10, dir=0)` must report approximately `-0.8` in the forward wall bin (matching `sensors.test.ts` tolerance).
- **Scenario B (The Hazard Check)**: With `CFG.collision.hitScale = 1.0`, a snake of radius 10 facing a segment centered at `(x=70, y=0)` must report hazard clearance near `0.0` in the forward bin (matching `sensors.test.ts`).
- **Scenario C (The Batch Check)**: Deterministic batch inference (`DenseHead` / `MLP`) must match per-sample inference within the same tolerance used by `brains/ops.test.ts`.

---

## Native Rust Crate Exhaustive Refactor Checkoff List

### Phase 0: Research & Specification

- [x] Analyze archived plans for detail level requirements
- [x] Identify TS behavioral tests for parity targets (`sensors.test.ts`, `spatialHash.test.ts`, `brains/ops.test.ts`)
- [x] **EXHAUSTIVE PLAN READY**: Massive `native_refactor_plan.md` created with hundreds of paragraphs.

### Phase 1: The Great Gutting

- [ ] Approval by User.
- [ ] Add the modular structure (`math.rs`, `kernels.rs`, `spatial_hash.rs`, `brain.rs`, `physics.rs`, `sensors.rs`) alongside the existing `lib.rs`.
- [ ] Replace `lib.rs` only after the module path passes the parity checks.

### Phase 2: Primitives & Kernels

- [ ] Implement `native/src/math.rs`
  - [ ] `sigmoid` (f64, exact parity with `Math.exp`)
  - [ ] `tanh` (f64, exact parity with `Math.tanh`)
  - [ ] `ang_norm` (range `[-PI, PI]`, multi-wrap safety)
  - [ ] `lerp` (f64)
  - [ ] `clamp` (f64)
- [ ] Implement `native/src/kernels.rs` (OG WASM Port)
  - [ ] `dense_dot` (Fixed-order loop; f64 accumulate, f32 truncate on write)
  - [ ] `gru_step` (Update, Reset, Candidate logic)
  - [ ] `lstm_step` (Forget bias 0.6 logic)
  - [ ] `rru_step` (Minimal gating logic)

### Phase 3: Geometry & Brains

- [ ] Implement `native/src/spatial_hash.rs`
  - [ ] `head`, `next`, `ent`, `seg` buffer allocation.
  - [ ] `insert` (offset mapping, cell floor parity).
  - [ ] `query_cell` (Linked-list traversal parity).
- [ ] Implement `native/src/brain.rs`
  - [ ] Weight buffer slicing (ArchInfo parity).
  - [ ] Recurrent state persistence (zero allocation update).

### Phase 4: Perception & Integrators

- [ ] Implement `native/src/sensors.rs` (Parity Logic)
  - [ ] `angle_to_centered_bin` (Bin 0 = Forward).
  - [ ] **Wall Bins** (Ray-Circle intersection math).
  - [ ] **Food Bins** (Spatial query + distance weighting).
  - [ ] **Hazard Bins** (hitScale + minimum clearance logic).
  - [ ] **Head Bins** (v2 head-only clearance).
  - [ ] **Speed/Boost Scalars** (v2 indices 5/6).
- [ ] Implement `native/src/physics.rs`
  - [ ] `Snake` integration (semi-implicit Euler, `lerp(..., 1 - exp(-dt * 6.5))`).
  - [ ] Turn rate scaling via `computeSnakeTurnRateByLen`.
  - [ ] Boundary death on `hypot(x, y) + radius >= CFG.worldRadius`.
  - [ ] Tail tracking (Fixed spacing `snakeSpacing` maintenance).

### Phase 5: Verification Suite (`tests.rs`)

- [ ] `test_math_parity` (Sigmoid/Tanh/AngNorm incl. large-magnitude angles)
- [ ] `test_kernel_parity` (DenseHead/MLP batch vs single-sample parity)
- [ ] `test_hash_parity` (Insertion/Query coordination)
- [ ] `test_sensor_parity` (Centered bin mapping + wall/hazard/head checks)
- [ ] `test_physics_parity` (Speed lerp + boundary death path)
- [ ] `test_no_alloc_in_step` (Allocator instrumentation or capacity-stability sentinel)

### Phase 6: Integration & Sync

- [ ] Finalize `native/src/lib.rs` (N-API PhysicsBackend bridge + `syncTo`).
- [ ] Execute `napi build`.
- [ ] Verify `World` <-> Native backend sync in browser.

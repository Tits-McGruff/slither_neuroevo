#![deny(clippy::all)]

use napi_derive::napi;
use std::f64::consts::PI;

// --- Constants (Matching src/config.ts defaults) ---
const SNAKE_RADIUS: f64 = 9.0;
const SNAKE_RADIUS_MAX: f64 = 18.0;
const SNAKE_THICKNESS_SCALE: f64 = 2.9;
const SNAKE_THICKNESS_LOG_DIV: f64 = 30.0;
const SNAKE_START_LEN: f64 = 5.0;
const SNAKE_MAX_LEN: f64 = 10000.0;
const SNAKE_TURN_RATE: f64 = 3.2;
const SNAKE_TURN_PENALTY: f64 = 1.4;
const WORLD_RADIUS: f64 = 3500.0; // Default fallback

// --- Math Helpers ---

#[inline(always)]
fn sigmoid(x: f64) -> f64 {
  1.0 / (1.0 + (-x).exp())
}

#[inline(always)]
fn tanh(x: f64) -> f64 {
  x.tanh()
}

#[inline(always)]
fn clamp(x: f64, min: f64, max: f64) -> f64 {
  if x < min {
    min
  } else if x > max {
    max
  } else {
    x
  }
}

#[inline(always)]
fn ang_norm(a: f64) -> f64 {
  let mut x = a % (2.0 * PI);
  if x < -PI {
    x += 2.0 * PI;
  }
  if x > PI {
    x -= 2.0 * PI;
  }
  x
}

// --- Data Structures ---

#[napi(object)]
#[derive(Clone, Debug)]
pub struct Vector2 {
  pub x: f64,
  pub y: f64,
}

#[napi(object)]
#[derive(Clone, Debug)]
pub struct WorldSettings {
  pub world_radius: f64,
  pub snake_count: u32,
  pub pellet_count: u32,
  pub tick_rate: u32,
}

#[napi(object)]
#[derive(Clone, Debug)]
pub struct Pellet {
  pub x: f64,
  pub y: f64,
  pub value: f64,
  pub type_: String,
}

// --- Neural Network Logic ---

#[derive(Clone, Debug)]
enum LayerType {
  Dense,
  Gru,
  Lstm,
  Rru,
}

#[napi(object)]
#[derive(Clone, Debug)]
pub struct Brain {
  pub weights: Vec<f64>,
  pub input_size: u32,
  pub layer_sizes: Vec<u32>,
  /// Flattened layer types: 0=Dense, 1=Gru, 2=Lstm, 3=Rru.
  /// Matches the stack order.
  pub layer_types: Vec<u32>,
  /// Recurrent state buffer (flattened)
  pub state: Vec<f64>,
}

impl Brain {
  fn forward(&mut self, inputs: &[f64]) -> Vec<f64> {
    let mut activations = inputs.to_vec();
    let mut weight_offset = 0;
    let mut state_offset = 0;

    let sizes = self.layer_sizes.clone();
    let types = self.layer_types.clone();

    for (i, &layer_size) in sizes.iter().enumerate() {
      let _in_size = activations.len();
      let out_size = layer_size as usize;
      let layer_type_code = types.get(i).cloned().unwrap_or(0); // Default Dense

      let next_activations = match layer_type_code {
        0 => self.forward_dense(&activations, out_size, &mut weight_offset),
        1 => self.forward_gru(
          &activations,
          out_size,
          &mut weight_offset,
          &mut state_offset,
        ),
        2 => self.forward_lstm(
          &activations,
          out_size,
          &mut weight_offset,
          &mut state_offset,
        ),
        3 => self.forward_rru(
          &activations,
          out_size,
          &mut weight_offset,
          &mut state_offset,
        ),
        _ => self.forward_dense(&activations, out_size, &mut weight_offset), // Fallback
      };
      activations = next_activations;
    }
    activations
  }

  fn forward_dense(&self, inputs: &[f64], out_size: usize, w_off: &mut usize) -> Vec<f64> {
    let mut output = Vec::with_capacity(out_size);
    let in_size = inputs.len();
    for _ in 0..out_size {
      let mut sum = 0.0;
      for k in 0..in_size {
        sum += self.weights[*w_off] * inputs[k];
        *w_off += 1;
      }
      sum += self.weights[*w_off]; // Bias
      *w_off += 1;
      output.push(tanh(sum));
    }
    output
  }

  fn forward_gru(
    &mut self,
    inputs: &[f64],
    hidden: usize,
    w_off: &mut usize,
    s_off: &mut usize,
  ) -> Vec<f64> {
    let in_size = inputs.len();
    let current_state_start = *s_off;
    let mut next_h = Vec::with_capacity(hidden);

    let wz_base = *w_off;
    let wr_base = wz_base + hidden * in_size;
    let wh_base = wr_base + hidden * in_size;
    let uz_base = wh_base + hidden * in_size;
    let ur_base = uz_base + hidden * hidden;
    let uh_base = ur_base + hidden * hidden;
    let bz_base = uh_base + hidden * hidden;
    let br_base = bz_base + hidden;
    let bh_base = br_base + hidden;

    *w_off = bh_base + hidden;

    for j in 0..hidden {
      let prev_h = self.state[current_state_start + j];

      // Z Gate
      let mut sum_z = self.weights[bz_base + j];
      for k in 0..in_size {
        sum_z += self.weights[wz_base + j * in_size + k] * inputs[k];
      }
      for k in 0..hidden {
        sum_z += self.weights[uz_base + j * hidden + k] * self.state[current_state_start + k];
      }
      let z = sigmoid(sum_z);

      // R Gate
      let mut sum_r = self.weights[br_base + j];
      for k in 0..in_size {
        sum_r += self.weights[wr_base + j * in_size + k] * inputs[k];
      }
      for k in 0..hidden {
        sum_r += self.weights[ur_base + j * hidden + k] * self.state[current_state_start + k];
      }
      let r = sigmoid(sum_r);

      // Candidate H
      let mut sum_h = self.weights[bh_base + j];
      for k in 0..in_size {
        sum_h += self.weights[wh_base + j * in_size + k] * inputs[k];
      }
      for k in 0..hidden {
        sum_h += self.weights[uh_base + j * hidden + k] * (r * self.state[current_state_start + k]);
      }
      let h_tilde = tanh(sum_h);

      let new_h = (1.0 - z) * prev_h + z * h_tilde;
      next_h.push(new_h);
    }

    for (j, &val) in next_h.iter().enumerate() {
      self.state[current_state_start + j] = val;
    }
    *s_off += hidden;
    next_h
  }

  fn forward_lstm(
    &mut self,
    inputs: &[f64],
    hidden: usize,
    w_off: &mut usize,
    s_off: &mut usize,
  ) -> Vec<f64> {
    let in_size = inputs.len();
    let state_h_start = *s_off;
    let state_c_start = *s_off + hidden;
    let mut next_h = Vec::with_capacity(hidden);

    let wi_base = *w_off;
    let wf_base = wi_base + hidden * in_size;
    let wo_base = wf_base + hidden * in_size;
    let wg_base = wo_base + hidden * in_size;

    let ui_base = wg_base + hidden * in_size;
    let uf_base = ui_base + hidden * hidden;
    let uo_base = uf_base + hidden * hidden;
    let ug_base = uo_base + hidden * hidden;

    let bi_base = ug_base + hidden * hidden;
    let bf_base = bi_base + hidden;
    let bo_base = bf_base + hidden;
    let bg_base = bo_base + hidden;

    *w_off = bg_base + hidden;

    for j in 0..hidden {
      let prev_h = self.state[state_h_start + j];
      let prev_c = self.state[state_c_start + j];

      // I Gate
      let mut sum_i = self.weights[bi_base + j];
      for k in 0..in_size {
        sum_i += self.weights[wi_base + j * in_size + k] * inputs[k];
      }
      for k in 0..hidden {
        sum_i += self.weights[ui_base + j * hidden + k] * self.state[state_h_start + k];
      }
      let i_gate = sigmoid(sum_i);

      // F Gate
      let mut sum_f = self.weights[bf_base + j];
      for k in 0..in_size {
        sum_f += self.weights[wf_base + j * in_size + k] * inputs[k];
      }
      for k in 0..hidden {
        sum_f += self.weights[uf_base + j * hidden + k] * self.state[state_h_start + k];
      }
      let f_gate = sigmoid(sum_f);

      // O Gate
      let mut sum_o = self.weights[bo_base + j];
      for k in 0..in_size {
        sum_o += self.weights[wo_base + j * in_size + k] * inputs[k];
      }
      for k in 0..hidden {
        sum_o += self.weights[uo_base + j * hidden + k] * self.state[state_h_start + k];
      }
      let o_gate = sigmoid(sum_o);

      // G Gate (Cell Input)
      let mut sum_g = self.weights[bg_base + j];
      for k in 0..in_size {
        sum_g += self.weights[wg_base + j * in_size + k] * inputs[k];
      }
      for k in 0..hidden {
        sum_g += self.weights[ug_base + j * hidden + k] * self.state[state_h_start + k];
      }
      let g_gate = tanh(sum_g);

      let next_c = f_gate * prev_c + i_gate * g_gate;
      let next_h_val = o_gate * tanh(next_c);

      next_h.push((next_h_val, next_c));
    }

    for (j, (nh, nc)) in next_h.iter().enumerate() {
      self.state[state_h_start + j] = *nh;
      self.state[state_c_start + j] = *nc;
    }
    *s_off += 2 * hidden;

    next_h.iter().map(|(h, _)| *h).collect()
  }

  fn forward_rru(
    &mut self,
    inputs: &[f64],
    hidden: usize,
    w_off: &mut usize,
    s_off: &mut usize,
  ) -> Vec<f64> {
    let in_size = inputs.len();
    let current_state_start = *s_off;
    let mut next_h = Vec::with_capacity(hidden);

    let wc_base = *w_off;
    let wr_base = wc_base + hidden * in_size;
    let uc_base = wr_base + hidden * in_size;
    let ur_base = uc_base + hidden * hidden;
    let bc_base = ur_base + hidden * hidden;
    let br_base = bc_base + hidden;

    *w_off = br_base + hidden;

    for j in 0..hidden {
      let prev_h = self.state[current_state_start + j];

      // Candidate C
      let mut sum_c = self.weights[bc_base + j];
      for k in 0..in_size {
        sum_c += self.weights[wc_base + j * in_size + k] * inputs[k];
      }
      for k in 0..hidden {
        sum_c += self.weights[uc_base + j * hidden + k] * self.state[current_state_start + k];
      }
      let cand = tanh(sum_c);

      // Reu Gate R
      let mut sum_r = self.weights[br_base + j];
      for k in 0..in_size {
        sum_r += self.weights[wr_base + j * in_size + k] * inputs[k];
      }
      for k in 0..hidden {
        sum_r += self.weights[ur_base + j * hidden + k] * self.state[current_state_start + k];
      }
      let gate = sigmoid(sum_r);

      let new_h = (1.0 - gate) * prev_h + gate * cand;
      next_h.push(new_h);
    }

    for (j, &val) in next_h.iter().enumerate() {
      self.state[current_state_start + j] = val;
    }
    *s_off += hidden;
    next_h
  }
}

// --- Physics & Collision ---

struct SpatialHash {
  cell_size: f64,
  cells: std::collections::HashMap<(i32, i32), Vec<usize>>,
}

impl SpatialHash {
  fn new(cell_size: f64) -> Self {
    SpatialHash {
      cell_size,
      cells: std::collections::HashMap::new(),
    }
  }

  fn clear(&mut self) {
    self.cells.clear();
  }

  fn insert(&mut self, x: f64, y: f64, id: usize) {
    let cx = (x / self.cell_size).floor() as i32;
    let cy = (y / self.cell_size).floor() as i32;
    self.cells.entry((cx, cy)).or_default().push(id);
  }

  fn query(&self, x: f64, y: f64, radius: f64) -> Vec<usize> {
    let mut result = Vec::new();
    let min_cx = ((x - radius) / self.cell_size).floor() as i32;
    let max_cx = ((x + radius) / self.cell_size).floor() as i32;
    let min_cy = ((y - radius) / self.cell_size).floor() as i32;
    let max_cy = ((y + radius) / self.cell_size).floor() as i32;

    for cx in min_cx..=max_cx {
      for cy in min_cy..=max_cy {
        if let Some(ids) = self.cells.get(&(cx, cy)) {
          result.extend_from_slice(ids);
        }
      }
    }
    result
  }
}

#[napi(object)]
#[derive(Clone, Debug)]
pub struct Snake {
  pub id: u32,
  pub x: f64,
  pub y: f64,
  pub dir: f64,
  pub alive: bool,
  pub points_score: f64,
  pub brain: Brain,
  pub speed: f64,
  pub radius: f64,
  pub target_len: f64,
  pub boost: f64,
  pub turn_input: f64,
  pub points: Vec<Vector2>,
}

#[napi]
pub struct World {
  settings: WorldSettings,
  pub tick_id: u32,
  pellets: Vec<Pellet>,
  snakes: Vec<Snake>,
  grid: SpatialHash,
}

#[napi]
impl World {
  #[napi(constructor)]
  pub fn new(settings: WorldSettings) -> Self {
    let cell_size = 50.0;
    World {
      settings,
      tick_id: 0,
      pellets: Vec::new(),
      snakes: Vec::new(),
      grid: SpatialHash::new(cell_size),
    }
  }

  #[napi]
  pub fn step(&mut self) {
    self.tick_id += 1;
    let radius = self.settings.world_radius;

    // 1. Rebuild Grid First
    self.grid.clear();
    for (i, snake) in self.snakes.iter().enumerate() {
      if snake.alive {
        // Simplified head insertion for parity stability check
        self.grid.insert(snake.x, snake.y, i);
      }
    }

    // 2. Brain & Physics Step
    for i in 0..self.snakes.len() {
      if !self.snakes[i].alive {
        continue;
      }

      // --- SENSORS: Wall/Food/Haz Bubbles ---
      let snake = &self.snakes[i];
      let input_size = snake.brain.input_size as usize;
      // Default to zero if mismatched
      let mut sensors = vec![0.0; input_size];

      // 0-1: Heading
      sensors[0] = snake.dir.sin();
      sensors[1] = snake.dir.cos();

      // 2: Size Fraction
      let denom = (SNAKE_MAX_LEN - SNAKE_START_LEN).max(1.0);
      let len = snake.points.len() as f64;
      let size_norm = clamp((len - SNAKE_START_LEN) / denom, 0.0, 1.0);
      sensors[2] = clamp(size_norm * 2.0 - 1.0, -1.0, 1.0);

      // 3: Boost Margin (placeholder)
      sensors[3] = 1.0;

      // 4: Log Percentile (placeholder)
      sensors[4] = 0.0;

      // 5: Speed Ratio
      // 6: Boost Ratio

      // Bubble Logic (bins)
      let bins = 16;
      let r_near = 520.0 + 260.0 * size_norm;

      // Wall Bins (Offset 7 + bins + bins if using V2, or similar)
      // Hardcoding standard V2 layout offsets for parity check
      // Food: 7, Haz: 7+bins, Wall: 7+2*bins
      let wall_off = 7 + bins + bins;
      if wall_off + bins <= sensors.len() {
        for b in 0..bins {
          let theta = snake.dir + (-PI + (b as f64 / bins as f64) * 2.0 * PI);
          let ux = theta.cos();
          let uy = theta.sin();
          let bb = snake.x * ux + snake.y * uy;
          let cc = snake.x * snake.x + snake.y * snake.y - radius * radius;
          let disc = bb * bb - cc;
          let dist = if disc <= 0.0 { 0.0 } else { -bb + disc.sqrt() };
          let ratio = clamp(dist / r_near, 0.0, 1.0);
          sensors[wall_off + b] = ratio * 2.0 - 1.0;
        }
      }

      // Inference
      let output = self.snakes[i].brain.forward(&sensors);

      // Apply
      if !output.is_empty() {
        let turn = output[0];
        let penalty = 1.0 + SNAKE_TURN_PENALTY * size_norm;
        let rate = SNAKE_TURN_RATE / penalty;
        self.snakes[i].dir += turn * rate * 0.016;
      }

      // Move
      let speed = 165.0;
      self.snakes[i].x += self.snakes[i].dir.cos() * speed * 0.016;
      self.snakes[i].y += self.snakes[i].dir.sin() * speed * 0.016;

      // Wrap
      let dist = (self.snakes[i].x.powi(2) + self.snakes[i].y.powi(2)).sqrt();
      if dist > radius {
        let angle = self.snakes[i].y.atan2(self.snakes[i].x);
        self.snakes[i].x = angle.cos() * radius;
        self.snakes[i].y = angle.sin() * radius;
      }
    }

    // 3. Resolve Collisions
    let collision_radius = 10.0;
    let mut dead_indices = Vec::new();

    for (i, snake) in self.snakes.iter().enumerate() {
      if !snake.alive {
        continue;
      }
      let neighbors = self.grid.query(snake.x, snake.y, collision_radius);
      for &other_idx in &neighbors {
        if i != other_idx {
          let other = &self.snakes[other_idx];
          if other.alive {
            let dx = snake.x - other.x;
            let dy = snake.y - other.y;
            if (dx * dx + dy * dy) < (collision_radius * collision_radius) {
              dead_indices.push(i);
              break;
            }
          }
        }
      }
    }

    for idx in dead_indices {
      self.snakes[idx].alive = false;
    }
  }

  #[napi]
  pub fn get_settings(&self) -> WorldSettings {
    self.settings.clone()
  }
  #[napi]
  pub fn get_snakes(&self) -> Vec<Snake> {
    self.snakes.clone()
  }
  #[napi]
  pub fn get_pellets(&self) -> Vec<Pellet> {
    self.pellets.clone()
  }
}

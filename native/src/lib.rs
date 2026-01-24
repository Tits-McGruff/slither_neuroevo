#![deny(clippy::all)]

use napi_derive::napi;

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

#[napi(object)]
#[derive(Clone, Debug)]
pub struct Brain {
  pub weights: Vec<f64>,
  pub input_size: u32,
  pub layer_sizes: Vec<u32>,
}

impl Brain {
  fn forward(&self, inputs: &[f64]) -> Vec<f64> {
    let mut activations = inputs.to_vec();
    let mut offset = 0;

    for (i, &layer_size) in self.layer_sizes.iter().enumerate() {
      let in_size = activations.len();
      let out_size = layer_size as usize;
      let mut next_activations = Vec::with_capacity(out_size);

      for _ in 0..out_size {
        let mut sum = 0.0;
        for k in 0..in_size {
          sum += self.weights[offset] * activations[k];
          offset += 1;
        }
        // Bias
        sum += self.weights[offset];
        offset += 1;

        // Activation (Tanh)
        next_activations.push(sum.tanh());
      }
      activations = next_activations;
    }
    activations
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
}

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

  #[allow(dead_code)]
  fn clear(&mut self) {
    self.cells.clear();
  }

  #[allow(dead_code)]
  fn insert(&mut self, x: f64, y: f64, id: usize) {
    let cx = (x / self.cell_size).floor() as i32;
    let cy = (y / self.cell_size).floor() as i32;
    self.cells.entry((cx, cy)).or_default().push(id);
  }

  #[allow(dead_code)]
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
    let cell_size = 50.0; // Hardcoded default for now
    World {
      settings,
      tick_id: 0,
      pellets: Vec::new(),
      snakes: Vec::new(),
      grid: SpatialHash::new(cell_size),
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

  #[napi]
  pub fn step(&mut self) {
    self.tick_id += 1;
    let radius = self.settings.world_radius;

    // 1. Brain Step: Compute sensors and run inference
    for snake in &mut self.snakes {
      if !snake.alive {
        continue;
      }

      // TODO: Implement actual raycasting. For now, use dummy sensors to test pipe.
      // Input size must match brain input size.
      let input_size = snake.brain.input_size as usize;
      let sensors = vec![0.0; input_size];

      let output = snake.brain.forward(&sensors);

      // Apply outputs
      // Output 0: Turn [-1, 1]
      // Output 1: Boost [0, 1] (if present)
      if !output.is_empty() {
        let turn = output[0];
        snake.dir += turn * 0.1; // Simple turn rate
      }
    }

    // 2. Physics Step: Move snakes
    for snake in &mut self.snakes {
      if !snake.alive {
        continue;
      }

      let speed = 1.0;
      snake.x += snake.dir.cos() * speed;
      snake.y += snake.dir.sin() * speed;

      let dist = (snake.x * snake.x + snake.y * snake.y).sqrt();
      if dist > radius {
        let angle = snake.y.atan2(snake.x);
        snake.x = angle.cos() * radius;
        snake.y = angle.sin() * radius;
      }
    }

    // 3. Rebuild Grid
    self.grid.clear();
    for (i, snake) in self.snakes.iter().enumerate() {
      if snake.alive {
        self.grid.insert(snake.x, snake.y, i);
      }
    }

    // 4. Resolve Collisions
    let collision_radius = 10.0; // Hardcoded radius for now
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
            // Simple distance check
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
}

// utils.ts
// General utility functions and constants used throughout the simulation.

/** Two-pi constant for common trigonometric calculations. */
export const TAU = Math.PI * 2;

/**
 * Deeply clones a plain object via JSON serialisation.
 * @param obj - Object to clone.
 * @returns Deep-cloned object.
 */
export function deepClone<T>(obj: T): T {
  return JSON.parse(JSON.stringify(obj)) as T;
}

/**
 * Constrains a value to the inclusive range [a, b].
 * @param x - Value to clamp.
 * @param a - Minimum value.
 * @param b - Maximum value.
 * @returns Clamped value.
 */
export function clamp(x: number, a: number, b: number): number {
  return Math.max(a, Math.min(b, x));
}

/**
 * Linear interpolation between a and b by t.
 * @param a - Start value.
 * @param b - End value.
 * @param t - Interpolation factor in [0,1].
 * @returns Interpolated value.
 */
export function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

/**
 * Uniform random number between b and a (defaults to [0,1]).
 * @param a - Max value (default 1).
 * @param b - Min value (default 0).
 * @returns Random value in [b,a].
 */
export function rand(a = 1, b = 0): number {
  return b + Math.random() * (a - b);
}

/**
 * Returns a random integer in [0, n).
 * @param n - Exclusive upper bound.
 * @returns Random integer in [0,n).
 */
export function randInt(n: number): number {
  return (Math.random() * n) | 0;
}

/**
 * Hypotenuse helper that wraps Math.hypot for brevity.
 * @param x - X component.
 * @param y - Y component.
 * @returns Hypotenuse length.
 */
export function hypot(x: number, y: number): number {
  return Math.hypot(x, y);
}

/**
 * Normalises an angle to the range [−π, π].
 * @param a - Angle in radians.
 * @returns Normalized angle in radians.
 */
export function angNorm(a: number): number {
  while (a > Math.PI) a -= TAU;
  while (a < -Math.PI) a += TAU;
  return a;
}

/**
 * Retrieves a nested value from an object given a dot‑separated path.
 * @param obj - Object to read from.
 * @param path - Dot-separated path to read.
 */
export function getByPath(obj: object, path: string): unknown {
  const parts = path.split(".");
  let cur: unknown = obj as Record<string, unknown>;
  for (const p of parts) cur = (cur as Record<string, unknown>)[p];
  return cur;
}

/**
 * Sets a nested value on an object given a dot‑separated path.
 * @param obj - Object to mutate.
 * @param path - Dot-separated path to set.
 * @param value - Value to assign.
 */
export function setByPath(obj: object, path: string, value: unknown): void {
  const parts = path.split(".");
  let cur: Record<string, unknown> = obj as Record<string, unknown>;
  for (let i = 0; i < parts.length - 1; i++) {
    const key = parts[i];
    if (key == null) return;
    cur = (cur as Record<string, unknown>)[key] as Record<string, unknown>;
  }
  const lastKey = parts[parts.length - 1];
  if (lastKey == null) return;
  cur[lastKey] = value;
}

/**
 * Formats a number to a fixed number of decimal places.  Integers are rounded.
 * @param x - Number to format.
 * @param decimals - Number of decimal places.
 * @returns Formatted string.
 */
export function fmtNumber(x: number, decimals: number): string {
  if (decimals === 0) return String(Math.round(x));
  return Number(x).toFixed(decimals);
}

/**
 * Converts HSV colour to an RGB tuple.  Helper for generating distinct snake colours.
 * @param h - Hue component in [0,1].
 * @param s - Saturation component in [0,1].
 * @param v - Value component in [0,1].
 * @returns RGB tuple in [0,255] integer space.
 */
export function hsvToRgb(h: number, s: number, v: number): [number, number, number] {
  const i = Math.floor(h * 6);
  const f = h * 6 - i;
  const p = v * (1 - s);
  const q = v * (1 - f * s);
  const t = v * (1 - (1 - f) * s);
  let r = 0;
  let g = 0;
  let b = 0;
  switch (i % 6) {
    case 0:
      r = v;
      g = t;
      b = p;
      break;
    case 1:
      r = q;
      g = v;
      b = p;
      break;
    case 2:
      r = p;
      g = v;
      b = t;
      break;
    case 3:
      r = p;
      g = q;
      b = v;
      break;
    case 4:
      r = t;
      g = p;
      b = v;
      break;
    case 5:
      r = v;
      g = p;
      b = q;
      break;
  }
  return [(r * 255) | 0, (g * 255) | 0, (b * 255) | 0];
}

/**
 * Generates a pseudo‑random but evenly distributed colour based on an index.
 * @param i - Index used to seed the color.
 * @returns RGB color string.
 */
export function hashColor(i: number): string {
  const h = (i * 0.61803398875) % 1;
  const rgb = hsvToRgb(h, 0.65, 0.95);
  return `rgb(${rgb[0]},${rgb[1]},${rgb[2]})`;
}

/**
 * Generates a normally distributed random number with mean 0 and variance 1.
 * Uses the Box–Muller transform.
 * @returns Standard normal sample.
 */
export function gaussian(): number {
  let u = 0,
    v = 0;
  while (u === 0) u = Math.random();
  while (v === 0) v = Math.random();
  return Math.sqrt(-2.0 * Math.log(u)) * Math.cos(TAU * v);
}

// utils.js
// General utility functions and constants used throughout the simulation.

// Turn two‑pi constant for common trigonometric calculations.
export const TAU = Math.PI * 2;

/**
 * Deeply clones a plain object via JSON serialisation.
 * @param {Object} obj
 * @returns {Object}
 */
export function deepClone(obj) {
  return JSON.parse(JSON.stringify(obj));
}

/**
 * Constrains a value to the inclusive range [a, b].
 * @param {number} x
 * @param {number} a
 * @param {number} b
 * @returns {number}
 */
export function clamp(x, a, b) {
  return Math.max(a, Math.min(b, x));
}

/**
 * Linear interpolation between a and b by t.
 * @param {number} a
 * @param {number} b
 * @param {number} t
 * @returns {number}
 */
export function lerp(a, b, t) {
  return a + (b - a) * t;
}

/**
 * Uniform random number between b and a (defaults to [0,1]).
 * @param {number} [a=1]
 * @param {number} [b=0]
 * @returns {number}
 */
export function rand(a = 1, b = 0) {
  return b + Math.random() * (a - b);
}

/**
 * Returns a random integer in [0, n).
 * @param {number} n
 * @returns {number}
 */
export function randInt(n) {
  return (Math.random() * n) | 0;
}

/**
 * Hypotenuse helper that wraps Math.hypot for brevity.
 * @param {number} x
 * @param {number} y
 * @returns {number}
 */
export function hypot(x, y) {
  return Math.hypot(x, y);
}

/**
 * Normalises an angle to the range [−π, π].
 * @param {number} a
 * @returns {number}
 */
export function angNorm(a) {
  while (a > Math.PI) a -= TAU;
  while (a < -Math.PI) a += TAU;
  return a;
}

/**
 * Retrieves a nested value from an object given a dot‑separated path.
 * @param {Object} obj
 * @param {string} path
 */
export function getByPath(obj, path) {
  const parts = path.split(".");
  let cur = obj;
  for (const p of parts) cur = cur[p];
  return cur;
}

/**
 * Sets a nested value on an object given a dot‑separated path.
 * @param {Object} obj
 * @param {string} path
 * @param {any} value
 */
export function setByPath(obj, path, value) {
  const parts = path.split(".");
  let cur = obj;
  for (let i = 0; i < parts.length - 1; i++) cur = cur[parts[i]];
  cur[parts[parts.length - 1]] = value;
}

/**
 * Formats a number to a fixed number of decimal places.  Integers are rounded.
 * @param {number} x
 * @param {number} decimals
 * @returns {string}
 */
export function fmtNumber(x, decimals) {
  if (decimals === 0) return String(Math.round(x));
  return Number(x).toFixed(decimals);
}

/**
 * Converts HSV colour to an RGB tuple.  Helper for generating distinct snake colours.
 * @param {number} h
 * @param {number} s
 * @param {number} v
 * @returns {Array<number>}
 */
export function hsvToRgb(h, s, v) {
  const i = Math.floor(h * 6);
  const f = h * 6 - i;
  const p = v * (1 - s);
  const q = v * (1 - f * s);
  const t = v * (1 - (1 - f) * s);
  let r, g, b;
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
 * @param {number} i
 * @returns {string}
 */
export function hashColor(i) {
  const h = (i * 0.61803398875) % 1;
  const rgb = hsvToRgb(h, 0.65, 0.95);
  return `rgb(${rgb[0]},${rgb[1]},${rgb[2]})`;
}

/**
 * Generates a normally distributed random number with mean 0 and variance 1.
 * Uses the Box–Muller transform.
 * @returns {number}
 */
export function gaussian() {
  let u = 0,
    v = 0;
  while (u === 0) u = Math.random();
  while (v === 0) v = Math.random();
  return Math.sqrt(-2.0 * Math.log(u)) * Math.cos(TAU * v);
}
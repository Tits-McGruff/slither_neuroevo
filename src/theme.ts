// theme.ts
// Centralized color palette and theme management for the simulation.
// Using HSLA helper for consistent, vibrant visualization.

import { deepClone } from './utils.ts';

export const THEME = {
  // Background/Grid
  gridLine: 'rgba(255,255,255,0.04)',
  worldBorder: 'rgba(255,255,255,0.08)',
  
  // Snake
  snakeSelfEye: 'rgba(0,0,0,0.55)',
  snakeSnakeSelfHalo: 'rgba(255,255,255,0.55)',
  snakeBoostRefill: 'rgba(100,255,100,0.7)',
  snakeBoostActive: 'rgba(255,100,100,0.8)',
  snakeSteerArrow: 'rgba(255,255,0,0.8)',
  snakeWallGauge: 'rgba(100,200,255,0.5)',

  // Pellets
  pelletAmbient: 'rgba(180, 200, 255, 0.75)',
  pelletBoost: 'rgba(255, 120, 120, 0.85)', // Slightly reddish for boost drops
  pelletCorpse: 'rgba(255, 220, 100, 0.9)', // Golden for corpse

  // Glows (shadow colors)
  glowAmbient: '#aaccff',
  glowBoost: '#ff5555',
  glowCorpse: '#ffcc00',
  glowSnake: '#ffffff'
};

/**
 * Returns a CSS color string for a given pellet type.
 */
export function getPelletColor(p: { color?: string | null; kind?: string; [key: string]: unknown }): string {
  if (p.color) return p.color;
  switch (p.kind) {
    case 'boost': return THEME.pelletBoost;
    case 'corpse_big': 
    case 'corpse_small': return THEME.pelletCorpse;
    default: return THEME.pelletAmbient;
  }
}

/**
 * Returns the glow color (shadowColor) for a given pellet type.
 */
export function getPelletGlow(p: { kind?: string; [key: string]: unknown }): string {
  switch (p.kind) {
    case 'boost': return THEME.glowBoost;
    case 'corpse_big':
    case 'corpse_small': return THEME.glowCorpse;
    default: return THEME.glowAmbient;
  }
}

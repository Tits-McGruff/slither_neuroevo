/** Generate a retained current-TypeScript binary display-frame v1 fixture. */

import { WorldSerializer } from '../../src/serializer.ts';

/** Git revision whose current serializer source is executed by this fixture. */
const SOURCE_REVISION = '7925faf7aef33bd3de3e1b6d3c021c4320a8dd68';
/** Largest integer exactly represented by the Float32 frame-v1 ID field. */
const MAX_EXACT_FRAME_ID = 16_777_216;

/** Presentation-only values explicitly echoed into the compatibility header. */
const VIEW = {
  cameraX: 12.75,
  cameraY: -44.5,
  zoom: 0.625
} as const;

/** Rust-facing source values, kept separate from TypeScript object layout. */
const SOURCE = {
  generation: 7,
  worldRadius: 3500,
  snakes: [
    {
      id: 1,
      frameV1Id: 17,
      radius: 10.25,
      skin: 2,
      x: 12.5,
      y: -3.25,
      direction: Math.PI / 3,
      boost: true,
      alive: true,
      body: [
        { x: 12.5, y: -3.25 },
        { x: 8, y: -1 },
        { x: 2, y: 4.5 }
      ]
    },
    {
      id: 2,
      frameV1Id: 18,
      radius: 10.25,
      skin: 0,
      x: 99,
      y: 99,
      direction: Math.PI / 3,
      boost: true,
      alive: false,
      body: []
    },
    {
      id: 3,
      frameV1Id: MAX_EXACT_FRAME_ID,
      radius: 6.5,
      skin: 1,
      x: -9,
      y: 7,
      direction: -1.2,
      boost: false,
      alive: true,
      body: [{ x: -9, y: 7 }]
    }
  ],
  pellets: [
    { id: 10, x: 1, y: 2, value: 1.5, kind: 0, color: 0 },
    { id: 11, x: -3, y: 4, value: 2, kind: 1, color: 17 },
    {
      id: 12,
      x: 5.25,
      y: -6.75,
      value: 0.75,
      kind: 2,
      color: MAX_EXACT_FRAME_ID
    },
    { id: 13, x: 8.5, y: 9.25, value: 0.25, kind: 3, color: 0 }
  ]
} as const;

/** Convert the retained numeric kind into the current serializer's string input. */
function pelletKind(kind: number): 'ambient' | 'corpse_big' | 'corpse_small' | 'boost' {
  if (kind === 1) return 'corpse_big';
  if (kind === 2) return 'corpse_small';
  if (kind === 3) return 'boost';
  return 'ambient';
}

/** Encode one Float32 by its exact big-endian-readable bit pattern. */
function float32Hex(value: number): string {
  const buffer = new ArrayBuffer(4);
  const view = new DataView(buffer);
  view.setFloat32(0, value, false);
  return `0x${view.getUint32(0, false).toString(16).padStart(8, '0')}`;
}

const frame = WorldSerializer.serialize({
  generation: SOURCE.generation,
  worldRadius: SOURCE.worldRadius,
  cameraX: VIEW.cameraX,
  cameraY: VIEW.cameraY,
  zoom: VIEW.zoom,
  snakes: SOURCE.snakes.map(snake => ({
    id: snake.frameV1Id,
    radius: snake.radius,
    skin: snake.skin,
    x: snake.x,
    y: snake.y,
    dir: snake.direction,
    boost: snake.boost ? 1 : 0,
    alive: snake.alive,
    points: snake.body.map(point => ({ ...point }))
  })),
  pellets: SOURCE.pellets.map(pellet => ({
    x: pellet.x,
    y: pellet.y,
    v: pellet.value,
    kind: pelletKind(pellet.kind),
    colorId: pellet.color
  }))
});

process.stdout.write(`${JSON.stringify({
  evidenceKind: 'current-source execution',
  sourceRevision: SOURCE_REVISION,
  command:
    'node .\\node_modules\\tsx\\dist\\cli.mjs scripts\\stage6\\generate-frame-v1-fixture.ts',
  view: VIEW,
  source: SOURCE,
  expected: {
    floatLength: frame.length,
    byteLength: frame.byteLength,
    floatBits: Array.from(frame, float32Hex)
  }
}, null, 2)}\n`);

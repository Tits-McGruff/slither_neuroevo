import { CFG } from '../src/config.ts';
import { getSensorLayout, getSensorSpec } from '../src/protocol/sensors.ts';
import type { SensorSpec } from './protocol.ts';

/**
 * Build the sensor specification sent to clients during handshake.
 * @returns Sensor spec containing order and count.
 */
export function buildSensorSpec(): SensorSpec {
  const layoutVersion = CFG.sense?.layoutVersion ?? 'v2';
  const layout = getSensorLayout(CFG.sense?.bubbleBins ?? 16, layoutVersion);
  return getSensorSpec(layout);
}

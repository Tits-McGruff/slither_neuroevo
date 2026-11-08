/** Match a canvas buffer to its CSS rectangle, drawing in CSS pixels at any DPI.
 * Hidden surfaces keep their last buffer. Unchanged dimensions never reallocate it.
 */
export function fitCanvasToDisplay(canvas: HTMLCanvasElement, ctx: CanvasRenderingContext2D, ratio = window.devicePixelRatio || 1): { width: number; height: number; changed: boolean } {
  const width = canvas.clientWidth;
  const height = canvas.clientHeight;
  if (!(width > 0 && height > 0)) return { width: 0, height: 0, changed: false };
  const pixelWidth = Math.max(1, Math.round(width * ratio));
  const pixelHeight = Math.max(1, Math.round(height * ratio));
  const changed = canvas.width !== pixelWidth || canvas.height !== pixelHeight;
  if (canvas.width !== pixelWidth) canvas.width = pixelWidth;
  if (canvas.height !== pixelHeight) canvas.height = pixelHeight;
  ctx.setTransform(pixelWidth / width, 0, 0, pixelHeight / height, 0, 0);
  return { width, height, changed };
}

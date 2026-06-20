// Pure spatial-crop geometry. The crop rectangle is stored in SOURCE pixels
// (the coordinate space ffmpeg's crop filter wants), mapped from the pointer's
// position over the rendered video box. Kept pure + unit-tested; the component
// only supplies the live DOMRect and the source dimensions.

/**
 * @typedef {{ x: number, y: number, w: number, h: number }} CropRect
 * @typedef {{ left: number, top: number, width: number, height: number }} BoxRect
 */

/**
 * Round down to the nearest even integer (yuv420p needs even dimensions).
 * @param {number} n
 * @returns {number}
 */
function evenDown(n) {
  n = Math.round(n);
  return n - (n % 2);
}

/**
 * Map a screen point to source pixels using the rendered video box `rect`
 * (videoEl.getBoundingClientRect()) and the Clip's source dimensions. The result
 * is clamped to [0, vidW] x [0, vidH] so a drag past the frame edge stays valid.
 * @param {number} clientX
 * @param {number} clientY
 * @param {BoxRect} rect   rendered video box (left/top/width/height in CSS px)
 * @param {number} vidW    source width in px
 * @param {number} vidH    source height in px
 * @returns {{ sx: number, sy: number }}
 */
export function screenToSource(clientX, clientY, rect, vidW, vidH) {
  const fx = rect.width > 0 ? (clientX - rect.left) / rect.width : 0;
  const fy = rect.height > 0 ? (clientY - rect.top) / rect.height : 0;
  /** @param {number} v @param {number} max */
  const clamp = (v, max) => Math.max(0, Math.min(v, max));
  return { sx: clamp(fx * vidW, vidW), sy: clamp(fy * vidH, vidH) };
}

/**
 * Normalise a drag from `(x0,y0)` to `(x1,y1)` (source px, any direction) into a
 * crop rect: clamped in-bounds, integer, and forced to even dimensions. Returns
 * null when the result is smaller than `minSize` on either axis — the caller
 * treats that as "no crop" (a stray click), so the rect is never degenerate.
 * @param {number} x0 @param {number} y0 @param {number} x1 @param {number} y1
 * @param {number} vidW @param {number} vidH @param {number} [minSize]
 * @returns {CropRect | null}
 */
export function normalizeCrop(x0, y0, x1, y1, vidW, vidH, minSize = 16) {
  /** @param {number} v */
  const cx = (v) => Math.max(0, Math.min(v, vidW));
  /** @param {number} v */
  const cy = (v) => Math.max(0, Math.min(v, vidH));
  const left = evenDown(cx(Math.min(x0, x1)));
  const top = evenDown(cy(Math.min(y0, y1)));
  const w = evenDown(cx(Math.max(x0, x1)) - left);
  const h = evenDown(cy(Math.max(y0, y1)) - top);
  if (w < minSize || h < minSize) return null;
  return { x: left, y: top, w, h };
}

/**
 * Convert a source-px crop rect into percentage offsets within the video box,
 * for positioning the overlay rectangle. Null in → null out.
 * @param {CropRect | null} rect @param {number} vidW @param {number} vidH
 * @returns {BoxRect | null}
 */
export function cropToPercent(rect, vidW, vidH) {
  if (!rect || vidW <= 0 || vidH <= 0) return null;
  return {
    left: (rect.x / vidW) * 100,
    top: (rect.y / vidH) * 100,
    width: (rect.w / vidW) * 100,
    height: (rect.h / vidH) * 100,
  };
}

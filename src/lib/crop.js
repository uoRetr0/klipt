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
 * Translate an existing crop rect by `(dx, dy)` source px, clamped so it stays
 * fully in-bounds (the size is preserved — moving never resizes). Used to drag
 * a finished crop around the frame.
 * @param {CropRect} rect @param {number} dx @param {number} dy
 * @param {number} vidW @param {number} vidH
 * @returns {CropRect}
 */
export function moveCrop(rect, dx, dy, vidW, vidH) {
  const x = evenDown(Math.max(0, Math.min(rect.x + dx, vidW - rect.w)));
  const y = evenDown(Math.max(0, Math.min(rect.y + dy, vidH - rect.h)));
  return { x, y, w: rect.w, h: rect.h };
}

/**
 * Resize a crop rect by dragging the `handle` edge/corner to source point
 * `(sx, sy)`. `handle` is a compass string built from n/s + w/e (e.g. "nw", "e",
 * "se"); the opposite edges stay anchored. The result is clamped in-bounds, kept
 * at least `minSize` on each axis, and forced to even dimensions.
 * @param {CropRect} rect @param {string} handle @param {number} sx @param {number} sy
 * @param {number} vidW @param {number} vidH @param {number} [minSize]
 * @returns {CropRect}
 */
export function resizeCrop(rect, handle, sx, sy, vidW, vidH, minSize = 16) {
  const cx = (/** @type {number} */ v) => Math.max(0, Math.min(v, vidW));
  const cy = (/** @type {number} */ v) => Math.max(0, Math.min(v, vidH));
  const min = evenDown(minSize);
  let l = rect.x;
  let t = rect.y;
  let r = rect.x + rect.w;
  let b = rect.y + rect.h;
  if (handle.includes("w")) l = Math.min(cx(sx), r - min);
  if (handle.includes("e")) r = Math.max(cx(sx), l + min);
  if (handle.includes("n")) t = Math.min(cy(sy), b - min);
  if (handle.includes("s")) b = Math.max(cy(sy), t + min);
  const left = evenDown(l);
  const top = evenDown(t);
  return { x: left, y: top, w: Math.max(min, evenDown(r - left)), h: Math.max(min, evenDown(b - top)) };
}

/**
 * Hit-test a source point against a crop rect: returns a handle string
 * ("nw"/"n"/"ne"/"e"/"se"/"s"/"sw"/"w") when within `tol` source px of an edge or
 * corner, "move" when inside the body, or null when outside. Drives the pointer
 * gesture (resize vs move vs draw-new) and the hover cursor.
 * @param {number} sx @param {number} sy @param {CropRect | null} rect @param {number} tol
 * @returns {string | null}
 */
export function hitTestCrop(sx, sy, rect, tol) {
  if (!rect) return null;
  const l = rect.x;
  const t = rect.y;
  const r = rect.x + rect.w;
  const b = rect.y + rect.h;
  if (sx < l - tol || sx > r + tol || sy < t - tol || sy > b + tol) return null;
  let h = "";
  if (Math.abs(sy - t) <= tol) h += "n";
  else if (Math.abs(sy - b) <= tol) h += "s";
  if (Math.abs(sx - l) <= tol) h += "w";
  else if (Math.abs(sx - r) <= tol) h += "e";
  if (h) return h;
  return sx >= l && sx <= r && sy >= t && sy <= b ? "move" : null;
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

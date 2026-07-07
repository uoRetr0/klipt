// Pure Region math. Sliding moves the whole keep-window (in + out together)
// by a time delta while preserving its length, clamped so the Region can never
// leave the Clip's [0, duration] bounds. Drag-select shapes a new Region from
// a press-anchor and the live pointer position. The single contiguous Region
// is a domain invariant (see CONTEXT.md) — neither ever splits it.

/**
 * @param {number} deltaSecs  signed seconds to shift the Region by
 * @param {number} inPoint    current in-point (drag-start base)
 * @param {number} outPoint   current out-point (drag-start base)
 * @param {number} duration   Clip duration in seconds
 * @returns {{inPoint: number, outPoint: number}}
 */
export function slideRegion(deltaSecs, inPoint, outPoint, duration) {
  const length = outPoint - inPoint;
  // Shift, then clamp the in-point so the whole length stays inside the Clip.
  let nextIn = inPoint + deltaSecs;
  const maxIn = duration - length;
  if (nextIn > maxIn) nextIn = maxIn;
  if (nextIn < 0) nextIn = 0;
  return { inPoint: nextIn, outPoint: nextIn + length };
}

/**
 * Shape a Region from a drag: the press point anchors one edge, the live
 * pointer position is the other. Direction-agnostic (drag left or right of
 * the anchor), clamped to the Clip, and never thinner than `minLength` —
 * when the pointer sits on the anchor the Region grows away from it, flipping
 * direction only if the Clip boundary leaves no room.
 *
 * @param {number} anchorSecs   time under the pointer at press
 * @param {number} pointerSecs  time under the pointer now
 * @param {number} duration     Clip duration in seconds
 * @param {number} [minLength]  minimum Region length in seconds
 * @returns {{inPoint: number, outPoint: number}}
 */
export function dragSelect(anchorSecs, pointerSecs, duration, minLength = 0.05) {
  if (duration <= minLength) return { inPoint: 0, outPoint: Math.max(0, duration) };
  const clamp = (/** @type {number} */ v) => Math.max(0, Math.min(duration, v));
  const a = clamp(anchorSecs);
  const p = clamp(pointerSecs);
  let inPoint = Math.min(a, p);
  let outPoint = Math.max(a, p);
  if (outPoint - inPoint < minLength) {
    // Too thin: extend in the drag direction (forward on a pure tap), then
    // pull the far edge back if the Clip boundary cut the extension short.
    if (p >= a) outPoint = inPoint + minLength;
    else inPoint = outPoint - minLength;
    if (outPoint > duration) { outPoint = duration; inPoint = duration - minLength; }
    if (inPoint < 0) { inPoint = 0; outPoint = minLength; }
  }
  return { inPoint, outPoint };
}

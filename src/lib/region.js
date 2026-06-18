// Pure Region math. Sliding moves the whole keep-window (in + out together)
// by a time delta while preserving its length, clamped so the Region can never
// leave the Clip's [0, duration] bounds. The single contiguous Region is a
// domain invariant (see CONTEXT.md) — this never splits or resizes it.

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

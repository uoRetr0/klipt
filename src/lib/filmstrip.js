// Pure mapping for thumbnail-scrubbing: turn a pointer position over a strip
// (the Timeline, or a library card) into a time and the filmstrip frame index
// to preview. The filmstrip is a sprite of `cols` evenly-spaced frames in one
// row; `frameIndexAt` picks which cell to show. Kept DOM-free so the geometry
// is unit-testable (the component passes in the element's measured rect).

/**
 * Map a pointer x to a time in [0, duration], clamped to the strip.
 * @param {number} clientX   pointer x in client coords
 * @param {number} rectLeft  strip's left edge in client coords
 * @param {number} rectWidth strip width in px
 * @param {number} duration  Clip duration in seconds
 * @returns {number} time in seconds
 */
export function hoverTime(clientX, rectLeft, rectWidth, duration) {
  if (rectWidth <= 0 || duration <= 0) return 0;
  const f = Math.max(0, Math.min(1, (clientX - rectLeft) / rectWidth));
  return f * duration;
}

/**
 * The filmstrip cell index for a time, clamped to [0, cols-1]. The sprite holds
 * `cols` frames sampled evenly across the whole Clip, so cell = floor(t/dur*cols).
 * @param {number} time     seconds into the Clip
 * @param {number} cols     number of frames in the sprite
 * @param {number} duration Clip duration in seconds
 * @returns {number} integer frame index
 */
export function frameIndexAt(time, cols, duration) {
  if (cols <= 0 || duration <= 0) return 0;
  const idx = Math.floor((time / duration) * cols);
  return Math.max(0, Math.min(cols - 1, idx));
}

// Pure time <-> frame conversions for frame-accurate playhead navigation.
// fps comes from probe_clip; if it's unknown (0 or negative) these return 0
// rather than NaN/Infinity so callers degrade gracefully.

/** @param {number} timeSecs @param {number} fps @returns {number} nearest frame index */
export function frameOf(timeSecs, fps) {
  if (!(fps > 0)) return 0;
  return Math.round(timeSecs * fps);
}

/** @param {number} frame @param {number} fps @returns {number} start time of that frame */
export function timeOf(frame, fps) {
  if (!(fps > 0)) return 0;
  return frame / fps;
}

// Pure playback-boundary logic. Given the live playhead and the current in/out
// points, decide what the rAF watcher should do at this frame: keep playing,
// wrap back to the start (loop on), or stop at the end (loop off — the default).
// Reading in/out fresh each call is what lets the loop respect handles that are
// being dragged live (see CONTEXT.md: the Region is a single contiguous
// keep-range).
//
// `selectionOnly` chooses the playback scope: when on (the default), the bounds
// are the Region (in → out) so the moment can be previewed in isolation; when
// off, the bounds are the whole Clip (0 → duration) so playback ignores the trim
// and runs end-to-end. Loop wraps within whichever scope is active.

/**
 * @param {number} currentTime  the playhead, in seconds
 * @param {number} inPoint      Region in-point, in seconds
 * @param {number} outPoint     Region out-point, in seconds
 * @param {boolean} loopEnabled whether looping is on
 * @param {boolean} [selectionOnly=true]  confine playback to the Region
 * @param {number}  [duration=Infinity]   Clip duration (the whole-Clip end bound)
 * @returns {{action: "play"|"wrap"|"stop", seekTo?: number}}
 *   - "play": within the active scope, let it run
 *   - "wrap": reached the end with loop on — seek to `seekTo` (the scope's start)
 *   - "stop": reached the end with loop off — pause at `seekTo` (the scope's end)
 */
export function loopDecision(
  currentTime,
  inPoint,
  outPoint,
  loopEnabled,
  selectionOnly = true,
  duration = Infinity,
) {
  const start = selectionOnly ? inPoint : 0;
  const end = selectionOnly ? outPoint : duration;
  if (currentTime >= end) {
    return loopEnabled
      ? { action: "wrap", seekTo: start }
      : { action: "stop", seekTo: end };
  }
  return { action: "play" };
}

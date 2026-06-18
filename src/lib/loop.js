// Pure playback-boundary logic for the Region. Given the live playhead and the
// current in/out points, decide what the rAF watcher should do at this frame:
// keep playing, wrap back to the in-point (loop on), or stop at the out-point
// (loop off — the default). Reading in/out fresh each call is what lets the loop
// respect handles that are being dragged live (see CONTEXT.md: the Region is a
// single contiguous keep-range).

/**
 * @param {number} currentTime  the playhead, in seconds
 * @param {number} inPoint      Region in-point, in seconds
 * @param {number} outPoint     Region out-point, in seconds
 * @param {boolean} loopEnabled whether looping is on
 * @returns {{action: "play"|"wrap"|"stop", seekTo?: number}}
 *   - "play": within the Region, let it run
 *   - "wrap": reached the out-point with loop on — seek to `seekTo` (in-point)
 *   - "stop": reached the out-point with loop off — pause at `seekTo` (out-point)
 */
export function loopDecision(currentTime, inPoint, outPoint, loopEnabled) {
  if (currentTime >= outPoint) {
    return loopEnabled
      ? { action: "wrap", seekTo: inPoint }
      : { action: "stop", seekTo: outPoint };
  }
  return { action: "play" };
}

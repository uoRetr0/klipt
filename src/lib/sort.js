// Pure library-grid ordering. The watched-folder scan returns Clips newest-first
// already; this re-orders the (already-filtered) list by the user's chosen key
// and direction without mutating the input. Kept pure so the windowed grid can
// sort upstream of its render window and stay unit-testable.

/**
 * @typedef {import('$lib/types').ClipEntry} ClipEntry
 */

/**
 * Return a new array of `clips` ordered by `key` ('date' | 'name' | 'size') in
 * `dir` ('asc' | 'desc'). Date sorts on mtime, size on bytes, name with a
 * numeric-aware locale compare (so clip2 precedes clip10). Unknown keys fall
 * back to date. The input array is never mutated.
 * @param {ClipEntry[]} clips
 * @param {string} key
 * @param {string} dir
 * @returns {ClipEntry[]}
 */
export function sortClips(clips, key, dir) {
  const mul = dir === "asc" ? 1 : -1;
  const out = clips.slice();
  out.sort((a, b) => {
    let r;
    if (key === "name") {
      r = a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" });
    } else if (key === "size") {
      r = a.size_bytes - b.size_bytes;
    } else {
      r = a.modified - b.modified;
    }
    // Stable tiebreak on name so equal keys keep a deterministic order.
    if (r === 0 && key !== "name") {
      r = a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" });
    }
    return r * mul;
  });
  return out;
}

// Pure date-range predicate for the library grid's date filter. Extracted from
// the `filteredClips` derivation so the today/7d/30d boundary logic is
// deterministic and unit-testable: the clock is injected (`nowSecs`) rather than
// read from `Date.now()` inside, and the span table lives in one place.

/** Window lengths in seconds for each named filter. */
const SPANS = { today: 86400, "7d": 7 * 86400, "30d": 30 * 86400 };

/**
 * Does a clip's modification time fall within the selected date window?
 *
 * `"all"` (and any unrecognised filter) imposes no constraint. A clip is kept
 * when it was modified no longer ago than the window — i.e. `nowSecs - modified
 * <= span`. A future `modified` (clock skew / mtime ahead of now) yields a
 * negative age, so it always passes, matching the pre-extraction behaviour.
 *
 * @param {number} modifiedSecs  clip mtime, seconds since epoch
 * @param {string} filter        "all" | "today" | "7d" | "30d"
 * @param {number} nowSecs       current time, seconds since epoch (injected)
 * @returns {boolean} true if the clip should be shown
 */
export function matchesDateFilter(modifiedSecs, filter, nowSecs) {
  const span = SPANS[/** @type {keyof typeof SPANS} */ (filter)];
  if (span === undefined) return true; // "all" or unknown → no constraint
  return nowSecs - modifiedSecs <= span;
}

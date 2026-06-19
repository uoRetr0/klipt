// Pure geometry for the windowed (virtualized) library grid. The grid renders
// only the cards in (or near) the viewport, so a library of any size keeps a
// constant number of DOM nodes. These functions are DOM-free so the row math is
// unit-testable; the component feeds in measured pixel sizes.

/**
 * Columns a CSS `repeat(auto-fill, minmax(min, 1fr))` grid yields for a content
 * box `width` px wide with `gap` px between tracks. Mirrors the browser's own
 * track count so the windowing math agrees with the real layout. Always >= 1.
 * @param {number} width content-box width in px
 * @param {number} min   the minmax() floor (min track width) in px
 * @param {number} gap   grid gap in px
 * @returns {number} column count (>= 1)
 */
export function gridColumns(width, min, gap) {
  if (width <= 0 || min <= 0) return 1;
  // n tracks of `min` plus (n-1) gaps must fit: n*min + (n-1)*gap <= width
  // → n <= (width + gap) / (min + gap).
  return Math.max(1, Math.floor((width + gap) / (min + gap)));
}

/**
 * Which slice of items to render for a windowed grid, plus the spacer geometry.
 *
 * @param {number} scrollTop scroll container's current scrollTop, px
 * @param {number} viewportH scroll container's visible height, px
 * @param {number} gridTop   grid's top offset within the scroll content, px
 *                           (so a header above the grid is accounted for)
 * @param {number} rowH      one row's pitch (card height + gap), px
 * @param {number} total     total item count
 * @param {number} cols      columns per row (from `gridColumns`)
 * @param {number} [buffer]  extra rows rendered above & below the viewport
 * @returns {{startIdx:number, endIdx:number, padTop:number, totalHeight:number, rows:number}}
 *   - startIdx/endIdx: render `items.slice(startIdx, endIdx)`
 *   - padTop: translateY to push the rendered block to its true scroll position
 *   - totalHeight: full height the spacer must reserve so the scrollbar is right
 *   - rows: total row count
 */
export function rowWindow(scrollTop, viewportH, gridTop, rowH, total, cols, buffer = 2) {
  const c = Math.max(1, cols);
  const rows = Math.ceil(total / c);
  if (total <= 0 || rowH <= 0) {
    return { startIdx: 0, endIdx: 0, padTop: 0, totalHeight: 0, rows: 0 };
  }
  // rowH folds in the inter-row gap (it's the row pitch), so rows*rowH reserves
  // the full scroll height. The real last row has no trailing gap, so this leaves
  // one gap's worth of harmless slack at the bottom of the scroll range.
  const totalHeight = rows * rowH;
  // How far the viewport top has scrolled into the grid's own coordinate space.
  const into = Math.max(0, scrollTop - gridTop);
  const lastRow = Math.min(rows - 1, Math.ceil((into + viewportH) / rowH) + buffer);
  // Clamp to lastRow so a scroll overshoot (e.g. the list shrank under a stale
  // scrollTop after a search) renders the last row instead of going blank.
  const firstRow = Math.min(Math.max(0, Math.floor(into / rowH) - buffer), lastRow);
  const startIdx = firstRow * c;
  const endIdx = Math.min(total, (lastRow + 1) * c);
  return { startIdx, endIdx, padTop: firstRow * rowH, totalHeight, rows };
}

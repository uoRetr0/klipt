import { describe, it, expect } from "vitest";
import { gridColumns, rowWindow } from "./grid.js";

describe("gridColumns", () => {
  it("counts tracks the way auto-fill minmax does", () => {
    // 190px min, 15px gap. 800px → floor((800+15)/(190+15)) = floor(3.97) = 3.
    expect(gridColumns(800, 190, 15)).toBe(3);
    // Exactly enough for 4: 4*190 + 3*15 = 805.
    expect(gridColumns(805, 190, 15)).toBe(4);
    // One px short of 4 stays at 3.
    expect(gridColumns(804, 190, 15)).toBe(3);
  });

  it("never returns less than one column", () => {
    expect(gridColumns(50, 190, 15)).toBe(1);
    expect(gridColumns(0, 190, 15)).toBe(1);
    expect(gridColumns(-100, 190, 15)).toBe(1);
  });
});

describe("rowWindow", () => {
  // 4 cols, rowH 180, viewport 720 (4 rows tall), grid starts at scroll y=100.
  const base = { viewportH: 720, gridTop: 100, rowH: 180, cols: 4, buffer: 2 };
  /** @param {number} scrollTop @param {number} total */
  const win = (scrollTop, total) =>
    rowWindow(scrollTop, base.viewportH, base.gridTop, base.rowH, total, base.cols, base.buffer);

  it("renders from the top (plus buffer) when unscrolled", () => {
    const r = win(0, 100); // 100 items / 4 = 25 rows
    expect(r.rows).toBe(25);
    expect(r.totalHeight).toBe(25 * 180);
    expect(r.startIdx).toBe(0); // firstRow 0
    // into=0 → lastRow ceil(720/180)+2 = 4+2 = 6 → endIdx (6+1)*4 = 28.
    expect(r.endIdx).toBe(28);
    expect(r.padTop).toBe(0);
  });

  it("windows around the scroll position once past the grid top", () => {
    // scrollTop 1900 → into = 1800 → firstRow floor(1800/180)-2 = 10-2 = 8.
    const r = win(1900, 400);
    expect(r.startIdx).toBe(8 * 4); // 32
    expect(r.padTop).toBe(8 * 180); // 1440
    // lastRow = ceil((1800+720)/180)+2 = 14+2 = 16 → endIdx 17*4 = 68.
    expect(r.endIdx).toBe(68);
  });

  it("renders the last row (not blank) when scroll overshoots the end", () => {
    const r = win(100000, 30); // 30 items / 4 = 8 rows; scroll way past the end
    expect(r.rows).toBe(8);
    expect(r.endIdx).toBe(30); // clamped to item count
    expect(r.startIdx).toBe(28); // last row start (7 * 4), so the slice isn't empty
    expect(r.endIdx).toBeGreaterThan(r.startIdx);
  });

  it("is empty for an empty list", () => {
    expect(win(0, 0)).toEqual({ startIdx: 0, endIdx: 0, padTop: 0, totalHeight: 0, rows: 0 });
  });

  it("treats scroll within the header (above the grid) as the top", () => {
    // scrollTop 50 < gridTop 100 → into clamps to 0 → same as unscrolled.
    expect(win(50, 100).startIdx).toBe(0);
    expect(win(50, 100).padTop).toBe(0);
  });
});

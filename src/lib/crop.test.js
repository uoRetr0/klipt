import { describe, it, expect } from "vitest";
import { screenToSource, normalizeCrop, cropToPercent } from "./crop.js";

describe("screenToSource", () => {
  const rect = { left: 100, top: 50, width: 800, height: 450 };

  it("maps a screen point to source px by fraction of the video box", () => {
    // Centre of an 800x450 box over a 1920x1080 source.
    expect(screenToSource(500, 275, rect, 1920, 1080)).toEqual({ sx: 960, sy: 540 });
  });

  it("clamps points outside the video box into [0, dims]", () => {
    expect(screenToSource(50, 10, rect, 1920, 1080)).toEqual({ sx: 0, sy: 0 });
    expect(screenToSource(9999, 9999, rect, 1920, 1080)).toEqual({ sx: 1920, sy: 1080 });
  });

  it("returns origin for a zero-size box (no divide-by-zero)", () => {
    expect(screenToSource(10, 10, { left: 0, top: 0, width: 0, height: 0 }, 1920, 1080)).toEqual({
      sx: 0,
      sy: 0,
    });
  });
});

describe("normalizeCrop", () => {
  it("orders corners regardless of drag direction", () => {
    const a = normalizeCrop(300, 200, 700, 500, 1920, 1080);
    const b = normalizeCrop(700, 500, 300, 200, 1920, 1080);
    expect(a).toEqual(b);
    expect(a).toEqual({ x: 300, y: 200, w: 400, h: 300 });
  });

  it("forces even dimensions and offset for yuv420p", () => {
    // 101→100, 51→50, w=300, h=evenDown(250)=250 — every component even.
    expect(normalizeCrop(101, 51, 400, 300, 1920, 1080)).toEqual({ x: 100, y: 50, w: 300, h: 250 });
  });

  it("clamps in-bounds so x+w never exceeds the source", () => {
    expect(normalizeCrop(-50, -50, 5000, 5000, 1920, 1080)).toEqual({ x: 0, y: 0, w: 1920, h: 1080 });
  });

  it("returns null for a too-small (stray-click) selection", () => {
    expect(normalizeCrop(100, 100, 105, 105, 1920, 1080)).toBeNull();
    expect(normalizeCrop(100, 100, 100, 100, 1920, 1080)).toBeNull();
  });
});

describe("cropToPercent", () => {
  it("expresses the rect as percentages of the source dims", () => {
    expect(cropToPercent({ x: 192, y: 108, w: 960, h: 540 }, 1920, 1080)).toEqual({
      left: 10,
      top: 10,
      width: 50,
      height: 50,
    });
  });

  it("returns null for a null rect or zero dims", () => {
    expect(cropToPercent(null, 1920, 1080)).toBeNull();
    expect(cropToPercent({ x: 0, y: 0, w: 10, h: 10 }, 0, 0)).toBeNull();
  });
});

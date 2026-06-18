import { describe, it, expect } from "vitest";
import { hoverTime, frameIndexAt } from "./filmstrip.js";

describe("hoverTime", () => {
  it("maps the middle of the strip to the middle of the Clip", () => {
    // strip at left=100, width=200; x=200 is halfway → 30s of 60s.
    expect(hoverTime(200, 100, 200, 60)).toBeCloseTo(30, 10);
  });

  it("clamps before the left edge to 0 and past the right edge to duration", () => {
    expect(hoverTime(50, 100, 200, 60)).toBe(0);
    expect(hoverTime(999, 100, 200, 60)).toBeCloseTo(60, 10);
  });

  it("is 0 for a degenerate strip or unknown duration", () => {
    expect(hoverTime(150, 100, 0, 60)).toBe(0);
    expect(hoverTime(150, 100, 200, 0)).toBe(0);
  });
});

describe("frameIndexAt", () => {
  it("maps time to the evenly-spaced cell", () => {
    // 16 cells over 60s → each ~3.75s. t=0 → 0, t=30 → 8.
    expect(frameIndexAt(0, 16, 60)).toBe(0);
    expect(frameIndexAt(30, 16, 60)).toBe(8);
  });

  it("clamps the end-point to the last cell", () => {
    // t==duration would compute index == cols; clamp to cols-1.
    expect(frameIndexAt(60, 16, 60)).toBe(15);
    expect(frameIndexAt(120, 16, 60)).toBe(15);
  });

  it("guards bad inputs", () => {
    expect(frameIndexAt(10, 0, 60)).toBe(0);
    expect(frameIndexAt(10, 16, 0)).toBe(0);
    expect(frameIndexAt(-5, 16, 60)).toBe(0);
  });
});

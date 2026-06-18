import { describe, it, expect } from "vitest";
import { frameOf, timeOf } from "./frames.js";

describe("frameOf", () => {
  it("maps time to the nearest frame index", () => {
    expect(frameOf(0, 60)).toBe(0);
    expect(frameOf(1, 60)).toBe(60);
    expect(frameOf(0.5, 30)).toBe(15);
  });
  it("rounds to the closest frame", () => {
    expect(frameOf(0.016, 60)).toBe(1); // 0.96 -> 1
    expect(frameOf(0.008, 60)).toBe(0); // 0.48 -> 0
  });
  it("returns 0 for unknown fps rather than NaN/Infinity", () => {
    expect(frameOf(5, 0)).toBe(0);
    expect(frameOf(5, -1)).toBe(0);
  });
});

describe("timeOf", () => {
  it("maps a frame index back to its start time", () => {
    expect(timeOf(60, 60)).toBeCloseTo(1, 10);
    expect(timeOf(15, 30)).toBeCloseTo(0.5, 10);
  });
  it("returns 0 for unknown fps", () => {
    expect(timeOf(5, 0)).toBe(0);
  });
  it("round-trips with frameOf at frame boundaries", () => {
    for (const [t, fps] of [[2, 60], [0.25, 24], [10, 29.97]]) {
      expect(frameOf(timeOf(frameOf(t, fps), fps), fps)).toBe(frameOf(t, fps));
    }
  });
});

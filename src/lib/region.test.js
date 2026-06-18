import { describe, it, expect } from "vitest";
import { slideRegion } from "./region.js";

const len = (r) => r.outPoint - r.inPoint;

describe("slideRegion", () => {
  it("slides the whole Region by the delta when it stays in bounds", () => {
    const r = slideRegion(2, 10, 15, 60);
    expect(r).toEqual({ inPoint: 12, outPoint: 17 });
  });

  it("slides backwards on a negative delta", () => {
    const r = slideRegion(-3, 10, 15, 60);
    expect(r).toEqual({ inPoint: 7, outPoint: 12 });
  });

  it("preserves Region length regardless of delta", () => {
    for (const d of [-1000, -5, 0, 4, 1000]) {
      expect(len(slideRegion(d, 10, 15, 60))).toBeCloseTo(5, 10);
    }
  });

  it("clamps at the end: cannot push the out-point past the duration", () => {
    const r = slideRegion(1000, 50, 55, 60);
    expect(r).toEqual({ inPoint: 55, outPoint: 60 });
  });

  it("clamps at the start: cannot push the in-point below zero", () => {
    const r = slideRegion(-1000, 5, 10, 60);
    expect(r).toEqual({ inPoint: 0, outPoint: 5 });
  });

  it("is a no-op for a zero delta", () => {
    expect(slideRegion(0, 10, 15, 60)).toEqual({ inPoint: 10, outPoint: 15 });
  });
});

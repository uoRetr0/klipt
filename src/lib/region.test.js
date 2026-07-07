import { describe, it, expect } from "vitest";
import { slideRegion, dragSelect } from "./region.js";

/** @param {{inPoint: number, outPoint: number}} r */
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

describe("dragSelect", () => {
  it("shapes a forward drag (anchor before pointer)", () => {
    expect(dragSelect(10, 20, 60)).toEqual({ inPoint: 10, outPoint: 20 });
  });

  it("shapes a backward drag the same as forward (direction-agnostic)", () => {
    expect(dragSelect(20, 10, 60)).toEqual({ inPoint: 10, outPoint: 20 });
  });

  it("clamps the pointer past the Clip end", () => {
    const r = dragSelect(10, 999, 60);
    expect(r.outPoint).toBe(60);
    expect(r).toEqual({ inPoint: 10, outPoint: 60 });
  });

  it("clamps the pointer below zero", () => {
    const r = dragSelect(10, -5, 60);
    expect(r.inPoint).toBe(0);
    expect(r).toEqual({ inPoint: 0, outPoint: 10 });
  });

  it("clamps an out-of-bounds anchor too", () => {
    expect(dragSelect(-10, 20, 60)).toEqual({ inPoint: 0, outPoint: 20 });
    expect(dragSelect(1000, 20, 60)).toEqual({ inPoint: 20, outPoint: 60 });
  });

  it("grows a too-thin forward drag to minLength (pointer >= anchor)", () => {
    const r = dragSelect(10, 10, 60);
    expect(r.inPoint).toBeCloseTo(10, 10);
    expect(r.outPoint).toBeCloseTo(10.05, 10);
  });

  it("flips a too-thin drag backward at the Clip end instead of overflowing", () => {
    const r = dragSelect(60, 60, 60);
    expect(r.inPoint).toBeCloseTo(59.95, 10);
    expect(r.outPoint).toBeCloseTo(60, 10);
  });

  it("grows a too-thin backward drag backward (pointer < anchor)", () => {
    const r = dragSelect(10, 9.98, 60);
    expect(r.inPoint).toBeCloseTo(9.95, 10);
    expect(r.outPoint).toBeCloseTo(10, 10);
  });

  it("degenerates to {0, duration} when duration <= minLength", () => {
    expect(dragSelect(0, 0, 0)).toEqual({ inPoint: 0, outPoint: 0 });
    expect(dragSelect(0, 0, 0.03)).toEqual({ inPoint: 0, outPoint: 0.03 });
  });

  it("respects a custom minLength", () => {
    const r = dragSelect(10, 10, 60, 1);
    expect(r).toEqual({ inPoint: 10, outPoint: 11 });
  });
});

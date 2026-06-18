import { describe, it, expect } from "vitest";
import { loopDecision } from "./loop.js";

describe("loopDecision", () => {
  it("keeps playing while the playhead is inside the Region", () => {
    expect(loopDecision(10, 5, 15, true)).toEqual({ action: "play" });
    expect(loopDecision(10, 5, 15, false)).toEqual({ action: "play" });
  });

  it("wraps to the in-point when looping and the out-point is reached", () => {
    expect(loopDecision(15, 5, 15, true)).toEqual({ action: "wrap", seekTo: 5 });
  });

  it("wraps even if the playhead overshoots the out-point", () => {
    expect(loopDecision(16.4, 5, 15, true)).toEqual({ action: "wrap", seekTo: 5 });
  });

  it("stops at the out-point when looping is disabled", () => {
    expect(loopDecision(15, 5, 15, false)).toEqual({ action: "stop", seekTo: 15 });
  });

  it("respects live-updated bounds (out dragged in past the playhead → wrap)", () => {
    // The out handle was just dragged to 10 while playing at 12.
    expect(loopDecision(12, 0, 10, true)).toEqual({ action: "wrap", seekTo: 0 });
  });

  it("wraps to a live-updated in-point", () => {
    // The in handle was dragged to 4 mid-loop; the wrap target follows it.
    expect(loopDecision(15, 4, 15, true)).toEqual({ action: "wrap", seekTo: 4 });
  });
});

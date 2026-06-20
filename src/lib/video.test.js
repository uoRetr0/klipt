import { describe, it, expect } from "vitest";
import { VIDEO_EXTS, isVideoFile } from "./video.js";

describe("isVideoFile", () => {
  it("accepts every supported extension, case-insensitively", () => {
    for (const e of VIDEO_EXTS) {
      expect(isVideoFile(`clip.${e}`)).toBe(true);
      expect(isVideoFile(`CLIP.${e.toUpperCase()}`)).toBe(true);
    }
  });
  it("rejects non-video files and edge cases", () => {
    expect(isVideoFile("notes.txt")).toBe(false);
    expect(isVideoFile("clip.mp4.txt")).toBe(false);
    expect(isVideoFile("mp4")).toBe(false);
    expect(isVideoFile("")).toBe(false);
  });
});

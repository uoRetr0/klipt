import { describe, it, expect } from "vitest";
import { sortClips } from "./sort.js";

/**
 * @param {{ name: string, modified?: number, size_bytes?: number }} o
 * @returns {import('$lib/types').ClipEntry}
 */
const clip = (o) => ({ path: o.name, game: "", modified: 0, size_bytes: 0, kind: /** @type {"video"} */ ("video"), ...o });

const clips = [
  clip({ name: "clip10.mp4", modified: 300, size_bytes: 50 }),
  clip({ name: "clip2.mp4", modified: 100, size_bytes: 200 }),
  clip({ name: "clip1.mp4", modified: 200, size_bytes: 10 }),
];

describe("sortClips", () => {
  it("sorts by date descending (newest first) by default direction", () => {
    expect(sortClips(clips, "date", "desc").map((c) => c.modified)).toEqual([300, 200, 100]);
  });

  it("sorts by date ascending", () => {
    expect(sortClips(clips, "date", "asc").map((c) => c.modified)).toEqual([100, 200, 300]);
  });

  it("sorts by size", () => {
    expect(sortClips(clips, "size", "asc").map((c) => c.size_bytes)).toEqual([10, 50, 200]);
    expect(sortClips(clips, "size", "desc").map((c) => c.size_bytes)).toEqual([200, 50, 10]);
  });

  it("sorts by name with numeric awareness (clip2 before clip10)", () => {
    expect(sortClips(clips, "name", "asc").map((c) => c.name)).toEqual([
      "clip1.mp4",
      "clip2.mp4",
      "clip10.mp4",
    ]);
  });

  it("does not mutate the input array", () => {
    const before = clips.map((c) => c.name);
    sortClips(clips, "size", "asc");
    expect(clips.map((c) => c.name)).toEqual(before);
  });

  it("falls back to date for an unknown key", () => {
    expect(sortClips(clips, "bogus", "desc").map((c) => c.modified)).toEqual([300, 200, 100]);
  });
});

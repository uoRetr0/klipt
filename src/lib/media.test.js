import { describe, expect, it } from "vitest";
import { mediaKind, isMediaFile, kindBadge, MEDIA_EXTS, ANIM_EXTS, AUDIO_EXTS } from "./media.js";
import { VIDEO_EXTS } from "./video.js";

describe("mediaKind", () => {
  it("classifies videos, animated images, and audio", () => {
    expect(mediaKind("C:/clips/moment.mp4")).toBe("video");
    expect(mediaKind("C:\\clips\\moment.MKV")).toBe("video"); // case + backslash
    expect(mediaKind("C:/out/moment_gif.gif")).toBe("anim");
    expect(mediaKind("C:/out/loop.WebP")).toBe("anim");
    expect(mediaKind("C:/out/callout_audio.m4a")).toBe("audio");
    expect(mediaKind("C:/out/song.MP3")).toBe("audio");
  });

  it("returns null for non-media and pathological paths", () => {
    expect(mediaKind("notes.txt")).toBe(null);
    expect(mediaKind("no_extension")).toBe(null);
    expect(mediaKind("")).toBe(null);
    expect(mediaKind("ends.with.dot.")).toBe(null);
    // A folder named like an extension must not classify the file inside it.
    expect(mediaKind("C:/gif/notes.txt")).toBe(null);
  });
});

describe("isMediaFile", () => {
  it("accepts every listed extension", () => {
    for (const e of MEDIA_EXTS) expect(isMediaFile(`clip.${e}`)).toBe(true);
  });
  it("rejects non-media", () => {
    expect(isMediaFile("clip.txt")).toBe(false);
  });
});

describe("kindBadge", () => {
  it("tags non-video media with the uppercased format", () => {
    expect(kindBadge("a.gif")).toBe("GIF");
    expect(kindBadge("a.webp")).toBe("WEBP");
    expect(kindBadge("a.m4a")).toBe("M4A");
    expect(kindBadge("a.mp3")).toBe("MP3");
  });
  it("returns null for videos and unknowns", () => {
    expect(kindBadge("a.mp4")).toBe(null);
    expect(kindBadge("a.txt")).toBe(null);
  });
});

describe("extension lists", () => {
  it("keeps the three kind lists disjoint", () => {
    const all = [...VIDEO_EXTS, ...ANIM_EXTS, ...AUDIO_EXTS];
    expect(new Set(all).size).toBe(all.length);
  });
});

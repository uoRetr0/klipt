import { describe, it, expect } from "vitest";
import { fmt, fmtSize, waveformPath, previewName } from "./format.js";

describe("fmt", () => {
  it("formats seconds as MM:SS.cc", () => {
    expect(fmt(0)).toBe("00:00.00");
    expect(fmt(5.5)).toBe("00:05.50");
    expect(fmt(75.25)).toBe("01:15.25");
    expect(fmt(3661)).toBe("61:01.00");
  });
  it("clamps negative and non-finite input to zero", () => {
    expect(fmt(-3)).toBe("00:00.00");
    expect(fmt(NaN)).toBe("00:00.00");
    expect(fmt(Infinity)).toBe("00:00.00");
  });
});

describe("fmtSize", () => {
  it("renders empty for falsy / zero sizes", () => {
    expect(fmtSize(0)).toBe("");
    expect(fmtSize(undefined)).toBe("");
    expect(fmtSize(null)).toBe("");
  });
  it("formats MB with one decimal", () => {
    expect(fmtSize(1024 * 1024)).toBe("1.0 MB");
    expect(fmtSize(25 * 1024 * 1024)).toBe("25.0 MB");
  });
  it("switches to GB just under 1 GB to avoid '1000+ MB'", () => {
    // 1000 MB -> GB
    expect(fmtSize(1000 * 1024 * 1024)).toBe("0.98 GB");
    expect(fmtSize(2 * 1024 * 1024 * 1024)).toBe("2.00 GB");
    // 999 MB stays in MB
    expect(fmtSize(999 * 1024 * 1024)).toBe("999.0 MB");
  });
});

describe("waveformPath", () => {
  it("returns an empty path for null / empty input", () => {
    expect(waveformPath(null)).toBe("");
    expect(waveformPath([])).toBe("");
  });
  it("emits one centred bar subpath per bucket", () => {
    const d = waveformPath([0, 1]);
    // bucket 0 (p=0): height clamps to 1, y=50
    // bucket 1 (p=1): height 92, y=4
    expect(d).toBe("M0.15 50h0.7v1h-0.7zM1.15 4h0.7v92h-0.7z");
  });
});

describe("previewName", () => {
  it("substitutes tokens and appends the extension", () => {
    expect(previewName("{name}_{action}", "clip", "trim", "mp4")).toBe("clip_trim.mp4");
    expect(previewName("{action}-{name}", "raw", "small", "mp4")).toBe("small-raw.mp4");
  });
  it("defaults a blank scheme to {name}_{action}", () => {
    expect(previewName("", "clip", "trim", "mkv")).toBe("clip_trim.mkv");
    expect(previewName("   ", "clip", "gif", "gif")).toBe("clip_gif.gif");
    expect(previewName(null, "clip", "trim", "mp4")).toBe("clip_trim.mp4");
  });
  it("strips illegal filename characters", () => {
    expect(previewName('a<b>:c{name}', "raw", "trim", "mp4")).toBe("abcraw.mp4");
  });
  it("falls back to {name}_{action} when the scheme collapses to nothing", () => {
    expect(previewName("///", "raw", "trim", "mp4")).toBe("raw_trim.mp4");
  });
});

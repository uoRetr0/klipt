import { describe, it, expect } from "vitest";
import { resolve } from "./keymap.js";

// Build a key event the way the view will: a plain object, so the keymap is
// testable without a DOM. `code` distinguishes Space; `key` covers the rest.
/**
 * @param {string} key
 * @param {{code?: string, ctrlKey?: boolean, metaKey?: boolean, altKey?: boolean, shiftKey?: boolean}} [opts]
 */
const ev = (key, opts = {}) => ({
  key,
  code: opts.code ?? "",
  ctrlKey: opts.ctrlKey ?? false,
  metaKey: opts.metaKey ?? false,
  altKey: opts.altKey ?? false,
  shiftKey: opts.shiftKey ?? false,
});
const editor = { hasClip: true, isTyping: false };

describe("keymap.resolve — bindings (editor, not typing)", () => {
  it("Enter → trim", () => {
    expect(resolve(ev("Enter"), editor)).toBe("trim");
  });
  it("Escape → back", () => {
    expect(resolve(ev("Escape"), editor)).toBe("back");
  });
  it("F11 → fullscreen", () => {
    expect(resolve(ev("F11"), editor)).toBe("fullscreen");
  });
  it("Space → playPause (matched by code, not key)", () => {
    expect(resolve(ev(" ", { code: "Space" }), editor)).toBe("playPause");
  });
  it("J / K / L → shuttle rewind / pause / forward (case-insensitive)", () => {
    expect(resolve(ev("j"), editor)).toBe("shuttleRewind");
    expect(resolve(ev("K"), editor)).toBe("shuttlePause");
    expect(resolve(ev("l"), editor)).toBe("shuttleForward");
    expect(resolve(ev("L"), editor)).toBe("shuttleForward");
  });
  it("I / O → setIn / setOut (existing, preserved)", () => {
    expect(resolve(ev("i"), editor)).toBe("setIn");
    expect(resolve(ev("O"), editor)).toBe("setOut");
  });
  it("A → selectAll (case-insensitive)", () => {
    expect(resolve(ev("a"), editor)).toBe("selectAll");
    expect(resolve(ev("A", { shiftKey: true }), editor)).toBe("selectAll");
  });
  it("ArrowLeft / ArrowRight → frameBack / frameForward", () => {
    expect(resolve(ev("ArrowLeft"), editor)).toBe("frameBack");
    expect(resolve(ev("ArrowRight"), editor)).toBe("frameForward");
  });
  it("comma / period → frameBack / frameForward", () => {
    expect(resolve(ev(","), editor)).toBe("frameBack");
    expect(resolve(ev("."), editor)).toBe("frameForward");
  });
  it("unmapped key → null", () => {
    expect(resolve(ev("z"), editor)).toBe(null);
  });
});

describe("keymap.resolve — suppression rules", () => {
  it("returns null for every binding while typing in a field", () => {
    const typing = { hasClip: true, isTyping: true };
    for (const k of ["Enter", "Escape", "j", "k", "l", "i", "o", "a"]) {
      expect(resolve(ev(k), typing)).toBe(null);
    }
    expect(resolve(ev(" ", { code: "Space" }), typing)).toBe(null);
  });
  it("returns null when no Clip is open", () => {
    const noClip = { hasClip: false, isTyping: false };
    expect(resolve(ev("Enter"), noClip)).toBe(null);
    expect(resolve(ev("l"), noClip)).toBe(null);
    expect(resolve(ev("a"), noClip)).toBe(null);
  });
  it("ignores combos with ctrl / meta / alt so OS & browser shortcuts pass through", () => {
    expect(resolve(ev("Enter", { ctrlKey: true }), editor)).toBe(null);
    expect(resolve(ev("l", { metaKey: true }), editor)).toBe(null);
    expect(resolve(ev("i", { altKey: true }), editor)).toBe(null);
  });
  it("Ctrl+A stays the OS select-all (ctrlKey suppresses the binding)", () => {
    expect(resolve(ev("a", { ctrlKey: true }), editor)).toBe(null);
  });
  it("still resolves when only Shift is held (Shift just uppercases the letter)", () => {
    expect(resolve(ev("I", { shiftKey: true }), editor)).toBe("setIn");
  });
});

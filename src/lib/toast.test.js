import { describe, it, expect } from "vitest";
import {
  trashedToast,
  deletedToast,
  restoringToast,
  restoredToast,
  restoreFailedToast,
  undoAvailable,
} from "./toast.js";

describe("trashedToast", () => {
  it("carries the save result and offers Undo", () => {
    const t = trashedToast(
      { path: "C:/out/clip_trim.mp4", size_bytes: 1234, encoder: "NVENC (GPU)" },
      "C:/src/clip.mp4",
    );
    expect(t.kind).toBe("ok");
    expect(t.path).toBe("C:/out/clip_trim.mp4");
    expect(t.size_bytes).toBe(1234);
    expect(t.encoder).toBe("NVENC (GPU)");
    expect(t.trashed).toBe(true);
    expect(t.trashedPath).toBe("C:/src/clip.mp4");
    expect(undoAvailable(t)).toBe(true);
  });
});

describe("deletedToast", () => {
  it("marks a library delete with Undo available", () => {
    const t = deletedToast("C:/src/clip.mp4");
    expect(t.deleted).toBe(true);
    expect(t.trashedPath).toBe("C:/src/clip.mp4");
    expect(undoAvailable(t)).toBe(true);
  });
});

describe("undo availability window", () => {
  it("is unavailable while a restore is in flight", () => {
    const t = restoringToast(deletedToast("C:/src/clip.mp4"));
    expect(undoAvailable(t)).toBe(false);
  });

  it("is unavailable once restored", () => {
    const t = restoredToast(deletedToast("C:/src/clip.mp4"));
    expect(t.restored).toBe(true);
    expect(t.trashed).toBe(false);
    expect(undoAvailable(t)).toBe(false);
  });

  it("becomes available again after a failed restore, with a message", () => {
    const base = deletedToast("C:/src/clip.mp4");
    const failed = /** @type {Record<string, any>} */ (restoreFailedToast(restoringToast(base), "boom"));
    expect(failed.restoreError).toBe("boom");
    expect(failed.restored).toBeFalsy();
    expect(undoAvailable(failed)).toBe(true);
  });

  it("clears a prior restore error once the restore finally succeeds", () => {
    const base = deletedToast("C:/src/clip.mp4");
    const failed = restoreFailedToast(base, "boom");
    const ok = restoredToast(failed);
    expect(ok.restoreError).toBeUndefined();
    expect(ok.restored).toBe(true);
  });

  it("treats null / non-trashed toasts as not undoable", () => {
    expect(undoAvailable(null)).toBe(false);
    expect(undoAvailable({ kind: "ok", path: "x" })).toBe(false);
    expect(undoAvailable({ kind: "err", msg: "no" })).toBe(false);
  });
});

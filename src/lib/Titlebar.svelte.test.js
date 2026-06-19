import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";

// Mock the Tauri boundary so the component can be tested without a real window.
// This is the pattern every future +page.svelte carve will reuse: hoisted mocks
// for @tauri-apps/api modules, then assert the component calls them.
const minimize = vi.fn();
const close = vi.fn();
const invoke = vi.fn();

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ minimize, close }),
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (/** @type {string} */ cmd) => invoke(cmd),
}));

import Titlebar from "./Titlebar.svelte";

describe("Titlebar", () => {
  beforeEach(() => {
    minimize.mockClear();
    close.mockClear();
    invoke.mockClear();
  });

  it("renders the brand + the three window controls", () => {
    render(Titlebar);
    expect(screen.getByText("klipt")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Minimize" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Maximize" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Close" })).toBeInTheDocument();
  });

  it("minimizes the window", async () => {
    render(Titlebar);
    await fireEvent.click(screen.getByRole("button", { name: "Minimize" }));
    expect(minimize).toHaveBeenCalledOnce();
  });

  it("toggles maximize via the backend command (keeps the custom DWM chrome)", async () => {
    render(Titlebar);
    await fireEvent.click(screen.getByRole("button", { name: "Maximize" }));
    expect(invoke).toHaveBeenCalledWith("toggle_maximize");
  });

  it("closes the window", async () => {
    render(Titlebar);
    await fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(close).toHaveBeenCalledOnce();
  });
});

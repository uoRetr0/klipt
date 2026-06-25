import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import Dropdown from "./Dropdown.svelte";

// First component test — also the smoke test for the harness itself (svelte
// plugin + jsdom + testing-library). Dropdown is a good first subject: it's
// self-contained (no Tauri/$lib deps) but exercises render, ARIA, and a
// $bindable prop.
const OPTIONS = [
  { value: "all", label: "All games", count: 12 },
  { value: "apex", label: "Apex Legends", count: 7 },
  { value: "cs", label: "Counter-Strike", count: 5 },
];

describe("Dropdown", () => {
  it("renders the selected option's label on the trigger", () => {
    render(Dropdown, { props: { options: OPTIONS, value: "apex", ariaLabel: "Game filter" } });
    const trigger = screen.getByRole("combobox", { name: "Game filter" });
    expect(trigger).toHaveTextContent("Apex Legends");
    expect(trigger).toHaveAttribute("aria-expanded", "false");
  });

  it("opens on click and shows every option in a listbox", async () => {
    render(Dropdown, { props: { options: OPTIONS, value: "all", ariaLabel: "Game filter" } });

    await fireEvent.click(screen.getByRole("combobox", { name: "Game filter" }));

    expect(screen.getByRole("listbox")).toBeInTheDocument();
    expect(screen.getAllByRole("option")).toHaveLength(3);
    // The current value is marked selected for assistive tech.
    expect(screen.getByRole("option", { name: /All games/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("updates the bound selection when an option is chosen", async () => {
    render(Dropdown, { props: { options: OPTIONS, value: "all", ariaLabel: "Game filter" } });

    await fireEvent.click(screen.getByRole("combobox", { name: "Game filter" }));
    await fireEvent.click(screen.getByRole("option", { name: /Counter-Strike/ }));

    // The menu closes and the trigger reflects the new selection.
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "Game filter" })).toHaveTextContent("Counter-Strike");
  });

  // The optional `custom` prop adds an in-popup numeric entry so a value off the
  // preset list (e.g. a 37 MB target size) is still reachable.
  const SIZES = [
    { value: 10, label: "10 MB" },
    { value: 25, label: "25 MB" },
    { value: 50, label: "50 MB" },
  ];
  const CUSTOM = { min: 1, max: 500, unit: "MB" };

  it("shows an off-preset value formatted with the unit on the trigger", () => {
    render(Dropdown, { props: { options: SIZES, value: 37, ariaLabel: "Size", custom: CUSTOM } });
    expect(screen.getByRole("combobox", { name: "Size" })).toHaveTextContent("37 MB");
  });

  it("applies a typed custom value to the bound selection", async () => {
    render(Dropdown, { props: { options: SIZES, value: 25, ariaLabel: "Size", custom: CUSTOM } });

    await fireEvent.click(screen.getByRole("combobox", { name: "Size" }));
    await fireEvent.input(screen.getByLabelText("Custom Size"), { target: { value: "99" } });

    expect(screen.getByRole("combobox", { name: "Size" })).toHaveTextContent("99 MB");
  });

  it("clamps a typed custom value to the configured max", async () => {
    render(Dropdown, { props: { options: SIZES, value: 25, ariaLabel: "Size", custom: CUSTOM } });

    await fireEvent.click(screen.getByRole("combobox", { name: "Size" }));
    await fireEvent.input(screen.getByLabelText("Custom Size"), { target: { value: "999" } });

    expect(screen.getByRole("combobox", { name: "Size" })).toHaveTextContent("500 MB");
  });
});

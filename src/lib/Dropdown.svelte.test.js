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
});

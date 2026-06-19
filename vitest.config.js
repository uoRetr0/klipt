import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { svelteTesting } from "@testing-library/svelte/vite";
import path from "node:path";

// Two kinds of tests share this config:
//   * pure-logic modules (no DOM) — the original suite
//   * Svelte component tests (@testing-library/svelte) — need the svelte plugin
//     to compile `.svelte`, jsdom for a DOM, and the browser resolve conditions
//     so Svelte 5 mounts client-side instead of using its SSR build.
export default defineConfig({
  plugins: [svelte(), svelteTesting()],
  resolve: {
    alias: { $lib: path.resolve("./src/lib") },
    conditions: ["browser"],
  },
  test: {
    include: ["src/**/*.test.js"],
    environment: "jsdom",
    setupFiles: ["./vitest-setup.js"],
  },
});

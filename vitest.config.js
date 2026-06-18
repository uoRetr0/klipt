import { defineConfig } from "vitest/config";

// Unit tests target pure logic modules (no DOM, no SvelteKit). Keeping a
// dedicated config avoids pulling the sveltekit plugin into the test run.
export default defineConfig({
  test: {
    include: ["src/**/*.test.js"],
    environment: "node",
  },
});

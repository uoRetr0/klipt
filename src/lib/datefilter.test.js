import { describe, it, expect } from "vitest";
import { matchesDateFilter } from "./datefilter.js";

// A fixed "now" so the boundaries are deterministic.
const NOW = 1_000_000_000;
const DAY = 86400;

describe("matchesDateFilter", () => {
  it("keeps everything for 'all'", () => {
    expect(matchesDateFilter(0, "all", NOW)).toBe(true);
    expect(matchesDateFilter(NOW, "all", NOW)).toBe(true);
    expect(matchesDateFilter(NOW + 5 * DAY, "all", NOW)).toBe(true);
  });

  it("treats an unknown filter as no constraint", () => {
    expect(matchesDateFilter(0, "year", NOW)).toBe(true);
    expect(matchesDateFilter(0, "", NOW)).toBe(true);
  });

  it("today: keeps clips within the last 24h, drops older", () => {
    expect(matchesDateFilter(NOW, "today", NOW)).toBe(true); // just now
    expect(matchesDateFilter(NOW - DAY, "today", NOW)).toBe(true); // exactly on the boundary
    expect(matchesDateFilter(NOW - DAY - 1, "today", NOW)).toBe(false); // a second too old
  });

  it("7d: boundary at exactly 7 days", () => {
    expect(matchesDateFilter(NOW - 7 * DAY, "7d", NOW)).toBe(true);
    expect(matchesDateFilter(NOW - 7 * DAY - 1, "7d", NOW)).toBe(false);
    expect(matchesDateFilter(NOW - 3 * DAY, "7d", NOW)).toBe(true);
  });

  it("30d: boundary at exactly 30 days", () => {
    expect(matchesDateFilter(NOW - 30 * DAY, "30d", NOW)).toBe(true);
    expect(matchesDateFilter(NOW - 30 * DAY - 1, "30d", NOW)).toBe(false);
    expect(matchesDateFilter(NOW - 8 * DAY, "30d", NOW)).toBe(true);
  });

  it("keeps clips with a future mtime (negative age) for any window", () => {
    expect(matchesDateFilter(NOW + DAY, "today", NOW)).toBe(true);
    expect(matchesDateFilter(NOW + 100 * DAY, "30d", NOW)).toBe(true);
  });
});

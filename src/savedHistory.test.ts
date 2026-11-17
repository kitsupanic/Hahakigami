import { describe, expect, it } from "vitest";
import { formatScanTimestamp, groupSavedScans } from "./savedHistory";
import type { SavedEntry } from "./types";

function entry(
  id: string,
  rootPath: string,
  completedAtMs: number,
  savedAtMs = completedAtMs,
): SavedEntry {
  return {
    id,
    rootPath,
    completedAtMs,
    savedAtMs,
    name: rootPath.split("/").at(-1) || "/",
    startedAtMs: completedAtMs - 100,
    entries: 1,
    logicalBytes: 1,
    allocatedBytes: 1,
    fileBytes: 1,
    incomplete: false,
    errors: 0,
    excluded: 0,
  };
}

describe("groupSavedScans", () => {
  it("groups nonadjacent exact paths, ordered by scan completion instead of import time", () => {
    const input = [
      entry("old-import", "/Volumes/A", 1000, 9000),
      entry("other", "/Volumes/B", 2000),
      entry("recent-scan", "/Volumes/A", 3000, 3000),
    ];
    const groups = groupSavedScans(input);
    expect(groups.map((group) => group.rootPath)).toEqual([
      "/Volumes/A",
      "/Volumes/B",
    ]);
    expect(groups[0].entries.map((item) => item.id)).toEqual([
      "recent-scan",
      "old-import",
    ]);
  });

  it("preserves distinct paths with matching basenames, case differences, and trailing separators", () => {
    const paths = ["/a/Data", "/b/Data", "/a/data", "/a/Data/"];
    expect(
      groupSavedScans(paths.map((path, i) => entry(`${i}`, path, i))).map(
        (group) => group.rootPath,
      ),
    ).toEqual([...paths].reverse());
  });

  it("uses deterministic saved-time, ID, and group-path tie breaks without mutating inputs", () => {
    const input = Object.freeze([
      Object.freeze(entry("a", "/b", 100, 100)),
      Object.freeze(entry("z", "/b", 100, 100)),
      Object.freeze(entry("saved-later", "/b", 100, 200)),
      Object.freeze(entry("a", "/a", 100, 1)),
    ]);
    const before = [...input];
    const groups = groupSavedScans(input);
    expect(groups.map((group) => group.rootPath)).toEqual(["/a", "/b"]);
    expect(groups[1].entries.map((item) => item.id)).toEqual([
      "saved-later",
      "z",
      "a",
    ]);
    expect(input).toEqual(before);
    expect(groups[1].entries[2]).toBe(input[0]);
    expect(groupSavedScans([...input].reverse())).toEqual(groups);
  });

  it("records same-second and identical completion collisions only within each path", () => {
    const groups = groupSavedScans([
      entry("a", "/a", 1001),
      entry("b", "/a", 1999),
      entry("c", "/a", 1999),
      entry("d", "/a", 2000),
      entry("e", "/a", -1),
      entry("f", "/a", -999),
      entry("g", "/b", 1999),
    ]);
    expect([...groups[0].duplicateSeconds]).toEqual([1, -1]);
    expect([...groups[0].duplicateCompletions]).toEqual([1999]);
    expect(groups[1].duplicateSeconds.size).toBe(0);
    expect(groups[1].duplicateCompletions.size).toBe(0);
  });

  it("places unavailable completions last, accepts epoch zero, and ignores invalid collisions", () => {
    const groups = groupSavedScans([
      entry("nan", "/a", NaN),
      entry("infinity", "/a", Infinity, NaN),
      entry("outside-range", "/b", 8.64e15 + 1),
      entry("epoch", "/a", 0),
      entry("another-invalid", "/a", NaN, 100),
    ]);
    expect(groups.map((group) => group.rootPath)).toEqual(["/a", "/b"]);
    expect(groups[0].entries.map((item) => item.id)).toEqual([
      "epoch",
      "another-invalid",
      "nan",
      "infinity",
    ]);
    expect(groups[0].duplicateSeconds.size).toBe(0);
    expect(groups[0].duplicateCompletions.size).toBe(0);
    expect(groupSavedScans([])).toEqual([]);
  });
});

describe("formatScanTimestamp", () => {
  it("uses local calendar days around midnight and year rollover", () => {
    const now = new Date(2026, 0, 1, 0, 1);
    expect(
      formatScanTimestamp(new Date(2026, 0, 1, 0, 0).getTime(), now).day,
    ).toBe("Today");
    expect(
      formatScanTimestamp(new Date(2025, 11, 31, 0, 0).getTime(), now).day,
    ).toBe("Yesterday");
    const older = new Date(2025, 11, 30, 23, 59);
    expect(formatScanTimestamp(older.getTime(), now).day).toBe(
      older.toLocaleDateString(undefined, {
        month: "short",
        day: "numeric",
        year: "numeric",
      }),
    );
    const currentYear = new Date(2026, 0, 2);
    expect(formatScanTimestamp(currentYear.getTime(), now).day).toBe(
      currentYear.toLocaleDateString(undefined, {
        month: "short",
        day: "numeric",
      }),
    );
  });

  it("uses seconds and h23 midnight, adding three fractional digits only when requested", () => {
    const date = new Date(2026, 5, 10, 0, 2, 3, 45);
    const options: Intl.DateTimeFormatOptions = {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hourCycle: "h23",
    };
    const basic = formatScanTimestamp(date.getTime(), date);
    expect(basic.time).toBe(date.toLocaleTimeString(undefined, options));
    expect(basic.label).toBe(`Today, ${basic.time}`);
    expect(formatScanTimestamp(date.getTime(), date, true).time).toBe(
      date.toLocaleTimeString(undefined, {
        ...options,
        fractionalSecondDigits: 3,
      }),
    );
    expect(basic.absolute).toBe(
      date.toLocaleString(undefined, {
        ...options,
        year: "numeric",
        month: "short",
        day: "numeric",
        fractionalSecondDigits: 3,
        timeZoneName: "longOffset",
      }),
    );
    expect(basic.iso).toBe(date.toISOString());
  });

  it.each([NaN, Infinity, -Infinity, 8.64e15 + 1, -8.64e15 - 1])(
    "handles invalid timestamp %s without throwing",
    (ms) => {
      expect(formatScanTimestamp(ms)).toEqual({
        day: "Date unavailable",
        time: "",
        label: "Date unavailable",
        absolute: "Date unavailable",
        iso: undefined,
      });
    },
  );

  it.each([0, 8.64e15, -8.64e15])(
    "accepts valid Date boundaries and epoch %s",
    (ms) => {
      expect(formatScanTimestamp(ms).iso).toBe(new Date(ms).toISOString());
    },
  );
});

import { expect, it } from "vitest";
import type { QueueEntry } from "./cleanupTypes";
import { bandSegments, bandSummary } from "./queueBands";

const entry = (
  id: string,
  allocatedBytes: number,
  status: QueueEntry["status"] = "ready",
): QueueEntry => ({
  id,
  name: id,
  path: `/${id}`,
  rootPath: "/",
  allocatedBytes,
  status,
  reason: null,
});
const fmt = (n: number) => `${n}B`;

it("maps entry state to band segments in reading order", () => {
  const here = entry("here", 60);
  const entries = [
    entry("missing", 5, "missing"),
    entry("scan", 10, "needsScan"),
    entry("elsewhere", 20),
    here,
    entry("changed", 5, "changed"),
  ];
  const segments = bandSegments(entries, (e) => e === here, 1, fmt);
  expect(segments.map((s) => [s.kind, s.bytes])).toEqual([
    ["ready", 60],
    ["elsewhere", 20],
    ["needsScan", 10],
    ["attention", 10],
  ]);
  expect(segments[0].label).toBe("Ready to review 60B");
});

it("omits empty states and hides labels on slices too narrow to hold them", () => {
  const here = entry("here", 95);
  const segments = bandSegments(
    [here, entry("scan", 5, "needsScan")],
    (e) => e === here,
    1,
    fmt,
  );
  expect(segments.map((s) => s.kind)).toEqual(["ready", "needsScan"]);
  expect(segments.map((s) => s.showLabel)).toEqual([true, false]);
  // The same slices on a short band: nothing fits.
  expect(
    bandSegments([here], (e) => e === here, 0.1, fmt).map((s) => s.showLabel),
  ).toEqual([false]);
});

it("reads the band as a sentence", () => {
  const here = entry("here", 11);
  const segments = bandSegments(
    [here, entry("scan", 1, "needsScan")],
    (e) => e === here,
    1,
    fmt,
  );
  expect(bandSummary(segments, fmt)).toBe(
    "12B: 11B ready to review, 1B needs a scan",
  );
});

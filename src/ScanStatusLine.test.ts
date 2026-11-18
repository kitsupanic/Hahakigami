// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => new Promise(() => {})),
}));
import ScanStatusLine from "./ScanStatusLine.svelte";
import { act, createTestRoot, type Root } from "./testUtils";
import { formatScanTimestamp } from "./uiFormat";
import type { ScanSummary } from "./types";

let container: HTMLDivElement, root: Root;
const base: ScanSummary = {
  scanId: "scan",
  rootId: 1,
  state: "running",
  scanPhase: "scanning",
  workers: 4,
  workerStats: [],
  finalizationUs: 0,
  entries: 1250,
  allocatedBytes: 2048,
  elapsedMs: 1234,
  errors: 0,
};
beforeEach(() => {
  container = document.createElement("div");
  document.body.append(container);
  root = createTestRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
});
async function render(
  summary: Partial<ScanSummary> = {},
  props: {
    cancelling?: boolean;
  } = {},
) {
  await act(() =>
    root.render(ScanStatusLine, {
      summary: { ...base, ...summary },
      cancelling: false,
      ...props,
    }),
  );
}
const status = () => container.querySelector('[role="status"]')?.textContent;
const popover = () =>
  container.querySelector<HTMLElement>(".scan-details-popover");
const press = (selector: string) =>
  act(() => container.querySelector<HTMLElement>(selector)!.click());

it("announces phases without announcing rapidly updating counters or early completion", async () => {
  await render({ scanPhase: "checking", entries: 0, elapsedMs: 0 });
  expect(status()).toBe("Checking previous scan…");
  expect(
    container.querySelector(".scan-information-description")?.textContent,
  ).toContain("Changes may require a full scan.");
  expect(popover()?.textContent).toContain("0.0 seconds");
  await render();
  expect(status()).toBe("Scanning…");
  expect(container.querySelector(".scan-timestamp")?.textContent).toBe(
    "Scanning Counting files…",
  );
  expect(container.querySelector(".scan-progress")).not.toBeNull();
  expect(
    [...container.querySelectorAll(".scan-totals .status-value")].map(
      (v) => v.textContent,
    ),
  ).toEqual(["Counting files…", "1.2 seconds", "2 KiB", "1.3K"]);
  // Scanned and Time sit inside the totals button, so they open Summary too.
  expect(container.querySelector(".scan-totals .scan-timestamp")).not.toBeNull();
  expect(container.querySelector(".scan-totals .scan-elapsed")).not.toBeNull();
  expect(popover()?.textContent).toContain("1,250");
  await render({ scanPhase: "verifying", walkMs: 900 });
  expect(status()).toBe("Verifying shared clone sizes…");
  expect(
    container.querySelector(".scan-information-description")?.textContent,
  ).toContain("can be explored");
  await render({ scanPhase: "caching", completedAtMs: 100 });
  expect(status()).toBe("Finishing scan…");
  expect(container.querySelector('[role="progressbar"]')).toBeNull();
  expect(container.querySelector(".scan-timestamp small")?.textContent).toBe(
    "Scanning",
  );
  await render({ state: "completed", scanPhase: "done" });
  expect(status()).toBe("Scan complete");
  expect(container.querySelector(".scan-timestamp small")?.textContent).toBe(
    "Scanned",
  );
  expect(container.querySelector(".scan-progress")).toBeNull();
  expect(container.querySelector(".scan-state-description")).toBeNull();
});
it("shows the phase instead of a chip or timestamp while a scan runs", async () => {
  await render({ startedAtMs: 1000 });
  expect(
    container.querySelector(".scan-timestamp .status-value")?.textContent,
  ).toBe("Counting files…");
  await render({}, { cancelling: true });
  expect(status()).toBe("Stopping scan…");
  expect(
    container.querySelector(".scan-timestamp .status-value")?.textContent,
  ).toBe("Stopping…");
  expect(
    container.querySelector(".scan-information-description")?.textContent,
  ).toContain("current filesystem operation");
});
it("keeps cancellation and failure as partial results with guidance inline", async () => {
  await render({ state: "cancelled" });
  expect(status()).toBe("Scan cancelled · partial results");
  expect(container.querySelector(".scan-timestamp small")?.textContent).toBe(
    "Cancelled",
  );
  expect(
    container.querySelector(".scan-state-description")?.textContent,
  ).toContain("Scan again for complete results.");
  await render({ state: "failed", message: "Filesystem unavailable" });
  expect(status()).toBe("Scan failed");
  expect(container.querySelector(".scan-timestamp small")?.textContent).toBe(
    "Scan failed",
  );
  expect(
    container.querySelector(".scan-state-description")?.textContent,
  ).toContain("results shown are partial");
  expect(popover()?.textContent).toContain("Filesystem unavailable");
});
it.each([
  [{ errors: 3, restrictedSystemIssues: 1 }, "Scan complete · scan issues"],
  [{ errors: 2, restrictedSystemIssues: -1 }, "Scan complete · scan issues"],
  [{ incomplete: true }, "Scan complete · limited coverage"],
])("reports incomplete coverage for %j", async (summary, title) => {
  await render({ state: "completed", ...summary });
  expect(status()).toBe(title);
  expect(container.querySelector(".scan-information-description")).toBeNull();
  expect(container.querySelector(".scan-coverage-chip")).not.toBeNull();
});
it("treats restricted-only and excluded-only scans as plain complete with no coverage chip or tab", async () => {
  for (const summary of [
    { errors: 2, restrictedSystemIssues: 99 },
    { excluded: 6, restrictedSystemIssues: 99 },
  ]) {
    await render({ state: "completed", ...summary });
    expect(status()).toBe("Scan complete");
    expect(container.querySelector(".scan-coverage-chip")).toBeNull();
    expect(
      [...popover()!.querySelectorAll('[role="tab"]')].map((n) => n.textContent),
    ).toEqual(["Summary", "Workers4"]);
    expect(container.textContent).not.toContain("restricted");
    expect(container.textContent).not.toContain("excluded locations");
  }
});
it("puts totals, coverage and workers in one popover with a tab each", async () => {
  await render({
    state: "completed",
    scanMode: "incremental",
    reusedEntries: 42,
    accountingModel: "fullClonesV1",
  });
  expect(container.querySelectorAll(".scan-details-popover")).toHaveLength(1);
  expect(container.querySelectorAll(".scan-status-line details")).toHaveLength(0);
  expect(container.textContent).not.toMatch(/\bInfo\b/);
  expect(popover()?.hidden).toBe(true);
  expect(
    [...popover()!.querySelectorAll('[role="tab"]')].map((n) => n.textContent),
  ).toEqual(["Summary", "Workers4"]);
  expect(
    [...popover()!.querySelectorAll(".scan-facts dt")].map((n) =>
      n.textContent?.trim(),
    ),
  ).toEqual(["On disk", "Entries", "Scan duration", "Scan type"]);
  expect(popover()!.querySelector(".scan-facts .active dt")?.textContent).toBe(
    "On disk",
  );
  expect(popover()!.textContent).toContain("42 metadata entries reused");
  expect(popover()!.textContent).not.toContain("How sizes are counted");
  await press(".scan-totals");
  expect(popover()?.hidden).toBe(false);
  expect(
    popover()!.querySelector('[aria-selected="true"]')?.textContent,
  ).toBe("Summary");
  await press('[role="tab"]:last-child');
  expect(popover()!.querySelector(".worker-diagnostics")?.closest("[hidden]")).toBeNull();
  await press(".scan-totals");
  expect(popover()?.hidden).toBe(true);
  await press(".scan-totals");
  await render({ scanId: "next-scan", state: "completed" });
  expect(popover()?.hidden).toBe(true);
});
it("shows the exact on-disk size in the popover", async () => {
  await render({});
  expect(popover()?.textContent).toContain("2 KiB (2.05 KB)");
  expect(popover()?.textContent).toContain("On disk so far");
});
it("shows nonfatal cache trouble with its exact reason discoverable", async () => {
  await render({
    state: "completed",
    cacheWarning: "Cache disk is full",
    scanMode: "full",
    fallbackReason: "Previous metadata is stale",
  });
  expect(status()).toBe("Scan complete");
  expect(container.querySelector(".scan-cache-note")?.textContent).toBe(
    "Rescan cache unavailable",
  );
  expect(
    container.querySelector(".scan-cache-note")?.getAttribute("title"),
  ).toBe("Cache disk is full");
  expect(popover()?.textContent).toContain("Cache disk is full");
  expect(popover()?.textContent).not.toContain("Previous metadata is stale");
});
it("shows an epoch-zero completion time rather than treating it as missing", async () => {
  await render({ state: "completed", completedAtMs: 0, startedAtMs: 1000 });
  expect(
    container.querySelector(".scan-timestamp .status-value")?.textContent,
  ).toBe(formatScanTimestamp(0).label);
  expect(
    container.querySelector(".scan-timestamp")?.getAttribute("title"),
  ).toBe(`Scan completed ${formatScanTimestamp(0).absolute}`);
});
it.each([
  [61234, "1m 1s"],
  [3661234, "1h 1m 1s"],
])("formats reported scan time %d", async (elapsedMs, label) => {
  await render({ elapsedMs });
  expect(popover()?.textContent).toContain(label);
});
it("uses completion time for a finished live scan", async () => {
  await render({ state: "completed", startedAtMs: 1000, completedAtMs: 2000 });
  expect(
    container.querySelector(".scan-timestamp")?.getAttribute("title"),
  ).toBe(`Scan completed ${formatScanTimestamp(2000).absolute}`);
  await render({ state: "cancelled", startedAtMs: 1000 });
  expect(
    container.querySelector(".scan-timestamp")?.getAttribute("title"),
  ).toBe(`Scan started ${formatScanTimestamp(1000).absolute}`);
});
it("opens the shared popover on Coverage from the coverage cell", async () => {
  await render({ state: "completed", errors: 2, restrictedSystemIssues: 0 });
  expect(
    [...popover()!.querySelectorAll('[role="tab"]')].map((n) => n.textContent),
  ).toEqual(["Summary", "Coverage2", "Workers4"]);
  await press(".scan-coverage-chip");
  expect(popover()?.hidden).toBe(false);
  expect(
    popover()!.querySelector('[aria-selected="true"]')?.textContent,
  ).toBe("Coverage2");
  expect(container.querySelector(".scan-status-details")).not.toBeNull();
  // Switching opener keeps one popover and moves to that opener's tab.
  await press(".scan-totals");
  expect(
    popover()!.querySelector('[aria-selected="true"]')?.textContent,
  ).toBe("Summary");
  expect(container.querySelector(".scan-status-details")).toBeNull();
  await press(".scan-coverage-chip");
  const tab = popover()!.querySelector<HTMLElement>('[aria-selected="true"]')!;
  tab.focus();
  await act(() =>
    tab.dispatchEvent(
      new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }),
    ),
  );
  expect(document.activeElement?.textContent).toBe("Workers4");
  await act(() =>
    tab.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    ),
  );
  expect(popover()?.hidden).toBe(true);
  expect(document.activeElement).toBe(
    container.querySelector(".scan-coverage-chip"),
  );
  await render({ state: "completed" });
  expect(container.querySelector(".scan-coverage-chip")).toBeNull();
});
it("lists worker measures as rows so every worker fits as a column", async () => {
  const worker = {
    directories: 5,
    relativeOpens: 5,
    absoluteReopens: 0,
    busyUs: 1000,
    cpuUs: null,
    metadataUs: 0,
    storeWaitUs: 0,
    commitUs: 0,
    queueWaitUs: 0,
    progressUs: 0,
    elapsedUs: 2000,
    errors: 0,
  };
  await render({
    state: "completed",
    entries: 101,
    workerStats: [
      { ...worker, id: 1, entries: 75 },
      { ...worker, id: 2, entries: 25 },
    ] as ScanSummary["workerStats"],
  });
  expect(
    [...container.querySelectorAll(".share-bars tbody tr")].map(
      (row) => row.textContent,
    ),
  ).toEqual(["17575.0%", "22525.0%"]);
  expect(container.querySelectorAll(".measures tbody tr")).toHaveLength(14);
  expect(container.querySelectorAll(".measures thead th")).toHaveLength(3);
  expect(
    container.querySelector(".measures tbody tr:nth-child(5)")?.textContent,
  ).toBe("CPUUnavailableUnavailable");
});

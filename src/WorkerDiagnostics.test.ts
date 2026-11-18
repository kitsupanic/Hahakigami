// @vitest-environment jsdom
import { act, createTestRoot as createRoot, type Root } from "./testUtils";
import { expect, it, vi } from "vitest";
import WorkerDiagnostics, { entryShare } from "./WorkerDiagnostics.svelte";
import type { ScanSummary, WorkerStats } from "./types";

const worker = (id: number, entries: number): WorkerStats => ({
  id,
  entries,
  directories: 2,
  errors: 0,
  elapsedUs: 10_000_000,
  busyUs: 4_000_000,
  cpuUs: 1_200_000,
  queueWaitUs: 6_000_000,
  storeWaitUs: 1_400_000,
  metadataUs: 1_600_000,
  commitUs: 500_000,
  progressUs: 500_000,
  qos: "default",
  relativeOpens: 2,
  absoluteReopens: 0,
});
const summary = (state: ScanSummary["state"] = "completed"): ScanSummary => ({
  scanId: "scan-1",
  rootId: 0,
  state,
  workers: 2,
  qos: "userInitiated",
  entries: 11,
  errors: 0,
  elapsedMs: 10200,
  allocatedBytes: 1024,
  finalizationUs: 200_000,
  descriptorLimit: 192,
  descriptorPeak: 180,
  workerStats: [worker(1, 3), { ...worker(2, 7), cpuUs: null }],
});
it("defines result share using accepted entries without the pre-created root", () => {
  expect(entryShare(3, 11)).toBe(30);
  expect(entryShare(7, 11)).toBe(70);
  expect(entryShare(0, 1)).toBe(0);
  expect(entryShare(0, 0)).toBe(0);
});
it("distinguishes processing/CPU/waits and live versus unavailable CPU readings", async () => {
  const element = document.createElement("div"),
    root = createRoot(element);
  try {
    await act(async () =>
      root.render(WorkerDiagnostics, { summary: summary("running") }),
    );
    expect(element.querySelectorAll(".share-bars tbody tr")).toHaveLength(2);
    expect((element.textContent ?? "").replace(/\s+/g, " ")).toContain("30.0%");
    expect((element.textContent ?? "").replace(/\s+/g, " ")).toContain("70.0%");
    expect((element.textContent ?? "").replace(/\s+/g, " ")).toContain("Pending");
    expect((element.textContent ?? "").replace(/\s+/g, " ")).toContain("1.20 s");
    expect((element.textContent ?? "").replace(/\s+/g, " ")).toContain("4.00 s");
    expect((element.textContent ?? "").replace(/\s+/g, " ")).toContain("6.00 s");
    expect(element.textContent).not.toContain("P/E core placement");
    await act(async () =>
      root.render(WorkerDiagnostics, { summary: summary() }),
    );
    expect((element.textContent ?? "").replace(/\s+/g, " ")).toContain("Unavailable");
    expect((element.textContent ?? "").replace(/\s+/g, " ")).toContain("200.0 ms");
    expect((element.textContent ?? "").replace(/\s+/g, " ")).toContain("180 peak / 192 limit");
    await act(async () =>
      root.render(WorkerDiagnostics, {
        summary: { ...summary(), entries: 1, workerStats: [worker(1, 0)] },
      }),
    );
    expect((element.textContent ?? "").replace(/\s+/g, " ")).toContain("0.0%");
    expect((element.textContent ?? "").replace(/\s+/g, " ")).not.toContain("NaN");
  } finally {
    await act(async () => root.unmount());
  }
});
it("copies a bounded report with explicit shares and timing semantics", async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  const original = Object.getOwnPropertyDescriptor(navigator, "clipboard");
  Object.defineProperty(navigator, "clipboard", {
    value: { writeText },
    configurable: true,
  });
  const element = document.createElement("div"),
    root = createRoot(element);
  try {
    await act(async () =>
      root.render(WorkerDiagnostics, { summary: summary() }),
    );
    await act(async () =>
      (element.querySelector("button") as HTMLButtonElement).click(),
    );
    const report = JSON.parse(writeText.mock.calls[0][0]);
    expect(report.qos).toBe("userInitiated");
    expect(report.scanId).toBe("scan-1");
    expect(report.workerStats[0].entrySharePercent).toBe(30);
    expect(report.workerStats[1].cpuUs).toBeNull();
    expect(report.finalizationUs).toBe(200000);
    expect(report.descriptorLimit).toBe(192);
    expect(report.workerStats[0].relativeOpens).toBe(2);
    expect(report.notes).toContain(
      "does not identify Performance/Efficiency core placement",
    );
    expect((element.textContent ?? "").replace(/\s+/g, " ")).toContain("Worker report copied.");
    writeText.mockRejectedValueOnce(new Error("Clipboard blocked"));
    await act(async () =>
      (element.querySelector("button") as HTMLButtonElement).click(),
    );
    expect((element.textContent ?? "").replace(/\s+/g, " ")).toContain("Clipboard blocked");
  } finally {
    await act(async () => root.unmount());
    if (original) Object.defineProperty(navigator, "clipboard", original);
    else Reflect.deleteProperty(navigator, "clipboard");
  }
});

it("reports a QoS setter failure separately from filesystem errors", async () => {
  const element = document.createElement("div"),
    root = createRoot(element);
  try {
    await act(async () =>
      root.render(WorkerDiagnostics, {
        summary: {
          ...summary(),
          workerStats: [{ ...worker(1, 10), qosError: 1 }],
        },
      }),
    );
    expect(element.querySelector('[role="alert"]')?.textContent).toContain(
      "could not apply",
    );
    expect((element.textContent ?? "").replace(/\s+/g, " ")).toContain("Prioritize scan");
    expect(
      element.querySelector(".measures tbody tr:last-child td")?.textContent,
    ).toBe("1");
  } finally {
    await act(async () => root.unmount());
  }
});

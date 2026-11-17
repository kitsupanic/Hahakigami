// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import ScanTransferFeedback from "./ScanTransferFeedback.svelte";
import { act, createTestRoot, type Root } from "./testUtils";
import type { ScanTransferReport } from "./scanTransferTypes";

const base: ScanTransferReport = {
  id: 1,
  operation: "import",
  state: "pending",
  phase: "choosing",
  bytes: 0,
  entry: null,
  error: null,
};
let container: HTMLDivElement, root: Root;
beforeEach(() => {
  vi.clearAllMocks();
  container = document.createElement("div");
  document.body.append(container);
  root = createTestRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});
async function render(report: Partial<ScanTransferReport> = {}) {
  await act(() =>
    root.render(ScanTransferFeedback, {
      report: { ...base, ...report },
    }),
  );
}
const text = () => container.textContent?.replace(/\s+/g, " ");

it.each(["import", "export"] as const)(
  "explains %s file selection without a pretend cancellation action",
  async (operation) => {
    await render({ operation });
    expect(text()).toContain(
      operation === "import"
        ? "Choose a shared scan file in the file dialog."
        : "Choose a destination in the save dialog.",
    );
    expect(container.querySelector(".transfer-activity")).toBeNull();
    expect(container.querySelector("button")).toBeNull();
    expect(container.querySelector("section")?.dataset.operation).toBe(
      operation,
    );
    expect(container.querySelector('[role="status"]')?.textContent).toContain(
      operation === "import"
        ? "Choose a shared scan file in the file dialog."
        : "Choose a destination in the save dialog.",
    );
  },
);
it.each(["import", "export"] as const)(
  "labels %s processing without promising totals or success",
  async (operation) => {
    await render({ operation, phase: "processing", bytes: 4096 });
    expect(text()).toContain(
      operation === "import"
        ? "Checking the archive and adding it to Saved Scans…"
        : "Preparing and writing the shared scan…",
    );
    const status = container.querySelector('[role="status"]')!;
    expect(status.querySelector("h2")?.textContent).toBe(
      operation === "import" ? "Importing scan…" : "Exporting scan…",
    );
    expect(status.textContent).toContain(
      operation === "import"
        ? "Checking the archive and adding it to Saved Scans…"
        : "Preparing and writing the shared scan…",
    );
    const processingAnnouncement = status.textContent;
    expect(status.textContent).not.toContain("4");
    expect(container.querySelector(".transfer-bytes")?.textContent).toContain(
      "Scan data processed",
    );
    expect(
      container.querySelector(".transfer-bytes")?.getAttribute("title"),
    ).toContain("can start over");
    expect(
      container
        .querySelector(".transfer-activity")
        ?.getAttribute("aria-hidden"),
    ).toBe("true");
    await render({ operation, phase: "processing", bytes: 0 });
    expect(container.querySelector(".transfer-bytes")).toBeNull();
    expect(container.querySelector('[role="status"]')?.textContent).toBe(
      processingAnnouncement,
    );
    await render({ operation, phase: "finishing", bytes: 100000 });
    expect(text()).toContain(`Finishing ${operation}…`);
    expect(container.querySelector('[role="status"]')?.textContent).toContain(
      `Finishing ${operation}…`,
    );
    expect(container.querySelector('[role="status"]')?.textContent).not.toBe(
      processingAnnouncement,
    );
    expect(text()).not.toMatch(/Scan imported|Scan exported|%|ETA/);
    expect(container.querySelector(".transfer-bytes")).toBeNull();
  },
);

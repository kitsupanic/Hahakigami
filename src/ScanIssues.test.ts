// @vitest-environment jsdom
import { act, createTestRoot as createRoot, type Root } from "./testUtils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
// The coverage panel is driven through the popover that owns its chip and tab.
import ScanDetails from "./ScanDetails.svelte";
import ScanIssues from "./ScanIssues.svelte";
import type { ScanSummary, ScanIssuePage } from "./types";
const mock = vi.hoisted(() => ({ invoke: vi.fn(), writeText: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mock.invoke }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: mock.writeText,
}));
const summary: ScanSummary = {
  scanId: "scan-1",
  rootId: 0,
  state: "completed",
  workers: 4,
  workerStats: [],
  finalizationUs: 0,
  entries: 10,
  allocatedBytes: 1,
  elapsedMs: 10,
  errors: 2,
  excluded: 1,
};
let container: HTMLDivElement, root: Root;
const detailsProps = {
  description: "",
};
const page = (
  kind: "errors" | "other" | "restricted" | "excluded" = "other",
  offset = 0,
): ScanIssuePage => ({
  scanId: "scan-1",
  nodeId: 0,
  kind,
  offset,
  total: 2,
  retainedTotal: 2,
  items: [
    {
      path: "/fixture/private",
      message: "Permission denied",
      errorKind: "PermissionDenied",
      rawOsError: 13,
    },
    {
      path: "/fixture/gone",
      message: "Entry disappeared",
      errorKind: "NotFound",
      rawOsError: 2,
    },
  ],
});
async function click(label: string) {
  if (
    label === "Copy errors" &&
    !container.querySelector(".scan-status-details")
  ) {
    await click("Show scan details");
  }
  const button = Array.from(container.querySelectorAll("button")).find(
    (b) => (b.getAttribute("aria-label") ?? b.textContent) === label,
  )!;
  expect(button).toBeTruthy();
  await act(async () => button.click());
}
beforeEach(async () => {
  mock.writeText.mockReset().mockResolvedValue(undefined);
  mock.invoke
    .mockReset()
    .mockImplementation(async (_command, args) => page(args.kind, args.offset));
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await act(async () => root.render(ScanDetails, {
      ...detailsProps, summary: summary }));
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});
it("shows exact paths and different reasons without suggesting FDA for all errors", async () => {
  expect(mock.invoke).not.toHaveBeenCalled();
  await click("Show scan details");
  expect(mock.invoke).toHaveBeenCalledWith("get_scan_issues", {
    scanId: "scan-1",
    kind: "other",
    offset: 0,
    limit: 100,
  });
  expect((container.textContent ?? "").replace(/\s+/g, " ")).toContain(
    "/fixture/private",
  );
  expect((container.textContent ?? "").replace(/\s+/g, " ")).toContain(
    "Entry disappeared",
  );
  expect((container.textContent ?? "").replace(/\s+/g, " ")).toContain(
    "OS error 13",
  );
  expect((container.textContent ?? "").replace(/\s+/g, " ")).toContain(
    "One folder problem can affect everything inside it",
  );
  expect((container.textContent ?? "").replace(/\s+/g, " ")).toContain(
    "Full Disk Access can't get past file permissions",
  );
  expect((container.textContent ?? "").replace(/\s+/g, " ")).toContain(
    "doesn't tell you which one blocked the scan",
  );
});
it("hides expected exclusions from the main view", async () => {
  await act(async () =>
    root.render(ScanDetails, {
      ...detailsProps,
      summary: { ...summary, errors: 0, excluded: 3 },
    }),
  );
  expect(container.querySelector(".scan-issue-warning")).toBeNull();
  expect(container.querySelector(".scan-coverage-chip")).toBeNull();
  expect(container.textContent).not.toContain("excluded locations");
  expect(container.textContent).not.toContain("Limited coverage");
  expect(container.textContent).not.toContain("Copy errors");
});
it("paginates retained details and shows truncation", async () => {
  mock.invoke.mockImplementation(async (_command, args) => ({
    ...page(args.kind, args.offset),
    total: 500,
    retainedTotal: 101,
    items: [page().items[0]],
  }));
  await click("Show scan details");
  expect((container.textContent ?? "").replace(/\s+/g, " ")).toContain(
    "101 of 500 events",
  );
  await click("Next details");
  expect(mock.invoke).toHaveBeenLastCalledWith("get_scan_issues", {
    scanId: "scan-1",
    kind: "other",
    offset: 100,
    limit: 100,
  });
  expect(
    Array.from(container.querySelectorAll("button")).find(
      (b) => b.textContent === "Next details",
    )!.disabled,
  ).toBe(true);
  await click("Previous details");
  expect(mock.invoke.mock.calls.at(-1)![1].offset).toBe(0);
});
it("retries failed reads", async () => {
  mock.invoke.mockRejectedValueOnce(new Error("Read failed"));
  await click("Show scan details");
  expect((container.textContent ?? "").replace(/\s+/g, " ")).toContain(
    "Read failed",
  );
  mock.invoke.mockResolvedValueOnce(page());
  await click("Retry scan details");
  expect((container.textContent ?? "").replace(/\s+/g, " ")).toContain(
    "/fixture/private",
  );
});
it("rejects mismatched native scan IDs", async () => {
  mock.invoke.mockResolvedValue({ ...page(), scanId: "old-scan" });
  await click("Show scan details");
  expect((container.textContent ?? "").replace(/\s+/g, " ")).toContain(
    "does not match",
  );
  expect((container.textContent ?? "").replace(/\s+/g, " ")).not.toContain(
    "/fixture/private",
  );
});

it("resets expanded details for a new keyed scan and discards its predecessor response", async () => {
  let resolveOld: (value: ScanIssuePage) => void = () => {};
  mock.invoke.mockImplementation(
    async () =>
      new Promise((resolve) => {
        resolveOld = resolve;
      }),
  );
  await click("Show scan details");
  await act(async () =>
    root.render(ScanDetails, {
      ...detailsProps,
      key: "scan-2",
      summary: { ...summary, scanId: "scan-2" },
    }),
  );
  await act(async () => resolveOld(page()));
  expect((container.textContent ?? "").replace(/\s+/g, " ")).not.toContain(
    "/fixture/private",
  );
  expect(
    container.querySelector('button[aria-label="Show scan details"]'),
  ).not.toBeNull();
});

it("R11 opens the other-issues list after a running scan and reloads it on reopen", async () => {
  await act(async () =>
    root.render(ScanDetails, {
      ...detailsProps,
      key: "live",
      summary: { ...summary, state: "running", errors: 0, excluded: 0 },
    }),
  );
  expect(container.querySelector(".scan-coverage-chip")).toBeNull();
  await act(async () =>
    root.render(ScanDetails, {
      ...detailsProps,
      key: "live",
      summary: { ...summary, state: "completed", errors: 2, excluded: 0 },
    }),
  );
  await click("Show scan details");
  expect(mock.invoke).toHaveBeenLastCalledWith("get_scan_issues", {
    scanId: "scan-1",
    kind: "other",
    offset: 0,
    limit: 100,
  });
  await click("Hide scan details");
  await click("Show scan details");
  expect(mock.invoke).toHaveBeenLastCalledWith("get_scan_issues", {
    scanId: "scan-1",
    kind: "other",
    offset: 0,
    limit: 100,
  });
});

it("copies all retained error pages preserving unusual paths and repeated events", async () => {
  const items = Array.from({ length: 103 }, (_, i) => ({
    path: i < 2 ? '/fixture/quote" 日本語\nchild' : `/fixture/failure-${i}`,
    message: "Operation not permitted (os error 1)",
    errorKind: "PermissionDenied",
    rawOsError: 1,
  }));
  mock.invoke.mockImplementation(async (_command, args) => ({
    ...page(args.kind, args.offset),
    total: 103,
    retainedTotal: 103,
    items:
      args.kind === "errors"
        ? items.slice(args.offset, args.offset + args.limit)
        : [],
  }));
  await click("Show scan details");
  mock.invoke.mockClear();
  await click("Copy errors");
  expect(mock.invoke.mock.calls.map((call) => call[1])).toEqual([
    { scanId: "scan-1", kind: "errors", offset: 0, limit: 100 },
    { scanId: "scan-1", kind: "errors", offset: 100, limit: 100 },
  ]);
  expect(mock.writeText).toHaveBeenCalledTimes(1);
  const report = JSON.parse(mock.writeText.mock.calls[0][0]);
  expect(report.issues).toEqual(items);
  expect(report.scanId).toBe("scan-1");
  expect(report.rootNodeId).toBe(0);
  expect(report.totalIssues).toBe(103);
  expect(report.copiedIssues).toBe(103);
  expect(report.truncated).toBe(false);
  expect(report.omittedIssues).toBe(0);
  await vi.waitFor(() =>
    expect((container.textContent ?? "").replace(/\s+/g, " ")).toContain(
      "Copied 103 scan issues.",
    ),
  );
});

it("reports omitted details and partial scan state in the copied report", async () => {
  await act(async () =>
    root.render(ScanDetails, {
      ...detailsProps, summary: { ...summary, state: "cancelled" } }),
  );
  mock.invoke.mockResolvedValue({
    ...page("errors"),
    total: 12000,
    retainedTotal: 2,
  });
  await click("Copy errors");
  const report = JSON.parse(mock.writeText.mock.calls[0][0]);
  expect(report.truncated).toBe(true);
  expect(report.omittedIssues).toBe(11998);
  expect(report.scanState).toBe("cancelled");
  expect(report.note).toContain("partial");
  expect((container.textContent ?? "").replace(/\s+/g, " ")).toContain(
    "details truncated",
  );
});

it("does not write a partial report if a later page fails or returns a stale scan", async () => {
  mock.invoke.mockImplementation(async (_command, args) => {
    if (args.offset) throw new Error("Results changed after cleanup");
    return {
      ...page("errors"),
      total: 101,
      retainedTotal: 101,
      items: Array.from({ length: 100 }, () => page("errors").items[0]),
    };
  });
  await click("Copy errors");
  expect(mock.writeText).not.toHaveBeenCalled();
  expect((container.textContent ?? "").replace(/\s+/g, " ")).toContain(
    "Could not copy errors",
  );
  mock.invoke.mockResolvedValue({ ...page("errors"), scanId: "stale-scan" });
  await click("Copy errors");
  expect(mock.writeText).not.toHaveBeenCalled();
  expect((container.textContent ?? "").replace(/\s+/g, " ")).toContain(
    "does not match",
  );
});

it("disables copying while pending and discards an old scan request before writing the clipboard", async () => {
  let resolve: (value: ScanIssuePage) => void = () => {};
  mock.invoke.mockImplementation(
    async () =>
      new Promise<ScanIssuePage>((r) => {
        resolve = r;
      }),
  );
  await click("Copy errors");
  const button = Array.from(container.querySelectorAll("button")).find(
    (b) => b.textContent === "Copying errors…",
  )!;
  expect(button.disabled).toBe(true);
  await act(async () =>
    root.render(ScanDetails, {
      ...detailsProps,
      key: "new-scan",
      summary: { ...summary, scanId: "scan-2" },
    }),
  );
  await act(async () => resolve(page("errors")));
  expect(mock.writeText).not.toHaveBeenCalled();
  expect((container.textContent ?? "").replace(/\s+/g, " ")).not.toContain(
    "Copied",
  );
});

it("surfaces native clipboard failures and allows retry", async () => {
  mock.writeText.mockRejectedValueOnce(new Error("Clipboard unavailable"));
  await click("Copy errors");
  expect((container.textContent ?? "").replace(/\s+/g, " ")).toContain(
    "Could not copy errors: Error: Clipboard unavailable",
  );
  await click("Copy errors");
  expect(mock.writeText).toHaveBeenCalledTimes(2);
  expect((container.textContent ?? "").replace(/\s+/g, " ")).toContain(
    "Copied 2 scan issues.",
  );
  expect((container.textContent ?? "").replace(/\s+/g, " ")).not.toContain(
    "Could not copy errors",
  );
});

it("hides restricted-only scans from the main view", async () => {
  await act(async () =>
    root.render(ScanDetails, {
      ...detailsProps,
      key: "restricted",
      summary: { ...summary, restrictedSystemIssues: 2, incomplete: true },
    }),
  );
  expect(container.querySelector(".scan-issue-warning")).toBeNull();
  expect(container.querySelector(".scan-coverage-chip")).toBeNull();
  expect(container.textContent).not.toContain("restricted system locations");
  expect(container.textContent).not.toContain("Coverage");
});
it("shows only other-issue counts for mixed scans and lists the other kind", async () => {
  await act(async () =>
    root.render(ScanDetails, {
      ...detailsProps,
      key: "mixed",
      summary: { ...summary, errors: 5, restrictedSystemIssues: 3 },
    }),
  );
  expect(container.querySelector(".has-other-issues")).not.toBeNull();
  await click("Show scan details");
  expect(mock.invoke.mock.calls.at(-1)![1].kind).toBe("other");
  const text = (container.textContent ?? "").replace(/\s+/g, " ");
  expect(text).toContain("Coverage2");
  expect(text).not.toContain("Restricted system locations");
  expect(text).not.toContain("Excluded locations");
  mock.invoke.mockResolvedValueOnce({
    ...page("errors"),
    total: 1,
    retainedTotal: 1,
    items: [
      {
        path: "/System/Library/Protected",
        message: "Restricted",
        category: "restrictedSystem",
      },
    ],
  });
  await click("Copy errors");
  expect(mock.invoke.mock.calls.at(-1)![1].kind).toBe("errors");
  const report = JSON.parse(mock.writeText.mock.calls.at(-1)![0]);
  expect(report.issues[0].category).toBe("restrictedSystem");
});

it("R2 keeps a same-scan copy alive and avoids detail reloads for elapsed-only progress", async () => {
  await click("Show scan details");
  const detailCalls = mock.invoke.mock.calls.length;
  await act(async () =>
    root.render(ScanDetails, {
      ...detailsProps, summary: { ...summary, elapsedMs: 11 } }),
  );
  expect(mock.invoke).toHaveBeenCalledTimes(detailCalls);
  let resolve!: (result: ScanIssuePage) => void;
  mock.invoke.mockImplementationOnce(
    () =>
      new Promise<ScanIssuePage>((done) => {
        resolve = done;
      }),
  );
  await click("Copy errors");
  await act(async () =>
    root.render(ScanDetails, {
      ...detailsProps, summary: { ...summary, elapsedMs: 12 } }),
  );
  expect(container.textContent).toContain("Copying errors…");
  expect(mock.invoke).toHaveBeenCalledTimes(detailCalls + 1);
  await act(async () => resolve(page("errors")));
  await vi.waitFor(() =>
    expect(container.textContent).toContain("Copied 2 scan issues."),
  );
  expect(mock.writeText).toHaveBeenCalledOnce();
  await act(async () =>
    root.render(ScanDetails, {
      ...detailsProps, summary: { ...summary, elapsedMs: 13 } }),
  );
  expect(container.textContent).toContain("Copied 2 scan issues.");
  expect(mock.invoke).toHaveBeenCalledTimes(detailCalls + 1);
});

it("keeps completeness visible and other-issue counts accessible while explanations are collapsed", async () => {
  await act(async () =>
    root.render(ScanDetails, {
      ...detailsProps,
      summary: {
        ...summary,
        errors: 5,
        restrictedSystemIssues: 3,
        excluded: 4,
      },
    }),
  );
  const status = container.querySelector(".scan-status-row")!;
  expect(status.textContent).toContain("Limited coverage");
  expect(status.textContent).toContain("2 issue events");
  expect(
    status.querySelector('[aria-label="2 other scan issue events"]'),
  ).not.toBeNull();
  expect(status.textContent).not.toContain("restricted");
  expect(status.textContent).not.toContain("excluded");
  expect(status.textContent).not.toContain("Copy errors");
  expect(status.querySelector("button")?.textContent).toBe(
    "CoverageLimited · Details",
  );
  expect(container.querySelector(".scan-status-details")).toBeNull();
  expect(container.textContent).not.toContain("Full Disk Access");
  expect(mock.invoke).not.toHaveBeenCalled();
  await click("Show scan details");
  expect(container.querySelector(".scan-status-details")).not.toBeNull();
  expect(
    container.querySelector('button[aria-label="Hide scan details"]')
      ?.textContent,
  ).toBe("CoverageLimited · Hide details");
  expect(
    container.querySelector(".scan-status-details")?.textContent,
  ).toContain("Copy errors");
  expect(container.textContent).toContain("Full Disk Access");
  await click("Hide scan details");
  expect(container.querySelector(".scan-status-details")).toBeNull();
  expect(status.textContent).toContain("Limited coverage");
});

it("shows coverage as a chip with accessible counts and dismisses details using Escape or outside click", async () => {
  const trigger = container.querySelector<HTMLButtonElement>(
    ".scan-coverage-chip",
  )!;
  expect(trigger.textContent).toBe("CoverageLimited · Details");
  expect(trigger.title).toContain("2 other scan issue events");
  expect(trigger.title).not.toContain("excluded");
  expect(trigger.title).not.toContain("restricted");
  expect(trigger.getAttribute("aria-describedby")).toBeTruthy();
  await click("Show scan details");
  const panel = container.querySelector<HTMLElement>(".scan-status-details")!;
  expect(panel.closest(".scan-details-popover")?.id).toBe(
    trigger.getAttribute("aria-controls"),
  );
  await act(() =>
    panel.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    ),
  );
  expect(container.querySelector(".scan-status-details")).toBeNull();
  expect(document.activeElement).toBe(trigger);
  await click("Show scan details");
  await act(() =>
    document.body.dispatchEvent(new Event("pointerdown", { bubbles: true })),
  );
  expect(container.querySelector(".scan-status-details")).toBeNull();
});
it("keeps an incomplete result with zero error events explicit and exclusions alone hidden", async () => {
  await act(() =>
    root.render(ScanDetails, {
      ...detailsProps,
      summary: { ...summary, errors: 0, excluded: 0, incomplete: true },
    }),
  );
  expect(container.querySelector(".scan-coverage-chip")?.textContent).toContain(
    "CoverageLimited",
  );
  await click("Show scan details");
  expect(
    container
      .querySelector(".scan-status-details")
      ?.textContent?.replace(/\s+/g, " "),
  ).toContain("No details were saved about why.");
  await act(() =>
    root.render(ScanDetails, {
      ...detailsProps,
      key: "excluded-only",
      summary: { ...summary, errors: 0, excluded: 2 },
    }),
  );
  expect(container.querySelector(".scan-coverage-chip")).toBeNull();
  expect(container.querySelector(".partial-coverage")).toBeNull();
});

it("keeps the loaded page visible while a progress event raises the error count", async () => {
  await click("Show scan details");
  let resolve!: (result: ScanIssuePage) => void;
  mock.invoke.mockImplementationOnce(
    () =>
      new Promise<ScanIssuePage>((done) => {
        resolve = done;
      }),
  );
  await act(async () =>
    root.render(ScanDetails, {
      ...detailsProps,
      summary: { ...summary, state: "running", errors: 3 },
    }),
  );
  expect(container.textContent).toContain("/fixture/private");
  expect(container.textContent).not.toContain("Loading scan details");
  const next = Array.from(container.querySelectorAll("button")).find(
    (b) => b.textContent === "Next details",
  )!;
  expect(next).toBeTruthy();
  await act(async () => resolve({ ...page(), total: 3, retainedTotal: 3 }));
  expect(container.textContent).toContain("of 3 retained details");
});

it("opens on the restricted category when it is the only one with entries", async () => {
  await act(async () =>
    root.render(ScanIssues, {
      summary: { ...summary, errors: 2, restrictedSystemIssues: 2 },
      active: true,
    }),
  );
  expect(mock.invoke).toHaveBeenLastCalledWith("get_scan_issues", {
    scanId: "scan-1",
    kind: "restricted",
    offset: 0,
    limit: 100,
  });
});

it("switches to real errors when they arrive after restricted-only issues", async () => {
  await act(async () =>
    root.render(ScanIssues, {
      summary: { ...summary, state: "running", errors: 2, restrictedSystemIssues: 2 },
      active: true,
    }),
  );
  await act(async () =>
    root.render(ScanIssues, {
      summary: { ...summary, state: "running", errors: 3, restrictedSystemIssues: 2 },
      active: true,
    }),
  );
  expect(mock.invoke).toHaveBeenLastCalledWith("get_scan_issues", {
    scanId: "scan-1",
    kind: "other",
    offset: 0,
    limit: 100,
  });
});

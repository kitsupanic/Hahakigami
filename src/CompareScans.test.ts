// @vitest-environment jsdom
import { act, createTestRoot as createRoot, type Root } from "./testUtils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setShowLogicalSize } from "./logicalSize.svelte";
import CompareScans, { type CompareScansProps } from "./CompareScans.svelte";
import type { Comparison, ComparisonRow } from "./comparisonTypes";
import type { LibraryProgress, SavedEntry } from "./types";
import { formatScanTimestamp } from "./savedHistory";

const saved = (
  id: string,
  completedAtMs: number,
  rootPath = "/fixture",
): SavedEntry => ({
  id,
  name: "Home",
  rootPath,
  startedAtMs: completedAtMs - 1000,
  completedAtMs,
  savedAtMs: completedAtMs,
  entries: 10,
  logicalBytes: 100,
  allocatedBytes: 200,
  incomplete: false,
  errors: 0,
  excluded: 0,
  fileBytes: 10,
});
const older = saved("before", 1000);
const newer = saved("after", 2000);
const row: ComparisonRow = {
  path: "/fixture/growing",
  kind: "file",
  status: "changed",
  beforeLogicalBytes: 100,
  afterLogicalBytes: 300,
  beforeAllocatedBytes: 200,
  afterAllocatedBytes: 500,
  logicalDelta: 200,
  allocatedDelta: 300,
};
const comparison: Comparison = {
  beforeId: "before",
  afterId: "after",
  rootPath: "/fixture",
  before: {
    entries: 10,
    logicalBytes: 100,
    allocatedBytes: 200,
    incomplete: false,
    errors: 0,
    excluded: 0,
  },
  after: {
    entries: 12,
    logicalBytes: 300,
    allocatedBytes: 100,
    incomplete: true,
    errors: 1,
    excluded: 0,
  },
  logicalDelta: 200,
  allocatedDelta: -100,
  added: 3,
  removed: 1,
  changed: 2,
  unchanged: 7,
  warnings: ["After scan has incomplete coverage."],
  rankings: [
    {
      metric: "allocated",
      kind: "files",
      direction: "growth",
      total: 180,
      items: [row],
    },
    {
      metric: "logical",
      kind: "folders",
      direction: "shrinkage",
      total: 1,
      items: [
        {
          ...row,
          path: "/fixture/shrinking",
          kind: "directory",
          logicalDelta: -300,
          beforeLogicalBytes: 400,
          afterLogicalBytes: 100,
        },
      ],
    },
  ],
};
let container: HTMLDivElement;
let root: Root;
let props: CompareScansProps;
async function render(update: Partial<CompareScansProps> = {}) {
  props = { ...props, ...update };
  await act(async () => root.render(CompareScans, { ...props }));
}
function button(name: string) {
  const match = Array.from(container.querySelectorAll("button")).find(
    (item) => item.textContent?.trim() === name,
  );
  expect(match).toBeTruthy();
  return match!;
}
function select(name: string) {
  const match = container.querySelector<HTMLSelectElement>(
    `select[aria-label="${name}"]`,
  );
  expect(match).toBeTruthy();
  return match!;
}
async function chooseView(direction: string) {
  await act(() =>
    container
      .querySelector<HTMLButtonElement>(`[data-direction="${direction}"]`)!
      .click(),
  );
}
const text = (selector: string) =>
  container.querySelector(selector)?.textContent?.replace(/\s+/g, " ").trim();
async function choose(name: string, value: string) {
  await act(async () => {
    const input = select(name);
    input.value = value;
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
}
beforeEach(() => {
  setShowLogicalSize(true);
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  props = {
    entries: [newer, older],
    disabled: false,
    result: null,
    progress: null,
    onCompare: vi.fn(),
    onClose: vi.fn(),
  };
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

it("defaults to an older and newer scan of the same location without automatically loading", async () => {
  await render({ entries: [saved("other", 4000, "/other"), older, newer] });
  expect(select("Before scan").value).toBe("before");
  expect(select("After scan").value).toBe("after");
  expect(props.onCompare).not.toHaveBeenCalled();
  await act(async () => button("Compare scans").click());
  expect(props.onCompare).toHaveBeenCalledExactlyOnceWith("before", "after");
  expect(container.textContent).toContain(
    "Comparing two past scans · Read-only",
  );
});

it("requires distinct IDs and matching roots", async () => {
  await render({ entries: [newer, older, saved("other", 3000, "/other")] });
  await choose("After scan", "before");
  expect(button("Compare scans").disabled).toBe(true);
  expect(container.textContent).toContain("Choose two different scans");
  await choose("After scan", "other");
  expect(button("Compare scans").disabled).toBe(true);
  expect(container.textContent).toContain(
    "Both scans must cover the same location",
  );
  expect(props.onCompare).not.toHaveBeenCalled();
});

it("disables pair changes and closing while busy and shows comparison progress", async () => {
  await render({ disabled: true });
  expect(select("Before scan").disabled).toBe(true);
  expect(select("After scan").disabled).toBe(true);
  expect(button("Compare scans").disabled).toBe(true);
  expect(button("Close comparison").disabled).toBe(true);
  expect(button("Swap scans").disabled).toBe(true);
  expect(
    container.querySelector(".comparison-panel")?.getAttribute("aria-busy"),
  ).toBe("true");
  // Main extends this shared progress union as part of command integration.
  await render({
    disabled: false,
    progress: {
      operation: "compare",
      phase: "readingAfter",
      bytes: 2048,
    } as unknown as LibraryProgress,
  });
  expect(container.querySelector('[role="status"]')?.textContent).toContain(
    "Opening after scan",
  );
  expect(container.querySelector('[role="status"]')?.textContent).toContain(
    "2 KiB (2.05 KB)",
  );
  expect(button("Compare scans").disabled).toBe(true);
});

it("shows warning, totals, signed deltas, counts and read-only ranked rows", async () => {
  await render({ result: comparison });
  expect(
    container.querySelector('[aria-label="Comparison warnings"]')?.textContent,
  ).toContain("incomplete coverage");
  const totals = container.querySelector(
    '[aria-label="Scan totals"]',
  )?.textContent;
  expect(totals).toContain("−100 B");
  expect(totals).toContain("+200 B");
  expect(totals).toContain("Incomplete");
  expect(
    container.querySelector('[aria-label="Entry changes"]')?.textContent,
  ).toContain("Added 3");
  const ranked = container.querySelector('[aria-label="Ranked changes"]');
  expect(ranked?.querySelector(".comparison-path")?.getAttribute("title")).toBe(
    "/fixture/growing",
  );
  expect(ranked?.textContent).toContain("+300 B");
  expect(ranked?.querySelectorAll("button").length).toBe(0);
  expect(text(".comparison-limit")).toContain(
    "Showing 1 of 180 matching files & other entries",
  );
  expect(container.textContent).toContain("Up to 100 largest changes");
  expect(container.textContent).not.toContain("NaN");
});

it("changes ranking metric, kind and direction without loading again", async () => {
  await render({ result: comparison });
  await choose("Comparison size metric", "logical");
  expect(container.textContent).toContain(
    "No files & other entries with growth in logical size",
  );
  await choose("Comparison entry type", "folders");
  await chooseView("shrinkage");
  expect(
    container
      .querySelector('[aria-label="Ranked changes"] .comparison-path')
      ?.getAttribute("title"),
  ).toBe("/fixture/shrinking");
  expect(
    container.querySelector('[aria-label="Ranked changes"]')?.textContent,
  ).toContain("−300 B");
  expect(props.onCompare).not.toHaveBeenCalled();
});

it("clears deleted selected IDs and hides results belonging to the prior pair", async () => {
  await render({ result: comparison });
  await act(async () => button("Compare scans").click());
  await render({ entries: [newer, saved("replacement", 500)] });
  expect(select("Before scan").value).toBe("");
  expect(select("After scan").value).toBe("after");
  expect(button("Compare scans").disabled).toBe(true);
  expect(container.querySelector('[aria-label="Ranked changes"]')).toBeNull();
  expect(container.textContent).toContain("You picked new scans");
});

it("renders empty library and closes without performing file actions", async () => {
  await render({ entries: [] });
  expect(container.textContent).toContain(
    "Save two scans of the same location",
  );
  expect(button("Compare scans").disabled).toBe(true);
  expect(select("Before scan").disabled).toBe(true);
  await act(async () => button("Close comparison").click());
  expect(props.onClose).toHaveBeenCalledOnce();
  expect(container.textContent).not.toContain("Trash");
  expect(container.textContent).not.toContain("Finder");
});

it("caps the ranked display at 100 and preserves rows with identical displayed paths", async () => {
  const items = Array.from({ length: 105 }, (_, index) => ({
    ...row,
    afterAllocatedBytes: 500 + index,
    allocatedDelta: 300 + index,
    path: index < 2 ? "/fixture/lossy-name" : `/fixture/row${index}`,
  }));
  await render({
    result: {
      ...comparison,
      rankings: [
        {
          metric: "allocated",
          kind: "files",
          direction: "growth",
          total: 105,
          items,
        },
      ],
    },
  });
  const ranked = container.querySelector('[aria-label="Ranked changes"]');
  expect(ranked?.querySelectorAll("tbody tr").length).toBe(100);
  expect(ranked?.querySelector('[title="/fixture/row100"]')).toBeNull();
  expect(text(".comparison-limit")).toContain(
    "Showing 100 of 105 matching files & other entries",
  );
  expect(
    Array.from(ranked!.querySelectorAll("tbody td:first-child")).filter(
      (item) => item.getAttribute("title") === "/fixture/lossy-name",
    ).length,
  ).toBe(2);
});

it("distinguishes incomplete coverage from expected exclusions", async () => {
  await render({
    result: {
      ...comparison,
      before: { ...comparison.before, errors: 1 },
      after: { ...comparison.after, incomplete: false, errors: 0, excluded: 1 },
    },
  });
  const coverage = Array.from(
    container.querySelectorAll('[aria-label="Scan totals"] tbody tr'),
  ).find((item) => item.textContent?.startsWith("Coverage"));
  expect(coverage?.textContent).toContain("Incomplete coverage");
  expect(coverage?.textContent).toContain("Has exclusions");
  expect(text(".comparison-coverage")).toContain("1 excluded location");
});

it("shows zero-byte added/removed and unchanged-size metadata changes", async () => {
  const directions = ["added", "removed", "changed"] as const;
  await render({
    result: {
      ...comparison,
      rankings: directions.map((direction) => ({
        metric: "allocated",
        kind: "files",
        direction,
        total: 1,
        items: [
          {
            ...row,
            path: `/fixture/${direction}-empty`,
            status: direction,
            beforeLogicalBytes: 0,
            afterLogicalBytes: 0,
            beforeAllocatedBytes: 0,
            afterAllocatedBytes: 0,
            logicalDelta: 0,
            allocatedDelta: 0,
          },
        ],
      })),
    },
  });
  for (const direction of directions) {
    await chooseView(direction);
    const ranked = container.querySelector('[aria-label="Ranked changes"]');
    expect(
      ranked?.querySelector(".comparison-path")?.getAttribute("title"),
    ).toBe(`/fixture/${direction}-empty`);
    expect(ranked?.textContent).toContain("0 B");
    const cells = ranked?.querySelectorAll("tbody tr td");
    expect(cells?.[2].textContent).toBe(direction === "added" ? "—" : "0 B");
    expect(cells?.[3].textContent).toBe(direction === "removed" ? "—" : "0 B");
    expect(ranked?.querySelector(".comparison-delta")?.textContent).toContain(
      "No size change",
    );
    expect(container.textContent).toContain("Up to 100 entries are shown");
  }
  await choose("Comparison entry type", "folders");
  expect(container.textContent).toContain("No folders with changed metadata");
  await chooseView("added");
  expect(container.textContent).toContain("No added folders");
  expect(props.onCompare).not.toHaveBeenCalled();
});

it("uses native root net deltas independently of ranking churn and changes both totals with the metric", async () => {
  await render({ result: { ...comparison, allocatedDelta: 0 } });
  expect(text(".comparison-net")).toContain("Net change · On disk");
  expect(text(".comparison-net")).toContain("No net size change");
  expect(
    container.querySelector(".comparison-net")?.getAttribute("data-trend"),
  ).toBe("unchanged");
  expect(text(".comparison-net")).not.toContain("+300 B");
  expect(text(".comparison-sizes")).toContain("200 B");
  await choose("Comparison size metric", "logical");
  expect(text(".comparison-net")).toContain("Net change · Logical");
  expect(text(".comparison-net")).toContain("+200 B");
  expect(text(".comparison-sizes")).toContain("300 B");
  expect(text(".comparison-net")).toContain("After − Before · entire location");
});

it("keeps overlapping view counts separate from all-entry global counts", async () => {
  await render({
    result: {
      ...comparison,
      rankings: [
        {
          metric: "allocated",
          kind: "files",
          direction: "growth",
          total: 10,
          items: [row],
        },
        {
          metric: "allocated",
          kind: "files",
          direction: "added",
          total: 7,
          items: [],
        },
        {
          metric: "allocated",
          kind: "folders",
          direction: "growth",
          total: 2,
          items: [],
        },
      ],
    },
  });
  expect(text('[data-direction="growth"]')).toBe("Grew 10");
  expect(text('[data-direction="added"]')).toBe("Added 7");
  expect(text(".comparison-view-copy")).toContain("including new entries");
  expect(text(".comparison-view-copy")).toContain(
    "These views overlap; do not add their counts together.",
  );
  expect(text('[aria-label="Entry changes"]')).toContain("Added 3");
  expect(text(".comparison-explanation")).toContain("every kind of entry");
  await chooseView("added");
  expect(
    container
      .querySelector('[data-direction="added"]')
      ?.getAttribute("aria-pressed"),
  ).toBe("true");
  await choose("Comparison entry type", "folders");
  expect(text('[data-direction="growth"]')).toBe("Grew 2");
  expect(text('[aria-label="Entry changes"]')).toContain("Added 3");
  expect(
    select("Comparison entry type").querySelector('[value="files"]')
      ?.textContent,
  ).toBe("Files & other entries");
  expect(container.querySelectorAll('[role="tab"]')).toHaveLength(0);
});

it("swaps only the selection, explains reversed order and waits for explicit comparison", async () => {
  await render({ result: comparison });
  await act(() => button("Swap scans").click());
  expect(select("Before scan").value).toBe("after");
  expect(select("After scan").value).toBe("before");
  expect(props.onCompare).not.toHaveBeenCalled();
  expect(container.querySelector('[aria-label="Ranked changes"]')).toBeNull();
  expect(text(".comparison-order-note")).toContain(
    "Before scan finished later",
  );
  await act(() => button("Compare scans").click());
  expect(props.onCompare).toHaveBeenCalledExactlyOnceWith("after", "before");
});

it("does not render matching IDs when the result location differs", async () => {
  await render({ result: { ...comparison, rootPath: "/other" } });
  expect(container.querySelector(".comparison-overview")).toBeNull();
  expect(container.textContent).toContain("You picked new scans");
});

it("keeps all warnings and complete totals accessible in closed disclosures", async () => {
  const warnings = [
    "Coverage warning",
    "Accounting model differs",
    "Location identity could not be verified",
  ];
  await render({ result: { ...comparison, warnings } });
  const notes = container.querySelector<HTMLDetailsElement>(
    ".comparison-warnings",
  )!;
  const totals = container.querySelector<HTMLDetailsElement>(
    ".comparison-total-details",
  )!;
  expect(notes.open).toBe(false);
  expect(totals.open).toBe(false);
  expect(notes.querySelector("summary")?.textContent).toBe(
    "Comparison notes (3)",
  );
  for (const warning of warnings) expect(notes.textContent).toContain(warning);
  expect(text(".comparison-notes-summary")).toContain(
    "whether it's the same place",
  );
  expect(text(".comparison-coverage")).toContain("After: Incomplete coverage");
});

it("disambiguates same-second scans, shared completion timestamps and epoch-zero completion", async () => {
  const ms = new Date(2024, 0, 1, 10, 30, 45).getTime();
  const a = saved("a", ms + 100),
    b = saved("b", ms + 900);
  const copy = {
    ...b,
    id: "copy",
    savedAtMs: ms + 1500,
    logicalBytes: b.logicalBytes + 100,
  };
  await render({ entries: [a, b, copy, saved("epoch", 0)] });
  expect(text(".comparison-order-note")).toContain(
    "same recorded completion time",
  );
  const options = [...select("Before scan").options];
  expect(options.find((option) => option.value === "a")?.textContent).toContain(
    formatScanTimestamp(a.completedAtMs, undefined, true).label,
  );
  const originalText = options.find(
    (option) => option.value === "b",
  )?.textContent;
  const copyText = options.find(
    (option) => option.value === "copy",
  )?.textContent;
  expect(originalText).not.toBe(copyText);
  expect(originalText).toContain("saved result");
  expect(copyText).toContain("saved result");
  expect(copyText).not.toContain("copy ");
  expect(copyText).toContain(
    `saved ${formatScanTimestamp(copy.savedAtMs, undefined, true).label}`,
  );
  expect(copyText).toContain("2024");
  await choose("Before scan", "epoch");
  const time = container.querySelector<HTMLTimeElement>(
    ".comparison-scan-time",
  )!;
  expect(time.dateTime).toBe(new Date(0).toISOString());
  expect(time.title).toBe(`Scan completed ${formatScanTimestamp(0).absolute}`);
  expect(time.textContent).toContain(formatScanTimestamp(0).label);
});

it("retains the current result through a same-pair comparison attempt", async () => {
  await render({ result: comparison });
  await act(() => button("Compare scans").click());
  await render({
    progress: { operation: "compare", phase: "comparing", bytes: 100 },
  });
  expect(
    container.querySelector('[aria-label="Ranked changes"]'),
  ).not.toBeNull();
  await render({ progress: null });
  expect(text(".comparison-net")).toContain("−100 B");
  expect(
    container.querySelector('[aria-label="Ranked changes"]'),
  ).not.toBeNull();
});

it("defaults by completion time even when an older scan was imported more recently", async () => {
  await render({ entries: [{ ...older, savedAtMs: 9000 }, newer] });
  expect(select("Before scan").value).toBe("before");
  expect(select("After scan").value).toBe("after");
});

it("offers the Current scan on either side, against saved scans of its location only", async () => {
  await render({
    entries: [older, saved("other", 3000, "/other")],
    current: { id: "scan-1", rootPath: "/fixture", completedAtMs: 5000 },
  });
  expect(select("Before scan").value).toBe("before");
  expect(select("After scan").value).toBe("scan-1");
  expect(container.textContent).toContain(
    "Comparing the Current scan with a saved scan",
  );
  const values = (name: string) =>
    [...select(name).options].map((option) => option.value);
  // The Current scan is After, so Before lists only saved scans of /fixture.
  expect(values("Before scan")).toEqual(["", "before"]);
  expect(values("After scan")).toEqual(["", "scan-1", "other", "before"]);
  await act(async () => button("Compare scans").click());
  expect(props.onCompare).toHaveBeenLastCalledWith("before", "scan-1");
  await act(async () => button("Swap scans").click());
  expect(select("Before scan").value).toBe("scan-1");
  expect(values("After scan")).toEqual(["", "before"]);
  await act(async () => button("Compare scans").click());
  expect(props.onCompare).toHaveBeenLastCalledWith("scan-1", "before");
  // A saved scan of another location is never offered the Current scan.
  await choose("Before scan", "before");
  await choose("After scan", "other");
  expect(values("Before scan")).toEqual(["", "other", "before"]);
});

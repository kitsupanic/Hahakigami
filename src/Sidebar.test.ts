// @vitest-environment jsdom
import { act, createTestRoot, type Root } from "./testUtils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import Sidebar from "./Sidebar.svelte";
import type createApp from "./createApp.svelte";
vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => true,
  invoke: vi.fn(),
}));
let container: HTMLDivElement, root: Root;
const exported = vi.fn(),
  removed = vi.fn();
const saved = [
  {
    id: "root-snapshot",
    name: "/",
    rootPath: "/",
    completedAtMs: new Date(2026, 9, 15, 20, 31).getTime(),
    savedAtMs: new Date(2026, 9, 15, 20, 32).getTime(),
    entries: 20,
    logicalBytes: 40,
    allocatedBytes: 50,
    incomplete: true,
    errors: 1,
  },
  {
    id: "downloads-snapshot",
    name: "Downloads",
    rootPath: "/Users/test/Downloads",
    completedAtMs: new Date(2026, 9, 15, 16, 41).getTime(),
    savedAtMs: new Date(2026, 9, 15, 16, 42).getTime(),
    entries: 10,
    logicalBytes: 20,
    allocatedBytes: 30,
    incomplete: false,
    errors: 0,
  },
];
function app(overrides = {}) {
  const fake = {
    accessOpen: false,
    removeEntry: null,
    trash: null,
    clearQueueOpen: false,
    queueOpen: false,
    homeOpen: false,
    scanSlots: {},
    shown: null,
    openDisk: vi.fn(),
    blocked: false,
    sessionBlocked: false,
    running: false,
    // Saved-scan controls wait for a scan on any disk, not only the one on screen.
    anyScanRunning: (overrides as { running?: boolean }).running ?? false,
    queue: { entries: [] },
    targets: [
      {
        id: "root",
        path: "/",
        name: "Test volume",
        totalBytes: 100,
        freeBytes: 20,
        volume: true,
      },
    ],
    historical: true,
    savedScansEnabled: true,
    comparisonOpen: false,
    target: null,
    access: true,
    saved,
    summary: { savedId: saved[0].id },
    quickLook: { dismiss: vi.fn() },
    exportScan: exported,
    setRemoveEntry: removed,
    showQueue: vi.fn(),
    openSaved: vi.fn(),
    importScan: vi.fn(),
    openComparison: vi.fn(),
    ...overrides,
  };
  const idle = !fake.running && !fake.sessionBlocked;
  return {
    ...fake,
    can: {
      chooseFolder: idle,
      cleanupQueue: !fake.blocked && !fake.trash,
      compareScans: fake.saved.length >= 2 && idle,
      importScan: idle,
      exportSaved: idle,
    },
  } as unknown as ReturnType<typeof createApp>;
}
beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(2026, 9, 15, 22, 42));
  container = document.createElement("div");
  document.body.append(container);
  root = createTestRoot(container);
  exported.mockClear();
  removed.mockClear();
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});
async function render(overrides = {}) {
  await act(() => root.render(Sidebar, { app: app(overrides) }));
}
function trigger(index = 0) {
  return container.querySelectorAll<HTMLButtonElement>(".saved-menu-trigger")[
    index
  ];
}
async function click(button: HTMLButtonElement) {
  await act(() => button.click());
}
it("labels root scans with their volume and includes date and time for each snapshot", async () => {
  await render();
  const summaries = container.querySelectorAll(".saved-summary");
  expect(container.querySelector(".saved-group-heading h3")?.textContent).toBe(
    "Test volume",
  );
  expect(summaries[0].querySelector(".saved-day")?.textContent).toBe("Today");
  expect(summaries[0].querySelector(".saved-clock")?.textContent).toBe(
    "20:31:00",
  );
  expect(summaries[1].querySelector(".saved-clock")?.textContent).toBe(
    "16:41:00",
  );
  expect(summaries[0].querySelector(".saved-metadata")?.textContent).toContain(
    "50 B on disk",
  );
  expect(summaries[0].getAttribute("title")).toContain("Scanned");
  expect(summaries[0].getAttribute("title")).toContain("Saved");
  expect(
    container.querySelector('[aria-label="Compare saved scans"]'),
  ).not.toBeNull();
  expect(container.querySelector(".queue-count")?.textContent).toBe("0");
  expect(container.querySelector(".saved-menu")).toBeNull();
  await render({ targets: [] });
  expect(container.querySelector(".saved-group-heading h3")?.textContent).toBe(
    "Startup disk",
  );
});
it("opens one menu, toggles it, and dismisses with Escape and outside pointer", async () => {
  await render();
  await click(trigger());
  expect(trigger().getAttribute("aria-expanded")).toBe("true");
  await click(trigger());
  expect(container.querySelector(".saved-menu")).toBeNull();
  await click(trigger());
  await click(trigger(1));
  expect(container.querySelectorAll(".saved-menu")).toHaveLength(1);
  await act(() =>
    trigger(1).dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    ),
  );
  expect(container.querySelector(".saved-menu")).toBeNull();
  expect(document.activeElement).toBe(trigger(1));
  await click(trigger());
  await act(() =>
    document.body.dispatchEvent(new Event("pointerdown", { bubbles: true })),
  );
  expect(container.querySelector(".saved-menu")).toBeNull();
});
it("exports and removes the exact selected snapshot and closes its menu", async () => {
  await render();
  await click(trigger(1));
  await click(
    container.querySelector<HTMLButtonElement>(
      '[aria-label="Export saved scan Downloads"]',
    )!,
  );
  expect(exported).toHaveBeenCalledExactlyOnceWith("downloads-snapshot");
  expect(container.querySelector(".saved-menu")).toBeNull();
  await click(trigger());
  await click(
    container.querySelector<HTMLButtonElement>(
      '[aria-label="Remove saved scan Test volume"]',
    )!,
  );
  expect(removed).toHaveBeenCalledExactlyOnceWith(saved[0]);
  expect(container.querySelector(".saved-menu")).toBeNull();
});
it("keeps saved actions unavailable during a scan or blocked session", async () => {
  await render({ running: true });
  expect(trigger().disabled).toBe(true);
  expect(
    container.querySelector<HTMLButtonElement>(
      '[aria-label="Compare saved scans"]',
    )?.disabled,
  ).toBe(true);
  await click(trigger());
  expect(container.querySelector(".saved-menu")).toBeNull();
  await render({ sessionBlocked: true });
  expect(trigger().disabled).toBe(true);
});

it("closes an open action menu when a scan starts", async () => {
  await render();
  await click(trigger());
  expect(container.querySelector(".saved-menu")).not.toBeNull();
  await render({ running: true });
  expect(container.querySelector(".saved-menu")).toBeNull();
  expect(trigger().disabled).toBe(true);
});

it("uses local calendar labels and includes years for older snapshots", async () => {
  const dates = [
    new Date(2026, 9, 14, 23, 59),
    new Date(2026, 9, 12, 0, 5),
    new Date(2025, 11, 31, 16, 41),
  ];
  await render({
    saved: dates.map((date, index) => ({
      ...saved[0],
      id: `older-${index}`,
      completedAtMs: date.getTime(),
    })),
  });
  await click(historyToggle());
  const labels = [...container.querySelectorAll(".saved-day")].map(
    (item) => item.textContent,
  );
  const clocks = [...container.querySelectorAll(".saved-clock")].map(
    (item) => item.textContent,
  );
  expect(labels[0]).toBe("Yesterday");
  expect(clocks).toEqual(["23:59:00", "00:05:00", "16:41:00"]);
  expect(labels[1]).toBe(
    dates[1].toLocaleDateString(undefined, { month: "short", day: "numeric" }),
  );
  expect(labels[2]).toBe(
    dates[2].toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
      year: "numeric",
    }),
  );
  expect(trigger().getAttribute("aria-label")).toContain("23:59:00");
});

it("groups by exact location, shows paths for matching names and leaves each snapshot actionable", async () => {
  const opened = vi.fn();
  const records = [
    {
      ...saved[1],
      id: "older",
      completedAtMs: new Date(2026, 9, 15, 16, 41, 2).getTime(),
      savedAtMs: Date.now() + 1000,
    },
    {
      ...saved[1],
      id: "other-location",
      rootPath: "/Volumes/External/Downloads",
      completedAtMs: new Date(2026, 9, 15, 17, 0).getTime(),
    },
    {
      ...saved[1],
      id: "newer",
      completedAtMs: new Date(2026, 9, 15, 16, 41, 52).getTime(),
    },
  ];
  await render({
    saved: records,
    openSaved: opened,
    summary: { savedId: "older" },
  });
  const groups = [...container.querySelectorAll(".saved-group")];
  expect(groups).toHaveLength(2);
  expect(groups.map((group) => group.querySelector("h3")?.textContent)).toEqual(
    ["Downloads", "Downloads"],
  );
  expect(
    groups.map(
      (group) => group.querySelector(".saved-location-path")?.textContent,
    ),
  ).toEqual(["/Volumes/External/Downloads", "/Users/test/Downloads"]);
  expect(
    [...groups[1].querySelectorAll(".saved-entry")].map((row) =>
      row.getAttribute("data-saved-id"),
    ),
  ).toEqual(["newer", "older"]);
  const active = container.querySelector<HTMLButtonElement>(
    '[data-saved-id="older"] .saved-summary',
  )!;
  expect(active.getAttribute("aria-current")).toBe("page");
  await click(active);
  expect(opened).toHaveBeenCalledExactlyOnceWith("older");
  const newer = container.querySelector<HTMLButtonElement>(
    '[data-saved-id="newer"] .saved-menu-trigger',
  )!;
  await click(newer);
  await click(
    container.querySelector<HTMLButtonElement>(
      '[aria-label="Export saved scan Downloads"]',
    )!,
  );
  expect(exported).toHaveBeenCalledExactlyOnceWith("newer");
  await click(newer);
  await click(
    container.querySelector<HTMLButtonElement>(
      '[aria-label="Remove saved scan Downloads"]',
    )!,
  );
  expect(removed).toHaveBeenCalledExactlyOnceWith(records[2]);
  expect(records.map((entry) => entry.id)).toEqual([
    "older",
    "other-location",
    "newer",
  ]);
});

it("distinguishes same-second scans and repeated copies with their saved times", async () => {
  const stamp = new Date(2026, 9, 15, 16, 41, 2, 100).getTime();
  await render({
    saved: [
      {
        ...saved[1],
        id: "first-copy",
        completedAtMs: stamp,
        savedAtMs: new Date(2026, 9, 15, 17, 1).getTime(),
      },
      {
        ...saved[1],
        id: "second-scan",
        completedAtMs: stamp + 600,
        savedAtMs: new Date(2026, 9, 15, 17, 2).getTime(),
      },
      {
        ...saved[1],
        id: "second-copy",
        completedAtMs: stamp,
        savedAtMs: new Date(2026, 9, 15, 17, 3).getTime(),
      },
    ],
  });
  await click(historyToggle());
  const clocks = [...container.querySelectorAll(".saved-clock")].map(
    (node) => node.textContent,
  );
  expect(clocks).toEqual(["16:41:02.700", "16:41:02.100", "16:41:02.100"]);
  const copies = [...container.querySelectorAll(".saved-copy-time")].map(
    (node) => node.textContent,
  );
  expect(copies).toEqual([
    "Saved Today, 17:03:00.000",
    "Saved Today, 17:01:00.000",
  ]);
  const labels = [...container.querySelectorAll(".saved-summary")].map((node) =>
    node.getAttribute("aria-label"),
  );
  expect(new Set(labels).size).toBe(3);
});

it("retains the empty-library state and suppresses current-entry styling in other views", async () => {
  await render({ saved: [] });
  expect(container.textContent).toContain("No saved scans yet.");
  expect(container.querySelector(".saved-group")).toBeNull();
  await render({ queueOpen: true });
  expect(container.querySelector(".saved-summary[aria-current]")).toBeNull();
  expect(container.querySelector(".saved-entry.active")).toBeNull();
});

it("distinguishes identical scan copies saved within the same second", async () => {
  const timestamp = new Date(2026, 9, 15, 17, 3, 0, 100).getTime();
  await render({
    saved: [
      { ...saved[1], id: "copy-a", savedAtMs: timestamp },
      { ...saved[1], id: "copy-b", savedAtMs: timestamp + 600 },
    ],
  });
  await click(historyToggle());
  const copied = [...container.querySelectorAll(".saved-copy-time")].map(
    (node) => node.textContent,
  );
  expect(copied).toEqual([
    "Saved Today, 17:03:00.700",
    "Saved Today, 17:03:00.100",
  ]);
});

function history(count = 6) {
  return Array.from({ length: count }, (_, index) => ({
    ...saved[1],
    id: `history-${index}`,
    completedAtMs: saved[1].completedAtMs - index * 1000,
  }));
}
function historyIds() {
  return [...container.querySelectorAll(".saved-entry")].map((row) =>
    row.getAttribute("data-saved-id"),
  );
}
function historyToggle() {
  return container.querySelector<HTMLButtonElement>(".saved-history-toggle")!;
}

it("shows the newest scan per location and keeps an older active scan actionable", async () => {
  const records = history();
  const opened = vi.fn();
  await render({
    saved: records,
    summary: { savedId: "history-5" },
    openSaved: opened,
  });
  expect(historyIds()).toEqual(["history-0", "history-5"]);
  expect(historyToggle().textContent).toBe("Show older scans (4)");
  expect(historyToggle().getAttribute("aria-expanded")).toBe("false");
  expect(
    document.getElementById(historyToggle().getAttribute("aria-controls")!),
  ).not.toBeNull();
  const active = container.querySelector<HTMLButtonElement>(
    '[data-saved-id="history-5"] .saved-summary',
  )!;
  expect(active.getAttribute("aria-current")).toBe("page");
  await click(active);
  expect(opened).toHaveBeenCalledExactlyOnceWith("history-5");
  await click(historyToggle());
  expect(historyIds()).toEqual(records.map((entry) => entry.id));
  expect(historyToggle().textContent).toBe("Show fewer");
  expect(historyToggle().getAttribute("aria-expanded")).toBe("true");
  await click(historyToggle());
  expect(historyIds()).toEqual(["history-0", "history-5"]);
});

it("keeps each location expansion through list updates and drops it when that location is removed", async () => {
  const records = history();
  const other = records.map((entry) => ({
    ...entry,
    id: `other-${entry.id}`,
    rootPath: "/Volumes/Other/Downloads",
  }));
  await render({ saved: [...records, ...other] });
  await click(historyToggle());
  expect(container.querySelectorAll(".saved-entry")).toHaveLength(7);
  await render({ saved: [...records.slice(0, 5), ...other] });
  expect(container.querySelectorAll(".saved-entry")).toHaveLength(6);
  expect(historyToggle().getAttribute("aria-expanded")).toBe("true");
  await render({ saved: other });
  await render({ saved: [...records, ...other] });
  expect(container.querySelectorAll(".saved-entry")).toHaveLength(2);
  expect(historyToggle().getAttribute("aria-expanded")).toBe("false");
});

it("keeps timestamp precision based on the full group and closes menus before collapsing", async () => {
  const records = history(5).map((entry, index) => ({
    ...entry,
    completedAtMs: saved[1].completedAtMs + [2000, 1000, 900, 600, 100][index],
  }));
  await render({ saved: records, summary: { savedId: records[2].id } });
  expect(container.querySelectorAll(".saved-clock")[1]?.textContent).toBe(
    "16:41:00.900",
  );
  await click(historyToggle());
  await click(trigger(4));
  expect(container.querySelector(".saved-menu")).not.toBeNull();
  await click(historyToggle());
  expect(container.querySelector(".saved-menu")).toBeNull();
  expect(historyIds()).toHaveLength(2);
});

it("disables history expansion when running or blocked", async () => {
  await render({ saved: history(), running: true });
  expect(historyToggle().disabled).toBe(true);
  await click(historyToggle());
  expect(historyIds()).toHaveLength(1);
  await render({ saved: history(), sessionBlocked: true });
  expect(historyToggle().disabled).toBe(true);
});

const nav = (name: string) =>
  [...container.querySelectorAll<HTMLButtonElement>("button.side-nav")].find(
    (item) => item.textContent?.includes(name),
  )!;
const slot = (
  state: string,
  scanId = "live",
  target: Record<string, unknown> = {
    id: "root",
    path: "/",
    name: "Test volume",
    volume: true,
  },
) => ({
  target,
  scanId,
  summary: { scanId, origin: "live", state, allocatedBytes: 2048 },
});
const folder = {
  id: "photos",
  path: "/Users/test/Photos",
  name: "Photos",
  totalBytes: null,
  freeBytes: null,
};

it("has no Locations section or Choose a folder button", async () => {
  await render();
  expect(container.textContent).not.toContain("Locations");
  expect(container.textContent).not.toContain("Choose a folder");
  expect(container.querySelector(".location-list")).toBeNull();
});

const disk = (name: string) =>
  [...container.querySelectorAll<HTMLButtonElement>("button.disk-nav")].find(
    (item) => item.getAttribute("aria-label")?.startsWith(`${name},`),
  )!;

it("has no Home or Current scan button: the Disks list comes first", async () => {
  await render();
  expect(container.querySelector(".home-nav")).toBeNull();
  expect(container.querySelector(".current-scan-nav")).toBeNull();
  expect(nav("Current scan")).toBeUndefined();
  const order = [...container.querySelectorAll("button.side-nav, .side-label")]
    .map((item) => item.textContent)
    .slice(0, 2);
  expect(order[0]?.trim()).toBe("Disks");
  expect(order[1]).toContain("Test volume");
});

it("keeps disks enabled while a scan runs unless the session is blocked", async () => {
  const openDisk = vi.fn();
  await render({
    running: true,
    historical: false,
    summary: { scanId: "live", state: "running" },
    scanSlots: { "/": slot("running") },
    openDisk,
  });
  expect(disk("Test volume").disabled).toBe(false);
  await click(disk("Test volume"));
  expect(openDisk).toHaveBeenCalledOnce();
  await render({ sessionBlocked: true });
  expect(disk("Test volume").disabled).toBe(true);
  // A disk with a scan is blocked too: opening it would replace the view mid-start.
  await render({ sessionBlocked: true, scanSlots: { "/": slot("completed") } });
  expect(disk("Test volume").disabled).toBe(true);
});

it("shows free space, usage and an unknown size", async () => {
  await render({
    targets: [
      {
        id: "a",
        path: "/",
        name: "Full",
        totalBytes: 100,
        freeBytes: 5,
        volume: true,
      },
      {
        id: "b",
        path: "/Volumes/B",
        name: "Net",
        totalBytes: null,
        freeBytes: null,
        volume: true,
      },
      {
        id: "c",
        path: "/Users/ada",
        name: "ada",
        totalBytes: null,
        freeBytes: null,
        home: true,
      },
    ],
  });
  expect(container.querySelectorAll("button.disk-nav")).toHaveLength(2);
  expect(disk("Full").textContent).toContain("5 B free of 100 B");
  expect(disk("Full").querySelector(".disk-track")?.classList).toContain(
    "full",
  );
  expect(disk("Net").textContent).toContain("Size unknown");
  expect(disk("Net").querySelector(".disk-track")?.classList).toContain(
    "unknown",
  );
});

const twoDisks = [
  {
    id: "root",
    path: "/",
    name: "Test volume",
    totalBytes: 10240,
    freeBytes: 6144,
    volume: true,
  },
  {
    id: "b",
    path: "/Volumes/B",
    name: "Backup",
    totalBytes: 100,
    freeBytes: 20,
    volume: true,
  },
];

it("tags each disk Scanning or Scanned from its own scan and leaves the others untagged", async () => {
  const openDisk = vi.fn();
  await render({
    targets: [
      ...twoDisks,
      { id: "c", path: "/Volumes/C", name: "Camera", volume: true },
    ],
    historical: false,
    openDisk,
    scanSlots: {
      "/": slot("completed"),
      "/Volumes/B": slot("running", "b-scan", twoDisks[1]),
    },
  });
  expect(disk("Test volume").getAttribute("aria-label")).toBe(
    "Test volume, 6 KiB free of 10 KiB, Scanned",
  );
  expect(disk("Backup").getAttribute("aria-label")).toBe(
    "Backup, 20 B free of 100 B, Scanning",
  );
  expect(disk("Backup").title).toBe("/Volumes/B\nScanning");
  expect(disk("Camera").getAttribute("aria-label")).toBe(
    "Camera, Size unknown",
  );
  expect(disk("Camera").querySelector("em")).toBeNull();
  // Only the scanning disk shows progress, against its used space: 2 KiB of 80 B is capped.
  expect(disk("Test volume").querySelector("[role=progressbar]")).toBeNull();
  expect(disk("Test volume").querySelector(".disk-track")).not.toBeNull();
  const bar = disk("Backup").querySelector("[role=progressbar]");
  expect(bar?.classList.contains("indeterminate")).toBe(false);
  expect(bar?.getAttribute("aria-valuenow")).toBe("99");
  expect(disk("Backup").querySelector(".disk-track")).toBeNull();
  await click(disk("Backup"));
  expect(openDisk).toHaveBeenCalledWith(twoDisks[1]);
  await click(disk("Camera"));
  expect(openDisk).toHaveBeenLastCalledWith(
    expect.objectContaining({ id: "c" }),
  );
});

it("measures a scanning disk's progress against its used space", async () => {
  await render({
    targets: twoDisks,
    historical: false,
    scanSlots: {
      "/": {
        ...slot("running"),
        summary: { scanId: "live", state: "running", allocatedBytes: 2048 },
      },
    },
  });
  // 2 KiB of the 4 KiB used.
  expect(
    disk("Test volume")
      .querySelector("[role=progressbar]")
      ?.getAttribute("aria-valuenow"),
  ).toBe("50");
});

it("highlights the selected disk on the welcome page and the disk whose results are on screen", async () => {
  const target = { id: "root", path: "/" };
  const results = {
    historical: false,
    summary: { scanId: "live", state: "completed" },
    scanSlots: { "/": slot("completed") },
    shown: { slot: "/" },
    target,
  };
  await render(results);
  expect(disk("Test volume").getAttribute("aria-current")).toBe("page");
  expect(disk("Test volume").classList.contains("active")).toBe(true);
  expect(disk("Test volume").classList.contains("scanned")).toBe(true);
  await render({ ...results, homeOpen: true });
  // Home hides the results; the selected disk is highlighted instead.
  expect(disk("Test volume").getAttribute("aria-current")).toBe("page");
  await render({ ...results, homeOpen: true, target: { id: "b", path: "/x" } });
  expect(disk("Test volume").getAttribute("aria-current")).toBeNull();
  await render({ summary: null, historical: false, target });
  expect(disk("Test volume").getAttribute("aria-current")).toBe("page");
  await render({ ...results, queueOpen: true });
  expect(disk("Test volume").getAttribute("aria-current")).toBeNull();
  // Results of another disk on screen do not highlight this one.
  await render({
    ...results,
    targets: twoDisks,
    shown: { slot: "/Volumes/B" },
    scanSlots: {
      "/": slot("completed"),
      "/Volumes/B": slot("completed", "b-scan", twoDisks[1]),
    },
  });
  expect(disk("Test volume").getAttribute("aria-current")).toBeNull();
  expect(disk("Backup").getAttribute("aria-current")).toBe("page");
});

it("lists the folder scan after the disks, with its own tag, and opens it", async () => {
  const openDisk = vi.fn();
  await render({ historical: false, openDisk });
  expect(container.querySelector(".folder-nav")).toBeNull();
  const scanSlots = { folder: slot("running", "f", folder) };
  await render({ historical: false, openDisk, scanSlots });
  const rows = [...container.querySelectorAll("button.disk-nav")];
  expect(rows).toHaveLength(2);
  const row = container.querySelector<HTMLButtonElement>(".folder-nav")!;
  expect(rows.at(-1)).toBe(row);
  expect(row.querySelector(".icon-folder, .folder-back")).not.toBeNull();
  expect(row.getAttribute("aria-label")).toBe("Photos, folder, Scanning");
  expect(row.querySelector("strong")?.textContent).toBe("Photos");
  expect(row.querySelector("em")?.textContent).toBe("Scanning");
  expect(
    row
      .querySelector("[role=progressbar]")
      ?.classList.contains("indeterminate"),
  ).toBe(true);
  await click(row);
  expect(openDisk).toHaveBeenCalledWith(folder);
  // Finished: the tag changes and the scan's size replaces the path.
  await render({
    historical: false,
    shown: { slot: "folder" },
    summary: { scanId: "f", state: "completed" },
    scanSlots: { folder: slot("completed", "f", folder) },
  });
  const done = container.querySelector<HTMLButtonElement>(".folder-nav")!;
  expect(done.getAttribute("aria-label")).toBe("Photos, folder, Scanned");
  expect(done.querySelector("small")?.textContent).toBe("2 KiB");
  expect(done.querySelector("[role=progressbar]")).toBeNull();
  expect(done.getAttribute("aria-current")).toBe("page");
  // Disks are not highlighted by a folder's results.
  expect(disk("Test volume").getAttribute("aria-current")).toBeNull();
  await render({
    historical: false,
    homeOpen: true,
    shown: { slot: "folder" },
    scanSlots: { folder: slot("completed", "f", folder) },
  });
  expect(
    container
      .querySelector<HTMLButtonElement>(".folder-nav")!
      .getAttribute("aria-current"),
  ).toBeNull();
  await render({
    sessionBlocked: true,
    scanSlots: { folder: slot("completed", "f", folder) },
  });
  expect(
    container.querySelector<HTMLButtonElement>(".folder-nav")!.disabled,
  ).toBe(true);
});

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
function app(overrides = {}) {
  const fake = {
    accessOpen: false,
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
    target: null,
    access: true,
    summary: null,
    showQueue: vi.fn(),
    ...overrides,
  };
  const idle = !fake.running && !fake.sessionBlocked;
  return {
    ...fake,
    can: {
      chooseFolder: idle,
      scanFolder: idle,
      cleanupQueue: !fake.blocked && !fake.trash,
      ...(overrides as { can?: object }).can,
    },
  } as unknown as ReturnType<typeof createApp>;
}
beforeEach(() => {
  container = document.createElement("div");
  document.body.append(container);
  root = createTestRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
});
async function render(overrides = {}) {
  await act(() => root.render(Sidebar, { app: app(overrides) }));
}
async function click(button: HTMLButtonElement) {
  await act(() => button.click());
}
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
  summary: { scanId, state, allocatedBytes: 2048 },
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

it("tags a failed or cancelled scan with its own words, not Scanned", async () => {
  await render({
    targets: twoDisks,
    scanSlots: {
      "/": slot("failed"),
      "/Volumes/B": slot("cancelled", "b-scan", twoDisks[1]),
    },
  });
  expect(disk("Test volume").getAttribute("aria-label")).toBe(
    "Test volume, 6 KiB free of 10 KiB, Scan failed",
  );
  expect(disk("Backup").getAttribute("aria-label")).toBe(
    "Backup, 20 B free of 100 B, Cancelled",
  );
});

it("tags each disk Scanning or Scanned from its own scan and leaves the others untagged", async () => {
  const openDisk = vi.fn();
  await render({
    targets: [
      ...twoDisks,
      { id: "c", path: "/Volumes/C", name: "Camera", volume: true },
    ],
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
  await render({ summary: null, target });
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
  await render({ openDisk });
  expect(container.querySelector(".folder-nav")).toBeNull();
  const scanSlots = { folder: slot("running", "f", folder) };
  await render({ openDisk, scanSlots });
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

const scanFolderButton = () =>
  container.querySelector<HTMLButtonElement>("button.scan-folder-nav")!;

it("scans a folder from the sidebar", async () => {
  const scanFolder = vi.fn();
  await render({ scanFolder });
  await click(scanFolderButton());
  expect(scanFolder).toHaveBeenCalledOnce();
});

it("disables Scan folder and says why while a folder scan is running", async () => {
  await render({ can: { scanFolder: false } });
  expect(scanFolderButton().disabled).toBe(true);
  expect(scanFolderButton().title).toBe("A folder scan is running");
});

it("disables Scan folder when a folder can't be chosen", async () => {
  await render({ can: { chooseFolder: false, scanFolder: false } });
  expect(scanFolderButton().disabled).toBe(true);
  expect(scanFolderButton().title).toBe("Choose a folder and scan it");
});

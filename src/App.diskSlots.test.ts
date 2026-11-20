// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, createTestRoot, type Root } from "./testUtils";
import App from "./App.svelte";
import type createApp from "./createApp.svelte";
import type { NodeView, ScanSummary, Target } from "./types";
import type { QueueEntry, QueueView } from "./cleanupTypes";

const mock = vi.hoisted(() => ({
  invoke: vi.fn(),
  app: null as ReturnType<typeof createApp> | null,
  channels: [] as { onmessage: (summary: unknown) => void }[],
}));
vi.mock("./createApp.svelte", async (original) => {
  const module = await original<typeof import("./createApp.svelte")>();
  return { default: () => (mock.app = module.default()) };
});
vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => true,
  invoke: mock.invoke,
  Channel: class {
    onmessage = (_: unknown) => {};
  },
}));
vi.mock("./DiskMap.svelte", async () => ({
  default: (await import("./TestMap.svelte")).default,
}));
const disk: Target = {
  id: "disk",
  name: "Macintosh HD",
  path: "/",
  totalBytes: 1000,
  freeBytes: 500,
  volume: true,
};
const backup: Target = {
  id: "backup",
  name: "Backup",
  path: "/Volumes/Backup",
  totalBytes: 1000,
  freeBytes: 500,
  volume: true,
};
const home: Target = {
  id: "home",
  name: "ada",
  path: "/Users/ada",
  totalBytes: null,
  freeBytes: null,
  home: true,
};
const photos: Target = {
  id: "photos",
  name: "Photos",
  path: "/Users/ada/Photos",
  totalBytes: null,
  freeBytes: null,
};
const music: Target = {
  ...photos,
  id: "music",
  name: "Music",
  path: "/Users/ada/Music",
};
let mounted: string[];
let discovered: Target[];
let chosen: Target;
let queueTarget: Target;
// What list_cleanup_queue answers with.
let queueEntries: QueueEntry[];
let failSummary: Set<string>;
// What the next scan reports to start with, and every scan by id.
let startState: ScanSummary["state"];
let scans: Map<string, ScanSummary>;
let started: number;
// While set, start_scan answers only once it resolves (its channel exists from the call on).
let startGate: Promise<void> | null;
const makeSummary = (
  scanId: string,
  state: ScanSummary["state"],
): ScanSummary => ({
  scanId,
  rootId: 0,
  state,
  workers: 1,
  workerStats: [],
  finalizationUs: 0,
  entries: 2,
  allocatedBytes: 2048,
  elapsedMs: 1000,
  errors: 0,
});
const node: NodeView = {
  id: 0,
  parentId: null,
  name: "root",
  path: "/",
  kind: "directory",
  allocatedBytes: 2048,
  childrenCount: 0,
  incomplete: false,
};
let container: HTMLDivElement, root: Root;
const app = () => mock.app!;
const calls = (command: string) =>
  mock.invoke.mock.calls.filter(([name]) => name === command);
const diskRow = (name: string) =>
  [...container.querySelectorAll<HTMLButtonElement>("button.disk-nav")].find(
    (item) => item.getAttribute("aria-label")?.startsWith(`${name},`),
  )!;
const tag = (name: string) => diskRow(name).querySelector("em")?.textContent;
const diskNames = () =>
  [...container.querySelectorAll("button.disk-nav strong")].map(
    (item) => item.textContent,
  );
const folderRow = () =>
  container.querySelector<HTMLButtonElement>("button.folder-nav");
const welcome = () => container.querySelector("#welcome-title");
const scanButton = () =>
  container.querySelector<HTMLButtonElement>("button.welcome-scan")!;
async function settle() {
  await act(async () => {
    for (let i = 0; i < 6; i++) await Promise.resolve();
  });
}
async function click(button: HTMLButtonElement) {
  await act(() => button.click());
  await settle();
}
const refresh = async () => {
  await act(() => window.dispatchEvent(new Event("focus")));
  await settle();
};
// Opens the disk's welcome page and scans it.
async function scanDisk(name: string) {
  await click(diskRow(name));
  await click(scanButton());
}
async function scanFolder(folder: Target) {
  chosen = folder;
  await act(() => app().showHome());
  await click(
    container.querySelector<HTMLButtonElement>("button.welcome-target")!,
  );
  await click(scanButton());
}
const progress = (scanId: string, change: Partial<ScanSummary>) =>
  act(() => {
    const channel = mock.channels[Number(scanId.split("-").at(-1)) - 1];
    channel.onmessage({ ...scans.get(scanId), ...change });
  });
beforeEach(async () => {
  startState = "completed";
  startGate = null;
  started = 0;
  scans = new Map();
  mock.channels = [];
  failSummary = new Set();
  queueEntries = [];
  mounted = ["/", "/Volumes/Backup"];
  discovered = [disk, backup, home];
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(async (command: string, args: any) => {
    switch (command) {
      case "get_explorer_preferences":
        return { sort: "sizeDescending", warning: null };
      case "full_disk_access_status":
        return { status: "on", scanAnyway: false, appPath: "/Fixture.app" };
      case "discover_targets":
        return discovered;
      case "mounted_volumes":
        return mounted;
      case "choose_folder":
        return chosen;
      case "cleanup_queue_target":
        return queueTarget;
      case "list_cleanup_queue":
        return {
          entries: queueEntries,
          revision: 0,
          warning: null,
          blocked: false,
        } satisfies QueueView;
      case "start_scan": {
        // Ids end with the start order, which is also the index of their channel.
        const id = `scan-${args.targetId}-${++started}`;
        scans.set(id, makeSummary(id, startState));
        mock.channels.push(args.onEvent);
        if (startGate) await startGate;
        return id;
      }
      case "scan_summary":
        if (failSummary.has(args.scanId))
          throw new Error("Summary unavailable");
        return scans.get(args.scanId);
      case "get_node":
        return String(args.scanId).startsWith("scan-backup")
          ? { ...node, name: "Backup", path: "/Volumes/Backup" }
          : node;
      case "get_map":
        return { nodes: [] };
      case "get_children":
        return { items: [], total: 0 };
      default:
        return null;
    }
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createTestRoot(container);
  await act(() => root.render(App));
  await settle();
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
});

it("keeps each disk's scan when another disk is scanned, and each row opens its own results", async () => {
  await scanDisk("Macintosh HD");
  await scanDisk("Backup");
  expect(tag("Macintosh HD")).toBe("Scanned");
  expect(tag("Backup")).toBe("Scanned");
  expect(app().summary?.scanId).toBe("scan-backup-2");
  await click(diskRow("Macintosh HD"));
  expect(app().summary?.scanId).toBe("scan-disk-1");
  expect(app().summary?.state).toBe("completed");
  expect(diskRow("Macintosh HD").getAttribute("aria-current")).toBe("page");
  expect(diskRow("Backup").getAttribute("aria-current")).toBeNull();
  // The toolbar's Rescan targets the disk whose results are on screen.
  expect(app().target).toEqual(disk);
  await click(diskRow("Backup"));
  expect(app().summary?.scanId).toBe("scan-backup-2");
  expect(app().target).toEqual(backup);
  expect(welcome()).toBeNull();
  expect(calls("start_scan")).toHaveLength(2);
  expect(calls("discard_scan")).toHaveLength(0);
});

it("opens a disk with no scan on the welcome page while others keep their results", async () => {
  await scanDisk("Macintosh HD");
  await click(diskRow("Backup"));
  expect(welcome()).not.toBeNull();
  expect(tag("Backup")).toBeUndefined();
  expect(tag("Macintosh HD")).toBe("Scanned");
  await click(diskRow("Macintosh HD"));
  expect(welcome()).toBeNull();
  expect(app().summary?.scanId).toBe("scan-disk-1");
});

it("rescanning a disk replaces only that disk's scan", async () => {
  await scanDisk("Macintosh HD");
  await scanDisk("Backup");
  const other = app().scanSlots["/Volumes/Backup"];
  await click(diskRow("Macintosh HD"));
  await act(() => app().rescan());
  await settle();
  expect(calls("start_scan")).toHaveLength(3);
  expect(app().scanSlots["/"].scanId).toBe("scan-disk-3");
  expect(app().summary?.scanId).toBe("scan-disk-3");
  expect(app().scanSlots["/Volumes/Backup"]).toBe(other);
  expect(Object.keys(app().scanSlots).sort()).toEqual(["/", "/Volumes/Backup"]);
  await click(diskRow("Backup"));
  expect(app().summary?.scanId).toBe("scan-backup-2");
});

it("routes progress to the scan's own row while another disk's results are on screen", async () => {
  await scanDisk("Macintosh HD");
  startState = "running";
  await scanDisk("Backup");
  await click(diskRow("Macintosh HD"));
  const shown = app().summary;
  expect(shown?.scanId).toBe("scan-disk-1");
  expect(tag("Backup")).toBe("Scanning");
  expect(diskRow("Backup").querySelector("[role=progressbar]")).not.toBeNull();
  // Scans run side by side: Macintosh HD is not scanning, so its Rescan stays available.
  expect(app().can.rescan).toBe(true);
  await progress("scan-backup-2", { allocatedBytes: 250, entries: 9 });
  expect(app().summary).toBe(shown);
  expect(app().scanSlots["/Volumes/Backup"].summary?.entries).toBe(9);
  expect(
    diskRow("Backup")
      .querySelector("[role=progressbar]")
      ?.getAttribute("aria-valuenow"),
  ).toBe("50");
  await progress("scan-backup-2", { state: "completed", entries: 11 });
  expect(app().summary).toBe(shown);
  expect(tag("Backup")).toBe("Scanned");
  expect(app().can.rescan).toBe(true);
  await click(diskRow("Backup"));
  expect(app().summary?.scanId).toBe("scan-backup-2");
  expect(app().summary?.entries).toBe(11);
  expect(app().summary?.state).toBe("completed");
});

it("keeps showing live progress after the user returns to a scanning disk", async () => {
  startState = "running";
  await scanDisk("Macintosh HD");
  await click(diskRow("Backup"));
  await click(diskRow("Macintosh HD"));
  expect(app().summary?.state).toBe("running");
  await progress("scan-disk-1", { entries: 40 });
  expect(app().summary?.entries).toBe(40);
  await progress("scan-disk-1", { state: "completed" });
  expect(app().summary?.state).toBe("completed");
  expect(tag("Macintosh HD")).toBe("Scanned");
});

it("keeps the chosen list view while the scan on screen reports progress", async () => {
  startState = "running";
  await scanDisk("Macintosh HD");
  await click(container.querySelector<HTMLButtonElement>("#file-types-tab")!);
  await progress("scan-disk-1", { entries: 40 });
  expect(
    container.querySelector("#file-types-tab")?.getAttribute("aria-selected"),
  ).toBe("true");
});
it("discards an ejected disk's scan, removes its row and opens the welcome page on the startup disk", async () => {
  await scanDisk("Macintosh HD");
  await scanDisk("Backup");
  expect(app().summary?.scanId).toBe("scan-backup-2");
  mounted = ["/"];
  discovered = [disk, home];
  await refresh();
  expect(calls("discard_scan")).toEqual([
    ["discard_scan", { scanId: "scan-backup-2" }],
  ]);
  expect(diskNames()).toEqual(["Macintosh HD"]);
  expect(Object.keys(app().scanSlots)).toEqual(["/"]);
  expect(app().summary).toBeNull();
  expect(welcome()).not.toBeNull();
  expect(app().target).toEqual(disk);
  expect(diskRow("Macintosh HD").getAttribute("aria-current")).toBe("page");
  // A scan that had finished leaves nothing to wait for.
  expect(scanButton().disabled).toBe(false);
  // The other disk's scan is untouched.
  await click(diskRow("Macintosh HD"));
  expect(app().summary?.scanId).toBe("scan-disk-1");
  expect(calls("discard_scan")).toHaveLength(1);
});

it("discards an ejected disk's scan without disturbing the results of another disk on screen", async () => {
  await scanDisk("Macintosh HD");
  await scanDisk("Backup");
  await click(diskRow("Macintosh HD"));
  const shown = app().summary;
  mounted = ["/"];
  discovered = [disk, home];
  await refresh();
  expect(calls("discard_scan")).toEqual([
    ["discard_scan", { scanId: "scan-backup-2" }],
  ]);
  expect(app().summary).toBe(shown);
  expect(welcome()).toBeNull();
  expect(app().scanSlots["/Volumes/Backup"]).toBeUndefined();
});

it.each([false, true])(
  "closes an open Trash review (queued: %s) when the shown disk is ejected",
  async (queued) => {
    await scanDisk("Backup");
    await act(() => app().setTrash({ scanId: "live", token: "ticket", items: [node], queued }));
    expect(container.querySelector("dialog, [role=dialog]")).not.toBeNull();
    mounted = ["/"];
    discovered = [disk, home];
    await refresh();
    expect(app().summary).toBeNull();
    expect(app().trash).toBeNull();
    expect(container.querySelector("dialog, [role=dialog]")).toBeNull();
  },
);

it("keeps a still-mounted disk's scan when that disk fails to register on a refresh", async () => {
  await scanDisk("Macintosh HD");
  await scanDisk("Backup");
  // Backup is still mounted, but discovery left it out (its register failed this time).
  mounted = ["/", "/Volumes/Backup", "/Volumes/Other"];
  discovered = [disk, home];
  await refresh();
  expect(calls("discard_scan")).toHaveLength(0);
  expect(diskNames()).toEqual(["Macintosh HD", "Backup"]);
  expect(app().summary?.scanId).toBe("scan-backup-2");
});

it("does not discard anything when a disk with no scan is ejected", async () => {
  await scanDisk("Macintosh HD");
  mounted = ["/"];
  discovered = [disk, home];
  await refresh();
  expect(diskNames()).toEqual(["Macintosh HD"]);
  expect(calls("discard_scan")).toHaveLength(0);
  expect(app().summary?.scanId).toBe("scan-disk-1");
});

it("lets other scans start while an ejected disk's running scan winds down, keeps global operations waiting for its end, and ignores its late events", async () => {
  startState = "running";
  await scanDisk("Macintosh HD");
  // Gone from the mount table too, as an ejected disk is.
  mounted = [];
  discovered = [home];
  await refresh();
  expect(calls("discard_scan")).toHaveLength(1);
  expect(app().scanSlots).toEqual({});
  expect(app().summary).toBeNull();
  // The slot is gone and does not stop another scan; the backend still counts the walk for the global operations.
  expect(welcome()).not.toBeNull();
  expect(scanButton().disabled).toBe(false);
  expect(app().can.rescan).toBe(true);
  expect(app().anyScanRunning).toBe(true);
  await progress("scan-disk-1", { state: "running", entries: 50 });
  expect(app().scanSlots).toEqual({});
  expect(app().anyScanRunning).toBe(true);
  await progress("scan-disk-1", { state: "cancelled" });
  expect(app().scanSlots).toEqual({});
  expect(app().summary).toBeNull();
  expect(app().anyScanRunning).toBe(false);
  expect(scanButton().disabled).toBe(false);
  expect(container.textContent).not.toContain("Scan cancelled");
});

it("lists a folder scan after the disks and replaces only the previous folder scan", async () => {
  await scanDisk("Macintosh HD");
  expect(folderRow()).toBeNull();
  await scanFolder(photos);
  expect(diskNames()).toEqual(["Macintosh HD", "Backup", "Photos"]);
  expect(folderRow()?.getAttribute("aria-label")).toBe(
    "Photos, folder, Scanned",
  );
  expect(folderRow()?.getAttribute("aria-current")).toBe("page");
  expect(tag("Macintosh HD")).toBe("Scanned");
  await click(diskRow("Macintosh HD"));
  expect(app().summary?.scanId).toBe("scan-disk-1");
  await click(folderRow()!);
  expect(app().summary?.scanId).toBe("scan-photos-2");
  expect(app().target).toEqual(photos);
  // A second folder scan takes over the folder slot; the disk keeps its own.
  await scanFolder(music);
  expect(Object.keys(app().scanSlots).sort()).toEqual(["/", "folder"]);
  expect(folderRow()?.getAttribute("aria-label")).toBe(
    "Music, folder, Scanned",
  );
  expect(container.querySelectorAll("button.folder-nav")).toHaveLength(1);
  await click(diskRow("Macintosh HD"));
  expect(app().summary?.scanId).toBe("scan-disk-1");
});

it("opens a scan that has not reported yet as an empty view for that disk, not another disk's results", async () => {
  await scanDisk("Macintosh HD");
  // The first summary never arrives, so Backup's slot has no summary.
  failSummary.add("scan-backup-2");
  await click(diskRow("Backup"));
  await click(scanButton());
  app().setError("");
  await settle();
  expect(app().scanSlots["/Volumes/Backup"].summary).toBeNull();
  await click(diskRow("Macintosh HD"));
  expect(app().summary?.scanId).toBe("scan-disk-1");
  await click(diskRow("Backup"));
  // Screen, selected disk and highlight agree: nothing of Macintosh HD is shown.
  expect(app().summary).toBeNull();
  expect(app().shown?.slot ?? null).toBeNull();
  expect(app().target).toEqual(backup);
  expect(welcome()).not.toBeNull();
  expect(diskRow("Backup").getAttribute("aria-current")).toBe("page");
  expect(diskRow("Macintosh HD").getAttribute("aria-current")).toBeNull();
  expect(tag("Backup")).toBe("Scanning");
  // Its first event fills the view.
  await progress("scan-backup-2", { state: "running", entries: 7 });
  expect(app().summary?.scanId).toBe("scan-backup-2");
  expect(app().shown?.slot ?? null).toBe("/Volumes/Backup");
  expect(app().scanSlots["/Volumes/Backup"].summary?.entries).toBe(7);
  await click(diskRow("Macintosh HD"));
  expect(app().summary?.scanId).toBe("scan-disk-1");
});

it("rescans a queue entry's disk in that disk's own slot without adding a second Disks row", async () => {
  await scanDisk("Macintosh HD");
  await scanDisk("Backup");
  const other = app().scanSlots["/Volumes/Backup"];
  const listed = app().targets;
  // The backend flags the disk like the Disks list does.
  queueTarget = { ...disk, id: "disk-queue", name: "/" };
  await act(() => app().scanQueue("entry"));
  await settle();
  expect(calls("start_scan").at(-1)?.[1].targetId).toBe("disk-queue");
  expect(diskNames()).toEqual(["Macintosh HD", "Backup"]);
  expect(app().targets).toBe(listed);
  expect(app().target).toBe(disk);
  expect(Object.keys(app().scanSlots).sort()).toEqual(["/", "/Volumes/Backup"]);
  expect(app().scanSlots["/"].scanId).toBe("scan-disk-queue-3");
  expect(app().scanSlots["/Volumes/Backup"]).toBe(other);
  expect(tag("Macintosh HD")).toBe("Scanned");
  expect(folderRow()).toBeNull();
  expect(diskRow("Macintosh HD").getAttribute("aria-current")).toBe("page");
});

it("leaves the view alone when the queue entry's lookup returns after the user switched disk", async () => {
  await scanDisk("Macintosh HD");
  const standard = mock.invoke.getMockImplementation()!;
  let release!: () => void;
  const lookup = new Promise<void>((resolve) => (release = resolve));
  mock.invoke.mockImplementation(async (command: string, args: any) => {
    if (command === "cleanup_queue_target") await lookup;
    return standard(command, args);
  });
  queueTarget = { ...photos, id: "photos-queue" };
  await act(() => void app().scanQueue("entry"));
  await click(diskRow("Backup"));
  const before = calls("start_scan").length;
  release();
  await settle();
  expect(calls("start_scan")).toHaveLength(before);
  expect(app().target).toBe(backup);
  expect(app().homeOpen).toBe(true);
});

it("lists a queue entry's folder as a folder scan", async () => {
  await scanDisk("Macintosh HD");
  queueTarget = { ...photos, id: "photos-queue" };
  await act(() => app().scanQueue("entry"));
  await settle();
  expect(app().targets.some((t) => t.id === "photos-queue")).toBe(true);
  expect(Object.keys(app().scanSlots).sort()).toEqual(["/", "folder"]);
  expect(folderRow()?.getAttribute("aria-label")).toBe(
    "Photos, folder, Scanned",
  );
});

// Parallel scans: one running scan per slot.
const stopButton = () =>
  container.querySelector<HTMLButtonElement>("button.rescan-action")!;

it("starts a scan on another disk while one runs, and each reports only to its own row", async () => {
  startState = "running";
  await scanDisk("Macintosh HD");
  await scanDisk("Backup");
  expect(tag("Macintosh HD")).toBe("Scanning");
  expect(tag("Backup")).toBe("Scanning");
  expect(calls("start_scan")).toHaveLength(2);
  expect(app().anyScanRunning).toBe(true);
  // The results on screen are the disk that was opened last.
  expect(app().summary?.scanId).toBe("scan-backup-2");
  const shown = app().summary;
  await progress("scan-disk-1", { allocatedBytes: 100, entries: 5 });
  expect(app().summary).toBe(shown);
  expect(app().scanSlots["/"].summary?.entries).toBe(5);
  expect(app().scanSlots["/Volumes/Backup"].summary?.entries).toBe(2);
  await progress("scan-backup-2", { allocatedBytes: 400, entries: 9 });
  expect(app().summary?.entries).toBe(9);
  expect(app().scanSlots["/"].summary?.entries).toBe(5);
  // One finishing does not end the other.
  await progress("scan-disk-1", { state: "completed", entries: 6 });
  expect(tag("Macintosh HD")).toBe("Scanned");
  expect(tag("Backup")).toBe("Scanning");
  expect(app().summary?.state).toBe("running");
  expect(app().anyScanRunning).toBe(true);
  await progress("scan-backup-2", { state: "completed", entries: 11 });
  expect(app().anyScanRunning).toBe(false);
  await click(diskRow("Macintosh HD"));
  expect(app().summary?.entries).toBe(6);
  await click(diskRow("Backup"));
  expect(app().summary?.entries).toBe(11);
});

it("lets a second disk be scanned while the first is still running, from either view", async () => {
  startState = "running";
  await scanDisk("Macintosh HD");
  // Home for another disk: its Scan is available while Macintosh HD scans.
  await click(diskRow("Backup"));
  expect(scanButton().disabled).toBe(false);
  expect(app().can.rescan).toBe(true);
  // The scanning disk's own welcome page keeps Scan off.
  await act(() => {
    app().setTarget(disk);
    app().showHome();
  });
  expect(scanButton().disabled).toBe(true);
  expect(app().can.rescan).toBe(false);
  await act(() => app().start(disk));
  await settle();
  expect(calls("start_scan")).toHaveLength(1);
  // Disks and the folder picker stay available.
  expect(diskRow("Backup").disabled).toBe(false);
  expect(app().can.chooseFolder).toBe(true);
});

it("cancels only the scan on screen and keeps its partial results while the others run", async () => {
  startState = "running";
  await scanDisk("Macintosh HD");
  await scanDisk("Backup");
  expect(stopButton().disabled).toBe(false);
  await click(stopButton());
  expect(calls("cancel_scan")).toEqual([
    ["cancel_scan", { scanId: "scan-backup-2" }],
  ]);
  await progress("scan-backup-2", { state: "cancelled", entries: 7 });
  expect(tag("Backup")).toBe("Cancelled");
  expect(app().scanSlots["/Volumes/Backup"].summary?.state).toBe("cancelled");
  expect(app().scanSlots["/Volumes/Backup"].summary?.entries).toBe(7);
  expect(app().summary?.state).toBe("cancelled");
  // Macintosh HD was not touched and keeps scanning.
  expect(tag("Macintosh HD")).toBe("Scanning");
  expect(app().scanSlots["/"].summary?.state).toBe("running");
  expect(app().anyScanRunning).toBe(true);
  await progress("scan-disk-1", { entries: 30 });
  expect(app().scanSlots["/"].summary?.entries).toBe(30);
  await click(diskRow("Macintosh HD"));
  expect(app().summary?.scanId).toBe("scan-disk-1");
  expect(app().summary?.state).toBe("running");
  await click(stopButton());
  expect(calls("cancel_scan").at(-1)).toEqual([
    "cancel_scan",
    { scanId: "scan-disk-1" },
  ]);
});

it("keeps every dismissed scan failure closed, not only the last one", async () => {
  startState = "running";
  await scanDisk("Macintosh HD");
  await scanDisk("Backup");
  await progress("scan-disk-1", { state: "failed" });
  await progress("scan-backup-2", { state: "failed" });
  const dialog = () => container.querySelector('[role="dialog"]');
  const dismiss = () =>
    act(() =>
      dialog()!.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      ),
    );
  expect(dialog()).not.toBeNull();
  await dismiss();
  await click(diskRow("Macintosh HD"));
  expect(dialog()).not.toBeNull();
  await dismiss();
  await click(diskRow("Backup"));
  expect(dialog()).toBeNull();
  await click(diskRow("Macintosh HD"));
  expect(dialog()).toBeNull();
});

it("keeps a failed scan's partial results and leaves the other scans running", async () => {
  startState = "running";
  await scanDisk("Macintosh HD");
  await scanDisk("Backup");
  await progress("scan-disk-1", { state: "failed", entries: 3 });
  expect(tag("Macintosh HD")).toBe("Scan failed");
  expect(app().scanSlots["/"].summary?.state).toBe("failed");
  expect(app().scanSlots["/"].summary?.entries).toBe(3);
  expect(tag("Backup")).toBe("Scanning");
  expect(app().summary?.scanId).toBe("scan-backup-2");
  expect(app().summary?.state).toBe("running");
});

it("keeps the rest of the app usable while one disk's start is stalled", async () => {
  await scanDisk("Backup");
  await act(() => app().showQueue(true));
  await settle();
  expect(app().queueOpen).toBe(true);
  let release!: () => void;
  startGate = new Promise<void>((resolve) => (release = resolve));
  // A sleeping disk: its start never answers.
  const stalled = app().start(disk);
  await settle();
  expect(app().busy).toBe(false);
  expect(app().queueRootBusy("/")).toBe(true);
  expect(app().queueRootBusy("/Volumes/Backup")).toBe(false);
  await act(() => app().showQueue(false));
  expect(app().queueOpen).toBe(false);
  await click(diskRow("Backup"));
  expect(app().target).toEqual(backup);
  expect(app().summary?.scanId).toBe("scan-backup-1");
  release();
  await act(async () => void (await stalled));
  await settle();
  expect(app().scanSlots["/"].scanId).toBe("scan-disk-2");
});

it("does not let a stalled rescan take the view after the user selected another disk", async () => {
  await scanDisk("Backup");
  let release!: () => void;
  startGate = new Promise<void>((resolve) => (release = resolve));
  const stalled = app().rescan();
  await settle();
  // Macintosh HD has no scan: its welcome page opens with it selected.
  await click(diskRow("Macintosh HD"));
  expect(welcome()).not.toBeNull();
  release();
  await act(async () => void (await stalled));
  await settle();
  expect(app().scanSlots["/Volumes/Backup"].scanId).toBe("scan-backup-2");
  expect(app().target).toEqual(disk);
  expect(welcome()).not.toBeNull();
  // The hidden results are still the old scan's, not the rescan's.
  expect(app().summary?.scanId).toBe("scan-backup-1");
});

it("shows the new scan when a rescan's start lands after the user reopened the old scan of that disk", async () => {
  await scanDisk("Macintosh HD");
  await scanDisk("Backup");
  let release!: () => void;
  startGate = new Promise<void>((resolve) => (release = resolve));
  const stalled = app().start(disk);
  await settle();
  // The backend already replaced the old scan; the sidebar still opens it.
  await click(diskRow("Macintosh HD"));
  expect(app().summary?.scanId).toBe("scan-disk-1");
  release();
  await act(async () => void (await stalled));
  await settle();
  expect(app().scanSlots["/"].scanId).toBe("scan-disk-3");
  expect(app().summary?.scanId).toBe("scan-disk-3");
});

it("labels the welcome page's Scan button while that disk's start is unanswered", async () => {
  startGate = new Promise<void>(() => {});
  await click(diskRow("Macintosh HD"));
  await click(scanButton());
  expect(scanButton().getAttribute("aria-busy")).toBe("true");
  expect(scanButton().textContent).toContain("Starting scan…");
  await click(diskRow("Backup"));
  expect(scanButton().getAttribute("aria-busy")).toBe("false");
});

it("keeps the sidebar usable while a scan started from the cleanup queue is stalled", async () => {
  queueTarget = { ...photos, id: "photos-queue" };
  startGate = new Promise<void>(() => {});
  await act(() => void app().scanQueue("entry"));
  await settle();
  expect(calls("start_scan")).toHaveLength(1);
  expect(diskRow("Backup").disabled).toBe(false);
  expect(app().can.chooseFolder).toBe(true);
});

it("removes an ejected disk while another disk's start is stalled", async () => {
  startGate = new Promise<void>(() => {});
  void app().start(disk);
  await settle();
  mounted = ["/"];
  discovered = [disk, home];
  await refresh();
  expect(diskNames()).toEqual(["Macintosh HD"]);
});

it("looks the disks up again when a scan is refused because another volume sits at the same path", async () => {
  const swapped = { ...backup, id: "backup-swapped" };
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command: string, args: any) => {
    if (command === "start_scan" && args.targetId === "backup")
      throw new Error("The chosen folder changed. Choose it again before scanning.");
    return standard(command, args);
  });
  await click(diskRow("Backup"));
  await click(scanButton());
  expect(app().error).toContain("chosen folder changed");
  // The mount table still lists the same paths.
  discovered = [disk, swapped, home];
  const before = calls("discover_targets").length;
  await refresh();
  expect(calls("discover_targets")).toHaveLength(before + 1);
  expect(app().targets.map((item) => item.id)).toEqual([
    "disk",
    "backup-swapped",
    "home",
  ]);
  expect(app().target?.id).toBe("backup-swapped");
  await click(scanButton());
  expect(calls("start_scan").at(-1)?.[1].targetId).toBe("backup-swapped");
  // The lookup ran once; unchanged paths do not trigger another.
  await refresh();
  expect(calls("discover_targets")).toHaveLength(before + 1);
});

it("tracks every start that overlaps another, and only the newest takes the view", async () => {
  startState = "running";
  let release!: () => void;
  startGate = new Promise<void>((resolve) => (release = resolve));
  // Both starts are in flight before either answers.
  const first = app().start(disk);
  const second = app().start(backup);
  await settle();
  // A start in flight blocks only its own disk, not the whole app.
  expect(app().busy).toBe(false);
  expect(app().queueRootBusy("/")).toBe(true);
  expect(app().queueRootBusy("/Volumes/Backup")).toBe(true);
  // The first scan reports before its start returns, after a newer start took over.
  await progress("scan-disk-1", { entries: 4 });
  expect(app().scanSlots["/"].scanId).toBe("scan-disk-1");
  expect(app().scanSlots["/"].summary?.entries).toBe(4);
  release();
  await act(async () => {
    await Promise.all([first, second]);
  });
  await settle();
  expect(app().busy).toBe(false);
  expect(Object.keys(app().scanSlots).sort()).toEqual(["/", "/Volumes/Backup"]);
  expect(app().scanSlots["/Volumes/Backup"].scanId).toBe("scan-backup-2");
  expect(tag("Macintosh HD")).toBe("Scanning");
  expect(tag("Backup")).toBe("Scanning");
  expect(app().summary?.scanId).toBe("scan-backup-2");
  await progress("scan-backup-2", { entries: 8 });
  expect(app().summary?.entries).toBe(8);
  await progress("scan-disk-1", { entries: 12 });
  expect(app().summary?.entries).toBe(8);
  expect(app().scanSlots["/"].summary?.entries).toBe(12);
});

it("tells which queue locations a scan cannot start on: only those whose own slot is scanning", async () => {
  startState = "running";
  await scanDisk("Backup");
  expect(app().queueRootBusy("/Volumes/Backup")).toBe(true);
  expect(app().queueRootBusy("/")).toBe(false);
  // A location that is not a listed disk uses the folder slot.
  expect(app().queueRootBusy("/Users/ada/Photos")).toBe(false);
  await scanFolder(photos);
  expect(app().queueRootBusy("/Users/ada/Photos")).toBe(true);
  expect(app().queueRootBusy("/Users/ada/Music")).toBe(true);
  expect(app().queueRootBusy("/")).toBe(false);
});

const queueEntry = (
  id: string,
  rootPath: string,
  status: QueueEntry["status"],
  // The scan the backend matched the entry to: its own root's scan, or a disk scan covering it.
  matchedScanId: string | null = null,
): QueueEntry => ({
  id,
  name: id,
  path: `${rootPath === "/" ? "" : rootPath}/${id}`,
  rootPath,
  allocatedBytes: 10,
  status,
  reason: status === "ready" ? null : "Scan the original location",
  matchedScanId,
});
const statusOf = (id: string) =>
  [...container.querySelectorAll(".cleanup-entry")]
    .find((entry) => entry.querySelector("strong")?.textContent === id)
    ?.getAttribute("data-status");

it("drops the other scans the backend invalidated after a Trash and keeps the rest", async () => {
  await scanDisk("Macintosh HD");
  await scanDisk("Backup");
  await scanFolder(photos);
  const fallback = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command: string, args: any) => {
    if (command === "prepare_cleanup_queue")
      return { token: "ticket", items: [node] };
    if (command === "commit_cleanup_queue")
      return {
        trashed: 1,
        failures: [],
        persistenceWarning: null,
        outcomes: [{ reviewIndex: 0, status: "trashed" }],
        invalidated: true,
        staleScanIds: ["scan-disk-1"],
      };
    return fallback(command, args);
  });
  await act(() => app().showQueue(true));
  await settle();
  await act(() => app().prepareQueue());
  await settle();
  await act(() => app().commitTrash());
  await settle();
  expect(app().scanSlots["/"]).toBeUndefined();
  expect(app().scanSlots["/Volumes/Backup"].scanId).toBe("scan-backup-2");
});

it("shows queue entries from several disks, each with its own status, and reviews only the disk on screen", async () => {
  await scanDisk("Macintosh HD");
  await scanDisk("Backup");
  queueEntries = [
    queueEntry("hd-item", "/", "ready", "scan-disk-1"),
    queueEntry("backup-item", "/Volumes/Backup", "ready", "scan-backup-2"),
    queueEntry("gone-item", "/Volumes/Gone", "needsScan"),
  ];
  await act(() => app().showQueue(true));
  await settle();
  expect(container.querySelectorAll(".cleanup-group")).toHaveLength(3);
  expect(statusOf("hd-item")).toBe("ready");
  expect(statusOf("backup-item")).toBe("ready");
  expect(statusOf("gone-item")).toBe("needsScan");
  const review = () =>
    [...container.querySelectorAll<HTMLButtonElement>("button")].find((b) =>
      /^Review \d+ ready$/.test(b.textContent?.trim() ?? ""),
    );
  const actions = (name: string) =>
    [...container.querySelectorAll(".cleanup-group")]
      .find((group) => group.querySelector("h2")?.textContent === name)!
      .querySelectorAll("button:not(.cleanup-remove)");
  const labels = (name: string) =>
    [...actions(name)].map((b) => b.textContent?.trim());
  // Backup is on screen: only its section can review; the startup disk and the missing one scan.
  expect(review()!.disabled).toBe(false);
  expect(labels("Backup")).toEqual(["Review 1 ready"]);
  expect(labels("Macintosh HD")).toEqual(["Scan Macintosh HD"]);
  expect(labels("Gone")).toEqual(["Scan Gone"]);
  // The tally counts every disk; Review counts only the scan on screen.
  expect(container.querySelector(".cleanup-tally")?.textContent).toContain(
    "3 items",
  );
  // A folder's entry that the backend matched to the Backup disk scan is reviewed from that scan.
  queueEntries = [
    queueEntry("hd-item", "/", "ready", "scan-disk-1"),
    queueEntry(
      "folder-item",
      "/Volumes/Backup/Photos",
      "ready",
      "scan-backup-2",
    ),
  ];
  await act(() => app().showQueue(false));
  await act(() => app().showQueue(true));
  await settle();
  expect(review()!.disabled).toBe(false);
  expect(labels("Photos")).toEqual(["Review 1 ready"]);
  // Only the other disk has ready entries: nothing to review here, and Backup offers its scan.
  queueEntries = [
    queueEntry("hd-item", "/", "ready", "scan-disk-1"),
    queueEntry("backup-item", "/Volumes/Backup", "needsScan"),
  ];
  await act(() => app().showQueue(false));
  await act(() => app().showQueue(true));
  await settle();
  expect(review()).toBeUndefined();
  expect(labels("Backup")).toEqual(["Scan Backup"]);
  // The Macintosh HD scan on screen reviews the entry matched to it.
  await act(() => app().showQueue(false));
  await click(diskRow("Macintosh HD"));
  await act(() => app().showQueue(true));
  await settle();
  expect(review()!.disabled).toBe(false);
  expect(labels("Macintosh HD")).toEqual(["Review 1 ready"]);
});

it("does not refresh the cleanup queue on progress of the scan on screen, only when its state changes", async () => {
  startState = "running";
  await scanDisk("Macintosh HD");
  await settle();
  const refreshes = () => calls("list_cleanup_queue").length;
  const before = refreshes();
  for (const entries of [10, 20, 30]) {
    await progress("scan-disk-1", { entries });
    await settle();
  }
  expect(refreshes()).toBe(before);
  await progress("scan-disk-1", { state: "completed" });
  await settle();
  expect(refreshes()).toBeGreaterThan(before);
});

it("refreshes the cleanup queue when any slot's scan ends or is dropped, not only the displayed one", async () => {
  startState = "running";
  await scanDisk("Macintosh HD");
  await scanDisk("Backup");
  await click(diskRow("Macintosh HD"));
  const refreshes = () => calls("list_cleanup_queue").length;
  // Progress that changes no state does not refresh.
  let before = refreshes();
  await progress("scan-backup-2", { entries: 50 });
  await settle();
  expect(refreshes()).toBe(before);
  // A scan that is not on screen completes.
  await progress("scan-backup-2", { state: "completed" });
  await settle();
  expect(refreshes()).toBeGreaterThan(before);
  // The displayed scan fails.
  before = refreshes();
  await progress("scan-disk-1", { state: "failed" });
  await settle();
  expect(refreshes()).toBeGreaterThan(before);
  // An ejected disk's slot is dropped.
  before = refreshes();
  mounted = ["/"];
  discovered = [disk, home];
  await refresh();
  expect(refreshes()).toBeGreaterThan(before);
});

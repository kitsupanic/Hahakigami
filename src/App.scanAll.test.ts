// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, createTestRoot, type Root } from "./testUtils";
import App from "./App.svelte";
import type createApp from "./createApp.svelte";
import type { NodeView, ScanSummary, Target } from "./types";

const mock = vi.hoisted(() => ({
  invoke: vi.fn(),
  app: null as ReturnType<typeof createApp> | null,
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
const disk = (id: string, name: string, path: string): Target => ({
  id,
  name,
  path,
  totalBytes: 1000,
  freeBytes: 500,
  volume: true,
});
const hd = disk("hd", "Macintosh HD", "/");
const backup = disk("backup", "Backup", "/Volumes/Backup");
const usb = disk("usb", "SANDISK", "/Volumes/SANDISK");
const photos: Target = {
  id: "photos",
  name: "Photos",
  path: "/Users/ada/Photos",
  totalBytes: null,
  freeBytes: null,
};
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
let discovered: Target[];
// Scans stay running until a test reports their end.
let channels: Map<string, { onmessage: (summary: unknown) => void }>;
let refused: Map<string, string>;
let started: string[];
let chosen: Target;
let accessStatus: "on" | "off";
// While set, start_scan answers only once it resolves.
let startGate: Promise<void> | null;
let container: HTMLDivElement, root: Root;
const app = () => mock.app!;
const calls = (command: string) =>
  mock.invoke.mock.calls.filter(([name]) => name === command);
const startedFor = (targetId: string) =>
  calls("start_scan").filter(([, args]) => args.targetId === targetId);
const headerButton = () =>
  container.querySelector<HTMLButtonElement>("button.scan-all")!;
const welcomeButton = () =>
  container.querySelector<HTMLButtonElement>("button.welcome-scan-all")!;
const indicator = () =>
  container.querySelector(".batch-status")?.textContent?.trim();
async function settle() {
  await act(async () => {
    for (let i = 0; i < 8; i++) await Promise.resolve();
  });
}
async function click(button: HTMLButtonElement) {
  await act(() => button.click());
  await settle();
}
async function report(scanId: string, state: ScanSummary["state"]) {
  await act(() => channels.get(scanId)!.onmessage(makeSummary(scanId, state)));
  await settle();
}
const idOf = (targetId: string) =>
  started.filter((id) => id.startsWith(`scan-${targetId}-`)).at(-1)!;
async function mountApp() {
  container = document.createElement("div");
  document.body.append(container);
  root = createTestRoot(container);
  await act(() => root.render(App));
  await settle();
}
async function mountAgain() {
  await act(() => root.unmount());
  container.remove();
  await mountApp();
}
beforeEach(async () => {
  discovered = [hd, backup, usb];
  channels = new Map();
  refused = new Map();
  started = [];
  accessStatus = "on";
  startGate = null;
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(async (command: string, args: any) => {
    switch (command) {
      case "get_explorer_preferences":
        return { sort: "sizeDescending", warning: null };
      case "full_disk_access_status":
        return {
          status: accessStatus,
          scanAnyway: false,
          appPath: "/Fixture.app",
        };
      case "scan_without_full_disk_access":
        accessStatus = "on";
        return { status: "off", scanAnyway: true, appPath: "/Fixture.app" };
      case "discover_targets":
        return discovered;
      case "mounted_volumes":
        return discovered.map((item) => item.path);
      case "choose_folder":
        return chosen;
      case "list_cleanup_queue":
        return { entries: [], revision: 0, warning: null, blocked: false };
      case "start_scan": {
        const error = refused.get(args.targetId);
        if (error) throw new Error(error);
        const id = `scan-${args.targetId}-${started.length + 1}`;
        started.push(id);
        channels.set(id, args.onEvent);
        if (startGate) await startGate;
        return id;
      }
      case "scan_summary":
        return makeSummary(args.scanId, "running");
      case "get_node":
        return node;
      case "get_map":
        return { nodes: [] };
      case "get_children":
        return { items: [], total: 0 };
      default:
        return null;
    }
  });
  await mountApp();
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
});

it("starts exactly the disks and leaves the folder slot alone", async () => {
  chosen = photos;
  await click(
    container.querySelector<HTMLButtonElement>("button.welcome-target")!,
  );
  await click(container.querySelector<HTMLButtonElement>("button.welcome-scan")!);
  const folderScan = app().scanSlots.folder.scanId;
  await report(folderScan, "completed");
  await act(() => app().openDisk(hd));
  await click(welcomeButton());
  expect(calls("start_scan").map(([, args]) => args.targetId).sort()).toEqual([
    "backup",
    "hd",
    "photos",
    "usb",
  ]);
  expect(Object.keys(app().scanSlots).sort()).toEqual([
    "/",
    "/Volumes/Backup",
    "/Volumes/SANDISK",
    "folder",
  ]);
  expect(app().scanSlots.folder.scanId).toBe(folderScan);
  expect(calls("cancel_scan")).toHaveLength(0);
});

it("cancels a running disk's scan, starts it again once it has ended, and starts the idle disks at once", async () => {
  await click(headerButton());
  const first = idOf("backup");
  expect(started).toHaveLength(3);
  await click(headerButton());
  // Every disk is running now: each is cancelled and none restarts before its scan has ended.
  expect(calls("cancel_scan").map(([, args]) => args.scanId).sort()).toEqual(
    started.slice(0, 3).sort(),
  );
  expect(started).toHaveLength(3);
  expect(headerButton().disabled).toBe(true);
  await report(first, "cancelled");
  expect(startedFor("backup")).toHaveLength(2);
  expect(startedFor("hd")).toHaveLength(1);
  await report(idOf("hd"), "cancelled");
  await report(idOf("usb"), "cancelled");
  expect(started).toHaveLength(6);
  expect(
    Object.values(app().scanSlots).map((slot) => slot.scanId).sort(),
  ).toEqual(started.slice(3).sort());
  expect(headerButton().disabled).toBe(false);
  // A restarted scan does not say "cancelled" about itself.
  expect(container.textContent).not.toContain("Scan cancelled");
});

it("keeps starting the other disks when one start is refused, and reports the refusal", async () => {
  refused.set("backup", "Other scans are holding every file this app may keep open.");
  await click(headerButton());
  expect(startedFor("hd")).toHaveLength(1);
  expect(startedFor("usb")).toHaveLength(1);
  expect(Object.keys(app().scanSlots).sort()).toEqual(["/", "/Volumes/SANDISK"]);
  expect(app().error).toContain("Backup: ");
  expect(app().error).toContain("holding every file");
  expect(headerButton().disabled).toBe(false);
});

it("shows the overall indicator only while two or more scans run, counting the ones that ended", async () => {
  expect(indicator()).toBe("");
  await click(headerButton());
  expect(indicator()).toBe("Scanning · 0 of 3 done");
  await report(idOf("hd"), "completed");
  expect(indicator()).toBe("Scanning · 1 of 3 done");
  await report(idOf("backup"), "cancelled");
  // One scan left: the indicator goes away, and the batch ends with it.
  expect(indicator()).toBe("");
  await report(idOf("usb"), "failed");
  expect(indicator()).toBe("");
  // A new batch counts from zero.
  await click(headerButton());
  expect(indicator()).toBe("Scanning · 0 of 3 done");
  expect(container.querySelector(".batch-status")?.getAttribute("aria-live")).toBe(
    "polite",
  );
});

it("enables both buttons together, and not without a disk or while starts are in flight", async () => {
  expect(headerButton().disabled).toBe(false);
  expect(welcomeButton().disabled).toBe(false);
  expect(app().can.scanAllDisks).toBe(true);
  // Both buttons read the same gate: no disk is nothing to scan.
  await act(() => root.unmount());
  container.remove();
  discovered = [];
  await mountApp();
  expect(app().can.scanAllDisks).toBe(false);
  expect(headerButton().disabled).toBe(true);
  expect(welcomeButton().disabled).toBe(true);
  await click(headerButton());
  expect(calls("start_scan")).toHaveLength(0);
});

it("disables the buttons while a scan start is in flight", async () => {
  let release!: () => void;
  const gate = new Promise<void>((resolve) => (release = resolve));
  const impl = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command: string, args: any) => {
    const result = impl(command, args);
    if (command === "start_scan") await gate;
    return result;
  });
  const pending = app().scanAllDisks();
  await settle();
  expect(headerButton().disabled).toBe(true);
  expect(welcomeButton().disabled).toBe(true);
  release();
  await act(async () => void (await pending));
  await settle();
  expect(headerButton().disabled).toBe(false);
});

const diskRow = (name: string) =>
  [...container.querySelectorAll<HTMLButtonElement>("button.disk-nav")].find(
    (item) => item.getAttribute("aria-label")?.startsWith(`${name},`),
  )!;
const welcome = () => container.querySelector("#welcome-title");
// Selects the disk on its welcome page and scans it.
async function scanDisk(name: string) {
  await click(diskRow(name));
  await click(container.querySelector<HTMLButtonElement>("button.welcome-scan")!);
}

it("takes the view for the selected idle disk only; the other disks update their rows", async () => {
  await click(diskRow("Backup"));
  expect(welcome()).not.toBeNull();
  await click(headerButton());
  expect(started).toHaveLength(3);
  expect(app().summary?.scanId).toBe(idOf("backup"));
  expect(welcome()).toBeNull();
  expect(app().shown?.slot ?? null).toBe("/Volumes/Backup");
});

it("keeps what is on screen while the other disks start, and the selected disk takes the view when its delayed restart begins", async () => {
  await scanDisk("Backup");
  const old = idOf("backup");
  expect(app().summary?.scanId).toBe(old);
  await click(headerButton());
  expect(startedFor("hd")).toHaveLength(1);
  expect(startedFor("usb")).toHaveLength(1);
  expect(app().summary?.scanId).toBe(old);
  await report(old, "cancelled");
  expect(app().summary?.scanId).toBe(idOf("backup"));
  expect(idOf("backup")).not.toBe(old);
  expect(app().shown?.slot ?? null).toBe("/Volumes/Backup");
});

it("leaves a selected folder's results on screen", async () => {
  chosen = photos;
  await click(
    container.querySelector<HTMLButtonElement>("button.welcome-target")!,
  );
  await click(container.querySelector<HTMLButtonElement>("button.welcome-scan")!);
  const folderScan = app().scanSlots.folder.scanId;
  await report(folderScan, "completed");
  expect(app().summary?.scanId).toBe(folderScan);
  await click(headerButton());
  expect(started).toHaveLength(4);
  expect(app().summary?.scanId).toBe(folderScan);
  expect(app().shown?.slot ?? null).toBe("folder");
  expect(app().scanSlots.folder.scanId).toBe(folderScan);
});

it("does not pull the user back to the selected disk when they opened something else during the wait", async () => {
  await scanDisk("Backup");
  const old = idOf("backup");
  await click(headerButton());
  await report(idOf("hd"), "running");
  await click(diskRow("Macintosh HD"));
  expect(app().summary?.scanId).toBe(idOf("hd"));
  await report(old, "cancelled");
  expect(startedFor("backup")).toHaveLength(2);
  expect(app().summary?.scanId).toBe(idOf("hd"));
  expect(app().shown?.slot ?? null).toBe("/");
});

it("asks about Full Disk Access once for Scan all and runs it for every disk after Scan anyway", async () => {
  accessStatus = "off";
  await mountAgain();
  await click(headerButton());
  expect(app().accessOpen).toBe(true);
  expect(started).toHaveLength(0);
  await act(() => app().scanAnyway());
  await settle();
  expect(app().accessOpen).toBe(false);
  expect(started).toHaveLength(3);
});

it("opens the Full Disk Access dialog once when the backend finds access off, instead of listing it per disk", async () => {
  for (const id of ["hd", "backup", "usb"])
    refused.set(id, "FULL_DISK_ACCESS_OFF: macOS would ask about each folder.");
  accessStatus = "off";
  await click(headerButton());
  expect(app().accessOpen).toBe(true);
  expect(app().access?.status).toBe("off");
  expect(app().error).toBe("");
  expect(started).toHaveLength(0);
});

it("does not start a disk again while a single start of it is in flight", async () => {
  let release!: () => void;
  startGate = new Promise<void>((resolve) => (release = resolve));
  const single = app().start(hd);
  await settle();
  // Only that disk is blocked: the other disks can still be scanned.
  expect(app().can.scanAllDisks).toBe(true);
  expect(headerButton().disabled).toBe(false);
  let all!: Promise<void>;
  await act(() => {
    all = app().scanAllDisks();
  });
  await settle();
  expect(startedFor("hd")).toHaveLength(1);
  expect(startedFor("backup")).toHaveLength(1);
  release();
  await act(async () => void (await Promise.all([single, all])));
  await settle();
  expect(startedFor("hd")).toHaveLength(1);
  expect(app().can.scanAllDisks).toBe(true);
});

it("skips a disk that was ejected while its scan was being cancelled", async () => {
  await click(headerButton());
  await click(headerButton());
  discovered = [hd, backup];
  await act(() => window.dispatchEvent(new Event("focus")));
  await settle();
  expect(Object.keys(app().scanSlots)).not.toContain("/Volumes/SANDISK");
  for (const id of started.slice(0, 3)) await report(id, "cancelled");
  expect(startedFor("usb")).toHaveLength(1);
  expect(startedFor("hd")).toHaveLength(2);
  expect(startedFor("backup")).toHaveLength(2);
  expect(app().error).toBe("");
  expect(headerButton().disabled).toBe(false);
});

it("follows the disk whose results are on screen even when another location is selected", async () => {
  await scanDisk("Backup");
  const old = idOf("backup");
  await act(() => app().setTarget(photos));
  await click(headerButton());
  expect(app().summary?.scanId).toBe(old);
  await report(old, "cancelled");
  expect(app().summary?.scanId).toBe(idOf("backup"));
  expect(idOf("backup")).not.toBe(old);
  expect(app().shown?.slot ?? null).toBe("/Volumes/Backup");
});

it("lets the disk on screen win over a different selected disk", async () => {
  await scanDisk("Backup");
  const old = idOf("backup");
  await act(() => app().setTarget(hd));
  await click(headerButton());
  // The selected disk is idle and starts at once, yet the screen stays with the on-screen disk's scan.
  expect(startedFor("hd")).toHaveLength(1);
  expect(app().summary?.scanId).toBe(old);
  await report(old, "cancelled");
  expect(app().summary?.scanId).toBe(idOf("backup"));
});

it("looks the disks up again when Scan all is refused because another volume sits at a disk's path", async () => {
  refused.set("usb", "The chosen folder changed. Choose it again before scanning.");
  await click(welcomeButton());
  discovered = [hd, backup, { ...usb, id: "usb-swapped" }];
  const before = calls("discover_targets").length;
  // The mount table lists the same paths, so only the refusal can trigger the lookup.
  await act(() => window.dispatchEvent(new Event("focus")));
  await settle();
  expect(calls("discover_targets")).toHaveLength(before + 1);
  expect(app().targets.map((item) => item.id)).toContain("usb-swapped");
});

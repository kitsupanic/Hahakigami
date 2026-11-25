// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, createTestRoot, fullEdition, type Root } from "./testUtils";
import App from "./App.svelte";
import type createApp from "./createApp.svelte";
import type { Edition, ScanSummary, Target } from "./types";

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
const lite: Edition = {
  productName: "Hahakigami Lite",
  automationScans: false,
  scanTargetsRequireAllowedLocation: true,
};
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
const summary = (scanId: string): ScanSummary => ({
  scanId,
  rootId: 0,
  state: "running",
  workers: 1,
  workerStats: [],
  finalizationUs: 0,
  entries: 2,
  allocatedBytes: 2048,
  elapsedMs: 1000,
  errors: 0,
});
// What the backend says is allowed, and what the next Open dialog would allow.
let edition: Edition;
let allowed: string[];
let allowing: Map<string, string[]>;
let allowFails: string | null;
let refused: Set<string>;
let mounted: string[];
let picked: Target | null;
let allowGate: Promise<void> | null;
let container: HTMLDivElement, root: Root;
const app = () => mock.app!;
const calls = (command: string) =>
  mock.invoke.mock.calls.filter(([name]) => name === command);
const welcomeScan = () =>
  container.querySelector<HTMLButtonElement>("button.welcome-scan")!;
const scanAllButton = () =>
  container.querySelector<HTMLButtonElement>("button.welcome-scan-all")!;
const diskRow = (name: string) =>
  [...container.querySelectorAll<HTMLButtonElement>("button.disk-nav")].find(
    (row) => row.querySelector("strong")?.textContent === name,
  )!;
const rowTag = (name: string) =>
  diskRow(name).querySelector("em")?.textContent;
const toast = () => container.querySelector(".toast")?.textContent ?? "";
async function settle() {
  await act(async () => {
    for (let i = 0; i < 8; i++) await Promise.resolve();
  });
}
async function click(button: HTMLButtonElement) {
  await act(() => button.click());
  await settle();
}
beforeEach(async () => {
  edition = lite;
  allowed = [];
  allowing = new Map();
  allowFails = null;
  refused = new Set();
  mounted = [hd.path, backup.path];
  picked = null;
  allowGate = null;
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(async (command: string, args: any) => {
    switch (command) {
      case "get_edition":
        return edition;
      case "get_explorer_preferences":
        return { sort: "sizeDescending", warning: null };
      case "full_disk_access_status":
        return { status: "on", scanAnyway: false, appPath: "/Fixture.app" };
      case "discover_targets":
        return [hd, backup].filter((item) => mounted.includes(item.path));
      case "mounted_volumes":
        return mounted;
      case "allowed_target_ids":
        return [...allowed];
      case "choose_folder":
        // Picking a folder makes it an Allowed location.
        if (picked) allowed = [...allowed, picked.id];
        return picked;
      case "allow_access":
        if (allowGate) await allowGate;
        if (allowFails) throw allowFails;
        // A cancelled dialog allows nothing.
        allowed = [...allowed, ...(allowing.get(args.targetId) ?? [])];
        return [...allowed];
      case "list_cleanup_queue":
        return { entries: [], revision: 0, warning: null, blocked: false };
      case "start_scan":
        if (refused.has(args.targetId))
          throw `NEEDS_ACCESS: Allow access to ${args.targetId} before scanning it.`;
        return `scan-${args.targetId}`;
      case "scan_summary":
        return summary(args.scanId);
      case "get_node":
        return {
          id: 0,
          parentId: null,
          name: "root",
          path: "/",
          kind: "directory",
          allocatedBytes: 2048,
          childrenCount: 0,
          incomplete: false,
        };
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
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
});
async function mount() {
  await act(() => root.render(App));
  await settle();
}

it("offers Allow access… instead of Scan for a disk that is not allowed, and does not scan it", async () => {
  await mount();
  expect(welcomeScan().textContent).toContain("Allow access…");
  expect(welcomeScan().textContent).not.toMatch(/^Scan/);
  expect(rowTag("Macintosh HD")).toBe("Needs access");
  await click(welcomeScan());
  expect(calls("start_scan")).toHaveLength(0);
});

it("asks the backend to open the Open dialog at the disk, which shows Allowing access… until it answers", async () => {
  allowing.set("hd", ["hd"]);
  let open!: () => void;
  allowGate = new Promise<void>((resolve) => (open = resolve));
  await mount();
  await click(welcomeScan());
  expect(calls("allow_access")).toEqual([
    ["allow_access", { targetId: "hd" }],
  ]);
  expect(welcomeScan().textContent).toContain("Allowing access…");
  expect(welcomeScan().disabled).toBe(true);
  open();
  await settle();
  expect(welcomeScan().textContent).not.toContain("Allowing access…");
});

it("scans a disk with the normal action once it is allowed, without another dialog", async () => {
  allowing.set("hd", ["hd"]);
  await mount();
  await click(welcomeScan());
  expect(welcomeScan().textContent).toContain("Scan Macintosh HD");
  expect(rowTag("Macintosh HD")).toBeUndefined();
  await click(welcomeScan());
  expect(calls("allow_access")).toHaveLength(1);
  expect(calls("start_scan").map(([, args]) => args.targetId)).toEqual(["hd"]);
});

it("keeps Allow access… when the dialog is cancelled or picks the wrong folder", async () => {
  await mount();
  await click(welcomeScan());
  expect(welcomeScan().textContent).toContain("Allow access…");
  expect(container.querySelector(".failure-results")).toBeNull();
  allowFails = "Choose Macintosh HD in the Open dialog to allow access to it.";
  await click(welcomeScan());
  expect(container.querySelector(".failure-results h2")?.textContent).toBe(
    "Couldn’t allow access",
  );
  expect(container.querySelector(".failure-results")?.textContent).toContain(
    "Choose Macintosh HD in the Open dialog",
  );
  await act(() => app().setError(""));
  expect(welcomeScan().textContent).toContain("Allow access…");
});

it("shows an allowed disk as scannable straight away, as after a relaunch", async () => {
  allowed = ["hd"];
  await mount();
  expect(welcomeScan().textContent).toContain("Scan Macintosh HD");
  expect(rowTag("Macintosh HD")).toBeUndefined();
  expect(rowTag("Backup")).toBe("Needs access");
});

it("lists a disk as allowed again once it is reconnected, without a dialog", async () => {
  allowed = ["backup"];
  await mount();
  expect(rowTag("Backup")).toBeUndefined();
  // Ejected: its bookmark cannot resolve until it is back.
  mounted = [hd.path];
  allowed = [];
  await act(() => window.dispatchEvent(new Event("focus")));
  await settle();
  expect(diskRow("Backup")).toBeUndefined();
  mounted = [hd.path, backup.path];
  allowed = ["backup"];
  await act(() => window.dispatchEvent(new Event("focus")));
  await settle();
  expect(rowTag("Backup")).toBeUndefined();
  await click(diskRow("Backup"));
  expect(welcomeScan().textContent).toContain("Scan Backup");
  expect(calls("allow_access")).toHaveLength(0);
});

it("Scan all disks scans only allowed disks and says how many it skipped, never opening a dialog", async () => {
  allowed = ["hd"];
  await mount();
  await click(scanAllButton());
  expect(calls("start_scan").map(([, args]) => args.targetId)).toEqual(["hd"]);
  expect(calls("allow_access")).toHaveLength(0);
  expect(toast()).toContain("1 disk skipped");
  expect(container.querySelector(".failure-results")).toBeNull();
});

it("Scan all disks with nothing allowed starts no scan and counts every disk", async () => {
  await mount();
  await click(scanAllButton());
  expect(calls("start_scan")).toHaveLength(0);
  expect(calls("allow_access")).toHaveLength(0);
  expect(toast()).toContain("2 disks skipped");
});

it("Scan all disks counts a disk the backend refuses for lack of access as skipped, not as a failure", async () => {
  // The list says allowed; the bookmark stopped working since.
  allowed = ["hd", "backup"];
  refused.add("backup");
  await mount();
  await click(scanAllButton());
  expect(calls("start_scan").map(([, args]) => args.targetId).sort()).toEqual([
    "backup",
    "hd",
  ]);
  expect(toast()).toContain("1 disk skipped");
  expect(container.querySelector(".failure-results")).toBeNull();
});

it("scans a folder picked with Scan folder without asking for access again", async () => {
  picked = {
    id: "photos",
    name: "Photos",
    path: "/Users/ada/Photos",
    totalBytes: null,
    freeBytes: null,
  };
  await mount();
  await click(
    container.querySelector<HTMLButtonElement>("button.scan-folder-nav")!,
  );
  expect(calls("allow_access")).toHaveLength(0);
  expect(calls("start_scan").map(([, args]) => args.targetId)).toEqual([
    "photos",
  ]);
});

it("leaves the Full edition's Disks list and Scan all disks as they were", async () => {
  edition = fullEdition;
  await mount();
  expect(welcomeScan().textContent).toContain("Scan Macintosh HD");
  expect(rowTag("Macintosh HD")).toBeUndefined();
  await click(scanAllButton());
  expect(calls("start_scan").map(([, args]) => args.targetId).sort()).toEqual([
    "backup",
    "hd",
  ]);
  expect(calls("allowed_target_ids")).toHaveLength(0);
  expect(toast()).not.toContain("skipped");
});

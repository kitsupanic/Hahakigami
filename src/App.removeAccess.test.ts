// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, createTestRoot, fullEdition, type Root } from "./testUtils";
import App from "./App.svelte";
import type createApp from "./createApp.svelte";
import type { QueueEntry, QueueView } from "./cleanupTypes";
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
const summary = (
  scanId: string,
  state: ScanSummary["state"] = "completed",
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
const queued = (id: string, path: string): QueueEntry => ({
  id,
  name: path.split("/").pop()!,
  path,
  rootPath: "/",
  allocatedBytes: 10,
  status: "ready",
  reason: null,
  matchedScanId: "scan-hd",
  onScanVolume: true,
});
// What the backend says is allowed, and how it answers.
let edition: Edition;
let allowed: string[];
let removeFails: string | null;
let scanState: ScanSummary["state"];
let refuseScans: boolean;
let queue: QueueView;
let container: HTMLDivElement, root: Root;
const app = () => mock.app!;
const calls = (command: string) =>
  mock.invoke.mock.calls.filter(([name]) => name === command);
const welcomeScan = () =>
  container.querySelector<HTMLButtonElement>("button.welcome-scan")!;
const removeButton = () =>
  container.querySelector<HTMLButtonElement>("button.remove-access");
const rescanButton = () =>
  container.querySelector<HTMLButtonElement>("button.rescan-action")!;
const diskRow = (name: string) =>
  [...container.querySelectorAll<HTMLButtonElement>("button.disk-nav")].find(
    (row) => row.querySelector("strong")?.textContent === name,
  )!;
const rowTag = (name: string) =>
  diskRow(name).querySelector("em")?.textContent;
async function settle() {
  await act(async () => {
    for (let i = 0; i < 8; i++) await Promise.resolve();
  });
}
async function click(button: HTMLButtonElement) {
  await act(() => button.click());
  await settle();
}
async function mount() {
  await act(() => root.render(App));
  await settle();
}
// The disk is scanned and its results are on screen.
async function scanHd() {
  await click(welcomeScan());
  expect(app().summary?.scanId).toBe("scan-hd");
}
beforeEach(() => {
  edition = lite;
  allowed = ["hd", "backup"];
  removeFails = null;
  scanState = "completed";
  refuseScans = false;
  queue = { entries: [], revision: 0, warning: null, blocked: false };
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
        return [hd, backup];
      case "mounted_volumes":
        return [hd.path, backup.path];
      case "allowed_target_ids":
        return [...allowed];
      case "remove_access":
        if (removeFails) throw removeFails;
        allowed = allowed.filter((id) => id !== args.targetId);
        return [...allowed];
      case "allow_access":
        return [...allowed];
      case "list_cleanup_queue":
        return queue;
      case "start_scan":
        if (refuseScans)
          throw "NEEDS_ACCESS: Allow access to / before scanning it.";
        return `scan-${args.targetId}`;
      case "scan_summary":
        return summary(args.scanId, scanState);
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

it("offers Remove access for an allowed place with no scan yet, and asks for access again afterwards", async () => {
  await mount();
  expect(welcomeScan().textContent).toContain("Scan Macintosh HD");
  await click(removeButton()!);
  expect(calls("remove_access")).toEqual([
    ["remove_access", { targetId: "hd" }],
  ]);
  expect(welcomeScan().textContent).toContain("Allow access…");
  expect(rowTag("Macintosh HD")).toBe("Needs access");
  expect(rowTag("Backup")).toBeUndefined();
  expect(removeButton()).toBeNull();
  expect(container.querySelector(".failure-results")).toBeNull();
});

it("offers no Remove access for a place that is not allowed", async () => {
  allowed = ["backup"];
  await mount();
  expect(welcomeScan().textContent).toContain("Allow access…");
  expect(removeButton()).toBeNull();
});

it("never offers Remove access in the Full edition, which allows everything", async () => {
  edition = fullEdition;
  await mount();
  expect(removeButton()).toBeNull();
  expect(calls("allowed_target_ids")).toHaveLength(0);
});

it("keeps the scan and its results on screen when access is removed, and the next scan action is Allow access…", async () => {
  await mount();
  await scanHd();
  expect(rowTag("Macintosh HD")).toBe("Scanned");
  await click(removeButton()!);
  expect(calls("remove_access")).toHaveLength(1);
  // The scan was not discarded, dropped or replaced.
  expect(app().summary?.scanId).toBe("scan-hd");
  expect(app().scanSlots["/"]?.scanId).toBe("scan-hd");
  expect(calls("discard_scan")).toHaveLength(0);
  expect(calls("start_scan")).toHaveLength(1);
  expect(container.querySelector(".window-toolbar")).not.toBeNull();
  expect(rescanButton().textContent).toContain("Allow access…");
  expect(rowTag("Macintosh HD")).toBe("Needs access");
  expect(diskRow("Macintosh HD").getAttribute("aria-label")).toContain(
    "Scanned, needs access",
  );
  // Rescanning asks for access instead of scanning.
  await click(rescanButton());
  expect(calls("allow_access")).toEqual([
    ["allow_access", { targetId: "hd" }],
  ]);
  expect(calls("start_scan")).toHaveLength(1);
});

it("leaves other disks as they were when one disk's access is removed", async () => {
  await mount();
  await scanHd();
  await click(removeButton()!);
  expect(rowTag("Backup")).toBeUndefined();
  await click(diskRow("Backup"));
  expect(welcomeScan().textContent).toContain("Scan Backup");
});

it("does not offer Remove access while that place is scanning", async () => {
  scanState = "running";
  await mount();
  await click(welcomeScan());
  expect(app().summary?.state).toBe("running");
  expect(removeButton()?.disabled ?? true).toBe(true);
});

it("reports a failed removal in a dialog and leaves the place allowed", async () => {
  removeFails = "Could not save.";
  await mount();
  await click(removeButton()!);
  expect(container.querySelector(".failure-results h2")?.textContent).toBe(
    "Couldn’t remove access",
  );
  await act(() => app().setError(""));
  expect(welcomeScan().textContent).toContain("Scan Macintosh HD");
});

it("quietly shows a scanned disk whose access was lost as needing access, with no dialog", async () => {
  await mount();
  await scanHd();
  // Its bookmark stopped resolving: the backend refuses the rescan and no longer lists the disk.
  allowed = ["backup"];
  refuseScans = true;
  await click(rescanButton());
  expect(container.querySelector(".failure-results")).toBeNull();
  expect(container.querySelector(".toast")?.textContent).toContain(
    "Macintosh HD",
  );
  expect(app().summary?.scanId).toBe("scan-hd");
  expect(rescanButton().textContent).toContain("Allow access…");
  expect(rowTag("Macintosh HD")).toBe("Needs access");
  expect(removeButton()).toBeNull();
});

it("quietly offers Allow access… when a scan is refused for access the view still thought it had", async () => {
  await mount();
  refuseScans = true;
  const lost = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command: string, args: any) => {
    if (command === "allowed_target_ids") return ["backup"];
    return lost(command, args);
  });
  await click(welcomeScan());
  expect(container.querySelector(".failure-results")).toBeNull();
  expect(container.querySelector(".toast")?.textContent).toContain(
    "Macintosh HD",
  );
  expect(welcomeScan().textContent).toContain("Allow access…");
});

it("keeps every Cleanup queue entry when access is removed", async () => {
  queue = {
    entries: [queued("a", "/Users/a/big.iso"), queued("b", "/other/x")],
    revision: 1,
    warning: null,
    blocked: false,
  };
  await mount();
  await scanHd();
  await click(removeButton()!);
  const mutating = mock.invoke.mock.calls
    .map(([name]) => name)
    .filter((name) => /remove_cleanup|clear_cleanup|add_cleanup/.test(name));
  expect(mutating).toEqual([]);
  expect(app().queue.entries.map((entry) => entry.id)).toEqual(["a", "b"]);
  expect(
    container.querySelector(".queue-nav")?.getAttribute("aria-label"),
  ).toBe("Cleanup queue · 2");
});

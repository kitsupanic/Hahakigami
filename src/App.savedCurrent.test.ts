// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, createTestRoot, type Root } from "./testUtils";
import App from "./App.svelte";
import type createApp from "./createApp.svelte";
import type { NodeView, SavedEntry, ScanSummary, Target } from "./types";

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
const home: Target = {
  id: "home",
  name: "ada",
  path: "/Users/ada",
  totalBytes: null,
  freeBytes: null,
  home: true,
};
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
  totalBytes: null,
  freeBytes: null,
  volume: true,
};
const entry = (id: string, rootPath: string): SavedEntry => ({
  id,
  name: rootPath.split("/").pop()!,
  rootPath,
  startedAtMs: 1,
  completedAtMs: 2,
  savedAtMs: id === "a" ? 4 : 3,
  entries: 2,
  logicalBytes: 4096,
  allocatedBytes: 2048,
  incomplete: false,
  errors: 0,
  excluded: 0,
  fileBytes: 10,
});
let saved: SavedEntry[];
let liveId: string;
const summary = (scanId: string, savedId?: string): ScanSummary => ({
  scanId,
  origin: savedId ? "saved" : "live",
  savedId: savedId ?? null,
  rootId: 0,
  state: "completed",
  workers: 1,
  workerStats: [],
  finalizationUs: 0,
  entries: 2,
  logicalBytes: 4096,
  allocatedBytes: 2048,
  elapsedMs: 1000,
  errors: 0,
});
const node: NodeView = {
  id: 0,
  parentId: null,
  name: "ada",
  path: home.path,
  kind: "directory",
  logicalBytes: 4096,
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
const row = (id: string) =>
  container.querySelector<HTMLButtonElement>(
    `[data-saved-id="${id}"] button.saved-summary`,
  )!;
const scanButton = () =>
  container.querySelector<HTMLButtonElement>("button.welcome-scan")!;
async function settle() {
  await act(async () => {
    for (let i = 0; i < 6; i++) await Promise.resolve();
  });
}
// A disk with a scan reopens its results, so the welcome page needs another one.
const otherDiskRow = () =>
  container.querySelectorAll<HTMLButtonElement>("button.disk-nav")[1];
async function click(button: HTMLButtonElement) {
  await act(() => button.click());
  await settle();
}
beforeEach(async () => {
  saved = [entry("a", "/Users/ada/A"), entry("b", "/Users/ada/B")];
  liveId = "live";
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(async (command: string, args: any) => {
    switch (command) {
      case "get_explorer_preferences":
        return { sort: "sizeDescending", warning: null };
      case "full_disk_access_status":
        return { status: "on", scanAnyway: false, appPath: "/Fixture.app" };
      case "discover_targets":
        return [disk, backup, home];
      case "list_saved_scans":
        return { entries: saved, lastOpenedId: null };
      case "list_cleanup_queue":
        return { entries: [], revision: 0, warning: null, blocked: false };
      case "start_scan":
        return liveId;
      case "scan_summary":
        return summary(args.scanId);
      case "begin_saved_open":
        return "request";
      case "open_saved_scan":
        return summary(`saved-${args.id}`, args.id);
      case "save_scan":
        return { ...entry("c", "/Users/ada"), savedAtMs: 9 };
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
  container = document.createElement("div");
  document.body.append(container);
  root = createTestRoot(container);
  await act(() => root.render(App));
  await settle();
  await click(scanButton());
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
});

it("opens a saved scan beside the live scan and switches back without any scan or open command", async () => {
  await click(row("a"));
  expect(app().summary?.scanId).toBe("saved-a");
  expect(row("a").getAttribute("aria-current")).toBe("page");
  expect(diskRow("Macintosh HD").getAttribute("aria-current")).toBeNull();
  expect(diskRow("Macintosh HD").disabled).toBe(false);
  expect(diskRow("Macintosh HD").querySelector("em")?.textContent).toBe(
    "Scanned",
  );
  const before = mock.invoke.mock.calls.length;
  await click(diskRow("Macintosh HD"));
  expect(app().summary?.scanId).toBe("live");
  expect(app().historical).toBe(false);
  expect(diskRow("Macintosh HD").getAttribute("aria-current")).toBe("page");
  expect(row("a").getAttribute("aria-current")).toBeNull();
  for (const [name] of mock.invoke.mock.calls.slice(before))
    expect([
      "start_scan",
      "rescan_saved_scan",
      "begin_saved_open",
      "open_saved_scan",
    ]).not.toContain(name);
});

it("replaces only the open saved scan when another one opens", async () => {
  await click(row("a"));
  await click(row("b"));
  expect(app().summary?.scanId).toBe("saved-b");
  expect(row("b").getAttribute("aria-current")).toBe("page");
  expect(row("a").getAttribute("aria-current")).toBeNull();
  await click(diskRow("Macintosh HD"));
  expect(app().summary?.scanId).toBe("live");
  expect(calls("open_saved_scan")).toHaveLength(2);
  expect(calls("start_scan")).toHaveLength(1);
});

it("keeps a saved live scan shown as the live scan", async () => {
  await act(() => app().saveScan());
  await settle();
  expect(calls("save_scan")).toHaveLength(1);
  expect(app().summary?.scanId).toBe("live");
  expect(app().historical).toBe(false);
  expect(diskRow("Macintosh HD").getAttribute("aria-current")).toBe("page");
  expect(row("c").getAttribute("aria-current")).toBeNull();
  await click(row("a"));
  await click(diskRow("Macintosh HD"));
  expect(app().summary?.scanId).toBe("live");
  expect(app().summary?.savedId).toBe("c");
});

it("adds another disk's scan when it starts while a saved scan is shown, keeping the first", async () => {
  await click(row("a"));
  liveId = "live-2";
  await click(otherDiskRow());
  await click(scanButton());
  expect(calls("start_scan")).toHaveLength(2);
  expect(app().summary?.scanId).toBe("live-2");
  expect(diskRow("Backup").getAttribute("aria-current")).toBe("page");
  await click(diskRow("Macintosh HD"));
  expect(app().summary?.scanId).toBe("live");
  await click(diskRow("Backup"));
  expect(app().summary?.scanId).toBe("live-2");
  expect(calls("start_scan")).toHaveLength(2);
});

it("switches back to the open saved scan without reopening it", async () => {
  await click(row("a"));
  await click(diskRow("Macintosh HD"));
  const before = mock.invoke.mock.calls.length;
  await click(row("a"));
  expect(app().summary?.scanId).toBe("saved-a");
  expect(row("a").getAttribute("aria-current")).toBe("page");
  expect(diskRow("Macintosh HD").getAttribute("aria-current")).toBeNull();
  for (const [name] of mock.invoke.mock.calls.slice(before))
    expect(["begin_saved_open", "open_saved_scan"]).not.toContain(name);
  await click(diskRow("Macintosh HD"));
  expect(app().summary?.scanId).toBe("live");
});

it("keeps the open saved scan instant after another disk's scan starts", async () => {
  await click(row("a"));
  liveId = "live-2";
  await click(otherDiskRow());
  await click(scanButton());
  await click(row("a"));
  expect(app().summary?.scanId).toBe("saved-a");
  expect(calls("open_saved_scan")).toHaveLength(1);
  await click(row("b"));
  await click(diskRow("Macintosh HD"));
  await click(row("a"));
  expect(calls("open_saved_scan")).toHaveLength(3);
});

it("closes a deleted saved scan that is shown and returns to the live scan", async () => {
  await click(row("a"));
  await act(() => app().setRemoveEntry(saved[0]));
  await act(() => app().removeSaved());
  await settle();
  expect(calls("delete_saved_scan")).toEqual([
    ["delete_saved_scan", { id: "a" }],
  ]);
  expect(app().summary?.scanId).toBe("live");
  expect(app().historical).toBe(false);
  expect(diskRow("Macintosh HD").getAttribute("aria-current")).toBe("page");
});

it("shows the welcome page when the deleted saved scan had no live scan behind it", async () => {
  await act(() => root.unmount());
  const base = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command: string, args: any) =>
    command === "list_saved_scans"
      ? { entries: saved, lastOpenedId: "a" }
      : base(command, args),
  );
  root = createTestRoot(container);
  await act(() => root.render(App));
  await settle();
  expect(app().summary?.scanId).toBe("saved-a");
  expect(app().scanSlots).toEqual({});
  await act(() => app().setRemoveEntry(saved[0]));
  await act(() => app().removeSaved());
  await settle();
  expect(app().summary).toBeNull();
  expect(app().historical).toBe(false);
  expect(container.querySelector("#welcome-title")).not.toBeNull();
});

it("does not offer the cleaned live scan for comparison after a cleanup moved files", async () => {
  const file: NodeView = {
    ...node,
    id: 1,
    parentId: 0,
    name: "f",
    path: "/Users/ada/f",
    kind: "file",
    childrenCount: 0,
  };
  const base = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command: string, args: any) => {
    switch (command) {
      case "prepare_trash":
        return { token: "t", items: [file] };
      case "commit_trash":
        return {
          trashed: 1,
          failures: [],
          persistenceWarning: null,
          outcomes: [{ reviewIndex: 0, status: "trashed" }],
        };
      case "start_scan":
        // The rescan after cleanup fails, so no new live scan replaces the stale one.
        throw "Choose a directory.";
      default:
        return base(command, args);
    }
  });
  expect(app().comparableCurrent?.id).toBe("live");
  await act(() => app().select(file));
  await act(() => app().prepareTrash());
  await settle();
  await act(() => app().commitTrash());
  await settle();
  expect(calls("commit_trash")).toHaveLength(1);
  expect(app().comparableCurrent).toBeNull();
  // The cleaned scan's slot is gone, so its disk is no longer tagged.
  expect(app().scanSlots).toEqual({});
  expect(diskRow("Macintosh HD").querySelector("em")).toBeNull();
});

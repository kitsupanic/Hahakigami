// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, createTestRoot, type Root } from "./testUtils";
import App from "./App.svelte";
import { menuItem } from "./fakeMenu";
import type createApp from "./createApp.svelte";
import type { NodeView, ScanSummary, Target } from "./types";

const mock = vi.hoisted(() => ({
  invoke: vi.fn(),
  app: null as ReturnType<typeof createApp> | null,
  channels: [] as { onmessage: (s: ScanSummary) => void }[],
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
    constructor() {
      mock.channels.push(this);
    }
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
  totalBytes: null,
  freeBytes: null,
  volume: true,
};
let discovered: Target[];
let mounted: string[];
const home: Target = {
  id: "home",
  name: "ada",
  path: "/Users/ada",
  totalBytes: null,
  freeBytes: null,
  home: true,
};
let state: ScanSummary["state"];
const summary = (): ScanSummary => ({
  scanId: "live",
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
  name: "ada",
  path: home.path,
  kind: "directory",
  allocatedBytes: 2048,
  childrenCount: 0,
  incomplete: false,
};
let container: HTMLDivElement, root: Root;
const diskRow = (name: string) =>
  [...container.querySelectorAll<HTMLButtonElement>("button.disk-nav")].find(
    (item) => item.getAttribute("aria-label")?.startsWith(`${name},`),
  )!;
const diskNames = () =>
  [...container.querySelectorAll("button.disk-nav strong")].map(
    (item) => item.textContent,
  );
const refresh = async () => {
  await act(() => window.dispatchEvent(new Event("focus")));
  await settle();
};
const app = () => mock.app!;
const calls = (command: string) =>
  mock.invoke.mock.calls.filter(([name]) => name === command);
const nav = (name: string) =>
  [...container.querySelectorAll<HTMLButtonElement>("button.side-nav")].find(
    (item) => item.textContent?.includes(name),
  )!;
const welcome = () => container.querySelector("#welcome-title");
const scanButton = () =>
  container.querySelector<HTMLButtonElement>("button.welcome-scan")!;
async function settle() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}
async function click(button: HTMLButtonElement) {
  await act(() => button.click());
  await settle();
}
beforeEach(async () => {
  state = "completed";
  discovered = [disk, backup, home];
  mounted = ["/", "/Volumes/Backup"];
  mock.invoke.mockReset();
  mock.channels.length = 0;
  mock.invoke.mockImplementation(async (command: string) => {
    switch (command) {
      case "get_explorer_preferences":
        return { sort: "sizeDescending", warning: null };
      case "full_disk_access_status":
        return { status: "on", scanAnyway: false, appPath: "/Fixture.app" };
      case "discover_targets":
        return discovered;
      case "mounted_volumes":
        return mounted;
      case "list_cleanup_queue":
        return { entries: [], revision: 0, warning: null, blocked: false };
      case "start_scan":
        return "live";
      case "scan_summary":
        return summary();
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
});
afterEach(async () => {
  vi.useRealTimers();
  await act(() => root.unmount());
  container.remove();
});

it("selects the startup disk, named after itself, by default and highlights it", async () => {
  expect(app().target).toEqual(disk);
  expect(scanButton().textContent).toContain("Scan Macintosh HD");
  expect(welcome()).not.toBeNull();
  expect(diskRow("Macintosh HD").getAttribute("aria-current")).toBe("page");
  expect(diskRow("Backup").getAttribute("aria-current")).toBeNull();
});

it("has no Current scan row, and no disk is tagged before the first scan", async () => {
  expect(welcome()).not.toBeNull();
  expect(nav("Current scan")).toBeUndefined();
  expect(container.querySelector("button.disk-nav em")).toBeNull();
  expect(container.querySelector("button.home-nav")).toBeNull();
  expect(container.textContent).not.toContain("Locations");
  expect(container.textContent).not.toContain("Choose a folder");
});

it("shows the welcome page from a disk while results are loaded, then returns without any new command", async () => {
  await click(scanButton());
  expect(welcome()).toBeNull();
  expect(diskRow("Macintosh HD").getAttribute("aria-current")).toBe("page");
  expect(diskRow("Macintosh HD").querySelector("em")?.textContent).toBe(
    "Scanned",
  );
  const summaryBefore = app().summary;
  const before = mock.invoke.mock.calls.length;
  await click(diskRow("Backup"));
  expect(welcome()).not.toBeNull();
  expect(diskRow("Backup").getAttribute("aria-current")).toBe("page");
  expect(diskRow("Macintosh HD").getAttribute("aria-current")).toBeNull();
  expect(app().summary).toBe(summaryBefore);
  expect(scanButton().disabled).toBe(false);
  await click(diskRow("Macintosh HD"));
  expect(welcome()).toBeNull();
  expect(diskRow("Macintosh HD").classList.contains("active")).toBe(true);
  expect(app().summary).toBe(summaryBefore);
  expect(calls("start_scan")).toHaveLength(1);
  for (const [name] of mock.invoke.mock.calls.slice(before))
    expect(name).not.toBe("start_scan");
});

it("keeps disks available during a scan, with only the scanning disk's Scan unavailable and the disk tagged Scanning", async () => {
  state = "running";
  await click(scanButton());
  expect(app().running).toBe(true);
  expect(diskRow("Macintosh HD").querySelector("em")?.textContent).toBe(
    "Scanning",
  );
  expect(diskRow("Backup").disabled).toBe(false);
  await click(diskRow("Backup"));
  expect(welcome()).not.toBeNull();
  // Another disk can be scanned beside the running one.
  expect(scanButton().disabled).toBe(false);
  expect(calls("start_scan")).toHaveLength(1);
  await click(diskRow("Macintosh HD"));
  expect(welcome()).toBeNull();
  expect(app().running).toBe(true);
});

it("stops a running scan from the menu while the welcome page is shown", async () => {
  state = "running";
  await click(scanButton());
  await click(diskRow("Backup"));
  expect(menuItem("stopScan").enabled).toBe(true);
  menuItem("stopScan").click();
  await settle();
  expect(calls("cancel_scan")).toHaveLength(1);
});

it("names the scan in the cancelled toast while the welcome page is shown", async () => {
  state = "running";
  await click(scanButton());
  await click(diskRow("Backup"));
  state = "cancelled";
  await act(() => mock.channels.at(-1)!.onmessage(summary()));
  const toast = container.querySelector(".toast")?.textContent;
  expect(toast).toContain("Scan of Macintosh HD cancelled.");
  expect(toast).not.toContain("Partial results");
});

it("does not navigate the hidden explorer from the welcome page", async () => {
  await click(scanButton());
  await click(diskRow("Backup"));
  expect(app().can.goBack).toBe(false);
  expect(app().can.enclosing).toBe(false);
});

it("leaves the welcome page when the next scan starts from it", async () => {
  await click(scanButton());
  await click(diskRow("Backup"));
  await click(scanButton());
  expect(calls("start_scan")).toHaveLength(2);
  expect(welcome()).toBeNull();
  expect(app().homeOpen).toBe(false);
});

it("lists only volume targets under Disks, with free space and a usage bar", async () => {
  expect(container.querySelector(".side-label")?.textContent?.trim()).toBe("Disks");
  expect(diskNames()).toEqual(["Macintosh HD", "Backup"]);
  expect(diskRow("Macintosh HD").textContent).toContain(
    "500 B free of 1,000 B",
  );
  expect(
    diskRow("Macintosh HD").querySelector<HTMLElement>(".disk-track > span")
      ?.style.width,
  ).toBe("50%");
  expect(diskRow("Backup").textContent).toContain("Size unknown");
  expect(diskRow("Backup").querySelector(".disk-track")?.classList).toContain(
    "unknown",
  );
});

it("selects a clicked disk and shows the welcome page ready to scan it", async () => {
  await click(diskRow("Backup"));
  expect(welcome()).not.toBeNull();
  expect(app().target).toEqual(backup);
  expect(diskRow("Backup").getAttribute("aria-current")).toBe("page");
  expect(diskRow("Macintosh HD").getAttribute("aria-current")).toBeNull();
  expect(scanButton().textContent).toContain("Scan Backup");
});

it("marks a scanned disk Scanned and reopens its scan without rescanning", async () => {
  await click(diskRow("Macintosh HD"));
  await click(scanButton());
  expect(calls("start_scan")).toHaveLength(1);
  expect(diskRow("Macintosh HD").textContent).toContain("Scanned");
  expect(diskRow("Macintosh HD").getAttribute("aria-label")).toContain(
    "Scanned",
  );
  expect(diskRow("Macintosh HD").querySelector("svg")).not.toBeNull();
  expect(diskRow("Backup").textContent).not.toContain("Scanned");
  await click(diskRow("Backup"));
  expect(welcome()).not.toBeNull();
  expect(diskRow("Macintosh HD").textContent).toContain("Scanned");
  const summaryBefore = app().summary;
  await click(diskRow("Macintosh HD"));
  expect(welcome()).toBeNull();
  expect(app().summary).toBe(summaryBefore);
  expect(calls("start_scan")).toHaveLength(1);
  // Selection follows the results on screen, so the toolbar rescans them.
  expect(app().target?.path).toBe("/");
  expect(app().liveTarget).toBe(app().target);
});

it("says Scanning on the disk while its scan runs", async () => {
  state = "running";
  await click(diskRow("Macintosh HD"));
  await click(scanButton());
  expect(diskRow("Macintosh HD").textContent).toContain("Scanning");
  expect(diskRow("Macintosh HD").textContent).not.toContain("Scanned");
});

it("adds a connected disk and drops an ejected one, keeping the others' ids", async () => {
  const usb: Target = {
    id: "usb-new",
    name: "USB",
    path: "/Volumes/USB",
    totalBytes: 10,
    freeBytes: 5,
    volume: true,
  };
  mounted = ["/", "/Volumes/Backup", "/Volumes/USB"];
  discovered = [
    { ...disk, id: "disk-2" },
    { ...backup, id: "backup-2" },
    usb,
    { ...home, id: "home-2" },
  ];
  await refresh();
  expect(diskNames()).toEqual(["Macintosh HD", "Backup", "USB"]);
  expect(app().targets.map((t) => t.id)).toEqual([
    "disk",
    "backup",
    "home",
    "usb-new",
  ]);
  expect(app().target).toEqual(disk);
  mounted = ["/", "/Volumes/USB"];
  discovered = [disk, usb, home];
  await refresh();
  expect(diskNames()).toEqual(["Macintosh HD", "USB"]);
  expect(app().targets.map((t) => t.id)).toEqual(["disk", "home", "usb-new"]);
});

it("brings a remounted disk back when a chosen folder shares its path", async () => {
  const folder: Target = { ...backup, id: "backup-folder", volume: false };
  await act(() => app().setTargets((old) => [...old, folder]));
  mounted = ["/"];
  discovered = [disk, home];
  await refresh();
  expect(diskNames()).toEqual(["Macintosh HD"]);
  mounted = ["/", "/Volumes/Backup"];
  discovered = [disk, { ...backup, id: "backup-2" }, home];
  await refresh();
  expect(diskNames()).toEqual(["Macintosh HD", "Backup"]);
  const after = calls("discover_targets").length;
  await refresh();
  expect(calls("discover_targets")).toHaveLength(after);
});

it("does not rediscover while the volume paths are unchanged", async () => {
  const before = calls("discover_targets").length;
  await refresh();
  expect(calls("mounted_volumes")).toHaveLength(1);
  expect(calls("discover_targets")).toHaveLength(before);
});

it("falls back to the startup disk when the selected disk is ejected, keeping the other disks' scans", async () => {
  await click(diskRow("Macintosh HD"));
  await click(scanButton());
  await click(diskRow("Backup"));
  expect(app().target).toEqual(backup);
  const scan = app().scanSlots["/"];
  mounted = ["/"];
  discovered = [disk, home];
  await refresh();
  expect(diskNames()).toEqual(["Macintosh HD"]);
  expect(app().target).toEqual(disk);
  expect(app().scanSlots["/"]).toBe(scan);
  expect(diskRow("Macintosh HD").textContent).toContain("Scanned");
  // Nothing was scanned on the ejected disk, so there is nothing to discard.
  expect(calls("discard_scan")).toHaveLength(0);
});

it("keeps a failed refresh quiet and tries again on the next one", async () => {
  mounted = ["/"];
  discovered = [disk, home];
  mock.invoke.mockImplementationOnce(async () => {
    throw new Error("busy");
  });
  await refresh();
  expect(diskNames()).toEqual(["Macintosh HD", "Backup"]);
  expect(container.querySelector("[role=alert]")).toBeNull();
  await refresh();
  expect(diskNames()).toEqual(["Macintosh HD"]);
});

it("polls every 3 seconds while visible and stops on unmount", async () => {
  await act(() => root.unmount());
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
  root = createTestRoot(container);
  await act(() => root.render(App));
  await settle();
  const polls = () => calls("mounted_volumes").length;
  const before = polls();
  await act(() => vi.advanceTimersByTime(3000));
  await settle();
  expect(polls()).toBe(before + 1);
  const visibility = vi
    .spyOn(document, "visibilityState", "get")
    .mockReturnValue("hidden");
  await act(() => vi.advanceTimersByTime(3000));
  await settle();
  expect(polls()).toBe(before + 1);
  visibility.mockRestore();
  await act(() => root.unmount());
  await act(() => vi.advanceTimersByTime(9000));
  expect(polls()).toBe(before + 1);
});

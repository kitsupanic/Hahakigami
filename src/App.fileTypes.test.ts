// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, createTestRoot, type Root } from "./testUtils";
import App from "./App.svelte";
import type createApp from "./createApp.svelte";
import type { NodeView, ScanSummary } from "./types";
import type { FileTypeStats } from "./fileTypes";

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
    onmessage = () => {};
  },
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {} }));
vi.mock("./DiskMap.svelte", async () => ({
  default: (await import("./TestFileTypeMap.svelte")).default,
}));
const folder: NodeView = {
  id: 1,
  parentId: 0,
  name: "Photos",
  path: "/fixture/Photos",
  kind: "directory",
  allocatedBytes: 20,
  childrenCount: 1,
  incomplete: false,
};
const scanRoot: NodeView = {
  ...folder,
  id: 0,
  parentId: null,
  name: "Fixture",
  path: "/fixture",
  childrenCount: 2,
};
const image: NodeView = {
  ...folder,
  id: 2,
  parentId: 1,
  name: "photo.JPG",
  path: "/fixture/Photos/photo.JPG",
  kind: "file",
  childrenCount: 0,
};
const archive: NodeView = {
  ...image,
  id: 3,
  parentId: 0,
  name: "backup.zip",
  path: "/fixture/backup.zip",
  allocatedBytes: 10,
};
const summary = (scanId = "live"): ScanSummary => ({
  scanId,
  rootId: 0,
  state: "completed",
  workers: 1,
  workerStats: [],
  finalizationUs: 0,
  entries: 4,
  allocatedBytes: 30,
  elapsedMs: 1000,
  errors: 1,
  incomplete: true,
});
const stats = (scope = 0): FileTypeStats => ({
  items: [
    { extension: "jpg", files: 1, allocatedBytes: 20 },
    ...(scope === 0
      ? [{ extension: "zip", files: 1, allocatedBytes: 10 }]
      : []),
  ],
  files: scope === 0 ? 2 : 1,
  allocatedBytes: scope === 0 ? 30 : 20,
  incomplete: true,
});
const deferred = <T>() => {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
};
let element: HTMLDivElement, root: Root;
// The id the next start_scan answers with.
let nextScan: string;
async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}
const app = () => mock.app!;
const panel = () => element.querySelector<HTMLElement>("#file-types-panel")!;
const map = () => element.querySelector<HTMLElement>(".test-file-type-map")!;
async function click(selector: string) {
  const button = element.querySelector<HTMLButtonElement>(selector)!;
  expect(button).toBeTruthy();
  await act(() => button.click());
  await flush();
}
async function openTypes() {
  await click("#file-types-tab");
}
async function selectType(extension: string) {
  const row = [
    ...panel().querySelectorAll<HTMLButtonElement>(".file-type-row"),
  ].find(
    (row) =>
      row.querySelector(".file-type-name strong")?.textContent === extension,
  )!;
  expect(row).toBeTruthy();
  await act(() => row.click());
  await flush();
}
beforeEach(async () => {
  vi.useFakeTimers();
  nextScan = "live";
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(async (command, args = {}) => {
    switch (command) {
      case "get_explorer_preferences":
        return { sort: "sizeDescending", warning: null };
      case "full_disk_access_status":
        return {
            status: "on",
            scanAnyway: false,
            appPath: "/Fixture.app",
        };
      case "discover_targets":
        return [
          {
            id: "fixture",
            name: "Fixture",
            path: "/fixture",
            totalBytes: null,
            freeBytes: null,
          },
        ];
      case "start_scan":
        return nextScan;
      case "scan_summary":
        return summary(args.scanId);
      case "get_node":
        return [scanRoot, folder, image, archive][args.nodeId] ?? scanRoot;
      case "get_children":
        return {
          items: args.nodeId === 1 ? [image] : [folder, archive],
          total: args.nodeId === 1 ? 1 : 2,
        };
      case "get_map":
        return {
          nodes:
            args.colorMode === "fileType"
              ? [
                  scanRoot,
                  folder,
                  {
                    ...image,
                    id: 0x80000000,
                    kind: "aggregate",
                    path: "",
                    name: ".jpg files",
                    fileType: "jpg",
                    fileCount: 1,
                  },
                ]
              : args.nodeId === 1
                ? [folder, image]
                : [scanRoot, folder, image, archive],
        };
      case "get_file_types":
        return stats(args.nodeId);
      case "get_file_type_highlights":
        return args.extension === "jpg"
          ? args.nodeIds.filter((id: number) => [0, 1, 2].includes(id))
          : [0, 3];
      case "filter_nodes":
        return {
          items: args.extension === "jpg" ? [image] : [archive],
          total: 1,
        };
      case "list_cleanup_queue":
        return { entries: [], revision: 0, blocked: false, warning: null };
      case "close_quick_look":
        return { open: false, scanId: null, nodeId: null, revision: 0 };
      default:
        return undefined;
    }
  });
  element = document.createElement("div");
  document.body.append(element);
  root = createTestRoot(element);
  await act(() => root.render(App));
  await flush();
  await click(".welcome-scan");
});
afterEach(async () => {
  await act(() => root.unmount());
  element.remove();
  vi.useRealTimers();
});

it("loads real scoped statistics on demand, links highlighting, and keeps file selection independent across tabs", async () => {
  expect(
    mock.invoke.mock.calls.some(([name]) => name === "get_file_types"),
  ).toBe(false);
  await act(() => app().select(archive));
  await flush();
  await openTypes();
  expect(panel().hidden).toBe(false);
  expect(mock.invoke).toHaveBeenCalledWith("get_file_types", {
    scanId: "live",
    nodeId: 0,
  });
  expect(panel().textContent).toContain("66.7%");
  expect(panel().textContent).toContain("Limited coverage");
  await selectType(".jpg");
  expect(map().dataset.type).toBe("jpg");
  // Opening File types colours the map by type, so only its non-aggregate tiles are queried.
  expect(app().mapColorMode).toBe("fileType");
  expect(map().dataset.ids).toBe("0,1");
  expect(app().selection.map((node) => node.id)).toEqual([3]);
  // The details strip stays beside the list; no tab switch is needed to see it.
  expect(panel().hidden).toBe(false);
  expect(element.querySelector("#selection-inspector")?.textContent).toContain(
    "backup.zip",
  );
  expect(map().dataset.type).toBe("jpg");
  await click('[aria-label="Clear file type highlight"]');
  expect(map().dataset.type).toBe("none");
  expect(map().dataset.ids).toBe("");
  expect(app().selection.map((node) => node.id)).toEqual([3]);
});

it("sorts by on-disk size, searches descriptions, and opens exact scoped matching-file results", async () => {
  await openTypes();
  await selectType(".jpg");
  expect(panel().querySelector(".file-type-name strong")?.textContent).toBe(
    ".jpg",
  );
  const search = panel().querySelector<HTMLInputElement>("input")!;
  await act(() => {
    search.value = "JPEG";
    search.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(panel().querySelectorAll(".file-type-row")).toHaveLength(1);
  await click(".file-type-show-files");
  expect(panel().hidden).toBe(true);
  expect(
    element.querySelector("#files-tab")?.getAttribute("aria-selected"),
  ).toBe("true");
  expect(map().dataset.type).toBe("jpg");
  expect(app().filters).toMatchObject({ extension: "jpg", scopeId: 0 });
  expect(mock.invoke).toHaveBeenCalledWith(
    "filter_nodes",
    expect.objectContaining({
      extension: "jpg",
      scopeId: 0,
    }),
  );
  expect(element.querySelector(".active-filters")?.textContent).toContain(
    ".jpg",
  );
  await click('[aria-label="Remove file type filter: .jpg"]');
  expect(app().filtered).toBe(false);
});

it("ignores stale statistics and highlight replies when scope or selected extension changes", async () => {
  const slowStats = deferred<FileTypeStats>(),
    slowHighlights = deferred<number[]>();
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "get_file_types" && args.nodeId === 0
      ? slowStats.promise
      : command === "get_file_type_highlights" && args.extension === "jpg"
        ? slowHighlights.promise
        : original(command, args),
  );
  await openTypes();
  await act(() => app().setFocus(folder));
  await flush();
  const scopeButtons = panel().querySelectorAll<HTMLButtonElement>(
    ".file-type-scope button",
  );
  await act(() => scopeButtons[1].click());
  await flush();
  expect(panel().textContent).toContain("Loading file types");
  expect(
    mock.invoke.mock.calls.filter(([name]) => name === "get_file_types"),
  ).toHaveLength(1);
  await act(() => slowStats.resolve(stats(0)));
  await flush();
  expect(panel().textContent).toContain("1 type");
  expect(panel().querySelectorAll(".file-type-row")).toHaveLength(1);
  await selectType(".jpg");
  expect(map().dataset.pending).toBe("true");
  await click('[aria-label="Clear file type highlight"]');
  await act(() => slowHighlights.resolve([1, 2]));
  await flush();
  expect(map().dataset.type).toBe("none");
  expect(map().dataset.ids).toBe("");
});

it("supports keyboard tabs, keeps the file-types panel open during navigation, and resets on scan replacement", async () => {
  await openTypes();
  await selectType(".jpg");
  const tab = element.querySelector<HTMLButtonElement>("#file-types-tab")!;
  await act(() =>
    tab.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Home",
        bubbles: true,
        cancelable: true,
      }),
    ),
  );
  expect(
    element.querySelector("#files-tab")?.getAttribute("aria-selected"),
  ).toBe("true");
  expect(document.activeElement?.id).toBe("files-tab");
  await click("#file-types-tab");
  await act(() =>
    element
      .querySelector<HTMLElement>('.rows [data-node-id="1"]')!
      .dispatchEvent(new MouseEvent("dblclick", { bubbles: true, detail: 2 })),
  );
  await flush();
  expect(
    element.querySelector<HTMLElement>("#selection-inspector")!.hidden,
  ).toBe(false);
  expect(panel().hidden).toBe(false);
  expect(map().dataset.type).toBe("jpg");
  nextScan = "second";
  await click(".rescan-action");
  expect(map().dataset.type).toBe("none");
  expect(map().dataset.ids).toBe("");
});

it("keeps the File types tab through a Home round trip on the same scan", async () => {
  await openTypes();
  await act(() => app().showHome());
  await flush();
  await act(() => app().showResults());
  await flush();
  expect(
    element.querySelector("#file-types-tab")?.getAttribute("aria-selected"),
  ).toBe("true");
  expect(panel().hidden).toBe(false);
});

it("labels matching-file results with their actual folder scope", async () => {
  await act(() => app().open(folder));
  await flush();
  await openTypes();
  await act(() =>
    panel()
      .querySelectorAll<HTMLButtonElement>(".file-type-scope button")[1]
      .click(),
  );
  await flush();
  await selectType(".jpg");
  await click(".file-type-show-files");
  expect(app().filters).toMatchObject({ extension: "jpg", scopeId: 1 });
  const scopeBadge = element.querySelector<HTMLElement>(".results-scope")!;
  expect(scopeBadge.textContent).toContain("Current folder · Photos");
  expect(scopeBadge.title).toBe("Current folder: /fixture/Photos");
  expect(element.querySelector(".list-heading strong")?.textContent).toContain(
    "Photos",
  );
});

it("requests type-aware map data, safely links grouped tiles to the sidebar, and restores folder data", async () => {
  // The inspector tab is the only colour control.
  expect(
    element.querySelector('[role="group"][aria-label="Map color mode"]'),
  ).toBeNull();
  const foldersTab = element.querySelector<HTMLButtonElement>("#files-tab")!;
  const typesTab = element.querySelector<HTMLButtonElement>("#file-types-tab")!;
  expect(app().mapColorMode).toBe("folder");
  await act(() => typesTab.click());
  await flush();
  expect(app().mapColorMode).toBe("fileType");
  expect(mock.invoke).toHaveBeenCalledWith(
    "get_map",
    expect.objectContaining({
      colorMode: "fileType",
      scanId: "live",
      nodeId: 0,
    }),
  );
  expect(app().map.some((node) => node.fileType === "jpg")).toBe(true);
  await click(".test-file-type-map button");
  expect(panel().hidden).toBe(false);
  expect(map().dataset.type).toBe("jpg");
  expect(app().selection).toEqual([]);
  expect(app().filters.extension).toBeUndefined();
  await act(() => foldersTab.click());
  await flush();
  expect(app().mapColorMode).toBe("folder");
  const calls = mock.invoke.mock.calls.filter(
    ([command]) => command === "get_map",
  );
  expect(calls.at(-1)?.[1]).not.toHaveProperty("colorMode");
  expect(app().map.some((node) => node.fileType !== undefined)).toBe(false);
});

it("ignores a late file-type map response after returning to folder colors", async () => {
  const slow = deferred<{ nodes: NodeView[] }>();
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "get_map" && args.colorMode === "fileType"
      ? slow.promise
      : original(command, args),
  );
  await act(() => app().changeMapColorMode("fileType"));
  await flush();
  await act(() => app().changeMapColorMode("folder"));
  await flush();
  await act(() =>
    slow.resolve({
      nodes: [{ ...image, id: 0x80000000, kind: "aggregate", fileType: "jpg" }],
    }),
  );
  await flush();
  expect(map().dataset.color).toBe("folder");
  expect(app().map.some((node) => node.fileType !== undefined)).toBe(false);
});

it("coalesces slow composition-map queries to the latest folder", async () => {
  const slow = deferred<{ nodes: NodeView[] }>();
  const original = mock.invoke.getMockImplementation()!;
  let first = true;
  mock.invoke.mockImplementation((command, args) => {
    if (command === "get_map" && args.colorMode === "fileType" && first) {
      first = false;
      return slow.promise;
    }
    return original(command, args);
  });
  await act(() => app().changeMapColorMode("fileType"));
  await flush();
  await act(() => app().setFocus(folder));
  await flush();
  await act(() => app().setFocus(scanRoot));
  await flush();
  await act(() => app().setFocus(folder));
  await flush();
  const typeCalls = () =>
    mock.invoke.mock.calls.filter(
      ([command, args]) =>
        command === "get_map" && args.colorMode === "fileType",
    );
  expect(typeCalls()).toHaveLength(1);
  await act(() => slow.resolve({ nodes: [scanRoot] }));
  await flush();
  expect(typeCalls()).toHaveLength(2);
  expect(typeCalls().at(-1)?.[1]).toMatchObject({ nodeId: 1 });
});

it("retains search and list state while switching tabs without another statistics query", async () => {
  await openTypes();
  const search = panel().querySelector<HTMLInputElement>("input")!;
  await act(() => {
    search.value = "JPEG";
    search.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const list = panel().querySelector<HTMLDivElement>(".file-type-list")!;
  await click("#files-tab");
  await click("#file-types-tab");
  expect(panel().querySelector("input")).toBe(search);
  expect(panel().querySelector(".file-type-list")).toBe(list);
  expect(search.value).toBe("JPEG");
  expect(
    mock.invoke.mock.calls.filter(([name]) => name === "get_file_types"),
  ).toHaveLength(1);
});

it("refreshes statistics of a live scan every ten seconds, not every revision", async () => {
  await openTypes();
  const channel = mock.invoke.mock.calls
    .filter(([name]) => name === "start_scan")
    .at(-1)![1].onEvent as { onmessage: (event: ScanSummary) => void };
  const queries = () =>
    mock.invoke.mock.calls.filter(([name]) => name === "get_file_types").length;
  const report = async (state: ScanSummary["state"], elapsedMs: number) => {
    await act(async () => channel.onmessage({ ...summary(), state, elapsedMs }));
    await flush();
  };
  await report("running", 1000);
  const before = queries();
  for (const elapsedMs of [3000, 5000, 9000]) await report("running", elapsedMs);
  expect(queries()).toBe(before);
  await report("running", 10000);
  expect(queries()).toBe(before + 1);
  await report("completed", 10500);
  expect(queries()).toBe(before + 2);
});

// The channel a live scan reports through; each report is a summary update.
function liveReports() {
  const channel = mock.invoke.mock.calls
    .filter(([name]) => name === "start_scan")
    .at(-1)![1].onEvent as { onmessage: (event: ScanSummary) => void };
  return async (state: ScanSummary["state"], elapsedMs: number) => {
    await act(async () => channel.onmessage({ ...summary(), state, elapsedMs }));
    await flush();
  };
}
const count = (command: string) =>
  mock.invoke.mock.calls.filter(([name]) => name === command).length;

it("keeps the statistics list on screen while a newer revision loads, even a slow one", async () => {
  const report = liveReports();
  await openTypes();
  await report("running", 1000);
  const row = panel().querySelector(".file-type-row")!;
  expect(row).toBeTruthy();
  const slow = deferred<FileTypeStats>();
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((name, args) =>
    name === "get_file_types" ? slow.promise : original(name, args),
  );
  await report("running", 10000);
  expect(panel().textContent).not.toContain("Loading file types");
  expect(panel().querySelector(".file-type-row")).toBe(row);
  // The reply is used although another revision began while it was in flight.
  await report("running", 20000);
  await act(() => slow.resolve(stats()));
  await flush();
  expect(panel().textContent).not.toContain("Loading file types");
  expect(panel().querySelectorAll(".file-type-row")).toHaveLength(2);
});

it("re-queries file-type highlights once per revision of a live scan, not on every map refresh", async () => {
  const report = liveReports();
  await openTypes();
  await selectType(".jpg");
  await report("running", 1000);
  const before = count("get_file_type_highlights");
  expect(map().dataset.ids).toBe("0,1");
  // Each second the map reloads into a new nodes array; the highlight stays without a query.
  for (const elapsedMs of [2000, 3000, 4000]) {
    await report("running", elapsedMs);
    expect(map().dataset.pending).toBe("false");
    expect(map().dataset.ids).toBe("0,1");
  }
  expect(count("get_file_type_highlights")).toBe(before);
  await report("running", 10000);
  expect(count("get_file_type_highlights")).toBe(before + 1);
  expect(map().dataset.pending).toBe("false");
  // Another extension never shows the previous one's ids.
  await selectType(".zip");
  expect(map().dataset.ids).toBe("0,3");
});

it("re-queries file-type highlights of a live scan when the map shows mostly other tiles", async () => {
  const report = liveReports();
  await openTypes();
  await selectType(".jpg");
  await report("running", 1000);
  const before = count("get_file_type_highlights");
  // Zooming deeper shows tiles the last query never asked about, within the same revision.
  const original = mock.invoke.getMockImplementation()!;
  const deeper = [5, 6, 7].map((id) => ({ ...image, id, parentId: 2 }));
  mock.invoke.mockImplementation((name, args) =>
    name === "get_map" ? Promise.resolve({ nodes: deeper }) : original(name, args),
  );
  await report("running", 2000);
  expect(count("get_file_type_highlights")).toBe(before + 1);
  const call = mock.invoke.mock.calls
    .filter(([name]) => name === "get_file_type_highlights")
    .at(-1)!;
  expect(call[1].nodeIds).toEqual([5, 6, 7]);
});

it("retains scan-wide totals while visiting the current-folder scope", async () => {
  await openTypes();
  await act(() => app().setFocus(folder));
  await flush();
  const buttons = panel().querySelectorAll<HTMLButtonElement>(
    ".file-type-scope button",
  );
  await act(() => buttons[1].click());
  await flush();
  expect(panel().textContent).toContain("1 type");
  await act(() => buttons[0].click());
  await flush();
  expect(panel().textContent).toContain("2 types");
  expect(
    mock.invoke.mock.calls.filter(
      ([name, args]) => name === "get_file_types" && args.nodeId === 0,
    ),
  ).toHaveLength(1);
});

it("keeps the displayed scan's file-type context when choosing the next scan location", async () => {
  await openTypes();
  await act(() =>
    app().setTarget({
      id: "other",
      name: "Other",
      path: "/other",
      totalBytes: null,
      freeBytes: null,
    }),
  );
  await flush();
  expect(panel().querySelector<HTMLElement>(".file-type-total")?.title).toBe(
    "/fixture",
  );
  expect(
    mock.invoke.mock.calls.filter(([name]) => name === "get_file_types"),
  ).toHaveLength(1);
});

it("reuses a valid statistics result completed while the panel was hidden", async () => {
  const slow = deferred<FileTypeStats>();
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((name, args) =>
    name === "get_file_types" ? slow.promise : original(name, args),
  );
  await openTypes();
  await click("#files-tab");
  await act(() => slow.resolve(stats()));
  await flush();
  await click("#file-types-tab");
  expect(panel().querySelectorAll(".file-type-row")).toHaveLength(2);
  expect(
    mock.invoke.mock.calls.filter(([name]) => name === "get_file_types"),
  ).toHaveLength(1);
});

it("does not repeat an in-flight query when reopening before it completes", async () => {
  const slow = deferred<FileTypeStats>();
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((name, args) =>
    name === "get_file_types" && args.nodeId === 0
      ? slow.promise
      : original(name, args),
  );
  await openTypes();
  await click("#files-tab");
  await click("#file-types-tab");
  expect(
    mock.invoke.mock.calls.filter(([name]) => name === "get_file_types"),
  ).toHaveLength(1);
  await act(() => slow.resolve(stats()));
  await flush();
  expect(panel().querySelectorAll(".file-type-row")).toHaveLength(2);
  expect(
    mock.invoke.mock.calls.filter(([name]) => name === "get_file_types"),
  ).toHaveLength(1);
  await act(() => app().setFocus(folder));
  await flush();
  await act(() =>
    panel()
      .querySelectorAll<HTMLButtonElement>(".file-type-scope button")[1]
      .click(),
  );
  await flush();
  expect(panel().textContent).toContain("1 type");
  expect(
    mock.invoke.mock.calls.filter(([name]) => name === "get_file_types"),
  ).toHaveLength(2);
});

it("reports query failures and retries without leaving stale statistics or highlights", async () => {
  const original = mock.invoke.getMockImplementation()!;
  let failStats = true;
  mock.invoke.mockImplementation((command, args) =>
    command === "get_file_types" && failStats
      ? Promise.reject("Metadata unavailable")
      : original(command, args),
  );
  await openTypes();
  expect(panel().querySelector('[role="alert"]')?.textContent).toContain(
    "Metadata unavailable",
  );
  failStats = false;
  const retryButton = [
    ...panel().querySelectorAll<HTMLButtonElement>("button"),
  ].find((button) => button.textContent === "Retry file types")!;
  await act(() => retryButton.click());
  await flush();
  expect(panel().querySelectorAll(".file-type-row")).toHaveLength(2);
});

it("keeps search copy and execution in the matching-file folder scope", async () => {
  await act(() => app().open(folder));
  await flush();
  await openTypes();
  await click(".file-type-scope button:last-child");
  await selectType(".jpg");
  await click(".file-type-show-files");
  const search = element.querySelector<HTMLInputElement>(
    ".header-search input",
  )!;
  expect(search.placeholder).toBe("Search Photos…");
  expect(search.title).toContain("/fixture/Photos");
  expect(
    element.querySelector("#search-scope-description")?.textContent,
  ).toContain("Clear filters to search the entire scan");
  expect(element.querySelector(".explorer")?.getAttribute("style")).toContain(
    "--list-share: 68fr",
  );
  await act(() => {
    search.value = "photo";
    search.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => vi.advanceTimersByTimeAsync(250));
  expect(mock.invoke).toHaveBeenCalledWith(
    "filter_nodes",
    expect.objectContaining({ scopeId: 1, query: "photo", extension: "jpg" }),
  );
  await act(() =>
    app().changeFilters({ ...app().filters, extension: null, scopeId: null }),
  );
  await flush();
  expect(search.placeholder).toBe("Search entire scan…");
});

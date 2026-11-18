// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, createTestRoot, type Root } from "./testUtils";
import App from "./App.svelte";
import type createApp from "./createApp.svelte";
import type { NodeView, ScanSummary } from "./types";

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
  default: (await import("./TestMap.svelte")).default,
}));
const app = () => mock.app!;
const folder: NodeView = {
  id: 1,
  parentId: 0,
  name: "Photos",
  path: "/fixture/Photos",
  kind: "directory",
  allocatedBytes: 100,
  childrenCount: 2,
  incomplete: false,
};
const scanRoot: NodeView = {
  ...folder,
  id: 0,
  parentId: null,
  name: "Fixture",
  path: "/fixture",
};
const image: NodeView = {
  ...folder,
  id: 2,
  parentId: 1,
  name: "photo.jpg",
  path: "/fixture/Photos/photo.jpg",
  kind: "file",
  childrenCount: 0,
};
const other: NodeView = {
  ...image,
  id: 3,
  name: "other.jpg",
  path: "/fixture/Photos/other.jpg",
};
const summary = (scanId = "live"): ScanSummary => ({
  scanId,
  rootId: 0,
  state: "completed",
  workers: 1,
  workerStats: [],
  finalizationUs: 0,
  entries: 4,
  allocatedBytes: 100,
  elapsedMs: 1000,
  errors: 0,
});
let container: HTMLDivElement, root: Root;
let previewRevision: number;
// The id the next start_scan answers with.
let nextScan: string;
async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}
const inspector = () =>
  container.querySelector<HTMLElement>("#selection-inspector")!;
function row(node: NodeView) {
  return [...container.querySelectorAll<HTMLElement>(".rows .row")].find(
    (el) => el.querySelector(".file-name")?.textContent === node.name,
  )!;
}
async function select(node: NodeView, additive = false) {
  expect(row(node)).toBeTruthy();
  await act(() =>
    row(node).dispatchEvent(
      new MouseEvent("click", { bubbles: true, ctrlKey: additive }),
    ),
  );
  await flush();
}
async function click(name: string) {
  const button = [...container.querySelectorAll("button")].find(
    (item) =>
      item.textContent?.trim() === name || item.getAttribute("aria-label") === name,
  )!;
  await act(() => button.click());
  await flush();
}
async function tab(id: string) {
  await act(() =>
    container.querySelector<HTMLButtonElement>(`#${id}`)!.click(),
  );
  await flush();
}
beforeEach(async () => {
  vi.useFakeTimers();
  previewRevision = 0;
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
        return args.nodeId === 1
          ? folder
          : args.nodeId === 2
            ? image
            : args.nodeId === 3
              ? other
              : scanRoot;
      case "get_children":
        return {
          items: args.nodeId === 1 ? [image, other] : [folder, image, other],
          total: args.nodeId === 1 ? 2 : 3,
        };
      case "search_nodes":
        return { items: [image, folder], total: 2 };
      case "get_map":
        return { nodes: [folder, image, other] };
      case "list_cleanup_queue":
        return { entries: [], revision: 0, blocked: false, warning: null };
      case "quick_look_node":
        return {
          open: true,
          scanId: args.scanId,
          nodeId: args.nodeId,
          revision: ++previewRevision,
        };
      case "close_quick_look":
        return {
          open: false,
          scanId: null,
          nodeId: null,
          revision: ++previewRevision,
        };
      default:
        return undefined;
    }
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createTestRoot(container);
  await act(() => root.render(App));
  await flush();
  await click("Scan Fixture");
});
afterEach(async () => {
  expect(
    mock.invoke.mock.calls.some(([command]) =>
      /prepare_trash|commit_trash|prepare_cleanup_queue|commit_cleanup_queue/.test(
        command,
      ),
    ),
  ).toBe(false);
  await act(() => root.unmount());
  container.remove();
  mock.app = null;
  vi.useRealTimers();
});

it("shows the list and current-folder details together, with file actions waiting for a selection", async () => {
  expect(app().mode).toBe("sunburst");
  expect(
    container.querySelector("#file-list-column .file-list"),
  ).not.toBeNull();
  expect(container.querySelector(".inspector-toggle")).toBeNull();
  expect(
    container.querySelector("#files-tab")?.getAttribute("aria-selected"),
  ).toBe("true");
  expect(container.querySelector("#inspector-details-tab")).toBeNull();
  expect(
    container
      .querySelector("#file-list-column")
      ?.getAttribute("aria-label"),
  ).toBe("File list and details");
  expect(
    container
      .querySelector('#file-list-column [role="tablist"]')
      ?.getAttribute("aria-label"),
  ).toBe("List view");
  expect(container.querySelector<HTMLElement>("#files-panel")!.hidden).toBe(
    false,
  );
  expect(inspector().hidden).toBe(false);
  expect(
    container.querySelector("#file-list-column")?.contains(inspector()),
  ).toBe(true);
  expect(inspector().getAttribute("aria-label")).toBe(
    "Current folder details",
  );
  expect(inspector().querySelector(".details-name")?.textContent).toBe(
    "Fixture",
  );
  const actions = [
    ...inspector().querySelectorAll<HTMLButtonElement>(
      ".inspector-actions button",
    ),
  ];
  expect(
    actions.filter((button) =>
      /Add to cleanup queue/.test(button.textContent ?? ""),
    ),
  ).toHaveLength(1);
  expect(
    actions.some((button) => /Move to Trash/.test(button.textContent ?? "")),
  ).toBe(false);
  // Nothing is selected, so there is nothing to queue yet.
  expect(
    actions.find((button) => /Add to cleanup queue/.test(button.textContent ?? ""))!
      .disabled,
  ).toBe(true);
  expect(inspector().querySelector(".allocation-warning")).toBeNull();
  expect(inspector().textContent).not.toContain("space freed");
});

it("keeps file-list selections visible beside the details and shares them", async () => {
  await select(image);
  expect(inspector().hidden).toBe(false);
  expect(inspector().querySelector(".details-name")?.textContent).toBe(
    image.name,
  );
  expect(row(image).getAttribute("aria-selected")).toBe("true");
  await select(other, true);
  expect(inspector().textContent).toContain("2 items selected");
  expect(inspector().querySelector(".details-name")?.textContent).toBe(
    "2 items selected",
  );
  expect(row(other).getAttribute("aria-selected")).toBe("true");
  await select(other, true);
  expect(inspector().querySelector(".details-name")?.textContent).toBe(
    image.name,
  );
  await act(() => app().setSelection([]));
  expect(inspector().hidden).toBe(false);
  expect(inspector().querySelector(".details-name")?.textContent).toBe(
    scanRoot.name,
  );
});

it.each(["files-tab", "file-types-tab"])(
  "keeps %s active while a map click updates the shared selection",
  async (tabId) => {
    await select(image);
    await tab(tabId);
    const mapSelect =
      container.querySelector<HTMLButtonElement>(".test-map-select")!;
    expect(mapSelect).not.toBeNull();
    await act(() => mapSelect.click());
    await flush();
    expect(app().selection.map((node) => node.id)).toEqual([folder.id]);
    expect(
      container.querySelector(`#${tabId}`)?.getAttribute("aria-selected"),
    ).toBe("true");
    expect(inspector().hidden).toBe(false);
    expect(inspector().querySelector(".details-name")?.textContent).toBe(
      folder.name,
    );
    expect(row(folder).getAttribute("aria-selected")).toBe("true");
  },
);

it("keeps file rows mounted with expanded branches and scroll across tab switches", async () => {
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "get_children" && args.nodeId === folder.id
      ? Promise.resolve({
          items: [
            { ...image, id: 4 },
            { ...other, id: 5 },
          ],
          total: 2,
        })
      : standard(command, args),
  );
  const rows = container.querySelector<HTMLElement>(".rows")!;
  const expand = row(folder).querySelector<HTMLButtonElement>(".disclosure")!;
  expect(expand).not.toBeNull();
  await act(() => expand.click());
  await flush();
  const childRows = rows.querySelectorAll(".row").length;
  expect(childRows).toBeGreaterThan(3);
  rows.scrollTop = 84;
  await tab("file-types-tab");
  expect(container.querySelector<HTMLElement>("#files-panel")!.hidden).toBe(
    true,
  );
  await tab("files-tab");
  expect(container.querySelector(".rows")).toBe(rows);
  expect(rows.querySelectorAll(".row")).toHaveLength(childRows);
  expect(rows.scrollTop).toBe(84);
});

it("resets to Files & Folders when another scan starts", async () => {
  await select(image);
  await tab("file-types-tab");
  nextScan = "second";
  await click("Rescan Fixture");
  expect(app().summary?.scanId).toBe("second");
  expect(inspector().hidden).toBe(false);
  expect(
    container.querySelector("#files-tab")?.getAttribute("aria-selected"),
  ).toBe("true");
});

it("opens Files & Folders when returning to search results", async () => {
  await act(() => app().changeQuery("photo"));
  await act(async () => vi.advanceTimersByTimeAsync(250));
  await select(image);
  await act(() => app().open(folder));
  await flush();
  expect(app().resultsReturn).not.toBeNull();
  await tab("file-types-tab");
  await act(() =>
    [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.includes("Back to results"))!
      .click(),
  );
  await flush();
  expect(app().selection.map((n) => n.id)).toEqual([image.id]);
  expect(
    container.querySelector("#files-tab")?.getAttribute("aria-selected"),
  ).toBe("true");
  expect(inspector().querySelector(".details-name")?.textContent).toBe(
    image.name,
  );
});

it("keeps a newer selection when a pending breadcrumb ancestor completes", async () => {
  const previous = mock.invoke.getMockImplementation()!;
  let finish!: (node: NodeView) => void;
  mock.invoke.mockImplementation((command, args) =>
    command === "get_node" && args.nodeId === 0
      ? new Promise<NodeView>((resolve) => {
          finish = resolve;
        })
      : previous(command, args),
  );
  let navigation!: Promise<void>;
  await act(() => {
    navigation = app().open(folder);
  });
  await flush();
  await select(other);
  await act(async () => {
    finish(scanRoot);
    await navigation;
  });
  expect(app().crumbs.map((n) => n.id)).toEqual([0, 1]);
  expect(app().selection.map((n) => n.id)).toEqual([other.id]);
  expect(inspector().hidden).toBe(false);
});

it("preserves Files & Folders when Space previews a completed live row", async () => {
  await act(() =>
    row(image).dispatchEvent(
      new KeyboardEvent("keydown", {
        key: " ",
        bubbles: true,
        cancelable: true,
      }),
    ),
  );
  await flush();
  expect(app().selection.map((n) => n.id)).toEqual([image.id]);
  expect(
    container.querySelector("#files-tab")?.getAttribute("aria-selected"),
  ).toBe("true");
  expect(
    mock.invoke.mock.calls.some(
      ([command, args]) =>
        command === "quick_look_node" && args.nodeId === image.id,
    ),
  ).toBe(true);
});

it("refreshes selected folder sizes and contents during scanning and at completion", async () => {
  let current = folder;
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "get_node" && args.nodeId === folder.id
      ? Promise.resolve(current)
      : standard(command, args),
  );
  await act(() =>
    app().setSummary({ ...summary(), state: "running" }),
  );
  await select(folder);
  current = { ...folder, allocatedBytes: 2048, childrenCount: 4 };
  await act(() => app().setSummary({ ...app().summary!, elapsedMs: 2000 }));
  await flush();
  expect(app().selection).toEqual([current]);
  current = {
    ...current,
    allocatedBytes: 4096,
    childrenCount: 7,
  };
  await act(() => app().setSummary({ ...app().summary!, state: "completed" }));
  await flush();
  expect(app().selection).toEqual([current]);
  expect(inspector().querySelector(".details-size")?.textContent).toContain(
    "4",
  );
});

it("refreshes every selected file before calculating completed selection totals", async () => {
  let completed = false;
  const freshImage = { ...image, allocatedBytes: 0 };
  const freshOther = { ...other, allocatedBytes: 4096 };
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) => {
    if (completed && command === "get_node") {
      if (args.nodeId === image.id) return Promise.resolve(freshImage);
      if (args.nodeId === other.id) return Promise.resolve(freshOther);
    }
    return standard(command, args);
  });
  await act(() =>
    app().setSummary({ ...summary(), state: "running" }),
  );
  await select(image);
  await select(other, true);
  completed = true;
  await act(() => app().setSummary({ ...app().summary!, state: "completed" }));
  await flush();
  expect(app().selection).toEqual([freshImage, freshOther]);
  expect(inspector().querySelector(".details-name")?.textContent).toBe(
    "2 items selected",
  );
});

it.each(["selection", "scan", "revision"])(
  "ignores a pending selection refresh after the %s changes",
  async (change) => {
    let finish!: (value: NodeView) => void;
    const standard = mock.invoke.getMockImplementation()!;
    mock.invoke.mockImplementation((command, args) =>
      command === "get_node" && args.nodeId === folder.id
        ? new Promise<NodeView>((resolve) => {
            finish = resolve;
          })
        : standard(command, args),
    );
    await select(folder);
    const stale = finish;
    expect(stale).toBeTypeOf("function");
    mock.invoke.mockImplementation(standard);
    if (change === "scan") {
      nextScan = "second";
      await click("Rescan Fixture");
      expect(app().summary?.scanId).toBe("second");
    }
    if (change === "revision") {
      await act(() => app().setSummary({ ...app().summary!, elapsedMs: 2000 }));
      await flush();
    } else {
      await select(other);
    }
    const expected = [...app().selection];
    await act(() => stale({ ...folder, allocatedBytes: 999999 }));
    await flush();
    expect(app().selection).toEqual(expected);
  },
);

it("wraps keyboard navigation across both list views with ARIA relationships", async () => {
  const ids = ["files-tab", "file-types-tab"];
  async function key(value: string, expected: string) {
    const active = container.querySelector<HTMLButtonElement>(
      '[role="tab"][aria-selected="true"]',
    )!;
    await act(() =>
      active.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: value,
          bubbles: true,
          cancelable: true,
        }),
      ),
    );
    expect(document.activeElement?.id).toBe(expected);
    for (const id of ids) {
      const button = container.querySelector<HTMLButtonElement>(`#${id}`)!;
      expect(button.tabIndex).toBe(id === expected ? 0 : -1);
      const panel = container.querySelector<HTMLElement>(
        `#${button.getAttribute("aria-controls")}`,
      )!;
      expect(panel.getAttribute("aria-labelledby")).toBe(id);
      expect(panel.hidden).toBe(id !== expected);
    }
    // The details strip is not part of the tab set and never hides.
    expect(inspector().hidden).toBe(false);
  }
  await key("ArrowLeft", "file-types-tab");
  await key("ArrowRight", "files-tab");
  await key("ArrowRight", "file-types-tab");
  await key("Home", "files-tab");
  await key("End", "file-types-tab");
});

it("hides and restores the file list without remounting rows or losing selection and scroll", async () => {
  await select(image);
  const sidebar = container.querySelector<HTMLElement>("#file-list-column")!;
  const rows = container.querySelector<HTMLElement>(".rows")!;
  rows.scrollTop = 84;
  const hide = container.querySelector<HTMLButtonElement>(
    '[aria-label="Hide file list"]',
  )!;
  expect(hide.getAttribute("aria-controls")).toBe("file-list-column");
  expect(hide.getAttribute("aria-expanded")).toBe("true");
  await act(() => hide.click());
  expect(sidebar.hidden).toBe(true);
  expect(container.querySelector(".panel-divider")).toBeNull();
  expect(container.querySelector(".rows")).toBe(rows);
  expect(app().selection.map((node) => node.id)).toEqual([image.id]);
  const show = container.querySelector<HTMLButtonElement>(
    '[aria-label="Show file list"]',
  )!;
  expect(show.getAttribute("aria-expanded")).toBe("false");
  await act(() => show.click());
  expect(sidebar.hidden).toBe(false);
  expect(container.querySelector("#file-list-column")).toBe(sidebar);
  expect(container.querySelector(".rows")).toBe(rows);
  expect(rows.scrollTop).toBe(84);
  expect(row(image).getAttribute("aria-selected")).toBe("true");
  expect(container.querySelector(".panel-divider")).not.toBeNull();
});

it("keeps File types selected while hidden map clicks update selection without reopening the file list", async () => {
  await select(image);
  await tab("file-types-tab");
  const sidebar = container.querySelector<HTMLElement>("#file-list-column")!;
  await act(() =>
    container
      .querySelector<HTMLButtonElement>('[aria-label="Hide file list"]')!
      .click(),
  );
  await act(() =>
    container.querySelector<HTMLButtonElement>(".test-map-select")!.click(),
  );
  await flush();
  expect(sidebar.hidden).toBe(true);
  expect(app().selection.map((node) => node.id)).toEqual([folder.id]);
  expect(
    container.querySelector("#file-types-tab")?.getAttribute("aria-selected"),
  ).toBe("true");
  await act(() =>
    container
      .querySelector<HTMLButtonElement>('[aria-label="Show file list"]')!
      .click(),
  );
  expect(sidebar.hidden).toBe(false);
  expect(inspector().hidden).toBe(false);
  expect(inspector().querySelector(".details-name")?.textContent).toBe(
      folder.name,
    );
});

it("reopens Folders and files when searching with the file list hidden", async () => {
  await tab("file-types-tab");
  await act(() =>
    container
      .querySelector<HTMLButtonElement>('[aria-label="Hide file list"]')!
      .click(),
  );
  const search = container.querySelector<HTMLInputElement>(
    '[aria-label="Search file names"]',
  )!;
  await act(() => {
    search.value = "photo";
    search.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => vi.advanceTimersByTimeAsync(250));
  expect(
    container.querySelector<HTMLElement>("#file-list-column")!.hidden,
  ).toBe(false);
  expect(
    container.querySelector("#files-tab")?.getAttribute("aria-selected"),
  ).toBe("true");
  expect(app().query).toBe("photo");
});

it("hides and restores locations without remounting its content or changing results", async () => {
  await select(image);
  const sidebar = container.querySelector<HTMLElement>("#locations-sidebar")!;
  const location = sidebar.querySelector(".target");
  const hide = container.querySelector<HTMLButtonElement>(
    '[aria-label="Hide sidebar"]',
  )!;
  expect(hide.getAttribute("aria-controls")).toBe("locations-sidebar");
  expect(hide.getAttribute("aria-expanded")).toBe("true");
  await act(() => hide.click());
  expect(sidebar.hidden).toBe(true);
  expect(
    container
      .querySelector(".app")
      ?.classList.contains("navigation-sidebar-hidden"),
  ).toBe(true);
  expect(app().summary?.scanId).toBe("live");
  expect(app().selection.map((node) => node.id)).toEqual([image.id]);
  const show = container.querySelector<HTMLButtonElement>(
    '[aria-label="Show sidebar"]',
  )!;
  expect(show.getAttribute("aria-expanded")).toBe("false");
  await act(() => show.click());
  expect(sidebar.hidden).toBe(false);
  expect(container.querySelector("#locations-sidebar")).toBe(sidebar);
  expect(sidebar.querySelector(".target")).toBe(location);
});

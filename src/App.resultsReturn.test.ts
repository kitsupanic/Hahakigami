// @vitest-environment jsdom
import { act, createTestRoot as createRoot, type Root } from "./testUtils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import App from "./App.svelte";
import { menuItem } from "./fakeMenu";
import type createApp from "./createApp.svelte";
import type { NodeView, Page, ScanSummary } from "./types";
const mock = vi.hoisted(() => ({
  invoke: vi.fn(),
  app: null as ReturnType<typeof createApp> | null,
}));
vi.mock("./createApp.svelte", async (original) => {
  const module = await original<typeof import("./createApp.svelte")>();
  return { default: () => (mock.app = module.default()) };
});
const app = () => mock.app!;
vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => true,
  invoke: mock.invoke,
  Channel: class {
    onmessage = () => {};
  },
}));
vi.mock("./DiskMap.svelte", async () => ({
  default: (await import("./TestMap.svelte")).default,
}));
const rootNode: NodeView = {
  id: 0,
  parentId: null,
  name: "Home",
  path: "/fixture",
  kind: "directory",
  allocatedBytes: 100,
  childrenCount: 2,
  incomplete: false,
};
const folder: NodeView = {
  ...rootNode,
  id: 1,
  parentId: 0,
  name: "Photos",
  path: "/fixture/Photos",
};
const deeper: NodeView = { ...folder, id: 3, parentId: 1, name: "Deeper" };
const image: NodeView = {
  ...rootNode,
  id: 2,
  parentId: 1,
  name: "image-hit.jpg",
  path: "/fixture/Photos/image-hit.jpg",
  kind: "file",
  childrenCount: 0,
};
const summary = (id = "live"): ScanSummary => ({
  scanId: id,
  rootId: 0,
  workers: 1,
  workerStats: [],
  finalizationUs: 0,
  state: "completed",
  entries: 3,
  allocatedBytes: 100,
  elapsedMs: 1000,
  errors: 0,
});
let container: HTMLDivElement, root: Root;
// An open native folder picker is an operation in flight, which blocks navigation like a Trash review does.
let closePicker: () => void;
let picking: Promise<unknown> | void;
const pickerOpen = [
  () => {
    picking = app().chooseFolder();
  },
  async () => {
    closePicker();
    await picking;
  },
];
function calls(command: string) {
  return mock.invoke.mock.calls.filter(([name]) => name === command);
}
async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}
async function advance(milliseconds = 250) {
  await act(async () => vi.advanceTimersByTimeAsync(milliseconds));
}
async function search(value: string) {
  await act(async () => {
    const node = container.querySelector<HTMLInputElement>(
      'input[aria-label="Search file names"]',
    )!;
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(node, value);
    node.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await flush();
}
async function click(name: string) {
  const node = Array.from(container.querySelectorAll("button")).find(
    (button) =>
      button.textContent?.trim() === name ||
      button.getAttribute("aria-label") === name,
  );
  expect(node).toBeTruthy();
  await act(async () => node!.click());
  await flush();
}
beforeEach(async () => {
  vi.useFakeTimers();
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(
    async (command: string, args: Record<string, unknown> = {}) => {
      switch (command) {
        case "get_explorer_preferences":
          return { sort: "sizeDescending", warning: null };
        case "set_explorer_sort":
          return { sort: args.sort, warning: null };
        case "full_disk_access_status":
          return {
            status: "on",
            scanAnyway: false,
            appPath: "/Applications/Fixture.app",
          };
        case "discover_targets":
          return [
            {
              id: "home",
              name: "Home",
              path: "/fixture",
              totalBytes: null,
              freeBytes: null,
              volume: true,
            },
            {
              id: "other",
              name: "Other",
              path: "/other",
              totalBytes: null,
              freeBytes: null,
              volume: true,
            },
          ];
        case "get_node":
          return args.nodeId === 3
            ? deeper
            : args.nodeId === 1
              ? folder
              : rootNode;
        case "get_children":
          return {
            items: args.nodeId === 1 ? [image] : [folder, image],
            total: 2,
          };
        case "get_map":
          return { nodes: [folder, image] };
        case "filter_nodes":
          return { items: [image, folder], total: 320 };
        case "search_nodes":
          return { items: [image, folder], total: 320 };
        case "choose_folder":
          return new Promise<null>((done) => {
            closePicker = () => done(null);
          });
        case "start_scan":
          return "live";
        case "scan_summary":
          return summary();
        default:
          return undefined;
      }
    },
  );
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => root.render(App));
  await flush();
  await click("Scan Home");
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.useRealTimers();
});

async function resultPage() {
  await act(() => {
    app().changeQuery(" photo ");
    app().changeFilters({ category: "images", minBytes: 10, maxBytes: null });
    app().changeSort("nameAscending");
  });
  await advance();
  await click("Next");
  await act(() => {
    app().select(image);
    app().setListScroll(280);
  });
}
async function browse() {
  await act(() => app().open(folder));
  await flush();
}
it("returns to the original results page after deeper browsing, including selection and scroll", async () => {
  await resultPage();
  await browse();
  expect(app().resultsReturn).toMatchObject({ query: " photo ", total: 320 });
  await act(() => app().open(deeper));
  await act(() => {
    app().changeSort("sizeAscending");
  });
  await flush();
  await click("Back to results");
  expect(calls("filter_nodes").at(-1)?.[1]).toMatchObject({
    scanId: "live",
    query: "photo",
    category: "images",
    minBytes: 10,
    sort: "nameAscending",
    offset: 150,
  });
  expect(app().query).toBe(" photo ");
  expect(app().debouncedQuery).toBe(" photo ");
  expect(app().focus?.id).toBe(0);
  expect(app().crumbs.map((n) => n.id)).toEqual([0]);
  expect(app().selection.map((n) => n.id)).toEqual([2]);
  expect(
    container.querySelector('.file-list [aria-selected="true"] .file-name')
      ?.textContent,
  ).toBe("image-hit.jpg");
  expect(app().resultsReturn).toBeNull();
  expect(container.textContent).not.toContain("Back to results");
  // The row component acknowledges real scroll restoration; controller token tests
  // below exercise pending requests without depending on jsdom layout dimensions.
});
it("preserves search-only global results and discards the return point on an explicit new query or filter", async () => {
  await search("photo");
  await advance();
  await click("Next");
  await browse();
  await act(() => app().returnToResults());
  expect(calls("search_nodes").at(-1)?.[1]).toMatchObject({
    query: "photo",
    offset: 150,
  });
  await browse();
  await act(() => app().changeQuery("new"));
  expect(app().resultsReturn).toBeNull();
  await advance();
  await browse();
  expect(app().resultsReturn).not.toBeNull();
  await act(() =>
    app().changeFilters({ category: "audio", minBytes: null, maxBytes: null }),
  );
  expect(app().resultsReturn).toBeNull();
  expect(calls("filter_nodes").at(-1)?.[1]).toMatchObject({
    category: "audio",
    query: "",
    offset: 0,
  });
});
it("retains the return point on a failed scan start, clears it after a fresh scan", async () => {
  await resultPage();
  await browse();
  const previous = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "start_scan"
      ? Promise.reject(new Error("Scan unavailable"))
      : previous(command, args),
  );
  await act(() => app().start());
  expect(app().resultsReturn).not.toBeNull();
  await act(() => app().returnToResults());
  expect(calls("filter_nodes").at(-1)?.[1].offset).toBe(150);
  await browse();
  mock.invoke.mockImplementation(previous);
  await act(() => app().start());
  expect(app().resultsReturn).toBeNull();
  await resultPage();
  await browse();
  await click("Rescan Home");
  expect(app().resultsReturn).toBeNull();
  expect(app().query).toBe("");
});
it("blocks returning while busy, previewing, or using a different active scan", async () => {
  await resultPage();
  await browse();
  const saved = app().resultsReturn;
  for (const [block, unblock] of [
    pickerOpen,
    [() => app().openAccess(), () => app().closeAccess()],
    [
      () => {
        app().activeId.current = "other";
      },
      () => {
        app().activeId.current = "live";
      },
    ],
  ]) {
    await act(block);
    await act(() => app().returnToResults());
    expect(app().resultsReturn).toBe(saved);
    expect(app().focus?.id).toBe(1);
    await act(unblock);
  }
  await act(() => app().returnToResults());
  expect(calls("filter_nodes").at(-1)?.[1].offset).toBe(150);
});
it("ignores delayed folder rows and breadcrumb failures after returning", async () => {
  await resultPage();
  const previous = mock.invoke.getMockImplementation()!;
  let folderRows!: (page: Page) => void;
  let crumbFailure!: (reason: Error) => void;
  let breadcrumbPending = true;
  mock.invoke.mockImplementation((command, args) => {
    if (command === "get_children" && args.nodeId === deeper.id)
      return new Promise<Page>((done) => {
        folderRows = done;
      });
    if (command === "get_node" && args.nodeId === folder.id && breadcrumbPending) {
      breadcrumbPending = false;
      return new Promise((_done, reject) => {
        crumbFailure = reject;
      });
    }
    return previous(command, args);
  });
  // The chain holds only the root, so the walk to the deeper folder asks the backend for its parent.
  let opening!: Promise<void>;
  await act(() => {
    opening = app().open(deeper);
  });
  await act(() => app().returnToResults());
  await act(async () => {
    folderRows({ items: [{ ...image, name: "stale-folder-row" }], total: 1 });
    crumbFailure(new Error("stale breadcrumb failure"));
    await opening;
  });
  expect(container.textContent).not.toContain("stale-folder-row");
  expect(app().error).not.toContain("stale breadcrumb failure");
  expect(app().crumbs.map((n) => n.id)).toEqual([0]);
  expect(calls("filter_nodes").at(-1)?.[1].offset).toBe(150);
  expect(app().page.total).toBe(320);
});
it("clamps restored result offsets through the ordinary fresh-result loader", async () => {
  await resultPage();
  await browse();
  const previous = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "filter_nodes"
      ? Promise.resolve({ items: args.offset ? [] : [image], total: 1 })
      : previous(command, args),
  );
  await act(() => app().returnToResults());
  await flush();
  expect(
    calls("filter_nodes")
      .slice(-2)
      .map((call) => call[1].offset),
  ).toEqual([150, 0]);
  expect(container.querySelector(".pagination")).toBeNull();
  expect(container.querySelector(".list-heading")?.textContent).toContain(
    "1 item",
  );
  expect(container.querySelector(".list-heading")?.textContent).not.toContain(
    "1 items",
  );
});
it("persists the restored sort after an outstanding folder sort save completes", async () => {
  await resultPage();
  await browse();
  const previous = mock.invoke.getMockImplementation()!;
  let finish!: (value: unknown) => void;
  mock.invoke.mockImplementation((command, args) =>
    command === "set_explorer_sort" && args.sort === "sizeAscending"
      ? new Promise((done) => {
          finish = done;
        })
      : previous(command, args),
  );
  await act(() => app().changeSort("sizeAscending"));
  await act(() => app().returnToResults());
  expect(app().sort).toBe("nameAscending");
  await act(() => finish({ sort: "sizeAscending", warning: "old warning" }));
  await flush();
  expect(calls("set_explorer_sort").at(-1)?.[1].sort).toBe("nameAscending");
  expect(app().sortWarning).toBeNull();
  expect(app().sortSaving).toBe(false);
});

it("keeps a pending scroll request until its matching acknowledgement and cancels it on a new query", async () => {
  await resultPage();
  await browse();
  const previous = mock.invoke.getMockImplementation()!;
  let finish!: (page: Page) => void;
  mock.invoke.mockImplementation((command, args) =>
    command === "filter_nodes"
      ? new Promise<Page>((done) => {
          finish = done;
        })
      : previous(command, args),
  );
  await act(() => app().returnToResults());
  const request = app().listScrollRestore!;
  expect(request.top).toBe(280);
  expect(app().listLoading).toBe(true);
  await act(() => app().finishListScrollRestore(request.id - 1));
  expect(app().listScrollRestore).toBe(request);
  await act(() => app().finishListScrollRestore(request.id));
  expect(app().listScrollRestore).toBeNull();
  await act(() => finish({ items: [image, folder], total: 320 }));
  mock.invoke.mockImplementation(previous);
  await browse();
  mock.invoke.mockImplementation((command, args) =>
    command === "filter_nodes"
      ? new Promise<Page>((done) => {
          finish = done;
        })
      : previous(command, args),
  );
  await act(() => app().returnToResults());
  expect(app().listScrollRestore!.id).toBeGreaterThan(request.id);
  await act(() => app().changeQuery("new"));
  expect(app().listScrollRestore).toBeNull();
  await act(() =>
    finish({ items: [{ ...image, name: "stale-return" }], total: 320 }),
  );
  expect(container.textContent).not.toContain("stale-return");
});

it("navigates back and forward through folders and discards forward visits on a new branch", async () => {
  expect(app().can.goBack).toBe(false);
  expect(app().can.goForward).toBe(false);
  await browse();
  await act(() => app().open(deeper));
  expect(app().crumbs.map((node) => node.id)).toEqual([0, 1, 3]);
  await act(() => app().goBack());
  expect(app().focus?.id).toBe(1);
  expect(app().crumbs.map((node) => node.id)).toEqual([0, 1]);
  expect(app().can.goForward).toBe(true);
  await act(() => app().goBack());
  expect(app().focus?.id).toBe(0);
  expect(app().can.goBack).toBe(false);
  await act(() => app().goForward());
  expect(app().focus?.id).toBe(1);
  await act(() => app().goForward());
  expect(app().focus?.id).toBe(3);
  expect(app().crumbs.map((node) => node.id)).toEqual([0, 1, 3]);
  expect(app().can.goForward).toBe(false);
  await act(() => app().goBack());
  await act(() => app().open(rootNode));
  expect(app().focus?.id).toBe(0);
  expect(app().can.goForward).toBe(false);
  await act(() => app().goBack());
  expect(app().focus?.id).toBe(1);
});

it("restores paged search, filters, selection, sort and scroll across history without losing Back to results", async () => {
  await resultPage();
  await browse();
  await act(() => {
    app().changeSort("sizeAscending");
    app().select(image);
  });
  const previous = mock.invoke.getMockImplementation()!;
  let finish!: (page: Page) => void;
  mock.invoke.mockImplementation((command, args) =>
    command === "filter_nodes"
      ? new Promise<Page>((done) => {
          finish = done;
        })
      : previous(command, args),
  );
  await act(() => app().goBack());
  expect(app().focus?.id).toBe(0);
  expect(app().query).toBe(" photo ");
  expect(app().debouncedQuery).toBe(" photo ");
  expect(app().filters).toMatchObject({ category: "images", minBytes: 10 });
  expect(app().sort).toBe("nameAscending");
  expect(app().selection.map((node) => node.id)).toEqual([2]);
  expect(app().listScrollRestore?.top).toBe(280);
  expect(app().resultsReturn).toBeNull();
  expect(calls("filter_nodes").at(-1)?.[1]).toMatchObject({
    offset: 150,
    sort: "nameAscending",
  });
  await act(() => finish({ items: [image, folder], total: 320 }));
  mock.invoke.mockImplementation(previous);
  await act(() => app().goForward());
  expect(app().focus?.id).toBe(1);
  expect(app().query).toBe("");
  expect(app().sort).toBe("sizeAscending");
  expect(app().selection.map((node) => node.id)).toEqual([2]);
  expect(app().resultsReturn?.query).toBe(" photo ");
  await act(() => app().returnToResults());
  expect(app().query).toBe(" photo ");
  await act(() => app().goBack());
  expect(app().focus?.id).toBe(1);
  expect(app().resultsReturn?.query).toBe(" photo ");
});

it("ignores files, excluded folders and current-folder visits without duplicating or clearing history", async () => {
  await act(() => app().open(rootNode));
  await act(() => app().open(image));
  await act(() => app().open({ ...folder, excluded: true }));
  expect(app().can.goBack).toBe(false);
  await browse();
  await act(() => app().goBack());
  await act(() => app().open(rootNode));
  expect(app().can.goBack).toBe(false);
  expect(app().can.goForward).toBe(true);
  await act(() => app().goForward());
  expect(app().focus?.id).toBe(1);
});

it("reconstructs pending breadcrumbs on Forward and ignores an obsolete open failure", async () => {
  const previous = mock.invoke.getMockImplementation()!;
  let rejectOld!: (reason: Error) => void;
  let first = true;
  mock.invoke.mockImplementation((command, args) => {
    if (command === "get_node" && args.nodeId === 0 && first) {
      first = false;
      return new Promise((_done, reject) => {
        rejectOld = reject;
      });
    }
    return previous(command, args);
  });
  let opening!: Promise<void>;
  await act(() => {
    opening = app().open(folder);
  });
  expect(app().focus?.id).toBe(1);
  await act(() => app().goBack());
  expect(app().crumbs.map((node) => node.id)).toEqual([0]);
  await act(() => app().goForward());
  expect(app().crumbs.map((node) => node.id)).toEqual([0, 1]);
  await act(async () => {
    rejectOld(new Error("obsolete open"));
    await opening;
  });
  expect(app().focus?.id).toBe(1);
  expect(app().crumbs.map((node) => node.id)).toEqual([0, 1]);
  expect(app().error).not.toContain("obsolete open");
});

it("keeps history through rejected scan changes and cancelled folder choices, then clears it on accepted sessions", async () => {
  await browse();
  const previous = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "start_scan"
      ? Promise.reject(new Error("Not available"))
      : command === "choose_folder"
        ? Promise.resolve(null)
        : previous(command, args),
  );
  await act(() => app().start());
  expect(app().can.goBack).toBe(true);
  await act(() => app().chooseFolder());
  expect(app().can.goBack).toBe(true);
  await act(() => app().goBack());
  expect(app().focus?.id).toBe(0);
  expect(app().can.goForward).toBe(true);
  mock.invoke.mockImplementation(previous);
  await act(() => app().start());
  expect(app().can.goBack).toBe(false);
  expect(app().can.goForward).toBe(false);
  await browse();
  await act(() => app().start());
  expect(app().can.goBack).toBe(false);
  expect(app().can.goForward).toBe(false);
});

it("blocks both directions while busy, in modal or alternate views, or using a different active scan", async () => {
  await browse();
  await act(() => app().open(deeper));
  await act(() => app().goBack());
  for (const [block, unblock] of [
    pickerOpen,
    [() => app().openAccess(), () => app().closeAccess()],
    [() => app().showQueue(true), () => app().showQueue(false)],
    [
      () => {
        app().activeId.current = "other";
      },
      () => {
        app().activeId.current = "live";
      },
    ],
  ]) {
    await act(block);
    expect(app().can.goBack).toBe(false);
    expect(app().can.goForward).toBe(false);
    await act(() => app().goBack());
    await act(() => app().goForward());
    expect(app().focus?.id).toBe(1);
    await act(unblock);
    expect(app().can.goBack).toBe(true);
    expect(app().can.goForward).toBe(true);
  }
  await act(() => app().goForward());
  expect(app().focus?.id).toBe(3);
});

it("blocks history during a pending Quick Look and dismisses the settled preview before moving", async () => {
  await browse();
  const previous = mock.invoke.getMockImplementation()!;
  let finish!: (value: {
    open: boolean;
    scanId: string;
    nodeId: number;
    revision: number;
  }) => void;
  mock.invoke.mockImplementation((command, args) => {
    if (command === "quick_look_node")
      return new Promise((done) => {
        finish = done;
      });
    if (command === "close_quick_look")
      return Promise.resolve({
        open: false,
        scanId: null,
        nodeId: null,
        revision: 2,
      });
    return previous(command, args);
  });
  await act(() => {
    app().quickLook.open({ scanId: "live", nodeId: image.id });
  });
  expect(app().quickLook.pending).toBe(true);
  expect(app().can.goBack).toBe(false);
  expect(upEnabled()).toBe(false);
  await up();
  expect(app().focus?.id).toBe(folder.id);
  await act(() => app().goBack());
  expect(app().focus?.id).toBe(1);
  await act(() =>
    finish({ open: true, scanId: "live", nodeId: image.id, revision: 1 }),
  );
  expect(app().quickLook.status.open).toBe(true);
  expect(app().can.goBack).toBe(true);
  await act(() => app().goBack());
  expect(app().focus?.id).toBe(0);
  expect(app().quickLook.status.open).toBe(false);
  expect(calls("close_quick_look")).toHaveLength(1);
});

// Enclosing Folder (⌘↑) is the only Up control, and a native menu item
// dispatches only while it is enabled.
const upEnabled = () => menuItem("enclosing").enabled;
async function up() {
  if (!upEnabled()) return;
  await act(async () => menuItem("enclosing").click());
  await flush();
}

it("moves up through parents to the scan root and records ordinary back and forward visits", async () => {
  expect(upEnabled()).toBe(false);
  expect(container.querySelector('[aria-label="Up"]')).toBeNull();
  await browse();
  await act(() => app().open(deeper));
  expect(upEnabled()).toBe(true);
  await up();
  expect(app().focus?.id).toBe(folder.id);
  await up();
  expect(app().focus?.id).toBe(rootNode.id);
  expect(app().focus?.path).toBe("/fixture");
  expect(upEnabled()).toBe(false);
  await up();
  expect(app().focus?.id).toBe(rootNode.id);
  await click("Back");
  expect(app().focus?.id).toBe(folder.id);
  await click("Back");
  expect(app().focus?.id).toBe(deeper.id);
  await click("Forward");
  expect(app().focus?.id).toBe(folder.id);
  expect(app().can.goForward).toBe(true);
  await up();
  expect(app().focus?.id).toBe(rootNode.id);
  expect(app().can.goForward).toBe(false);
});

it("uses the ordinary folder-opening search return behavior when moving Up", async () => {
  await browse();
  await search("photo");
  await advance();
  await up();
  expect(app().focus?.id).toBe(rootNode.id);
  expect(app().query).toBe("");
  expect(app().resultsReturn?.query).toBe("photo");
  await click("Back");
  expect(app().focus?.id).toBe(folder.id);
  expect(app().query).toBe("photo");
  await click("Forward");
  expect(app().focus?.id).toBe(rootNode.id);
  await click("Back to results");
  expect(app().focus?.id).toBe(folder.id);
  expect(app().query).toBe("photo");
});

it("disables Enclosing Folder for blocked views and while the parent breadcrumb is still loading", async () => {
  await browse();
  for (const [block, unblock] of [
    pickerOpen,
    [() => app().openAccess(), () => app().closeAccess()],
    [() => app().showQueue(true), () => app().showQueue(false)],
  ]) {
    await act(block);
    await flush();
    expect(upEnabled()).toBe(false);
    await up();
    expect(app().focus?.id).toBe(folder.id);
    await act(unblock);
    await flush();
    expect(upEnabled()).toBe(true);
  }
  // While the walk to a folder's ancestors is pending its parent is unknown, so Up waits for the chain.
  await act(() => app().goBack());
  const previous = mock.invoke.getMockImplementation()!;
  let finish!: (node: NodeView) => void;
  mock.invoke.mockImplementation((command, args) =>
    command === "get_node" && args.nodeId === folder.id
      ? new Promise<NodeView>((done) => {
          finish = done;
        })
      : previous(command, args),
  );
  let opening!: Promise<void>;
  await act(() => {
    opening = app().open(deeper);
  });
  await flush();
  expect(upEnabled()).toBe(false);
  await up();
  expect(app().focus?.id).toBe(deeper.id);
  await act(async () => {
    finish(folder);
    await opening;
  });
  expect(upEnabled()).toBe(true);
  await up();
  expect(app().focus?.id).toBe(folder.id);
});
it("hides the back bar and ignores the return on Home or the queue", async () => {
  await resultPage();
  await browse();
  expect(container.textContent).toContain("Back to results");
  const filters = calls("filter_nodes").length;
  for (const [show, hide] of [
    [
      () => app().openDisk(app().targets[1]),
      () => app().openDisk(app().targets[0]),
    ],
    [() => app().showQueue(true), () => app().showQueue(false)],
  ]) {
    await act(show);
    await flush();
    expect(container.textContent).not.toContain("Back to results");
    await act(() => app().returnToResults());
    expect(app().resultsReturn).not.toBeNull();
    expect(app().focus?.id).toBe(1);
    expect(calls("filter_nodes")).toHaveLength(filters);
    await act(hide);
    await flush();
  }
  expect(container.textContent).toContain("Back to results");
});

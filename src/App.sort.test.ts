// @vitest-environment jsdom
import { act, createTestRoot as createRoot, type Root, fullEdition } from "./testUtils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import App from "./App.svelte";
import type { NodeView, Page, ScanSummary } from "./types";
import type { ExplorerPreferences } from "./sortTypes";
import type createApp from "./createApp.svelte";
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
vi.mock("./DiskMap.svelte", async () => ({
  default: (await import("./TestMap.svelte")).default,
}));
const node: NodeView = {
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
  ...node,
  id: 1,
  parentId: 0,
  name: "Photos",
  path: "/fixture/Photos",
};
const file: NodeView = {
  ...node,
  id: 2,
  parentId: 1,
  name: "image.jpg",
  path: "/fixture/Photos/image.jpg",
  kind: "file",
  childrenCount: 0,
};
const rootFile: NodeView = {
  ...file,
  id: 3,
  parentId: 0,
  name: "root.jpg",
  path: "/fixture/root.jpg",
};
const summary = (id = "live"): ScanSummary => ({
  scanId: id,
  rootId: 0,
  state: "completed",
  entries: 3,
  allocatedBytes: 100,
  elapsedMs: 1000,
  workers: 1,
  workerStats: [],
  finalizationUs: 0,
  errors: 0,
});
let container: HTMLDivElement, root: Root;
function calls(command: string) {
  return mock.invoke.mock.calls.filter(([name]) => name === command);
}
async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}
async function mount(remount = false) {
  await act(async () => root.render(App));
  if (remount) {
    await act(async () => root.unmount());
    root = createRoot(container);
    await act(async () => root.render(App));
  }
  await flush();
  await click("Scan Home");
}
async function choose(name: string, value: string) {
  if (name === "Sort order") return chooseSort(value);
  const control = container.querySelector<HTMLSelectElement>(
    `select[aria-label="${name}"]`,
  );
  if (control?.closest<HTMLElement>("[hidden]")) {
    await act(async () =>
      container
        .querySelector<HTMLButtonElement>('[aria-controls="explorer-filters"]')!
        .click(),
    );
  }
  await act(async () => {
    const select = container.querySelector<HTMLSelectElement>(
      `select[aria-label="${name}"]`,
    )!;
    if (select.disabled) return;
    select.value = value;
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await flush();
}
function sortOrder() {
  return container.querySelector<HTMLElement>(".table-head")!.dataset.sort;
}
function sortButton(value: string) {
  return container.querySelector<HTMLButtonElement>(
    `.table-head button[aria-label="Sort by ${value.startsWith("name") ? "name" : "size"}"]`,
  )!;
}
async function chooseSort(value: string) {
  for (let attempt = 0; attempt < 2 && sortOrder() !== value; attempt++) {
    const header = sortButton(value);
    if (header.disabled) return;
    await act(async () => header.click());
    await flush();
  }
}
async function click(name: string) {
  const button = Array.from(container.querySelectorAll("button")).find(
    (button) =>
      button.textContent?.trim() === name ||
      button.getAttribute("aria-label") === name,
  );
  expect(button).toBeTruthy();
  await act(async () => button!.click());
  await flush();
}
beforeEach(() => {
  vi.useFakeTimers();
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(
    async (command: string, args: Record<string, unknown> = {}) => {
      switch (command) {
        case "get_edition":
          return fullEdition;
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
            },
          ];
        case "get_node":
          return args.nodeId === 1 ? folder : node;
        case "get_children":
          return {
            items: args.nodeId === 1 ? [file] : [folder, rootFile],
            total: 2,
          };
        case "filter_nodes":
          return { items: [folder, file], total: 320 };
        case "search_nodes":
          return { items: [file], total: 1 };
        case "get_map":
          return { nodes: [folder, file] };
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
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.useRealTimers();
});
it("restores preferences before the first list request while allowing maps to load", async () => {
  const previous = mock.invoke.getMockImplementation()!;
  let restore!: (value: ExplorerPreferences) => void;
  mock.invoke.mockImplementation(
    (command: string, args: Record<string, unknown>) =>
      command === "get_explorer_preferences"
        ? new Promise<ExplorerPreferences>((resolve) => {
            restore = resolve;
          })
        : previous(command, args),
  );
  await mount();
  expect(calls("get_children")).toHaveLength(0);
  expect(calls("get_map")).toHaveLength(1);
  await act(async () => restore({ sort: "nameAscending", warning: null }));
  await flush();
  expect(calls("get_children").at(-1)?.[1].sort).toBe("nameAscending");
  expect(sortOrder()).toBe("nameAscending");
});
it("changes sort immediately, resets page/selection, preserves filters and maps, and forwards all list calls", async () => {
  await mount();
  await vi.waitFor(async () => {
    await flush();
    expect(container.querySelector(".file-list .disclosure")).not.toBeNull();
  });
  const mapCalls = calls("get_map").length;
  const disclosure = container.querySelector<HTMLButtonElement>(
    ".file-list .disclosure",
  )!;
  await act(async () => disclosure.click());
  await flush();
  expect(calls("get_children").at(-1)?.[1]).toMatchObject({
    nodeId: 1,
    sort: "sizeDescending",
  });
  await choose("Sort order", "nameDescending");
  expect(
    container.querySelector('.file-list [aria-expanded="true"]'),
  ).toBeNull();
  await choose("File category", "images");
  await click("Next");
  await act(async () =>
    container
      .querySelector<HTMLElement>('.file-list [role="treeitem"]')!
      .click(),
  );
  await choose("Sort order", "sizeAscending");
  expect(calls("filter_nodes").at(-1)?.[1]).toMatchObject({
    category: "images",
    offset: 0,
    sort: "sizeAscending",
  });
  expect(
    container.querySelector('.file-list [aria-selected="true"]'),
  ).toBeNull();
  expect(calls("get_map")).toHaveLength(mapCalls);
  await click("Clear filters");
  const input = container.querySelector<HTMLInputElement>(
    'input[aria-label="Search file names"]',
  )!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(input, "image");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => vi.advanceTimersByTimeAsync(250));
  expect(calls("search_nodes").at(-1)?.[1]).toMatchObject({
    query: "image",
    sort: "sizeAscending",
  });
  expect(calls("get_map")).toHaveLength(mapCalls);
});
it("keeps browsing controls active while one save is pending and retains active order on failure/retry", async () => {
  await mount();
  const previous = mock.invoke.getMockImplementation()!;
  let reject!: (reason: unknown) => void;
  mock.invoke.mockImplementation(
    (command: string, args: Record<string, unknown>) =>
      command === "set_explorer_sort"
        ? new Promise((_, fail) => {
            reject = fail;
          })
        : previous(command, args),
  );
  await choose("Sort order", "nameAscending");
  expect(sortButton("name").disabled).toBe(true);
  expect(sortButton("size").disabled).toBe(true);
  expect(
    container.querySelector<HTMLSelectElement>(
      'select[aria-label="File category"]',
    )!.disabled,
  ).toBe(false);
  await choose("Sort order", "nameDescending");
  expect(calls("set_explorer_sort")).toHaveLength(1);
  expect(calls("get_children").at(-1)?.[1].sort).toBe("nameAscending");
  await choose("File category", "images");
  await act(async () => reject(new Error("disk write failed")));
  await flush();
  expect(sortOrder()).toBe("nameAscending");
  expect(container.textContent).toContain(
    "Sort order is on now, but it couldn't be saved",
  );
  mock.invoke.mockImplementation(
    (command: string, args: Record<string, unknown>) =>
      command === "set_explorer_sort"
        ? Promise.resolve({ sort: args.sort, warning: null })
        : previous(command, args),
  );
  await click("Save current order");
  expect(calls("set_explorer_sort").at(-1)?.[1].sort).toBe("nameAscending");
  expect(container.querySelector(".sort-warning")).toBeNull();
});
it("falls back after preference load failure and preserves chosen order across navigation/rescan", async () => {
  const previous = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(
    (command: string, args: Record<string, unknown>) =>
      command === "get_explorer_preferences"
        ? Promise.reject(new Error("unavailable"))
        : previous(command, args),
  );
  await mount();
  expect(calls("get_children").at(-1)?.[1].sort).toBe("sizeDescending");
  expect(container.textContent).toContain(
    "Could not load the saved sort order",
  );
  await choose("Sort order", "nameDescending");
  const directory = Array.from(
    container.querySelectorAll<HTMLElement>('.file-list [role="treeitem"]'),
  ).find((row) => row.textContent?.includes("Photos"))!;
  await act(async () =>
    directory.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })),
  );
  await flush();
  expect(calls("get_children").at(-1)?.[1]).toMatchObject({
    nodeId: 1,
    sort: "nameDescending",
  });
  await click("Rescan Home");
  expect(calls("get_children").at(-1)?.[1]).toMatchObject({
    scanId: "live",
    sort: "nameDescending",
  });
});
it("ignores old list results after a sort change", async () => {
  await mount();
  const previous = mock.invoke.getMockImplementation()!;
  let finish!: (value: Page) => void;
  mock.invoke.mockImplementation(
    (command: string, args: Record<string, unknown>) =>
      command === "filter_nodes" && args.sort === "sizeDescending"
        ? new Promise<Page>((resolve) => {
            finish = resolve;
          })
        : previous(command, args),
  );
  await choose("File category", "images");
  await choose("Sort order", "sizeAscending");
  await act(async () =>
    finish({ items: [{ ...file, name: "stale.jpg" }], total: 1 }),
  );
  await flush();
  expect(container.textContent).not.toContain("stale.jpg");
  expect(container.textContent).toContain("image.jpg");
});
it("unlocks the list when Back restores a view before the saved preferences arrive", async () => {
  const previous = mock.invoke.getMockImplementation()!;
  let restore!: (value: ExplorerPreferences) => void;
  mock.invoke.mockImplementation(
    (command: string, args: Record<string, unknown>) =>
      command === "get_explorer_preferences"
        ? new Promise<ExplorerPreferences>((resolve) => {
            restore = resolve;
          })
        : previous(command, args),
  );
  await mount();
  await act(() => mock.app!.open(folder));
  await flush();
  await act(() => mock.app!.goBack());
  await flush();
  expect(calls("get_children")).toHaveLength(0);
  await act(async () => restore({ sort: "nameAscending", warning: null }));
  await flush();
  expect(mock.app!.sortReady).toBe(true);
  expect(calls("get_children").at(-1)?.[1].sort).toBe("sizeDescending");
  expect(sortOrder()).toBe("sizeDescending");
});

it("Unmounted stale preference response cannot overwrite a later choice or pending save", async () => {
  const previous = mock.invoke.getMockImplementation()!;
  const restore: ((value: ExplorerPreferences) => void)[] = [];
  let saved!: (value: ExplorerPreferences) => void;
  mock.invoke.mockImplementation(
    (command: string, args: Record<string, unknown>) =>
      command === "get_explorer_preferences"
        ? new Promise<ExplorerPreferences>((resolve) => restore.push(resolve))
        : command === "set_explorer_sort"
          ? new Promise<ExplorerPreferences>((resolve) => {
              saved = resolve;
            })
          : previous(command, args),
  );
  await mount(true);
  expect(restore).toHaveLength(2);
  await act(async () => restore[1]({ sort: "sizeDescending", warning: null }));
  await flush();
  await choose("Sort order", "sizeAscending");
  await act(async () => restore[0]({ sort: "nameDescending", warning: null }));
  await flush();
  expect(sortOrder()).toBe("sizeAscending");
  expect(sortButton("name").disabled).toBe(true);
  await act(async () => saved({ sort: "sizeAscending", warning: null }));
  await flush();
  expect(sortButton("name").disabled).toBe(false);
});

it("exposes a native preference warning and saves the current default on retry", async () => {
  const previous = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(
    (command: string, args: Record<string, unknown>) =>
      command === "get_explorer_preferences"
        ? Promise.resolve({
            sort: "sizeDescending",
            warning: "Stored preferences are corrupt. Using the default.",
          })
        : previous(command, args),
  );
  await mount();
  expect(container.textContent).toContain("Stored preferences are corrupt");
  expect(calls("get_children").at(-1)?.[1].sort).toBe("sizeDescending");
  await click("Save current order");
  expect(calls("set_explorer_sort").at(-1)?.[1].sort).toBe("sizeDescending");
  expect(container.querySelector(".sort-warning")).toBeNull();
});

it("retains the active order when a rescan replaces the scan", async () => {
  const previous = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(
    (command: string, args: Record<string, unknown>) =>
      command === "start_scan"
        ? Promise.resolve("next")
        : command === "scan_summary" && args.scanId === "next"
          ? Promise.resolve(summary("next"))
          : previous(command, args),
  );
  await mount();
  await choose("Sort order", "nameAscending");
  await click("Rescan Home");
  expect(calls("get_children").at(-1)?.[1]).toMatchObject({
    scanId: "next",
    sort: "nameAscending",
  });
});

it("shows committed preference warnings without claiming the save failed", async () => {
  await mount();
  const previous = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(
    (command: string, args: Record<string, unknown>) =>
      command === "set_explorer_sort"
        ? Promise.resolve({
            sort: args.sort,
            warning:
              "Sort order was saved, but directory durability could not be confirmed.",
          })
        : previous(command, args),
  );
  await choose("Sort order", "nameAscending");
  expect(sortOrder()).toBe("nameAscending");
  expect(container.textContent).toContain("Sort order was saved");
  expect(container.textContent).not.toContain("could not be saved");
  expect(
    container.querySelector<HTMLButtonElement>(".sort-warning button")!
      .disabled,
  ).toBe(false);
});

it("toggles name and size from the column headers", async () => {
  await mount();
  expect(sortOrder()).toBe("sizeDescending");
  await act(async () => sortButton("name").click());
  await flush();
  expect(sortOrder()).toBe("nameAscending");
  await act(async () => sortButton("name").click());
  await flush();
  expect(sortOrder()).toBe("nameDescending");
  await act(async () => sortButton("size").click());
  await flush();
  expect(sortOrder()).toBe("sizeDescending");
  await act(async () => sortButton("size").click());
  await flush();
  expect(sortOrder()).toBe("sizeAscending");
});

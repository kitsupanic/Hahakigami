// @vitest-environment jsdom
import { act, createTestRoot as createRoot, type Root, fullEdition } from "./testUtils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import App from "./App.svelte";
import { menuItem } from "./fakeMenu";
import type { NodeView, Page, ScanSummary } from "./types";
const mock = vi.hoisted(() => ({ invoke: vi.fn() }));
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
async function choose(name: string, value: string) {
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
    const node = container.querySelector<HTMLSelectElement>(
      `select[aria-label="${name}"]`,
    )!;
    node.value = value;
    node.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await flush();
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
          return args.nodeId === 1 ? folder : rootNode;
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
          return { items: [image], total: 1 };
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
it("keeps filter values and parent paths visible with the popover closed and shares map branch colors", async () => {
  await search("image");
  await advance();
  await choose("File category", "images");
  await choose("Size range", "1gb");
  const toggle = container.querySelector<HTMLButtonElement>(
    '[aria-controls="explorer-filters"]',
  )!;
  await act(() => toggle.click());
  const chips = container.querySelector('[aria-label="Active filters"]')!;
  expect(chips.textContent).toContain("Images");
  expect(chips.textContent).toContain("On disk ≥ 1 GiB");
  expect(chips.closest("[hidden]")).toBeNull();
  expect(container.querySelector(".file-parent-path")?.textContent).toBe(
    "/fixture/Photos",
  );
  const names = container.querySelectorAll<HTMLElement>(".file-row-name");
  const branchColor = names[0].style.getPropertyValue("--branch-color");
  expect(branchColor).not.toBe("");
  expect(branchColor).toBe(names[1].style.getPropertyValue("--branch-color"));
  await click("Remove category filter: Images");
  expect(calls("filter_nodes").at(-1)?.[1]).toMatchObject({
    category: "all",
    minBytes: 1024 ** 3,
    query: "image",
  });
  await click("Remove size filter: On disk ≥ 1 GiB");
  expect(container.querySelector('[aria-label="Active filters"]')).toBeNull();
  expect(calls("search_nodes").at(-1)?.[1].query).toBe("image");
  expect(container.querySelector(".file-parent-path")?.textContent).toBe(
    "/fixture/Photos",
  );
});
it("filters the entire scan, combines debounced query and preserves maps across filters/pagination", async () => {
  const mapCalls = calls("get_map").length;
  await choose("File category", "images");
  expect(calls("filter_nodes").at(-1)?.[1]).toMatchObject({
    scanId: "live",
    category: "images",
    query: "",
    minBytes: null,
    maxBytes: null,
    offset: 0,
  });
  expect(calls("filter_nodes").at(-1)?.[1]).not.toHaveProperty("nodeId");
  expect(container.querySelector(".list-heading")?.textContent).toContain(
    "Filtered results",
  );
  expect(container.querySelector("#results-scope")?.textContent).toContain(
    "Entire scan",
  );
  expect(container.textContent).toContain("Unfiltered storage map and totals");
  expect(
    container.querySelector('.file-list [role="treeitem"] [aria-expanded]'),
  ).toBeNull();
  await click("Next");
  expect(calls("filter_nodes").at(-1)?.[1].offset).toBe(150);
  expect(calls("get_map").length).toBe(mapCalls);
  await search("image");
  expect(container.querySelector('.file-list [role="treeitem"]')).toBeNull();
  expect(container.textContent).toContain("Loading results");
  await advance(249);
  expect(calls("filter_nodes").at(-1)?.[1].query).toBe("");
  await advance(1);
  expect(calls("filter_nodes").at(-1)?.[1]).toMatchObject({
    query: "image",
    offset: 0,
  });
  expect(calls("get_map").length).toBe(mapCalls);
});
it("filter changes reset page and selection, Clear filters preserves query", async () => {
  await choose("File category", "images");
  await click("Next");
  await act(async () =>
    container
      .querySelector<HTMLElement>('.file-list [role="treeitem"]')!
      .click(),
  );
  expect(
    container.querySelector('.file-list [aria-selected="true"]'),
  ).not.toBeNull();
  await search("photo");
  await advance();
  await click("Clear filters");
  expect(
    container.querySelector<HTMLInputElement>(
      'input[aria-label="Search file names"]',
    )!.value,
  ).toBe("photo");
  expect(calls("search_nodes").at(-1)?.[1]).toMatchObject({
    query: "photo",
    offset: 0,
  });
  await click("Clear filters");
  expect(
    container.querySelector('.file-list [role="treeitem"]'),
  ).not.toBeNull();
});
it("ignores a stale native result immediately while a newer raw query is still debouncing", async () => {
  await choose("File category", "images");
  const previous = mock.invoke.getMockImplementation()!;
  let resolve!: (page: Page) => void;
  mock.invoke.mockImplementation(
    (command: string, args: Record<string, unknown>) =>
      command === "filter_nodes" && args.query === "old"
        ? new Promise<Page>((done) => {
            resolve = done;
          })
        : previous(command, args),
  );
  await search("old");
  await advance();
  await search("new");
  await act(async () =>
    resolve({ items: [{ ...image, name: "stale-result.jpg" }], total: 1 }),
  );
  expect(container.textContent).not.toContain("stale-result.jpg");
  expect(container.querySelector('.file-list [role="treeitem"]')).toBeNull();
  await advance();
  expect(calls("filter_nodes").at(-1)?.[1].query).toBe("new");
  expect(container.textContent).toContain("image-hit.jpg");
});
it("directory navigation and a fresh rescan clear filters and names", async () => {
  await choose("File category", "images");
  await search("Photos");
  await advance();
  const directory = Array.from(
    container.querySelectorAll<HTMLElement>('.file-list [role="treeitem"]'),
  ).find((node) => node.querySelector(".file-name")?.textContent === "Photos")!;
  await act(async () =>
    directory.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })),
  );
  await flush();
  expect(
    container.querySelector<HTMLSelectElement>(
      'select[aria-label="File category"]',
    )!.value,
  ).toBe("all");
  expect(
    container.querySelector<HTMLInputElement>(
      'input[aria-label="Search file names"]',
    )!.value,
  ).toBe("");
  expect(calls("get_children").at(-1)?.[1].nodeId).toBe(1);
  await choose("Size range", "1gb");
  await flush();
  expect(menuItem("trash").enabled).toBe(false);
  await click("Rescan Home");
  expect(
    container.querySelector<HTMLSelectElement>(
      'select[aria-label="Size range"]',
    )!.value,
  ).toBe("any");
  expect(calls("start_scan").length).toBe(2);
});

it("retains existing rows and selection while a background revision is pending", async () => {
  await click("Rescan Home");
  const firstRow = container.querySelector<HTMLElement>(
    '.file-list [role="treeitem"]',
  )!;
  await act(async () => firstRow.click());
  const previous = mock.invoke.getMockImplementation()!;
  let resolve!: (page: Page) => void;
  mock.invoke.mockImplementation(
    (command: string, args: Record<string, unknown>) =>
      command === "get_children"
        ? new Promise<Page>((done) => {
            resolve = done;
          })
        : previous(command, args),
  );
  const channel = calls("start_scan").at(-1)![1].onEvent as {
    onmessage: (event: ScanSummary) => void;
  };
  await act(async () =>
    channel.onmessage({
      ...summary(),
      state: "running",
      elapsedMs: 2000,
      entries: 4,
    }),
  );
  expect(container.querySelector('.file-list [role="treeitem"]')).toBe(
    firstRow,
  );
  expect(firstRow.getAttribute("aria-selected")).toBe("true");
  await act(async () => resolve({ items: [folder, image], total: 2 }));
  expect(container.querySelector('.file-list [role="treeitem"]')).toBe(
    firstRow,
  );
});

it("does not re-run a live search every revision tick, only when the scan ends", async () => {
  await click("Rescan Home");
  await search("image");
  await advance();
  const channel = calls("start_scan").at(-1)![1].onEvent as {
    onmessage: (event: ScanSummary) => void;
  };
  // The first running event changes the state, which refreshes once; later ticks must not.
  await act(async () =>
    channel.onmessage({ ...summary(), state: "running", elapsedMs: 1000 }),
  );
  await flush();
  const before = calls("search_nodes").length;
  for (const elapsedMs of [2000, 3000, 4000])
    await act(async () =>
      channel.onmessage({ ...summary(), state: "running", elapsedMs }),
    );
  await flush();
  expect(calls("search_nodes").length - before).toBe(0);
  await act(async () =>
    channel.onmessage({ ...summary(), state: "completed", elapsedMs: 5000 }),
  );
  await flush();
  expect(calls("search_nodes").length - before).toBe(1);
});

it("keeps refreshing a plain folder listing on every live tick", async () => {
  await click("Rescan Home");
  await advance();
  const channel = calls("start_scan").at(-1)![1].onEvent as {
    onmessage: (event: ScanSummary) => void;
  };
  const before = calls("get_children").length;
  for (const elapsedMs of [2000, 3000])
    await act(async () =>
      channel.onmessage({ ...summary(), state: "running", elapsedMs }),
    );
  await flush();
  expect(calls("get_children").length - before).toBeGreaterThanOrEqual(2);
});

it("clamps a filtered page when live or final size aggregation shrinks its total", async () => {
  await click("Rescan Home");
  await choose("Size range", "custom");
  const maximum = container.querySelector<HTMLInputElement>(
    'input[aria-label="Maximum size"]',
  )!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(maximum, "1");
    maximum.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await click("Apply range");
  await click("Next");
  expect(calls("filter_nodes").at(-1)?.[1].offset).toBe(150);
  const previous = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(
    (command: string, args: Record<string, unknown>) =>
      command === "filter_nodes"
        ? Promise.resolve({ items: args.offset === 0 ? [image] : [], total: 1 })
        : previous(command, args),
  );
  const channel = calls("start_scan").at(-1)![1].onEvent as {
    onmessage: (event: ScanSummary) => void;
  };
  await act(async () =>
    channel.onmessage({
      ...summary(),
      state: "running",
      elapsedMs: 2000,
    }),
  );
  await flush();
  expect(calls("filter_nodes").at(-1)?.[1].offset).toBe(0);
  expect(container.querySelector(".pagination")).toBeNull();
  expect(container.querySelector(".list-heading")?.textContent).toContain(
    "1 item",
  );
  expect(container.querySelector(".list-heading")?.textContent).not.toContain(
    "1 items",
  );
  expect(
    container.querySelector('.file-list [role="treeitem"]'),
  ).not.toBeNull();
  mock.invoke.mockImplementation(
    (command: string, args: Record<string, unknown>) =>
      command === "filter_nodes"
        ? Promise.resolve({ items: [], total: 0 })
        : previous(command, args),
  );
  await act(async () =>
    channel.onmessage({ ...summary(), elapsedMs: 3000 }),
  );
  await flush();
  expect(container.querySelector(".pagination")).toBeNull();
  expect(container.querySelector(".list-heading")?.textContent).toContain(
    "0 items",
  );
});

it("opens filters from the toolbar and dismisses with Escape or an outside click", async () => {
  const toggle = container.querySelector<HTMLButtonElement>(
    '[aria-controls="explorer-filters"]',
  )!;
  const popover = container.querySelector<HTMLElement>("#explorer-filters")!;
  expect(popover.hidden).toBe(true);
  toggle.focus();
  await act(async () => toggle.click());
  expect(toggle.getAttribute("aria-expanded")).toBe("true");
  expect(popover.hidden).toBe(false);
  const category = popover.querySelector<HTMLSelectElement>(
    'select[aria-label="File category"]',
  )!;
  category.focus();
  await act(async () =>
    category.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    ),
  );
  expect(popover.hidden).toBe(true);
  expect(document.activeElement).toBe(toggle);
  await act(async () => toggle.click());
  await act(async () =>
    container.dispatchEvent(new Event("pointerdown", { bubbles: true })),
  );
  expect(popover.hidden).toBe(true);
  expect(toggle.getAttribute("aria-expanded")).toBe("false");
});

it("keeps the filter popover within the visible workspace when its trigger is near the bottom", async () => {
  const trigger = container.querySelector<HTMLButtonElement>(
    '[aria-controls="explorer-filters"]',
  )!;
  const main = container.querySelector("main")!;
  Object.defineProperty(window, "innerWidth", {
    configurable: true,
    value: 900,
  });
  Object.defineProperty(window, "innerHeight", {
    configurable: true,
    value: 600,
  });
  vi.spyOn(main, "getBoundingClientRect").mockReturnValue({
    left: 208,
  } as DOMRect);
  vi.spyOn(trigger, "getBoundingClientRect").mockReturnValue({
    right: 885,
    bottom: 590,
  } as DOMRect);
  try {
    await act(() => trigger.click());
    const popover = container.querySelector<HTMLElement>("#explorer-filters")!;
    expect(parseFloat(popover.style.left)).toBeGreaterThanOrEqual(220);
    expect(
      parseFloat(popover.style.left) + parseFloat(popover.style.width),
    ).toBeLessThanOrEqual(888);
    expect(parseFloat(popover.style.top)).toBeGreaterThanOrEqual(12);
    expect(
      parseFloat(popover.style.top) + parseFloat(popover.style.maxHeight),
    ).toBeLessThanOrEqual(588);
    await act(() => main.dispatchEvent(new Event("scroll")));
    expect(popover.hidden).toBe(true);
  } finally {
    vi.restoreAllMocks();
    Object.defineProperty(window, "innerWidth", {
      configurable: true,
      value: 1024,
    });
    Object.defineProperty(window, "innerHeight", {
      configurable: true,
      value: 768,
    });
  }
});

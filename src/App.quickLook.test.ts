// @vitest-environment jsdom
import { act, createTestRoot as createRoot, type Root } from "./testUtils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import App from "./App.svelte";
import { menuItem } from "./fakeMenu";
import type { NodeView, ScanSummary } from "./types";
import type { QuickLookStatus } from "./quickLookTypes";
const mock = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  unlisten: vi.fn(),
  callbacks: [] as ((event: { payload: QuickLookStatus }) => void)[],
  scanState: "completed" as ScanSummary["state"],
  revision: 0,
}));
vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => true,
  invoke: mock.invoke,
  Channel: class {
    onmessage = () => {};
  },
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: mock.listen }));
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
  childrenCount: 5,
  incomplete: true,
};
const file: NodeView = {
  ...node,
  id: 1,
  parentId: 0,
  name: "report.pdf",
  path: "/fixture/report.pdf",
  kind: "file",
  childrenCount: 0,
};
const other: NodeView = {
  ...file,
  id: 2,
  name: "other.pdf",
  path: "/fixture/other.pdf",
};
const folder: NodeView = {
  ...node,
  id: 3,
  parentId: 0,
  name: "Folder",
  path: "/fixture/Folder",
  childrenCount: 0,
};
const link: NodeView = { ...file, id: 4, name: "link", kind: "symlink" };
const excluded: NodeView = {
  ...file,
  id: 5,
  name: "excluded.pdf",
  excluded: true,
};
const summary = (): ScanSummary => ({
  scanId: "scan",
  rootId: 0,
  state: mock.scanState,
  entries: 6,
  workers: 1,
  workerStats: [],
  finalizationUs: 0,
  allocatedBytes: 100,
  elapsedMs: 1000,
  errors: 2,
  incomplete: true,
});
let container: HTMLDivElement, root: Root;
let unmounted = false;
function calls(command: string) {
  return mock.invoke.mock.calls.filter(([name]) => name === command);
}
async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}
function button(name: string) {
  const item = Array.from(container.querySelectorAll("button")).find(
    (node) =>
      node.getAttribute("aria-label") === name ||
      node.textContent?.trim() === name ||
      (name === "Scan location" && node.classList.contains("welcome-scan")),
  );
  expect(item, name).toBeTruthy();
  return item!;
}
async function click(name: string) {
  await act(async () => button(name).click());
  await flush();
}
function row(name: string) {
  const item = Array.from(
    container.querySelectorAll<HTMLElement>('.file-list [role="treeitem"]'),
  ).find((item) => item.textContent?.includes(name));
  expect(item, name).toBeTruthy();
  return item!;
}
async function select(name: string, multi = false) {
  await act(async () =>
    row(name).dispatchEvent(
      new MouseEvent("click", { bubbles: true, ctrlKey: multi }),
    ),
  );
  await flush();
}
async function space(name: string, init: KeyboardEventInit = {}) {
  await act(async () =>
    row(name).dispatchEvent(
      new KeyboardEvent("keydown", { key: " ", bubbles: true, ...init }),
    ),
  );
  await flush();
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
    select.value = value;
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await flush();
}
async function chooseSort(value: string) {
  const head = () => container.querySelector<HTMLElement>(".table-head")!;
  for (let attempt = 0; attempt < 2 && head().dataset.sort !== value; attempt++) {
    const header = head().querySelector<HTMLButtonElement>(
      `button[aria-label="Sort by ${value.startsWith("name") ? "name" : "size"}"]`,
    )!;
    if (header.disabled) return;
    await act(async () => header.click());
    await flush();
  }
}
async function mount() {
  await act(async () => root.render(App));
  await flush();
  await click("Scan location");
}
async function event(value: QuickLookStatus) {
  mock.revision = Math.max(mock.revision, value.revision);
  await act(async () =>
    mock.callbacks.forEach((callback) => callback({ payload: value })),
  );
  await flush();
}
beforeEach(() => {
  vi.useFakeTimers();
  mock.invoke.mockReset();
  mock.listen.mockReset();
  mock.unlisten.mockReset();
  mock.callbacks.length = 0;
  mock.scanState = "completed";
  mock.revision = 0;
  mock.listen.mockImplementation(
    async (
      _name: string,
      callback: (event: { payload: QuickLookStatus }) => void,
    ) => {
      mock.callbacks.push(callback);
      return () => {
        mock.unlisten();
        const index = mock.callbacks.indexOf(callback);
        if (index >= 0) mock.callbacks.splice(index, 1);
      };
    },
  );
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
            },
          ];
        case "start_scan":
          return "scan";
        case "scan_summary":
          return summary();
        case "get_node":
          return args.nodeId === 3 ? folder : node;
        case "get_children":
          return {
            items:
              args.nodeId === 3 ? [] : [file, other, folder, link, excluded],
            total: args.nodeId === 3 ? 0 : 5,
          };
        case "filter_nodes":
          return { items: [file, other, folder], total: 320 };
        case "get_map":
          return { nodes: [node, file, other, folder] };
        case "search_nodes":
          return { items: [file], total: 1 };
        case "quick_look_node":
          return {
            open: true,
            scanId: args.scanId,
            nodeId: args.nodeId,
            revision: ++mock.revision,
          };
        case "close_quick_look":
          return {
            open: false,
            scanId: null,
            nodeId: null,
            revision: ++mock.revision,
          };
        case "prepare_trash":
          return { token: "review", items: [file] };
        default:
          return undefined;
      }
    },
  );
  unmounted = false;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  if (!unmounted) await act(async () => root.unmount());
  await flush();
  container.remove();
  vi.useRealTimers();
});

it("previews exactly one completed live regular file despite incomplete coverage, and preserves explorer data", async () => {
  await mount();
  const maps = calls("get_map").length;
  expect(button("Quick Look").disabled).toBe(true);
  await select("report.pdf");
  expect(button("Quick Look").disabled).toBe(false);
  await click("Quick Look");
  expect(calls("quick_look_node").at(-1)?.[1]).toEqual({
    scanId: "scan",
    nodeId: 1,
  });
  expect(button("Close Quick Look").disabled).toBe(false);
  expect(calls("get_map")).toHaveLength(maps);
  expect(container.textContent).toContain("Scan complete");
  expect(container.querySelector<HTMLElement>(".table-head")!.dataset.sort).toBe(
    "sizeDescending",
  );
  await click("Close Quick Look");
  expect(button("Quick Look").disabled).toBe(false);
});
it("offers Reveal in Finder and Quick Look as labelled icon-only buttons beside the queue action", async () => {
  await mount();
  await select("report.pdf");
  const strip = container.querySelector("#selection-inspector")!;
  for (const name of ["Reveal in Finder", "Quick Look"]) {
    const icon = strip.querySelector<HTMLButtonElement>(
      `button[aria-label="${name}"]`,
    )!;
    expect(icon, name).toBeTruthy();
    expect(icon.textContent?.trim()).toBe("");
    expect(icon.title).toBe(name);
    expect(icon.querySelector("svg")).not.toBeNull();
  }
  await click("Quick Look");
  const close = strip.querySelector<HTMLButtonElement>(
    'button[aria-label="Close Quick Look"]',
  )!;
  expect(close.title).toBe("Close Quick Look");
});
it.each(["running", "cancelled", "failed"] as const)(
  "blocks %s scans",
  async (state) => {
    mock.scanState = state;
    await mount();
    await select("report.pdf");
    expect(button("Quick Look").disabled).toBe(true);
    await space("report.pdf");
    expect(calls("quick_look_node")).toHaveLength(0);
  },
);
it("keeps Space selection behavior for folders, symlinks, exclusions and multi-selection", async () => {
  await mount();
  for (const name of ["Folder", "link", "excluded.pdf"]) {
    await space(name);
    expect(button("Quick Look").disabled).toBe(true);
  }
  await select("report.pdf");
  await select("other.pdf", true);
  await space("other.pdf");
  expect(calls("quick_look_node")).toHaveLength(0);
  expect(
    container.querySelectorAll('.file-list [aria-selected="true"]'),
  ).toHaveLength(1);
});
it("Space selects and previews its file without a self-selection closure, ignoring modifiers and dialogs", async () => {
  await mount();
  for (const modifier of [
    { ctrlKey: true },
    { metaKey: true },
    { altKey: true },
    { shiftKey: true },
    { repeat: true },
  ])
    await space("report.pdf", modifier);
  expect(calls("quick_look_node")).toHaveLength(0);
  await space("report.pdf");
  expect(button("Close Quick Look").disabled).toBe(false);
  expect(calls("close_quick_look")).toHaveLength(0);
  await space("other.pdf");
  expect(calls("quick_look_node").at(-1)?.[1].nodeId).toBe(2);
  expect(calls("close_quick_look").length).toBeGreaterThan(0);
  await click("Settings");
  const opens = calls("quick_look_node").length;
  await space("report.pdf");
  expect(calls("quick_look_node")).toHaveLength(opens);
  expect(button("Quick Look").disabled).toBe(true);
});
it("native Escape/close events update controls and older status events stay ignored", async () => {
  await mount();
  await select("report.pdf");
  await click("Quick Look");
  const openedRevision = mock.revision;
  await event({
    open: false,
    scanId: null,
    nodeId: null,
    revision: openedRevision + 1,
  });
  expect(button("Quick Look").disabled).toBe(false);
  await event({
    open: true,
    scanId: "scan",
    nodeId: 1,
    revision: openedRevision,
  });
  expect(button("Quick Look").disabled).toBe(false);
});
it("cancels a pending opening on query change and ignores its late response without discarding results", async () => {
  await mount();
  await select("report.pdf");
  const previous = mock.invoke.getMockImplementation()!;
  let finish!: (value: QuickLookStatus) => void;
  mock.invoke.mockImplementation(
    (command: string, args: Record<string, unknown>) =>
      command === "quick_look_node"
        ? new Promise<QuickLookStatus>((resolve) => {
            finish = resolve;
          })
        : previous(command, args),
  );
  await click("Quick Look");
  expect(button("Opening Quick Look…").disabled).toBe(true);
  expect(button("Rescan Home").disabled).toBe(true);
  const input = container.querySelector<HTMLInputElement>(
    'input[aria-label="Search file names"]',
  )!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(input, "report");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await flush();
  mock.revision = 20;
  await act(async () =>
    finish({ open: true, scanId: "scan", nodeId: 1, revision: 20 }),
  );
  await flush();
  await act(async () => vi.advanceTimersByTimeAsync(250));
  expect(container.querySelector('[aria-label="Close Quick Look"]')).toBeNull();
  expect(container.textContent).toContain("Scan complete");
  expect(calls("close_quick_look").length).toBeGreaterThan(0);
  expect(calls("quick_look_node")).toHaveLength(1);
});
it("closes on selection, filters, sort, pagination, folder navigation and dialog changes", async () => {
  await mount();
  await select("report.pdf");
  await click("Quick Look");
  await select("other.pdf");
  expect(container.querySelector('[aria-label="Close Quick Look"]')).toBeNull();
  await click("Quick Look");
  await choose("File category", "documents");
  expect(container.querySelector('[aria-label="Close Quick Look"]')).toBeNull();
  await select("report.pdf");
  await click("Quick Look");
  await choose("Sort order", "nameAscending");
  expect(container.querySelector('[aria-label="Close Quick Look"]')).toBeNull();
  await select("report.pdf");
  await click("Quick Look");
  await click("Next");
  expect(container.querySelector('[aria-label="Close Quick Look"]')).toBeNull();
  await select("report.pdf");
  await click("Quick Look");
  await act(async () =>
    row("Folder").dispatchEvent(new MouseEvent("dblclick", { bubbles: true })),
  );
  await flush();
  expect(container.querySelector('[aria-label="Close Quick Look"]')).toBeNull();
  expect(calls("close_quick_look").length).toBeGreaterThanOrEqual(5);
});
it("reports preview errors and permits recovery without resetting the scan", async () => {
  await mount();
  await select("report.pdf");
  const previous = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(
    (command: string, args: Record<string, unknown>) =>
      command === "quick_look_node"
        ? Promise.reject(
            new Error("File identity changed; rescan this location."),
          )
        : previous(command, args),
  );
  await click("Quick Look");
  expect(container.textContent).toContain("File identity changed");
  expect(container.textContent).toContain("Scan complete");
  expect(button("Quick Look").disabled).toBe(false);
});

it("closes for the Trash dialog and native close failures are retryable", async () => {
  await mount();
  await select("report.pdf");
  await click("Quick Look");
  await act(async () => menuItem("trash").click());
  await flush();
  expect(container.querySelector('[aria-label="Close Quick Look"]')).toBeNull();
  const cancelTrash = Array.from(
    container.querySelectorAll<HTMLButtonElement>('[role="dialog"] button'),
  ).find((button) => button.textContent?.trim() === "Cancel")!;
  await act(async () => cancelTrash.click());
  await flush();
  await click("Quick Look");
  const previous = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(
    (command: string, args: Record<string, unknown>) =>
      command === "close_quick_look"
        ? Promise.reject(new Error("Native close unavailable"))
        : previous(command, args),
  );
  await click("Close Quick Look");
  expect(container.textContent).toContain("Native close unavailable");
  mock.invoke.mockImplementation(previous);
  await click("Dismiss preview");
  expect(container.textContent).not.toContain("Native close unavailable");
  expect(container.textContent).toContain("Scan complete");
});

it("cleans up listeners even when asynchronous registration resolves after unmount", async () => {
  let registered!: (unlisten: () => void) => void;
  mock.listen.mockImplementation(
    () =>
      new Promise<() => void>((resolve) => {
        registered = resolve;
      }),
  );
  await mount();
  await act(async () => root.unmount());
  unmounted = true;
  await act(async () => registered(mock.unlisten));
  expect(mock.unlisten).toHaveBeenCalledOnce();
});

it("does not open another file after prior panel closure fails, and recovery preserves the new selection", async () => {
  await mount();
  await select("report.pdf");
  await click("Quick Look");
  const previous = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(
    (command: string, args: Record<string, unknown>) =>
      command === "close_quick_look"
        ? Promise.reject(new Error("Physical panel closure failed"))
        : previous(command, args),
  );
  await space("other.pdf");
  expect(calls("quick_look_node")).toHaveLength(1);
  expect(container.textContent).toContain(
    "Could not close the previous preview",
  );
  expect(row("other.pdf").getAttribute("aria-selected")).toBe("true");
  expect(container.textContent).toContain("Scan complete");
  mock.invoke.mockImplementation(previous);
  await click("Dismiss preview");
  await click("Quick Look");
  expect(calls("quick_look_node")).toHaveLength(2);
  expect(calls("quick_look_node").at(-1)?.[1].nodeId).toBe(2);
});

it("a second Space closes the same visible file without requesting another preview", async () => {
  await mount();
  await space("report.pdf");
  expect(button("Close Quick Look").disabled).toBe(false);
  await space("report.pdf");
  expect(calls("quick_look_node")).toHaveLength(1);
  expect(calls("close_quick_look")).toHaveLength(1);
  expect(button("Quick Look").disabled).toBe(false);
  expect(row("report.pdf").getAttribute("aria-selected")).toBe("true");
});

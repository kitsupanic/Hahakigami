// @vitest-environment jsdom
import { act, createTestRoot, type Root } from "./testUtils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import App from "./App.svelte";
import type {
  LibraryProgress,
  NodeView,
  SavedEntry,
  ScanSummary,
} from "./types";

const mock = vi.hoisted(() => ({ invoke: vi.fn() }));
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

const rootNode: NodeView = {
  id: 0,
  parentId: null,
  name: "Home",
  path: "/fixture",
  kind: "directory",
  allocatedBytes: 100,
  logicalBytes: 100,
  childrenCount: 1,
  incomplete: false,
};
const file: NodeView = {
  ...rootNode,
  id: 1,
  parentId: 0,
  name: "keep.txt",
  path: "/fixture/keep.txt",
  kind: "file",
  childrenCount: 0,
};
const saved: SavedEntry = {
  id: "saved",
  name: "Home history",
  rootPath: "/fixture",
  startedAtMs: 1,
  completedAtMs: 2,
  savedAtMs: 3,
  entries: 2,
  allocatedBytes: 100,
  logicalBytes: 100,
  incomplete: false,
  errors: 0,
  excluded: 0,
  fileBytes: 80,
};
const summary: ScanSummary = {
  scanId: "history",
  rootId: 0,
  origin: "saved",
  savedId: "saved",
  workers: 1,
  workerStats: [],
  finalizationUs: 0,
  state: "completed",
  entries: 2,
  allocatedBytes: 100,
  logicalBytes: 100,
  elapsedMs: 1,
  errors: 0,
};
function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
let container: HTMLDivElement, root: Root;
async function mount() {
  await act(() => root.render(App));
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}
beforeEach(() => {
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(async (command: string) => {
    switch (command) {
      case "get_explorer_preferences":
        return { sort: "sizeDescending", warning: null };
      case "full_disk_access_status":
        return {
          status: "on",
          scanAnyway: false,
          appPath: "/Applications/Fixture.app",
        };
      case "list_saved_scans":
        return { entries: [saved], lastOpenedId: saved.id };
      case "begin_saved_open":
        return "request-1";
      case "cancel_saved_open":
        return true;
      case "open_saved_scan":
        return summary;
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
        return rootNode;
      case "get_children":
        return { items: [file], total: 1 };
      case "get_map":
        return { nodes: [rootNode, file] };
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

it("shows library and restore phases instead of onboarding, then opens the explorer", async () => {
  const list = deferred<{ entries: SavedEntry[]; lastOpenedId: string }>();
  const opening = deferred<ScanSummary>();
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "list_saved_scans"
      ? list.promise
      : command === "open_saved_scan"
        ? opening.promise
        : original(command, args),
  );
  await mount();
  expect(container.querySelector(".saved-scan-loading")?.textContent).toContain(
    "Loading saved scans",
  );
  expect(container.querySelector(".welcome")).toBeNull();
  await act(() => list.resolve({ entries: [saved], lastOpenedId: saved.id }));
  const state = container.querySelector(".saved-scan-loading")!;
  expect(state.textContent).toContain("Opening saved scan");
  expect(state.textContent).toContain(saved.name);
  expect(state.textContent).toContain(saved.rootPath);
  expect(state.textContent).toContain("0 B");
  expect(container.querySelector(".library-progress")).toBeNull();
  const progress = mock.invoke.mock.calls.find(
    ([command]) => command === "open_saved_scan",
  )![1].onProgress as { onmessage: (progress: LibraryProgress) => void };
  await act(() =>
    progress.onmessage({ operation: "open", phase: "validating", bytes: 4096 }),
  );
  expect(state.querySelector('[role="status"]')?.textContent).toContain(
    "Validating saved results",
  );
  expect(state.textContent).toContain("4 KiB");
  await act(() => opening.resolve(summary));
  await act(() => new Promise((resolve) => setTimeout(resolve, 0)));
  expect(container.querySelector(".saved-scan-loading")).toBeNull();
  expect(container.querySelector(".file-list")?.textContent).toContain(
    "keep.txt",
  );
});

it("returns to first-use content with a recoverable error if restoration fails", async () => {
  const opening = deferred<ScanSummary>();
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "open_saved_scan" ? opening.promise : original(command, args),
  );
  await mount();
  expect(container.querySelector(".saved-scan-loading")).not.toBeNull();
  await act(async () => {
    opening.reject(new Error("Archive is damaged"));
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  expect(container.querySelector(".saved-scan-loading")).toBeNull();
  expect(container.querySelector('[role="dialog"] h2')?.textContent).toContain(
    "Couldn’t open the saved scan",
  );
  expect(
    container.querySelector("[role=dialog] details")?.textContent,
  ).toContain("Archive is damaged");
  expect(container.querySelector(".welcome")).not.toBeNull();
});

it("retains displayed results and selection while another archive opens and after failure", async () => {
  await mount();
  const row = container.querySelector<HTMLElement>(
    '.file-list [role="treeitem"]',
  )!;
  await act(() => row.click());
  expect(
    container.querySelector<HTMLElement>("#selection-inspector")?.hidden,
  ).toBe(false);
  expect(
    container.querySelector(".scan-status-line .status-chip")?.textContent,
  ).toContain("Saved scan");
  expect(container.textContent).not.toContain("Read-only");
  const opening = deferred<ScanSummary>();
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "open_saved_scan" ? opening.promise : original(command, args),
  );
  await act(() =>
    container.querySelector<HTMLButtonElement>(".saved-summary")!.click(),
  );
  expect(container.querySelector(".saved-scan-loading")).toBeNull();
  expect(container.querySelector('.file-list [aria-selected="true"]')).toBe(
    row,
  );
  expect(container.querySelector(".library-progress")?.textContent).toContain(
    "Opening saved scan",
  );
  expect(
    container.querySelector<HTMLButtonElement>("header .rescan-action")
      ?.disabled,
  ).toBe(true);
  await act(async () => {
    opening.reject(new Error("Cannot open archive"));
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  expect(container.querySelector('.file-list [aria-selected="true"]')).toBe(
    row,
  );
  expect(
    container.querySelector("[role=dialog] details")?.textContent,
  ).toContain("Cannot open archive");
  expect(
    container.querySelector<HTMLElement>("#selection-inspector")?.hidden,
  ).toBe(false);
  expect(
    container.querySelector(".scan-status-line .status-chip")?.textContent,
  ).toContain("Saved scan");
  expect(container.textContent).not.toContain("Read-only");
  expect(
    container.querySelector<HTMLButtonElement>("header .rescan-action")
      ?.disabled,
  ).toBe(false);
});

it("queues cancellation before request registration and stays busy until backend settles", async () => {
  const registration = deferred<string>();
  const opening = deferred<ScanSummary>();
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "begin_saved_open"
      ? registration.promise
      : command === "open_saved_scan"
        ? opening.promise
        : original(command, args),
  );
  await mount();
  const cancel = container.querySelector<HTMLButtonElement>(".loading-cancel")!;
  expect(cancel).not.toBeNull();
  await act(() => cancel.click());
  expect(
    container.querySelector<HTMLButtonElement>(".loading-cancel")?.disabled,
  ).toBe(true);
  expect(container.querySelector('[role="status"]')?.textContent).toContain(
    "Stopping saved scan opening",
  );
  expect(
    mock.invoke.mock.calls.some(([command]) => command === "cancel_saved_open"),
  ).toBe(false);
  await act(async () => {
    registration.resolve("early-request");
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  expect(mock.invoke).toHaveBeenCalledWith("cancel_saved_open", {
    requestId: "early-request",
  });
  expect(mock.invoke).toHaveBeenCalledWith(
    "open_saved_scan",
    expect.objectContaining({ requestId: "early-request" }),
  );
  expect(container.querySelector(".saved-scan-loading")).not.toBeNull();
  await act(async () => {
    opening.reject("Saved scan opening cancelled.");
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  expect(container.querySelector(".saved-scan-loading")).toBeNull();
  expect(container.querySelector(".welcome")).not.toBeNull();
  expect(container.querySelector('[role="alert"]')).toBeNull();
});

it("loads committed results when completion wins a late cancellation", async () => {
  const opening = deferred<ScanSummary>();
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "open_saved_scan"
      ? opening.promise
      : command === "cancel_saved_open"
        ? Promise.resolve(false)
        : original(command, args),
  );
  await mount();
  await act(() =>
    container.querySelector<HTMLButtonElement>(".loading-cancel")!.click(),
  );
  expect(container.querySelector(".saved-scan-loading")).not.toBeNull();
  await act(async () => {
    opening.resolve(summary);
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  expect(container.querySelector(".saved-scan-loading")).toBeNull();
  expect(container.querySelector(".file-list")?.textContent).toContain(
    "keep.txt",
  );
  expect(container.querySelector('.error[role="alert"]')).toBeNull();
});

it("keeps the previous workspace and selection after cancelling another open", async () => {
  await mount();
  const row = container.querySelector<HTMLElement>(
    '.file-list [role="treeitem"]',
  )!;
  await act(() => row.click());
  const opening = deferred<ScanSummary>();
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "open_saved_scan" ? opening.promise : original(command, args),
  );
  await act(async () => {
    container.querySelector<HTMLButtonElement>(".saved-summary")!.click();
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  const cancel = container.querySelector<HTMLButtonElement>(
    ".library-progress button",
  )!;
  expect(cancel.textContent).toContain("Cancel opening");
  await act(() => cancel.click());
  expect(cancel.disabled).toBe(true);
  expect(container.querySelector('.file-list [aria-selected="true"]')).toBe(
    row,
  );
  expect(
    container.querySelector<HTMLButtonElement>("header .rescan-action")
      ?.disabled,
  ).toBe(true);
  await act(async () => {
    opening.reject("Saved scan opening cancelled.");
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  expect(container.querySelector(".library-progress")).toBeNull();
  expect(container.querySelector('.file-list [aria-selected="true"]')).toBe(
    row,
  );
  expect(
    container.querySelector<HTMLButtonElement>("header .rescan-action")
      ?.disabled,
  ).toBe(false);
  expect(container.querySelector('.error[role="alert"]')).toBeNull();
});

// @vitest-environment jsdom
import { act, createTestRoot, type Root } from "./testUtils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import App from "./App.svelte";
import { menuItem } from "./fakeMenu";
import type { QueueView } from "./cleanupTypes";
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
const location = {
  id: "home",
  name: "Home",
  path: "/fixture",
  totalBytes: null,
  freeBytes: null,
};
const node = {
  id: 0,
  parentId: null,
  name: "Home",
  path: "/fixture",
  kind: "directory",
  allocatedBytes: 100,
  childrenCount: 1,
  incomplete: false,
};
const file = {
  ...node,
  id: 1,
  parentId: 0,
  name: "photo.jpg",
  path: "/fixture/photo.jpg",
  kind: "file",
  childrenCount: 0,
};
const summary = {
  scanId: "live",
  state: "completed",
  rootId: 0,
  workers: 1,
  workerStats: [],
  finalizationUs: 0,
  entries: 2,
  allocatedBytes: 100,
  elapsedMs: 10,
  errors: 0,
};
let queue: QueueView, container: HTMLDivElement, root: Root;
async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}
async function click(text: string) {
  const button = [...container.querySelectorAll("button")].find(
    (b) =>
      b.textContent?.trim() === text ||
      b.getAttribute("aria-label") === text ||
      (text === "Scan location" && b.classList.contains("welcome-scan")),
  )!;
  expect(button).toBeTruthy();
  await act(() => button.click());
  await flush();
}
const page = () => container.querySelector(".cleanup-page")!;
const buttonLabels = (scope: Element) =>
  [...scope.querySelectorAll("button")]
    .map((b) => b.textContent?.trim())
    .filter((label) => label && !label.startsWith("Remove"));
beforeEach(async () => {
  queue = {
    entries: [
      {
        id: "q1",
        name: "photo.jpg",
        path: file.path,
        rootPath: "/fixture",
        allocatedBytes: 100,
        status: "ready",
        reason: null,
        matchedScanId: "live",
      },
    ],
    revision: 1,
    blocked: false,
    warning: null,
  };
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(async (command, args) => {
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
        return [location];
      case "start_scan":
        return "live";
      case "scan_summary":
        return summary;
      case "get_node":
        return node;
      case "get_map":
        return { nodes: [file] };
      case "get_children":
      case "search_nodes":
        return { items: [file], total: 1 };
      case "list_cleanup_queue":
        return queue;
      case "add_cleanup_queue":
        return queue;
      case "remove_cleanup_queue":
      case "clear_cleanup_queue":
        queue = { ...queue, entries: [], revision: queue.revision + 1 };
        return queue;
      case "cleanup_queue_target":
        return location;
      case "prepare_cleanup_queue":
        return { token: "ticket", items: [file] };
      case "commit_cleanup_queue":
        return {
          trashed: 1,
          failures: [],
          persistenceWarning: null,
          outcomes: [{ reviewIndex: 0, status: "trashed" }],
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
});
afterEach(async () => {
  await root.unmount();
  container.remove();
});
it("shows saved candidates before a scan and removes them without touching files", async () => {
  await click("Cleanup queue · 1");
  expect(
    container.querySelector(".cleanup-entry .cleanup-figure"),
  ).not.toBeNull();
  await act(() =>
    container.querySelector<HTMLButtonElement>(".cleanup-remove")!.click(),
  );
  expect(container.textContent).toContain("Your cleanup queue is empty");
  expect(mock.invoke).toHaveBeenCalledWith("remove_cleanup_queue", {
    entryIds: ["q1"],
  });
  expect(
    mock.invoke.mock.calls.some(([c]) => c === "commit_cleanup_queue"),
  ).toBe(false);
});
it("distinguishes the whole queue from ready items in the current scan", async () => {
  queue = {
    ...queue,
    entries: [
      queue.entries[0],
      {
        ...queue.entries[0],
        id: "changed",
        path: "/fixture/old/photo.jpg",
        status: "changed",
        reason: "Changed since it was queued",
      },
      {
        ...queue.entries[0],
        id: "external",
        rootPath: "/external",
        path: "/external/photo.jpg",
        status: "needsScan",
        reason: "Scan the original location",
      },
    ],
  };
  await click("Scan location");
  await click("Cleanup queue · 3");
  const tally = container
    .querySelector(".cleanup-tally")
    ?.textContent?.replace(/\s+/g, " ")
    .trim();
  expect(tally).toMatch(/^3 items · .+ on disk · 2 not ready$/);
  expect(container.querySelectorAll(".cleanup-group")).toHaveLength(2);
  expect(page().textContent).not.toMatch(/B to review/);
  // Each disk carries its own action: the one the scan covers reviews, the other asks for a scan.
  const groups = container.querySelectorAll(".cleanup-group");
  expect(buttonLabels(groups[0])).toEqual(["Review 1 ready", "Scan Home"]);
  expect(buttonLabels(groups[1])).toEqual(["Scan external"]);
  expect(
    container.querySelector('[data-status="changed"]')?.textContent,
  ).toContain("Changed since it was queued");
  await click("Review 1 ready");
  expect(mock.invoke).toHaveBeenCalledWith("prepare_cleanup_queue", {
    scanId: "live",
  });
  const dialog = container.querySelector('[role="dialog"]')!;
  expect(dialog.querySelectorAll(".trash-item")).toHaveLength(1);
  expect(dialog.textContent).toContain(file.path);
  expect(dialog.textContent).not.toContain("/external/photo.jpg");
  expect(
    mock.invoke.mock.calls.some(
      ([command]) => command === "commit_cleanup_queue",
    ),
  ).toBe(false);
});
it("moves focus to the next row's Remove, then the previous, then Back, after a removal", async () => {
  const at = (id: string) => ({
    ...queue.entries[0],
    id,
    name: id,
    path: `/fixture/${id}`,
  });
  queue = { ...queue, entries: ["a", "b", "c"].map(at) };
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "remove_cleanup_queue") {
      queue = {
        ...queue,
        entries: queue.entries.filter(
          (entry) => !args.entryIds.includes(entry.id),
        ),
        revision: queue.revision + 1,
      };
      return queue;
    }
    return original(command, args);
  });
  await click("Scan location");
  await click("Cleanup queue · 3");
  const remove = (id: string) =>
    container.querySelector<HTMLButtonElement>(
      `[aria-label="Remove /fixture/${id} from cleanup queue"]`,
    )!;
  remove("b").focus();
  await click("Remove /fixture/b from cleanup queue");
  expect(document.activeElement).toBe(remove("c"));
  await click("Remove /fixture/c from cleanup queue");
  expect(document.activeElement).toBe(remove("a"));
  await click("Remove /fixture/a from cleanup queue");
  expect(document.activeElement).toBe(
    container.querySelector(".cleanup-back"),
  );
});
it("disambiguates same-named queue removals by full path and preserves other candidates", async () => {
  queue = {
    ...queue,
    entries: [
      queue.entries[0],
      {
        ...queue.entries[0],
        id: "external",
        rootPath: "/external",
        path: "/external/photo.jpg",
        status: "needsScan",
      },
    ],
  };
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "remove_cleanup_queue") {
      queue = {
        ...queue,
        entries: queue.entries.filter(
          (entry) => !args.entryIds.includes(entry.id),
        ),
        revision: queue.revision + 1,
      };
      return queue;
    }
    return original(command, args);
  });
  await click("Cleanup queue · 1");
  await click("Remove /external/photo.jpg from cleanup queue");
  expect(mock.invoke).toHaveBeenCalledWith("remove_cleanup_queue", {
    entryIds: ["external"],
  });
  expect(queue.entries.map((entry) => entry.id)).toEqual(["q1"]);
  expect(
    mock.invoke.mock.calls.some(([command]) =>
      /prepare.*trash|commit.*trash|commit_cleanup/.test(command),
    ),
  ).toBe(false);
});
it("says a shared folder once and groups rows under the folder that differs", async () => {
  const at = (id: string, path: string, allocatedBytes: number) => ({
    ...queue.entries[0],
    id,
    name: "coverage.ndjson",
    path,
    allocatedBytes,
  });
  queue = {
    ...queue,
    entries: [
      at("a", "/fixture/runs/cmp/warm-2/coverage.ndjson", 10),
      at("b", "/fixture/runs/cmp/initial-0/coverage.ndjson", 9),
      at("c", "/fixture/runs/v3/coverage.ndjson", 15),
    ],
  };
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "remove_cleanup_queue") {
      queue = {
        ...queue,
        entries: queue.entries.filter(
          (entry) => !args.entryIds.includes(entry.id),
        ),
        revision: queue.revision + 1,
      };
      return queue;
    }
    return original(command, args);
  });
  await click("Scan location");
  await click("Cleanup queue · 3");
  const group = page().querySelector(".cleanup-group")!;
  expect(group.querySelector(".cleanup-base")?.textContent).toContain(
    "/fixture/runs/",
  );
  const folder = group.querySelector(".cleanup-folder")!;
  expect(folder.querySelector(".cleanup-folder-head strong")?.textContent).toBe(
    "cmp/",
  );
  expect(
    [...folder.querySelectorAll(".cleanup-entry-heading .path")].map(
      (path) => path.textContent,
    ),
  ).toEqual(["in warm-2/", "in initial-0/"]);
  await click("Remove 2 items in cmp from cleanup queue");
  expect(mock.invoke).toHaveBeenCalledWith("remove_cleanup_queue", {
    entryIds: ["a", "b"],
  });
  expect(queue.entries.map((entry) => entry.id)).toEqual(["c"]);
});
it("keeps the requested selection count when selection changes during Trash preparation", async () => {
  const folder = {
    ...node,
    id: 2,
    parentId: 0,
    name: "Archive",
    path: "/fixture/Archive",
  };
  const child = {
    ...file,
    id: 3,
    parentId: 2,
    path: "/fixture/Archive/photo.jpg",
  };
  let resolveReview!: (review: {
    token: string;
    items: (typeof folder)[];
  }) => void;
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "get_node")
      return [node, file, folder, child].find(
        (item) => item.id === args.nodeId,
      );
    if (command === "get_children")
      return {
        items: args.nodeId === 2 ? [child] : [folder, file],
        total: args.nodeId === 2 ? 1 : 2,
      };
    if (command === "prepare_trash")
      return new Promise((resolve) => {
        resolveReview = resolve;
      });
    return original(command, args);
  });
  await click("Scan location");
  await click("Expand Archive");
  await act(() =>
    container.querySelector<HTMLElement>('.rows [data-node-id="2"]')!.click(),
  );
  await act(() =>
    container
      .querySelector<HTMLElement>('.rows [data-node-id="3"]')!
      .dispatchEvent(new MouseEvent("click", { bubbles: true, metaKey: true })),
  );
  await act(() => menuItem("trash").click());
  await flush();
  expect(mock.invoke).toHaveBeenCalledWith("prepare_trash", {
    scanId: "live",
    nodeIds: [2, 3],
  });
  await act(() =>
    container.querySelector<HTMLElement>('.rows [data-node-id="1"]')!.click(),
  );
  expect(container.querySelectorAll(".rows .row.selected")).toHaveLength(1);
  await act(() =>
    resolveReview({ token: "prepared-selection", items: [folder] }),
  );
  await flush();
  const dialog = container.querySelector('[role="dialog"]')!;
  expect(
    dialog
      .querySelector(".trash-scope-note")
      ?.textContent?.replace(/\s+/g, " "),
  ).toContain("2 selected; 1 item to move");
  expect(dialog.querySelector(".trash-item-path")?.textContent).toBe(
    folder.path,
  );
  expect(
    mock.invoke.mock.calls.some(([command]) => command === "commit_trash"),
  ).toBe(false);
});
it("confirms clear, focuses cancel and supports Escape without clearing", async () => {
  await click("Cleanup queue · 1");
  await click("Clear queue");
  expect(document.activeElement?.textContent).toBe("Cancel");
  await act(() =>
    container
      .querySelector('[role="dialog"]')!
      .dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      ),
  );
  expect(container.querySelector('[role="dialog"]')).toBeNull();
  expect(queue.entries).toHaveLength(1);
  await click("Clear queue");
  const dialog = container.querySelector('[role="dialog"]')!;
  await act(() =>
    [...dialog.querySelectorAll("button")]
      .find((b) => b.textContent === "Clear queue")!
      .click(),
  );
  await flush();
  expect(queue.entries).toHaveLength(0);
});
it("preserves search and selection when returning to the explorer", async () => {
  await click("Scan location");
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Search file names"]',
  )!;
  await act(() => {
    input.value = "photo";
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 270));
  });
  await flush();
  await act(() => container.querySelector<HTMLElement>(".rows .row")!.click());
  await click("Cleanup queue · 1");
  await click("Back to explorer");
  expect(
    container.querySelector<HTMLInputElement>(
      '[aria-label="Search file names"]',
    )!.value,
  ).toBe("photo");
  expect(container.querySelector(".rows .row.selected")).toBeTruthy();
});
it("reviews queued Trash and reports partial/persistence failures after original-target rescan", async () => {
  const other = {
    ...file,
    id: 2,
    name: "other.jpg",
    path: "/fixture/other.jpg",
  };
  queue = {
    ...queue,
    entries: [
      ...queue.entries,
      { ...queue.entries[0], id: "other", name: other.name, path: other.path },
    ],
  };
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "prepare_cleanup_queue")
      return { token: "ticket", items: [file, other] };
    if (command === "commit_cleanup_queue")
      return {
        trashed: 1,
        failures: ["Other candidate failed"],
        persistenceWarning: "Queue could not be saved",
        outcomes: [
          { reviewIndex: 0, status: "trashed" },
          {
            reviewIndex: 1,
            status: "failed",
            message: "Other candidate failed",
          },
        ],
      };
    return original(command, args);
  });
  await click("Scan location");
  await click("Cleanup queue · 2");
  await click("Review 2 ready");
  expect(document.activeElement?.textContent?.trim()).toBe("Cancel");
  await click("Move to Trash");
  await flush();
  await flush();
  expect(mock.invoke).toHaveBeenCalledWith("commit_cleanup_queue", {
    scanId: "live",
    token: "ticket",
  });
  expect(
    mock.invoke.mock.calls.filter(([c]) => c === "start_scan"),
  ).toHaveLength(2);
  expect(container.textContent).toContain("Other candidate failed");
  expect(container.textContent).toContain("Queue could not be saved");
  expect(
    container.querySelector('.cleanup-modal [role="status"]')?.textContent,
  ).toContain("Moved 1 of 2 items to Trash");
});
it("rescans after a failed Trash that still invalidated the scan", async () => {
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "prepare_cleanup_queue")
      return { token: "ticket", items: [file] };
    if (command === "commit_cleanup_queue")
      return {
        trashed: 0,
        failures: ["x"],
        outcomes: [{ reviewIndex: 0, status: "failed", message: "x" }],
        invalidated: true,
        persistenceWarning: null,
      };
    return original(command, args);
  });
  await click("Scan location");
  await click("Cleanup queue · 1");
  await click("Review 1 ready");
  await click("Move to Trash");
  await flush();
  await flush();
  expect(
    mock.invoke.mock.calls.filter(([c]) => c === "start_scan"),
  ).toHaveLength(2);
});
it.each([
  ["Nothing was moved: The review expired.", "No items moved to Trash", false],
  ["Disk read failed.", "Cleanup result unconfirmed", true],
])("reports a commit error %j as %s", async (reason, title, unconfirmed) => {
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "prepare_cleanup_queue")
      return { token: "ticket", items: [file] };
    if (command === "commit_cleanup_queue") throw reason;
    return original(command, args);
  });
  await click("Scan location");
  await click("Cleanup queue · 1");
  await click("Review 1 ready");
  await click("Move to Trash");
  await flush();
  await flush();
  const text = container.querySelector(".cleanup-modal")!.textContent!;
  expect(text).toContain(title);
  expect(text).toContain(
    unconfirmed
      ? `Cleanup could not be confirmed. ${reason} Check the items in Finder`
      : reason,
  );
  expect(text).not.toContain(unconfirmed ? "Nothing was moved" : "could not be confirmed");
  // An unconfirmed commit may have moved files, so it rescans; a plain refusal does not.
  expect(
    mock.invoke.mock.calls.filter(([c]) => c === "start_scan"),
  ).toHaveLength(unconfirmed ? 2 : 1);
});
it("opens the cleanup result as a modal that traps focus, closes with Done or Escape and returns focus", async () => {
  let trashed = 0;
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "prepare_trash") return { token: "ticket", items: [file] };
    if (command === "commit_trash")
      return {
        trashed,
        failures: trashed ? [] : ["photo.jpg: in use"],
        outcomes: [
          trashed
            ? { reviewIndex: 0, status: "trashed" }
            : { reviewIndex: 0, status: "failed", message: "In use" },
        ],
      };
    return original(command, args);
  });
  const result = () => container.querySelector<HTMLElement>('[role="dialog"]');
  // jsdom has no `inert` reflection, so read the property Svelte sets.
  const mainInert = () =>
    (container.querySelector("main") as HTMLElement & { inert: boolean }).inert;
  const key = (target: Element, name: string) =>
    act(() =>
      target.dispatchEvent(
        new KeyboardEvent("keydown", { key: name, bubbles: true }),
      ),
    );
  async function moveToTrash(opener?: HTMLElement) {
    opener?.focus();
    await act(() => menuItem("trash").click());
    await flush();
    await click("Move to Trash");
    await flush();
    await flush();
  }
  await click("Scan location");
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 270));
  });
  await flush();
  await act(() => container.querySelector<HTMLElement>(".rows .row")!.click());
  // Nothing moved: no rescan, so the page behind keeps its elements and focus can return.
  const opener = container.querySelector<HTMLElement>(
    "main button:not([disabled])",
  )!;
  await moveToTrash(opener);
  let dialog = result()!;
  expect(dialog.getAttribute("aria-modal")).toBe("true");
  expect(dialog.textContent).toContain("No items moved to Trash");
  // The result is not part of the page, so the map never moves.
  expect(container.querySelector("main")!.contains(dialog)).toBe(false);
  expect(mainInert()).toBe(true);
  expect(dialog.contains(document.activeElement)).toBe(true);
  const done = [...dialog.querySelectorAll("button")].at(-1)!;
  expect(done.textContent?.trim()).toBe("Done");
  await act(() => done.focus());
  await key(dialog, "Tab");
  expect(dialog.contains(document.activeElement)).toBe(true);
  await key(dialog, "Escape");
  expect(result()).toBeNull();
  expect(mainInert()).toBe(false);
  expect(document.activeElement).toBe(opener);
  // A cleanup that moved something shows the next steps, and Done closes it.
  trashed = 1;
  await act(() => container.querySelector<HTMLElement>(".rows .row")!.click());
  await moveToTrash();
  dialog = result()!;
  expect(dialog.textContent).toContain("Moved 1 item to Trash");
  expect(dialog.textContent).toContain("Empty the Trash in Finder");
  expect(dialog.textContent).toContain("Cloned files, hard links");
  expect(dialog.textContent).toContain("View moved and failed items");
  await click("Done");
  expect(result()).toBeNull();
});
it.each(["ready", "needsScan"] as const)(
  "shows a restored cleanup failure separately from %s readiness",
  async (status) => {
    queue = {
      ...queue,
      entries: queue.entries.map((entry) => ({
        ...entry,
        status,
        reason: status === "ready" ? null : "Scan this location first",
        lastError: "Native Trash failed previously",
      })),
    };
    await click("Cleanup queue · 1");
    expect(container.textContent).toContain(
      "Last cleanup error: Native Trash failed previously",
    );
    expect(
      container.querySelector(".cleanup-entry .queue-status")?.textContent,
    ).toBe(status === "ready" ? "Ready" : "Needs scan");
    if (status !== "ready")
      expect(container.textContent).toContain("Scan this location first");
  },
);
it("explains non-ready candidates and resolves a registered scan target", async () => {
  queue = {
    ...queue,
    entries: queue.entries.map((e) => ({
      ...e,
      status: "needsScan",
      reason: "Scan this location first",
    })),
  };
  await click("Cleanup queue · 1");
  expect(container.textContent).toContain("Scan this location first");
  // Nothing is reviewable here, so the disk offers only its scan.
  expect(buttonLabels(container.querySelector(".cleanup-group")!)).toEqual([
    "Scan Home",
  ]);
  expect(container.querySelector(".cleanup-primary")?.textContent).toBe(
    "Scan Home",
  );
  await click("Scan Home");
  expect(mock.invoke).toHaveBeenCalledWith("cleanup_queue_target", {
    entryId: "q1",
  });
  expect(
    mock.invoke.mock.calls.find(([c]) => c === "start_scan")?.[1].targetId,
  ).toBe("home");
});

it("adds selected live nodes and leaves Trash to the menu command and queue review", async () => {
  await emptyQueue();
  await selectFile();
  await click("Add to cleanup queue");
  expect(mock.invoke).toHaveBeenCalledWith("add_cleanup_queue", {
    scanId: "live",
    nodeIds: [1],
  });
  expect(
    [...container.querySelectorAll("button")].some(
      (b) => b.textContent?.trim() === "Move to Trash",
    ),
  ).toBe(false);
  await flush();
  expect(menuItem("trash").enabled).toBe(true);
});

it("keeps corrupt storage visible until confirmed clear and blocks additions", async () => {
  queue = {
    ...queue,
    entries: [],
    blocked: true,
    warning: "Unsupported queue version",
  };
  await click("Cleanup queue · 1");
  expect(container.textContent).toContain("Unsupported queue version");
  await click("Clear queue");
  await click("Cancel");
  expect(queue.blocked).toBe(true);
});
it("retains a candidate and surfaces a failed removal", async () => {
  const implementation = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "remove_cleanup_queue") throw "Storage unavailable";
    return implementation(command, args);
  });
  await click("Cleanup queue · 1");
  await act(() =>
    container.querySelector<HTMLButtonElement>(".cleanup-remove")!.click(),
  );
  expect(queue.entries).toHaveLength(1);
  expect(container.textContent).toContain("Storage unavailable");
});

async function emptyQueue() {
  await click("Cleanup queue · 1");
  await act(() =>
    container.querySelector<HTMLButtonElement>(".cleanup-remove")!.click(),
  );
  await click("Back to explorer");
}
async function selectFile() {
  await click("Scan location");
  await act(() => container.querySelector<HTMLElement>(".rows .row")!.click());
}
function queueButton(label: string) {
  const button = [...container.querySelectorAll("button")].find(
    (button) =>
      button.textContent?.trim() === label ||
      button.getAttribute("aria-label") === label ||
      (label === "Added to cleanup queue" && button.matches(".queue-toggle")),
  );
  expect(button).toBeTruthy();
  return button!;
}

it("shows pending addition then persistent selected-item confirmation", async () => {
  const candidate = queue.entries[0];
  await emptyQueue();
  await selectFile();
  const implementation = mock.invoke.getMockImplementation()!;
  let finish!: (value: QueueView) => void;
  mock.invoke.mockImplementation((command, args) => {
    if (command === "add_cleanup_queue") {
      return new Promise<QueueView>((resolve) => {
        finish = resolve;
      });
    }
    return implementation(command, args);
  });
  await click("Add to cleanup queue");
  expect(queueButton("Adding…").disabled).toBe(true);
  expect(
    container.querySelector<HTMLElement>('.queue-feedback[role="status"]')
      ?.hidden,
  ).toBe(true);
  expect(container.textContent).not.toContain("Added to cleanup queue");
  queue = { ...queue, entries: [candidate], revision: queue.revision + 1 };
  await act(() => finish(queue));
  await flush();
  expect(queueButton("Added to cleanup queue").disabled).toBe(false);
  expect(
    container.querySelector<HTMLElement>('.queue-feedback[role="status"]')
      ?.hidden,
  ).toBe(true);
  await click("Cleanup queue · 1");
  await click("Back to explorer");
  expect(queueButton("Added to cleanup queue").disabled).toBe(false);
  expect(
    mock.invoke.mock.calls.filter(
      ([command]) => command === "add_cleanup_queue",
    ),
  ).toHaveLength(1);
});

it("disables Trash while a queue addition is pending", async () => {
  await emptyQueue();
  await selectFile();
  await flush();
  expect(menuItem("trash").enabled).toBe(true);
  const implementation = mock.invoke.getMockImplementation()!;
  let finish!: (value: QueueView) => void;
  mock.invoke.mockImplementation((command, args) =>
    command === "add_cleanup_queue"
      ? new Promise<QueueView>((resolve) => (finish = resolve))
      : implementation(command, args),
  );
  await click("Add to cleanup queue");
  expect(menuItem("trash").enabled).toBe(false);
  await act(() => finish(queue));
  await flush();
  expect(menuItem("trash").enabled).toBe(true);
});

it("keeps a failed addition retryable without claiming success", async () => {
  const candidate = queue.entries[0];
  await emptyQueue();
  await selectFile();
  const implementation = mock.invoke.getMockImplementation()!;
  let attempts = 0;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "add_cleanup_queue") {
      if (++attempts === 1) throw "Storage unavailable";
      queue = { ...queue, entries: [candidate], revision: queue.revision + 1 };
      return queue;
    }
    return implementation(command, args);
  });
  await click("Add to cleanup queue");
  expect(
    [...container.querySelectorAll('[role="alert"]')].some((alert) =>
      alert.textContent?.includes("Storage unavailable"),
    ),
  ).toBe(true);
  expect(queueButton("Add to cleanup queue").disabled).toBe(false);
  expect(container.textContent).not.toContain("Added to cleanup queue");
  await click("Add to cleanup queue");
  expect(queueButton("Added to cleanup queue").disabled).toBe(false);
  expect(
    [...container.querySelectorAll('[role="alert"]')].some((alert) =>
      alert.textContent?.includes("Storage unavailable"),
    ),
  ).toBe(false);
  expect(attempts).toBe(2);
});

it.each([
  ["/fixture/photo.jpg", true],
  ["/fixture", true],
  ["/fixture/photo.jpg.old", false],
  ["/fixtur", false],
] as const)(
  "derives restored queue coverage from path %s",
  async (path, covered) => {
    queue = {
      ...queue,
      entries: queue.entries.map((entry) => ({
        ...entry,
        path,
        status: "needsScan",
        reason: "Scan this location first",
      })),
    };
    await root.unmount();
    root = createTestRoot(container);
    await act(() => root.render(App));
    await flush();
    await selectFile();
    if (path === "/fixture") {
      expect(container.querySelector(".queue-included")).toBeTruthy();
      expect(
        container.querySelector(".queue-inherited-feedback")?.textContent,
      ).toContain("queued parent folder");
      expect(container.querySelector(".queue-toggle")).toBeNull();
    } else {
      expect(
        queueButton(covered ? "Added to cleanup queue" : "Add to cleanup queue")
          .disabled,
      ).toBe(false);
    }
    expect(
      mock.invoke.mock.calls.some(
        ([command]) => command === "add_cleanup_queue",
      ),
    ).toBe(false);
  },
);

it("does not show another selection as adding while the queue request is pending", async () => {
  const candidate = queue.entries[0];
  await emptyQueue();
  const otherFile = {
    ...file,
    id: 2,
    name: "other.jpg",
    path: "/fixture/other.jpg",
  };
  const implementation = mock.invoke.getMockImplementation()!;
  let finish!: (value: QueueView) => void;
  mock.invoke.mockImplementation((command, args) => {
    if (command === "get_children")
      return Promise.resolve({ items: [file, otherFile], total: 2 });
    if (command === "add_cleanup_queue")
      return new Promise<QueueView>((resolve) => {
        finish = resolve;
      });
    return implementation(command, args);
  });
  await selectFile();
  await click("Add to cleanup queue");
  expect(queueButton("Adding…").disabled).toBe(true);
  await act(() =>
    container.querySelectorAll<HTMLElement>(".rows .row")[1].click(),
  );
  expect(queueButton("Add to cleanup queue").disabled).toBe(true);
  expect(
    container.querySelector('.queue-feedback[role="status"]')?.textContent ??
      "",
  ).not.toMatch(/adding/i);
  queue = { ...queue, entries: [candidate], revision: queue.revision + 1 };
  await act(() => finish(queue));
  await flush();
  expect(queueButton("Add to cleanup queue").disabled).toBe(false);
  await act(() =>
    container.querySelectorAll<HTMLElement>(".rows .row")[0].click(),
  );
  expect(queueButton("Added to cleanup queue").disabled).toBe(false);
});

const sized = (id: string, allocatedBytes: number, extra = {}) => ({
  ...queue.entries[0],
  id,
  name: id,
  path: `/fixture/${id}`,
  allocatedBytes,
  ...extra,
});
const rowNames = (scope: Element) =>
  [...scope.querySelectorAll(".cleanup-entry strong")].map(
    (strong) => strong.textContent,
  );

it("lists each disk's rows largest first with 0 B last, a Ready chip, and a section named for the disk", async () => {
  queue = {
    ...queue,
    entries: [
      sized("empty", 0),
      sized("small", 20),
      sized("big", 500),
      sized("pending", 30, { status: "needsScan", matchedScanId: null }),
    ],
  };
  await click("Scan location");
  await click("Cleanup queue · 4");
  const group = page().querySelector(".cleanup-group")!;
  // The accessible name is the disk's name, not its root path.
  expect(group.getAttribute("aria-labelledby")).toBe(
    group.querySelector("h2")!.id,
  );
  expect(group.querySelector("h2")?.textContent).toBe("Home");
  expect(group.querySelector(".path")?.textContent).toBe("/fixture");
  expect(rowNames(group)).toEqual(["big", "pending", "small", "empty"]);
  const rows = [...group.querySelectorAll(".cleanup-entry")];
  expect(
    rows.map((row) => row.querySelector(".queue-status")?.textContent),
  ).toEqual(["Ready", "Needs scan", "Ready", "Ready"]);
  expect(rows[3].textContent?.replace(/\s+/g, " ")).toContain(
    "0 B on disk — empty, or all its blocks are shared with clones",
  );
  expect(group.querySelectorAll(".cleanup-entry-reason")).toHaveLength(2);
  // Review is primary; the scan is offered too because "pending" is not reviewable here.
  expect(group.querySelector("button.primary")?.textContent).toBe(
    "Review 3 ready",
  );
  expect(buttonLabels(group)).toEqual(["Review 3 ready", "Scan Home"]);
  expect(group.querySelector("button:not(.primary)")?.textContent).toBe(
    "Scan Home",
  );
});

it("splits the band into reviewable, ready-elsewhere and needs-scan bytes and says so in text", async () => {
  queue = {
    ...queue,
    entries: [
      sized("here", 600),
      sized("elsewhere", 300, { matchedScanId: null }),
      sized("pending", 100, { status: "needsScan", matchedScanId: null }),
    ],
  };
  await click("Scan location");
  await click("Cleanup queue · 3");
  const band = page().querySelector(".cleanup-band")!;
  expect(band.getAttribute("aria-hidden")).toBe("true");
  expect(
    [...band.children].map((segment) => segment.getAttribute("data-kind")),
  ).toEqual(["ready", "elsewhere", "needsScan"]);
  expect(page().querySelector(".cleanup-band + .sr-only")?.textContent).toBe(
    "1,000 B: 600 B ready to review, 300 B ready once its scan is open, 100 B needs a scan",
  );
});

it("never shows a zero-byte headline when nothing is ready, and offers only the scan", async () => {
  queue = {
    ...queue,
    entries: queue.entries.map((entry) => ({
      ...entry,
      status: "needsScan" as const,
      reason: null,
      matchedScanId: null,
    })),
  };
  await click("Cleanup queue · 1");
  expect(page().textContent).not.toMatch(/0 B to review/);
  expect(page().textContent).toContain("Nothing moves until you confirm.");
  expect(buttonLabels(page().querySelector(".cleanup-group")!)).toEqual([
    "Scan Home",
  ]);
});

it("keeps Review disabled unless the scan on screen is completed", async () => {
  const implementation = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) =>
    command === "scan_summary"
      ? { ...summary, state: "running" }
      : implementation(command, args),
  );
  await click("Scan location");
  await click("Cleanup queue · 1");
  const review = [...page().querySelectorAll("button")].find(
    (b) => b.textContent === "Review 1 ready",
  )!;
  expect(review.disabled).toBe(true);
});

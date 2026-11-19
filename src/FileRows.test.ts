// @vitest-environment jsdom
import { act, createTestRoot as createRoot, type Root } from "./testUtils";
import { expect, it, vi } from "vitest";
import FileRows from "./FileRows.svelte";
import type { NodeView, Page } from "./types";
it("renders only a visible window while allowing access to the final paginated row", async () => {
  const items: NodeView[] = Array.from({ length: 150 }, (_, id) => ({
    id,
    parentId: 0,
    name: `file-${id}`,
    path: `/file-${id}`,
    kind: "file",
    allocatedBytes: 1,
    childrenCount: 0,
    incomplete: false,
  }));
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element);
  await act(async () =>
    root.render(FileRows, {
      items: items,
      selection: [],
      running: false,
      onSelect: () => {},
      onOpen: () => {},
    }),
  );
  expect(element.querySelectorAll("[role=treeitem]").length).toBeLessThan(20);
  const scroll = element.querySelector(".rows")!;
  await act(async () => {
    scroll.scrollTop = 150 * 36 - 230;
    scroll.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  expect(element.textContent).toContain("file-149");
  expect(element.textContent).not.toContain("file-0");
  await act(async () => root.unmount());
  element.remove();
});

it("expands a directory lazily, preserves selection, and exposes omitted children through navigation", async () => {
  const folder: NodeView = {
    id: 1,
    parentId: 0,
    name: "Projects",
    path: "/Projects",
    kind: "directory",
    allocatedBytes: 100,
    childrenCount: 300,
    incomplete: false,
  };
  const child: NodeView = {
    ...folder,
    id: 2,
    parentId: 1,
    name: "source.rs",
    path: "/Projects/source.rs",
    kind: "file",
    childrenCount: 0,
  };
  const selected: number[] = [],
    opened: number[] = [];
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element);
  await act(async () =>
    root.render(FileRows, {
      items: [folder],
      selection: [],
      running: false,
      onLoadChildren: async () => ({ items: [child], total: 300 }),
      onSelect: (n: NodeView) => selected.push(n.id),
      onOpen: (n: NodeView) => opened.push(n.id),
    }),
  );
  expect(element.textContent).not.toContain("source.rs");
  await act(async () => {
    (
      element.querySelector(
        '[aria-label="Expand Projects"]',
      ) as HTMLButtonElement
    ).click();
  });
  expect(selected).toEqual([]);
  expect(element.textContent).toContain("source.rs");
  const treeitems = element.querySelectorAll("[role=treeitem]");
  expect(treeitems[1].getAttribute("aria-level")).toBe("2");
  await act(async () => {
    (treeitems[1] as HTMLElement).click();
  });
  expect(selected).toEqual([2]);
  const more = element.querySelector(
    ".tree-message button",
  ) as HTMLButtonElement;
  await act(async () => more.click());
  expect(opened).toEqual([1]);
  await act(async () => {
    (
      element.querySelector(
        '[aria-label="Collapse Projects"]',
      ) as HTMLButtonElement
    ).click();
  });
  expect(element.textContent).not.toContain("source.rs");
  await act(async () => root.unmount());
  element.remove();
});

it("refreshes expanded folders with the latest loader and ignores older failures", async () => {
  const folder: NodeView = {
    id: 1,
    parentId: 0,
    name: "Projects",
    path: "/Projects",
    kind: "directory",
    allocatedBytes: 100,
    childrenCount: 1,
    incomplete: false,
  };
  const child: NodeView = {
    ...folder,
    id: 2,
    parentId: 1,
    name: "current.rs",
    kind: "file",
    childrenCount: 0,
  };
  let rejectOld!: (error: Error) => void;
  const oldResponse = new Promise<Page>((_, reject) => {
    rejectOld = reject;
  });
  const items = [folder];
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element);
  const render = (revision: string, load: (id: number) => Promise<Page>) =>
    root.render(FileRows, {
      items: items,
      selection: [1],
      running: false,
      revision: revision,
      onLoadChildren: load,
      onSelect: () => {},
      onOpen: () => {},
    });
  try {
    await act(async () => render("1", () => oldResponse));
    await act(async () => {
      element
        .querySelector<HTMLButtonElement>('[aria-label="Expand Projects"]')!
        .click();
    });
    expect(element.textContent).toContain("Loading…");
    await act(async () =>
      render("2", async () => ({ items: [child], total: 1 })),
    );
    expect(element.textContent).toContain("current.rs");
    await act(async () => rejectOld(new Error("Old request failed")));
    expect(element.textContent).toContain("current.rs");
    expect(element.textContent).not.toContain("Unable to load");
    expect(
      element.querySelector('[role="treeitem"]')?.getAttribute("aria-selected"),
    ).toBe("true");
  } finally {
    await act(async () => root.unmount());
    element.remove();
  }
});

it("keeps excluded directories selectable without presenting empty sizes or expanding them", async () => {
  const folder: NodeView = {
    id: 1,
    parentId: 0,
    name: "Other volume",
    path: "/Volumes/Other",
    kind: "directory",
    allocatedBytes: 0,
    childrenCount: 0,
    incomplete: false,
    excluded: true,
  };
  let loads = 0,
    opens = 0,
    selected = 0;
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element);
  await act(async () =>
    root.render(FileRows, {
      items: [folder],
      selection: [],
      running: false,
      onLoadChildren: async () => {
        loads++;
        return { items: [], total: 0 };
      },
      onSelect: () => {
        selected++;
      },
      onOpen: () => {
        opens++;
      },
    }),
  );
  const excludedRow = element.querySelector("[role=treeitem]")!;
  expect(excludedRow.children).toHaveLength(4);
  expect(excludedRow.children[1].className).toBe("file-relative");
  expect(excludedRow.children[1].textContent).toBe("");
  expect(excludedRow.querySelector(".file-share")).toBeNull();
  expect(excludedRow.children[2].textContent).toBe("Excluded");
  expect(element.textContent).toContain("Excluded");
  expect(element.textContent).toContain("Not scanned");
  expect(element.textContent).not.toContain("0 B");
  expect(element.textContent).not.toContain("0 items");
  expect(
    element.querySelector('[aria-label="Expand Other volume"]'),
  ).toBeNull();
  const row = element.querySelector('[role="treeitem"]') as HTMLElement;
  expect(row.hasAttribute("aria-expanded")).toBe(false);
  await act(async () => {
    row.click();
    row.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    row.dispatchEvent(
      new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }),
    );
    row.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    );
  });
  expect(selected).toBeGreaterThan(0);
  expect(loads).toBe(0);
  expect(opens).toBe(0);
  await act(async () => root.unmount());
  element.remove();
});

it("only previews plain Space on regular file rows and leaves folders/interactive targets alone", async () => {
  const file: NodeView = {
    id: 1,
    parentId: 0,
    name: "file",
    path: "/file",
    kind: "file",
    allocatedBytes: 1,
    childrenCount: 0,
    incomplete: false,
  };
  const folder: NodeView = {
    ...file,
    id: 2,
    name: "folder",
    kind: "directory",
  };
  let previews = 0,
    selections = 0,
    opens = 0;
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element);
  await act(async () =>
    root.render(FileRows, {
      items: [file, folder],
      selection: [],
      running: false,
      onSelect: () => selections++,
      onOpen: () => opens++,
      onPreview: () => {
        previews++;
        return true;
      },
    }),
  );
  const rows = element.querySelectorAll<HTMLElement>('[role="treeitem"]');
  await act(async () =>
    rows[0].dispatchEvent(
      new KeyboardEvent("keydown", { key: " ", bubbles: true }),
    ),
  );
  expect(previews).toBe(1);
  expect(selections).toBe(0);
  await act(async () =>
    rows[1].dispatchEvent(
      new KeyboardEvent("keydown", { key: " ", bubbles: true }),
    ),
  );
  expect(previews).toBe(1);
  expect(selections).toBe(1);
  for (const flags of [
    { metaKey: true },
    { ctrlKey: true },
    { shiftKey: true },
    { altKey: true },
    { repeat: true },
  ])
    await act(async () =>
      rows[0].dispatchEvent(
        new KeyboardEvent("keydown", { key: " ", bubbles: true, ...flags }),
      ),
    );
  const input = document.createElement("input"),
    button = document.createElement("button"),
    editable = document.createElement("span");
  editable.contentEditable = "true";
  for (const child of [input, button, editable]) {
    rows[0].append(child);
    await act(async () =>
      child.dispatchEvent(
        new KeyboardEvent("keydown", { key: " ", bubbles: true }),
      ),
    );
  }
  expect(previews).toBe(1);
  expect(selections).toBe(3);
  await act(async () =>
    rows[1].dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    ),
  );
  expect(opens).toBe(1);
  await act(async () => root.unmount());
  element.remove();
});

it("R4 preserves focused row identity when siblings reorder", async () => {
  const items: NodeView[] = [1, 2, 3].map((id) => ({
    id,
    parentId: 0,
    name: `file-${id}`,
    path: `/file-${id}`,
    kind: "file",
    allocatedBytes: id,
    childrenCount: 0,
    incomplete: false,
  }));
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element);
  const opened: number[] = [];
  const props = {
    selection: [],
    running: false,
    onSelect: () => {},
    onOpen: (node: NodeView) => {
      opened.push(node.id);
    },
  };
  try {
    await act(async () => root.render(FileRows, { ...props, items }));
    const focused =
      element.querySelectorAll<HTMLElement>('[role="treeitem"]')[1];
    focused.focus();
    await act(async () =>
      root.render(FileRows, {
        ...props,
        items: [items[0], items[2], items[1]],
      }),
    );
    expect(document.activeElement).toBe(focused);
    expect(focused.textContent).toContain("file-2");
    await act(async () =>
      focused.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      ),
    );
    expect(opened).toEqual([2]);
  } finally {
    await act(async () => root.unmount());
    element.remove();
  }
});

it("keeps row sizes compact with full details and updates proportional bars", async () => {
  const items: NodeView[] = [
    {
      id: 1,
      parentId: 0,
      name: "Projects",
      path: "/Projects",
      kind: "directory",
      allocatedBytes: 2048,
      childrenCount: 2,
      incomplete: false,
    },
    {
      id: 2,
      parentId: 0,
      name: "notes.txt",
      path: "/notes.txt",
      kind: "file",
      allocatedBytes: 1024,
      childrenCount: 0,
      incomplete: false,
    },
  ];
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element);
  const render = async () =>
    act(async () =>
      root.render(FileRows, {
        items,
        selection: [],
        running: false,
        onSelect: () => {},
        onOpen: () => {},
      }),
    );
  try {
    await render();
    const rows = element.querySelectorAll<HTMLElement>("[role=treeitem]");
    expect(rows[0].children).toHaveLength(4);
    expect(Array.from(rows[0].children, (cell) => cell.className)).toEqual([
      "file-row-name",
      "file-relative",
      "file-size",
      "file-contents",
    ]);
    expect(rows[0].querySelector(".file-size .file-share")).toBeNull();
    expect(rows[0].dataset.kind).toBe("directory");
    expect(rows[0].querySelector("svg.icon-folder")).not.toBeNull();
    expect(rows[1].querySelector("svg.icon-file")).not.toBeNull();
    expect(rows[0].querySelector(".file-size")?.textContent).toBe("2 KiB");
    expect(rows[0].querySelector(".file-size")?.getAttribute("title")).toBe(
      "2 KiB (2.05 KB)",
    );
    expect(
      rows[1].querySelector<HTMLElement>(".file-share > span")?.style.width,
    ).toBe("50%");
  } finally {
    await act(async () => root.unmount());
    element.remove();
  }
});

it("fills a tall viewport after resize while keeping distant rows virtualized", async () => {
  let resize: (() => void) | undefined;
  let viewportHeight = 230;
  let gutterWidth = 0;
  const onScrollbarWidthChange = vi.fn();
  const disconnect = vi.fn();
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(callback: () => void) {
        resize = callback;
      }
      observe(element: HTMLElement) {
        Object.defineProperties(element, {
          offsetWidth: { get: () => 800 },
          clientWidth: { get: () => 800 - gutterWidth },
        });
        Object.defineProperty(element, "clientHeight", {
          get: () => viewportHeight,
        });
      }
      disconnect = disconnect;
    },
  );
  const items: NodeView[] = Array.from({ length: 150 }, (_, id) => ({
    id,
    parentId: 0,
    name: `file-${id}`,
    path: `/file-${id}`,
    kind: "file",
    allocatedBytes: 1,
    childrenCount: 0,
    incomplete: false,
  }));
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element);
  try {
    await act(async () =>
      root.render(FileRows, {
        items,
        selection: [],
        running: false,
        onSelect: () => {},
        onOpen: () => {},
        onScrollbarWidthChange,
      }),
    );
    expect(onScrollbarWidthChange.mock.calls).toEqual([[0]]);
    await act(async () => resize!());
    expect(onScrollbarWidthChange).toHaveBeenCalledTimes(1);
    await act(async () => {
      gutterWidth = 15;
      resize!();
      resize!();
    });
    expect(onScrollbarWidthChange.mock.calls).toEqual([[0], [15]]);
    expect(element.textContent).not.toContain("file-20");
    await act(async () => {
      viewportHeight = 900;
      resize!();
    });
    expect(element.textContent).toContain("file-23");
    expect(element.textContent).not.toContain("file-100");
    expect(element.querySelectorAll("[role=treeitem]").length).toBeLessThan(40);
    const scroll = element.querySelector(".rows")!;
    await act(async () => {
      scroll.scrollTop = 36 * 60;
      scroll.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    expect(element.textContent).toContain("file-83");
    expect(element.textContent).not.toContain("file-0");
    await act(async () => {
      viewportHeight = 230;
      resize!();
    });
    expect(element.textContent).not.toContain("file-83");
  } finally {
    await act(async () => root.unmount());
    element.remove();
    vi.unstubAllGlobals();
  }
  expect(disconnect).toHaveBeenCalledOnce();
});

it("shows persistent queue badges across selection and queue updates", async () => {
  const base: NodeView = {
    id: 1,
    parentId: 0,
    name: "Downloads",
    path: "/Downloads",
    kind: "directory",
    allocatedBytes: 1,
    childrenCount: 2,
    incomplete: false,
  };
  const items: NodeView[] = [
    base,
    { ...base, id: 2, name: "games", path: "/Downloads/games" },
    {
      ...base,
      id: 3,
      name: "clip.mp4",
      path: "/Downloads/games/clip.mp4",
      kind: "file",
    },
    { ...base, id: 4, name: "games-old", path: "/Downloads/games-old" },
  ];
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element);
  const render = (queuedPaths: string[], selection: number[]) =>
    root.render(FileRows, {
      items,
      selection,
      queuedPaths: queuedPaths.map((path) => ({ path })),
      running: false,
      onSelect: () => {},
      onOpen: () => {},
    });
  try {
    await act(async () => render(["/Downloads/games"], [2]));
    let rows = element.querySelectorAll('[role="treeitem"]');
    expect(rows[0].getAttribute("data-queue-state")).toBe("contains");
    expect(rows[0].textContent).toContain("Queued inside");
    expect(rows[1].getAttribute("data-queue-state")).toBe("queued");
    expect(rows[2].getAttribute("data-queue-state")).toBe("queued");
    expect(rows[3].hasAttribute("data-queue-state")).toBe(false);
    expect(rows[2].querySelector(".queue-badge")?.getAttribute("title")).toBe(
      "clip.mp4: queued for cleanup",
    );
    await act(async () => render(["/Downloads/games"], [4]));
    rows = element.querySelectorAll('[role="treeitem"]');
    expect(rows[1].getAttribute("aria-selected")).toBe("false");
    expect(rows[1].getAttribute("data-queue-state")).toBe("queued");
    await act(async () => render([], [4]));
    expect(element.querySelector(".queue-badge")).toBeNull();
    expect(element.querySelector("[data-queue-state]")).toBeNull();
    await act(async () => render(["/Downloads/games/clip.mp4"], [4]));
    expect(element.querySelectorAll(".queue-contains")).toHaveLength(2);
  } finally {
    await act(async () => root.unmount());
    element.remove();
  }
});

it("shows each item's share of the scan, and handles an empty scan", async () => {
  const item: NodeView = {
    id: 1,
    parentId: 0,
    name: "Documents",
    path: "/Documents",
    kind: "directory",
    allocatedBytes: 20,
    childrenCount: 2,
    incomplete: false,
  };
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element);
  const render = (scanBytes: number | null) =>
    root.render(FileRows, {
      items: [item],
      scanBytes,
      selection: [],
      running: false,
      onSelect: () => {},
      onOpen: () => {},
    });
  try {
    await act(() => render(100));
    expect(element.querySelector(".file-relative")?.textContent?.trim()).toBe(
      "20.0%",
    );
    await act(() => render(200));
    expect(element.querySelector(".file-relative")?.textContent?.trim()).toBe(
      "10.0%",
    );
    await act(() => render(0));
    expect(element.querySelector(".file-relative")?.textContent?.trim()).toBe(
      "—",
    );
    await act(() => render(null));
    expect(element.querySelector(".file-share")).not.toBeNull();
  } finally {
    await act(() => root.unmount());
    element.remove();
  }
});

it("keeps the scroll position when a refresh changes the first row", async () => {
  const make = (start: number): NodeView[] =>
    Array.from({ length: 150 }, (_, i) => ({
      id: start + i,
      parentId: 0,
      name: `file-${start + i}`,
      path: `/file-${start + i}`,
      kind: "file",
      allocatedBytes: 1,
      childrenCount: 0,
      incomplete: false,
    }));
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element);
  const props = {
    selection: [],
    running: true,
    onSelect: () => {},
    onOpen: () => {},
  };
  try {
    await act(async () => root.render(FileRows, { ...props, items: make(0) }));
    const rows = element.querySelector<HTMLElement>(".rows")!;
    await act(async () => {
      rows.scrollTop = 1000;
      rows.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    await act(async () =>
      root.render(FileRows, { ...props, items: make(1000) }),
    );
    expect(rows.scrollTop).toBe(1000);
  } finally {
    await act(async () => root.unmount());
    element.remove();
  }
});

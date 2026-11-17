// @vitest-environment jsdom
import { act, createTestRoot } from "./testUtils";
import { expect, it, vi } from "vitest";
import FileRows from "./FileRows.svelte";
import { createBranchColorResolver } from "./branchColors";
import { createHoverTarget } from "./hoverContext";
import type { NodeView } from "./types";

const node = (id: number, path: string, parentId = 0): NodeView => ({
  id,
  parentId,
  path,
  name: path.split("/").at(-1)!,
  kind: "file",
  allocatedBytes: 100,
  logicalBytes: 100,
  childrenCount: 0,
  incomplete: false,
});

it("identifies same-named global results by parent path and keeps branch colors", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createTestRoot(container);
  const first = node(9, "/Projects/Alpha/report.txt", 3);
  const second = node(10, "/Projects/Beta/report.txt", 4);
  const branch = { ...node(1, "/Projects"), kind: "directory" as const };
  const colorForNode = createBranchColorResolver(
    [{ ...branch, id: 0, parentId: null, path: "/" }, branch],
    0,
  );
  const selected = vi.fn();
  const props = {
    items: [first, second],
    selection: [],
    metric: "allocated" as const,
    running: false,
    onSelect: selected,
    onOpen: vi.fn(),
    colorForNode,
  };
  try {
    await act(() => root.render(FileRows, { ...props, showParentPaths: true }));
    expect(
      Array.from(
        container.querySelectorAll(".file-parent-path"),
        (el) => el.textContent,
      ),
    ).toEqual(["/Projects/Alpha", "/Projects/Beta"]);
    expect(container.querySelector<HTMLElement>(".file-name")!.title).toBe(
      first.path,
    );
    const bars = container.querySelectorAll<HTMLElement>(".file-share > span");
    expect(bars[0].style.background).toBe(bars[1].style.background);
    await act(() =>
      container.querySelector<HTMLElement>('[role="treeitem"]')!.click(),
    );
    expect(selected).toHaveBeenCalledWith(first, false);
    await act(() =>
      root.render(FileRows, { ...props, showParentPaths: false }),
    );
    expect(container.querySelector(".file-parent-path")).toBeNull();
    expect(
      container.querySelector<HTMLElement>('[role="treeitem"]')!.style.height,
    ).toBe("36px");
  } finally {
    await act(() => root.unmount());
    container.remove();
  }
});

it("virtualizes two-line result rows through the last item", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createTestRoot(container);
  const items = Array.from({ length: 150 }, (_, id) =>
    node(id + 1, `/Folder/file-${id}`),
  );
  try {
    await act(() =>
      root.render(FileRows, {
        items,
        selection: [],
        metric: "allocated",
        running: false,
        showParentPaths: true,
        onSelect: vi.fn(),
        onOpen: vi.fn(),
      }),
    );
    expect(container.querySelectorAll('[role="treeitem"]').length).toBeLessThan(
      20,
    );
    const rows = container.querySelector<HTMLElement>(".rows")!;
    await act(() => {
      rows.scrollTop = 150 * 60 - 230;
      rows.dispatchEvent(new Event("scroll"));
    });
    expect(container.textContent).toContain("file-149");
    expect(container.textContent).not.toContain("file-0");
    expect(
      container.querySelector<HTMLElement>('[role="treeitem"]')!.style.height,
    ).toBe("60px");
  } finally {
    await act(() => root.unmount());
    container.remove();
  }
});

it("links a nested hover to its closest visible row without selecting or scrolling", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createTestRoot(container);
  const folder = { ...node(1, "/Projects"), kind: "directory" as const };
  const child = node(2, "/Projects/report.txt", 1);
  const onSelect = vi.fn(),
    onHover = vi.fn();
  const props = {
    items: [folder, node(3, "/Other")],
    selection: [3],
    metric: "allocated" as const,
    running: false,
    onSelect,
    onOpen: vi.fn(),
    onHover,
  };
  try {
    await act(() =>
      root.render(FileRows, {
        ...props,
        hovered: createHoverTarget(child, new Map([[1, folder]])),
      }),
    );
    expect(
      container.querySelector(".linked-hover")?.getAttribute("data-node-id"),
    ).toBe("1");
    expect(
      container.querySelector(".selected")?.getAttribute("data-node-id"),
    ).toBe("3");
    const rows = container.querySelector<HTMLElement>(".rows")!;
    expect(rows.scrollTop).toBe(0);
    expect(onSelect).not.toHaveBeenCalled();
    await act(() =>
      container
        .querySelector<HTMLElement>('[data-node-id="1"]')!
        .dispatchEvent(new MouseEvent("mousemove", { bubbles: true })),
    );
    expect(onHover.mock.lastCall?.[0].lineage).toEqual([1, 0]);
    await act(() =>
      container
        .querySelector<HTMLElement>('[data-node-id="1"]')!
        .dispatchEvent(new MouseEvent("mouseleave")),
    );
    expect(onHover).toHaveBeenLastCalledWith(null);
    await act(() => root.render(FileRows, { ...props, hovered: null }));
    expect(container.querySelector(".linked-hover")).toBeNull();
  } finally {
    await act(() => root.unmount());
    container.remove();
  }
});

it("publishes expanded child ancestry and does not scroll offscreen rows into hover", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createTestRoot(container);
  const folder = {
    ...node(1, "/Projects"),
    kind: "directory" as const,
    childrenCount: 1,
  };
  const child = node(200, "/Projects/report.txt", 1);
  const items = [
    folder,
    ...Array.from({ length: 30 }, (_, id) => node(id + 2, `/file-${id}`)),
  ];
  const onHover = vi.fn();
  const props = {
    items,
    selection: [],
    metric: "allocated" as const,
    running: false,
    onSelect: vi.fn(),
    onOpen: vi.fn(),
    onHover,
    onLoadChildren: async () => ({ items: [child], total: 1 }),
  };
  try {
    await act(() =>
      root.render(FileRows, {
        ...props,
        hovered: createHoverTarget(items[20], new Map()),
      }),
    );
    expect(container.querySelector(".linked-hover")).toBeNull();
    const rows = container.querySelector<HTMLElement>(".rows")!;
    expect(rows.scrollTop).toBe(0);
    await act(async () =>
      container
        .querySelector<HTMLButtonElement>('[aria-label="Expand Projects"]')!
        .click(),
    );
    await act(() =>
      container
        .querySelector<HTMLElement>('[data-node-id="200"]')!
        .dispatchEvent(new MouseEvent("mousemove", { bubbles: true })),
    );
    expect(onHover.mock.lastCall?.[0].lineage).toEqual([200, 1, 0]);
    await act(() => rows.dispatchEvent(new Event("scroll")));
    expect(onHover).toHaveBeenLastCalledWith(null);
    await act(() =>
      root.render(FileRows, {
        ...props,
        hovered: createHoverTarget(child, new Map([[1, folder]])),
      }),
    );
    expect(
      container.querySelector(".linked-hover")?.getAttribute("data-node-id"),
    ).toBe("200");
  } finally {
    await act(() => root.unmount());
    container.remove();
  }
});

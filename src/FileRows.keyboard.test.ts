// @vitest-environment jsdom
import { expect, it, vi } from "vitest";
import { act, createTestRoot } from "./testUtils";
import FileRows from "./FileRows.svelte";
import type { NodeView } from "./types";

const items: NodeView[] = Array.from({ length: 150 }, (_, id) => ({
  id,
  parentId: 0,
  name: `file-${id}`,
  path: `/file-${id}`,
  kind: "file",
  logicalBytes: 1,
  allocatedBytes: 1,
  childrenCount: 0,
  incomplete: false,
}));
async function fixture(extra: Record<string, unknown> = {}) {
  const element = document.createElement("div");
  document.body.append(element);
  const root = createTestRoot(element);
  let selected: number[] = [];
  const preview = vi.fn(() => true);
  await act(() =>
    root.render(FileRows, {
      items,
      selection: [],
      metric: "allocated",
      running: false,
      onSelect: (node: NodeView, multi: boolean) => {
        selected = multi
          ? selected.includes(node.id)
            ? selected.filter((id) => id !== node.id)
            : [...selected, node.id]
          : [node.id];
      },
      onOpen: () => {},
      onPreview: preview,
      ...extra,
    }),
  );
  const row = (id: number) =>
    element.querySelector<HTMLElement>(`[data-node-id="${id}"]`)!;
  const press = async (key: string, flags: KeyboardEventInit = {}) => {
    await act(() =>
      document.activeElement!.dispatchEvent(
        new KeyboardEvent("keydown", {
          key,
          bubbles: true,
          cancelable: true,
          ...flags,
        }),
      ),
    );
  };
  return {
    element,
    row,
    press,
    preview,
    selection: () => selected,
    async dispose() {
      await act(() => root.unmount());
      element.remove();
    },
  };
}

it("moves focus and selection across virtual windows and keeps a single tab stop", async () => {
  const f = await fixture();
  try {
    f.row(0).focus();
    await f.press("End");
    expect(document.activeElement).toBe(f.row(149));
    expect(f.selection()).toEqual([149]);
    expect(f.row(0)).toBeNull();
    expect(
      f.element.querySelectorAll('[role="treeitem"][tabindex="0"]'),
    ).toHaveLength(1);
    expect(f.element.querySelectorAll('[role="treeitem"]').length).toBeLessThan(
      20,
    );
    await f.press("PageUp");
    expect(document.activeElement).toBe(f.row(143));
    await f.press("ArrowUp");
    expect(document.activeElement).toBe(f.row(142));
    await f.press("Home");
    expect(document.activeElement).toBe(f.row(0));
    await f.press("PageDown");
    expect(document.activeElement).toBe(f.row(6));
    await f.press("ArrowDown");
    expect(document.activeElement).toBe(f.row(7));
    expect(f.selection()).toEqual([7]);
  } finally {
    await f.dispose();
  }
});

it("extends and contracts a keyboard selection and toggles without previewing", async () => {
  const f = await fixture();
  try {
    f.row(0).focus();
    await f.press("ArrowDown");
    await f.press("ArrowDown", { shiftKey: true });
    await f.press("ArrowDown", { shiftKey: true });
    expect(f.selection()).toEqual([1, 2, 3]);
    await f.press("ArrowUp", { shiftKey: true });
    expect(f.selection()).toEqual([1, 2]);
    await f.press("ArrowDown", { metaKey: true });
    expect(document.activeElement).toBe(f.row(3));
    expect(f.selection()).toEqual([1, 2]);
    await f.press(" ", { metaKey: true });
    expect(f.selection()).toEqual([1, 2, 3]);
    await f.press(" ", { ctrlKey: true });
    expect(f.selection()).toEqual([1, 2]);
    expect(f.preview).not.toHaveBeenCalled();
    await f.press(" ", { ctrlKey: true, repeat: true });
    expect(f.selection()).toEqual([1, 2]);
    await f.press(" ");
    expect(f.preview).toHaveBeenCalledExactlyOnceWith(items[3]);
    expect(f.selection()).toEqual([1, 2]);
  } finally {
    await f.dispose();
  }
});

it.each(["directory", "file"] as const)(
  "anchors Shift selection at the %s selected with plain Space",
  async (kind) => {
    const f = await fixture({
      items: items.map((node) => (node.id === 2 ? { ...node, kind } : node)),
      onPreview: () => false,
    });
    try {
      f.row(0).focus();
      await f.press("ArrowDown");
      await f.press("ArrowDown", { metaKey: true });
      expect(f.selection()).toEqual([1]);
      await f.press(" ");
      expect(f.selection()).toEqual([2]);
      await f.press("ArrowDown", { shiftKey: true });
      expect(f.selection()).toEqual([2, 3]);
    } finally {
      await f.dispose();
    }
  },
);

it("navigates loaded child rows and skips branch loading messages", async () => {
  const folder: NodeView = { ...items[0], kind: "directory", childrenCount: 3 };
  let resolve!: (value: { items: NodeView[]; total: number }) => void;
  const pending = new Promise<{ items: NodeView[]; total: number }>((done) => {
    resolve = done;
  });
  const f = await fixture({
    items: [folder, items[4]],
    onLoadChildren: () => pending,
  });
  try {
    f.row(0).focus();
    await f.press("ArrowRight");
    expect(f.element.textContent).toContain("Loading…");
    await f.press("ArrowDown");
    expect(document.activeElement).toBe(f.row(4));
    await act(() => resolve({ items: [items[1], items[2]], total: 3 }));
    await f.press("ArrowUp");
    expect(document.activeElement).toBe(f.row(2));
    expect(f.row(2).getAttribute("aria-level")).toBe("2");
    await f.press("Home");
    await f.press("ArrowLeft");
    expect(f.row(2)).toBeNull();
    expect(document.activeElement).toBe(f.row(0));
  } finally {
    await f.dispose();
  }
});

it("leaves blocked keyboards alone", async () => {
  const f = await fixture({ keyboardBlocked: true });
  try {
    f.row(0).focus();
    await f.press("End");
    expect(document.activeElement).toBe(f.row(0));
    expect(f.selection()).toEqual([]);
  } finally {
    await f.dispose();
  }
});

it("does not consume nested-control keys or reclaim focus moved outside during rendering", async () => {
  const f = await fixture();
  const outside = document.createElement("button");
  document.body.append(outside);
  try {
    const nested = document.createElement("button");
    f.row(0).append(nested);
    nested.focus();
    await f.press("End");
    expect(document.activeElement).toBe(nested);
    expect(f.selection()).toEqual([]);
    f.row(0).focus();
    await act(() => {
      f.row(0).dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "End",
          bubbles: true,
          cancelable: true,
        }),
      );
      outside.focus();
    });
    expect(document.activeElement).toBe(outside);
    expect(f.selection()).toEqual([149]);
  } finally {
    outside.remove();
    await f.dispose();
  }
});

it("uses the taller search-result rows for page navigation", async () => {
  const f = await fixture({ showParentPaths: true });
  try {
    f.row(0).focus();
    await f.press("PageDown");
    expect(document.activeElement).toBe(f.row(3));
    await f.press("End");
    expect(document.activeElement).toBe(f.row(149));
    expect(f.element.querySelector(".rows")!.scrollTop).toBe(150 * 60 - 230);
  } finally {
    await f.dispose();
  }
});

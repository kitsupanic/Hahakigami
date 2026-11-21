// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, createTestRoot, type Root } from "./testUtils";
import FileRows from "./FileRows.svelte";
import type { NodeView } from "./types";
import { queuePathKey } from "./queueMarkers";

let container: HTMLDivElement, root: Root, rows: HTMLElement;
const items: NodeView[] = Array.from({ length: 30 }, (_, id) => ({
  id,
  parentId: 100,
  name: `file-${id}`,
  path: `/fixture/file-${id}`,
  kind: "file",
  allocatedBytes: 100,
  childrenCount: 0,
  incomplete: false,
}));
const onScrollChange = vi.fn(),
  onScrollRestored = vi.fn();
const props = {
  items,
  selection: [],
  scanBytes: 100,
  running: false,
  onSelect: vi.fn(),
  onOpen: vi.fn(),
  onScrollChange,
  onScrollRestored,
};
beforeEach(async () => {
  onScrollChange.mockClear();
  onScrollRestored.mockClear();
  container = document.createElement("div");
  document.body.append(container);
  root = createTestRoot(container);
  await act(() => root.render(FileRows, props));
  rows = container.querySelector(".rows")!;
  Object.defineProperty(rows, "scrollHeight", {
    configurable: true,
    value: 1440,
  });
  Object.defineProperty(rows, "clientHeight", {
    configurable: true,
    value: 240,
  });
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
});

it("restores after result rows load and the first-item reset, then preserves user scrolling", async () => {
  const request = { id: 1, top: 280 };
  await act(() =>
    root.render(FileRows, {
      ...props,
      items: [],
      scrollLoading: true,
      restoreScroll: request,
    }),
  );
  expect(onScrollRestored).not.toHaveBeenCalled();
  await act(() =>
    root.render(FileRows, {
      ...props,
      scrollLoading: false,
      restoreScroll: request,
    }),
  );
  expect(rows.scrollTop).toBe(280);
  expect(onScrollRestored).toHaveBeenCalledExactlyOnceWith(1);
  expect(onScrollChange).toHaveBeenLastCalledWith(280);
  await act(() => {
    rows.scrollTop = 400;
    rows.dispatchEvent(new Event("scroll"));
  });
  expect(onScrollChange).toHaveBeenLastCalledWith(400);
  await act(() =>
    root.render(FileRows, {
      ...props,
      items: items.map((node) => ({ ...node })),
      restoreScroll: request,
    }),
  );
  expect(rows.scrollTop).toBe(400);
  expect(onScrollRestored).toHaveBeenCalledTimes(1);
});

it("clamps restored positions to the new content extent and consumes empty results", async () => {
  await act(() =>
    root.render(FileRows, { ...props, restoreScroll: { id: 2, top: 9000 } }),
  );
  expect(rows.scrollTop).toBe(1200);
  expect(onScrollRestored).toHaveBeenLastCalledWith(2);
  Object.defineProperty(rows, "scrollHeight", { configurable: true, value: 0 });
  await act(() =>
    root.render(FileRows, {
      ...props,
      items: [],
      restoreScroll: { id: 3, top: 280 },
    }),
  );
  expect(rows.scrollTop).toBe(0);
  expect(onScrollRestored).toHaveBeenLastCalledWith(3);
});

it("cancels an old pending restoration when another result context replaces it", async () => {
  await act(() =>
    root.render(FileRows, {
      ...props,
      scrollLoading: true,
      restoreScroll: { id: 4, top: 280 },
    }),
  );
  await act(() =>
    root.render(FileRows, {
      ...props,
      restoreScroll: null,
      scrollLoading: false,
    }),
  );
  expect(rows.scrollTop).toBe(0);
  expect(onScrollRestored).not.toHaveBeenCalled();
  await act(() =>
    root.render(FileRows, { ...props, restoreScroll: { id: 5, top: 180 } }),
  );
  expect(rows.scrollTop).toBe(180);
  expect(onScrollRestored).toHaveBeenCalledExactlyOnceWith(5);
});

it("scrolls a row selected elsewhere, such as on the map, into the middle of the list", async () => {
  await act(() => root.render(FileRows, { ...props, selection: [25] }));
  // Default 230px view: row 25 starts at 900, centred means 900 - (230 - 36) / 2.
  expect(rows.scrollTop).toBe(803);
  expect(onScrollChange).toHaveBeenLastCalledWith(803);
  await act(() => root.render(FileRows, { ...props, selection: [24] }));
  expect(rows.scrollTop).toBe(803);
  await act(() => root.render(FileRows, { ...props, selection: [1] }));
  expect(rows.scrollTop).toBe(0);
});

it("opens collapsed folders to reveal a row selected on the map", async () => {
  const node = (id: number, parentId: number, path: string, kind: NodeView["kind"]) => ({
    ...items[0],
    id,
    parentId,
    name: path.split("/").at(-1)!,
    path,
    kind,
    childrenCount: kind === "directory" ? 1 : 0,
  });
  const outer = node(50, 100, "/fixture/outer", "directory");
  const inner = node(51, 50, "/fixture/outer/inner", "directory");
  const leaf = node(52, 51, "/fixture/outer/inner/leaf.bin", "file");
  const children: Record<number, NodeView[]> = { 50: [inner], 51: [leaf] };
  const onLoadChildren = vi.fn(async (id: number) => ({ items: children[id], total: 1 }));
  const shown = { ...props, items: [...items, outer], onLoadChildren };
  await act(() => root.render(FileRows, shown));
  await act(() =>
    root.render(FileRows, { ...shown, selection: [52], selectionPath: queuePathKey(leaf) }),
  );
  for (let i = 0; i < 5; i++) await act(async () => {});
  expect(onLoadChildren.mock.calls.map(([id]) => id)).toEqual([50, 51]);
  // Row 32 of 33 (30 files, outer, inner, leaf), centred in the 230px view.
  expect(rows.scrollTop).toBe(32 * 36 - (230 - 36) / 2);
});

it("reveals through the folder with the same path key, not one whose name displays the same", async () => {
  const dir = (id: number, key: string): NodeView => ({
    ...items[0],
    id,
    parentId: 100,
    name: "caf\uFFFD",
    path: "/fixture/caf\uFFFD",
    pathKey: `/fixture/caf%${key}`,
    kind: "directory",
    childrenCount: 1,
  });
  const first = dir(60, "E9"),
    second = dir(61, "E8");
  const leaf: NodeView = {
    ...items[0],
    id: 62,
    parentId: 60,
    name: "leaf.bin",
    path: "/fixture/caf\uFFFD/leaf.bin",
    pathKey: "/fixture/caf%E9/leaf.bin",
  };
  const onLoadChildren = vi.fn(async (_id: number) => ({
    items: [leaf],
    total: 1,
  }));
  const shown = { ...props, items: [first, second], onLoadChildren };
  await act(() => root.render(FileRows, shown));
  await act(() =>
    root.render(FileRows, {
      ...shown,
      selection: [62],
      selectionPath: queuePathKey(leaf),
    }),
  );
  for (let i = 0; i < 5; i++) await act(async () => {});
  expect(onLoadChildren.mock.calls.map(([id]) => id)).toEqual([60]);
});

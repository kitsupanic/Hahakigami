// @vitest-environment jsdom
import { act, createTestRoot as createRoot, type Root } from "./testUtils";
import { afterEach, expect, it, vi } from "vitest";
import DiskMap from "./DiskMap.svelte";
import type { NodeView } from "./types";
import { layout, type Shape } from "./layout";
import { hitShape } from "./hitTest";
import { mapFontFamily } from "./uiFormat";
import { fileTypeColor } from "./fileTypes";
import { createHoverTarget, type HoverTarget } from "./hoverContext";

type TestGradient = CanvasGradient & { stops: [number, string][] };
function withGradients<T extends object>(context: T) {
  return Object.assign(context, {
    createLinearGradient: vi.fn(() => {
      const stops: [number, string][] = [];
      return {
        stops,
        addColorStop: vi.fn((offset: number, color: string) =>
          stops.push([offset, color]),
        ),
      } as unknown as TestGradient;
    }),
  });
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it("shows the focused total and labels only roomy sunburst branches", async () => {
  const gib = 1024 ** 3;
  const nodes: NodeView[] = [
    {
      id: 0,
      parentId: null,
      name: "Disk",
      allocatedBytes: gib,
    },
    {
      id: 1,
      parentId: 0,
      name: "Users",
      allocatedBytes: 0.93 * gib,
    },
    {
      id: 2,
      parentId: 0,
      name: "Tiny",
      allocatedBytes: 0.01 * gib,
    },
    {
      id: 3,
      parentId: 1,
      name: "Nested",
      allocatedBytes: 0.93 * gib,
    },
    {
      id: 4,
      parentId: 0,
      name: "Applications",
      allocatedBytes: 0.06 * gib,
    },
  ].map((node) => ({
    ...node,
    path: `/${node.name}`,
    kind: "directory",
    childrenCount: 0,
    incomplete: false,
  }));
  vi.stubGlobal(
    "Worker",
    class {
      onmessage:
        | ((event: { data: { requestId: number; shapes: Shape[] } }) => void)
        | null = null;
      postMessage(data: {
        requestId: number;
        width: number;
        height: number;
      }) {
        this.onmessage?.({
          data: {
            requestId: data.requestId,
            shapes: layout(
              nodes,
              0,
              data.width,
              data.height,
              "sunburst",
            ),
          },
        });
      }
      terminate() {}
    },
  );
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(private callback: (entries: unknown[]) => void) {}
      observe() {
        this.callback([{ contentRect: { width: 800, height: 800 } }]);
      }
      disconnect() {}
    },
  );
  const context = {
    ...Object.fromEntries(
      [
        "setLineDash",
        "setTransform",
        "clearRect",
        "beginPath",
        "rect",
        "arc",
        "closePath",
        "fill",
        "stroke",
        "save",
        "clip",
        "restore",
      ].map((key) => [key, vi.fn()]),
    ),
    measureText: (text: string) => ({ width: text.length * 6 }),
    fillText: vi.fn(),
  };
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
    context as unknown as CanvasRenderingContext2D,
  );
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element);
  const render = (queuedPaths: { path: string }[] = []) =>
    root.render(DiskMap, {
      nodes,
      rootId: 0,
      mode: "sunburst",
      selected: [],
      onSelect: vi.fn(),
      onOpen: vi.fn(),
      queuedPaths,
    });
  try {
    await act(() => render());
    expect(element.querySelector(".map-center")?.textContent).toContain(
      "1.0 GiB",
    );
    expect(element.querySelector(".map-center")?.textContent).toContain(
      "On disk",
    );
    expect(context.fillText).toHaveBeenCalledWith(
      "Users",
      expect.any(Number),
      expect.any(Number),
    );
    expect(context.fillText).toHaveBeenCalledWith(
      "93.0%",
      expect.any(Number),
      expect.any(Number),
    );
    expect(context.fillText).toHaveBeenCalledWith(
      "Applications",
      expect.any(Number),
      expect.any(Number),
    );
    const [, appX, appY] = context.fillText.mock.calls.find(
      ([text]) => text === "Applications",
    )!;
    const shapes = layout(nodes, 0, 800, 800, "sunburst");
    for (const dx of [-1, 1])
      for (const dy of [-1, 1])
        expect(
          hitShape(
            shapes,
            "sunburst",
            800,
            800,
            appX + dx * 38,
            appY + 16.2 + dy * 24.2,
          )?.id,
        ).toBe(4);
    expect(
      context.fillText.mock.calls.some(
        ([text]) => text === "Tiny" || text === "Nested",
      ),
    ).toBe(false);
    context.fillText.mockClear();
    await act(() => render([{ path: "/Users" }]));
    expect(context.fillText.mock.calls.some(([text]) => text === "Users")).toBe(
      false,
    );
    expect(context.fillText).toHaveBeenCalledWith(
      "✓",
      expect.any(Number),
      expect.any(Number),
    );
  } finally {
    await act(() => root.unmount());
    element.remove();
  }
});

it("keeps the first click target through canvas resizing and outside controls, invalidating changed contexts", async () => {
  let nodes: NodeView[] = ["directory", "directory", "file", "aggregate"].map(
    (kind, id) => ({
      id,
      parentId: id === 0 ? null : 0,
      name: `item-${id}`,
      path: `/item-${id}`,
      kind: kind as NodeView["kind"],
      allocatedBytes: 1,
      childrenCount: 0,
      incomplete: false,
    }),
  );
  let resized = false;
  let resize: (entries: unknown[]) => void = () => {};
  let finishLayout: (() => void) | null = null;
  let deferLayout = false;
  vi.stubGlobal(
    "Worker",
    class {
      onmessage:
        | ((event: { data: { requestId: number; shapes: Shape[] } }) => void)
        | null = null;
      postMessage(data: { requestId: number }) {
        const finish = () =>
          this.onmessage?.({
            data: {
              requestId: data.requestId,
              shapes: [1, 2, 3].map((id) => ({
                id,
                x: resized ? 100 + id * 20 : (id - 1) * 20,
                y: 0,
                width: 20,
                height: 20,
                start: 0,
                end: 0,
                inner: 0,
                outer: 0,
                depth: 0,
              })),
            },
          });
        if (deferLayout) finishLayout = finish;
        else finish();
      }
      terminate() {}
    },
  );
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(callback: typeof resize) {
        resize = callback;
      }
      observe() {}
      disconnect() {}
    },
  );
  const context = Object.fromEntries(
    [
      "setLineDash",
      "setTransform",
      "clearRect",
      "beginPath",
      "rect",
      "arc",
      "closePath",
      "fill",
      "stroke",
      "save",
      "clip",
      "restore",
      "fillText",
    ].map((key) => [key, vi.fn()]),
  );
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
    withGradients(context) as unknown as CanvasRenderingContext2D,
  );
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element),
    onSelect = vi.fn(),
    onOpen = vi.fn();
  const outside = document.createElement("button");
  const activate = vi.fn(),
    outsideDoubleClick = vi.fn();
  outside.onclick = activate;
  outside.ondblclick = outsideDoubleClick;
  document.body.append(outside);
  const outsideClick = async (detail: number, x = 10, type = "click") => {
    const event = new MouseEvent(type, {
      bubbles: true,
      cancelable: true,
      detail,
      clientX: x,
      clientY: 10,
    });
    await act(() => outside.dispatchEvent(event));
    return event;
  };
  let scanId = "scan-a",
    rootId = 0;
  let mode: "treemap" | "sunburst" = "treemap";
  const render = () =>
    root.render(DiskMap, {
      nodes,
      rootId,
      scanId,
      mode,
      selected: [],
      onSelect,
      onOpen,
    });
  try {
    await act(async () => render());
    const canvas = element.querySelector("canvas")!;
    const bounds = vi
      .spyOn(canvas, "getBoundingClientRect")
      .mockReturnValue({ left: 0, top: 0 } as DOMRect);
    const click = async (detail: number, x = 10, type = "click") =>
      act(async () => {
        canvas.dispatchEvent(
          new MouseEvent(type, {
            bubbles: true,
            detail,
            clientX: x,
            clientY: 10,
          }),
        );
      });
    for (const pending of [true, false]) {
      resized = false;
      await act(async () =>
        resize([{ contentRect: { width: 600, height: 400 } }]),
      );
      await click(1);
      bounds.mockClear();
      resized = true;
      deferLayout = pending;
      await act(async () =>
        resize([{ contentRect: { width: 400, height: 400 } }]),
      );
      await click(2);
      expect(onSelect).toHaveBeenLastCalledWith(nodes[1], false);
      await click(2, 10, "dblclick");
      expect(onOpen).toHaveBeenLastCalledWith(nodes[1]);
      const opens = onOpen.mock.calls.length;
      await click(2, 10, "dblclick");
      expect(onOpen).toHaveBeenCalledTimes(opens);
      expect(bounds).not.toHaveBeenCalled();
      if (pending) await act(async () => finishLayout?.());
      deferLayout = false;
    }
    resized = false;
    await act(async () =>
      resize([{ contentRect: { width: 600, height: 400 } }]),
    );
    // The second native click can now land on an inspector button outside
    // the resized canvas. It must never activate that button, even for a first
    // hit that deliberately captures no directory.
    for (const [x, expected] of [
      [10, nodes[1]],
      [30, null],
      [50, null],
      [90, null],
    ] as const) {
      activate.mockClear();
      outsideDoubleClick.mockClear();
      onOpen.mockClear();
      await click(1, x);
      expect((await outsideClick(2, x)).defaultPrevented).toBe(true);
      expect((await outsideClick(2, x, "dblclick")).defaultPrevented).toBe(
        true,
      );
      expect(activate).not.toHaveBeenCalled();
      expect(outsideDoubleClick).not.toHaveBeenCalled();
      if (expected) expect(onOpen).toHaveBeenCalledExactlyOnceWith(expected);
      else expect(onOpen).not.toHaveBeenCalled();
      await outsideClick(2, x, "dblclick");
      expect(onOpen).toHaveBeenCalledTimes(expected ? 1 : 0);
    }
    // A nearby second click is the same gesture; a distant native click is not.
    await click(1);
    expect((await outsideClick(2, 14)).defaultPrevented).toBe(true);
    expect((await outsideClick(2, 16)).defaultPrevented).toBe(false);
    activate.mockClear();
    outsideDoubleClick.mockClear();
    onOpen.mockClear();
    await click(1);
    expect((await outsideClick(1)).defaultPrevented).toBe(false);
    expect((await outsideClick(2)).defaultPrevented).toBe(false);
    expect((await outsideClick(2, 10, "dblclick")).defaultPrevented).toBe(
      false,
    );
    expect(activate).toHaveBeenCalledTimes(2);
    expect(outsideDoubleClick).toHaveBeenCalledOnce();
    expect(onOpen).not.toHaveBeenCalled();
    // The next two browser events point at a directory. Neither a blank,
    // aggregate, nor file first hit is allowed to turn into a folder open.
    for (const x of [30, 50, 90]) {
      onOpen.mockClear();
      await click(1, x);
      await click(2);
      await click(2, 10, "dblclick");
      expect(onOpen).not.toHaveBeenCalled();
    }
    const changes = [
      () => {
        scanId += "-new";
      },
      () => {
        rootId = 9;
      },
      () => {
        mode = "sunburst";
      },
      () => {
        nodes = nodes.map((n) =>
          n.id === 1 ? { ...n, path: "/reused-id" } : n,
        );
      },
      () => {
        nodes = nodes.map((n) => (n.id === 1 ? { ...n, kind: "file" } : n));
      },
      () => {
        nodes = nodes.map((n) => (n.id === 1 ? { ...n, parentId: 9 } : n));
      },
      () => {
        nodes = nodes.filter((n) => n.id !== 1);
      },
    ];
    for (const change of changes) {
      rootId = 0;
      mode = "treemap";
      nodes = nodes
        .filter((n) => n.id !== 1)
        .concat({
          id: 1,
          parentId: 0,
          name: "Folder",
          path: "/folder",
          kind: "directory",
          allocatedBytes: 1,
          childrenCount: 0,
          incomplete: false,
        });
      await act(async () => render());
      await click(1);
      onOpen.mockClear();
      await act(async () => {
        change();
        render();
      });
      expect((await outsideClick(2)).defaultPrevented).toBe(false);
      expect((await outsideClick(2, 10, "dblclick")).defaultPrevented).toBe(
        false,
      );
      await click(2);
      await click(2, 10, "dblclick");
      expect(onOpen).not.toHaveBeenCalled();
    }
    // A completed sequence cannot be reused by another isolated dblclick.
    await click(2, 10, "dblclick");
    expect(onOpen).not.toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    outside.remove();
    element.remove();
  }
});
it("hits once per pointer event on a full map, preserves actions, and refreshes its node index", async () => {
  const shapes: Shape[] = Array.from({ length: 4999 }, (_, i) => ({
    id: i + 1,
    x: (i % 100) * 10,
    y: Math.floor(i / 100) * 10,
    width: 10,
    height: 10,
    start: 0,
    end: 0,
    inner: 0,
    outer: 0,
    depth: 0,
  }));
  let nodes: NodeView[] = Array.from({ length: 5000 }, (_, id) => ({
    id,
    parentId: id === 0 ? null : 0,
    name: `item-${id}`,
    path: `/item-${id}`,
    kind: id === 4999 ? "directory" : id === 4998 ? "aggregate" : "file",
    allocatedBytes: 1,
    childrenCount: 0,
    incomplete: false,
  }));
  vi.stubGlobal(
    "Worker",
    class {
      onmessage:
        | ((event: { data: { requestId: number; shapes: Shape[] } }) => void)
        | null = null;
      postMessage(data: { requestId: number }) {
        this.onmessage?.({ data: { requestId: data.requestId, shapes } });
      }
      terminate() {}
    },
  );
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(private callback: (entries: unknown[]) => void) {}
      observe() {
        this.callback([{ contentRect: { width: 1000, height: 500 } }]);
      }
      disconnect() {}
    },
  );
  const ctx = Object.fromEntries(
    [
      "setLineDash",
      "setTransform",
      "clearRect",
      "beginPath",
      "rect",
      "arc",
      "closePath",
      "fill",
      "stroke",
      "save",
      "clip",
      "fillText",
      "strokeText",
      "restore",
    ].map((key) => [key, vi.fn()]),
  );
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
    withGradients(ctx) as unknown as CanvasRenderingContext2D,
  );
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element),
    onSelect = vi.fn(),
    onOpen = vi.fn();
  const render = () =>
    root.render(DiskMap, {
      nodes: nodes,
      rootId: 0,
      mode: "treemap",
      selected: [],
      onSelect: onSelect,
      onOpen: onOpen,
    });
  try {
    await act(async () => render());
    expect(ctx.createLinearGradient).not.toHaveBeenCalled();
    const canvas = element.querySelector("canvas")!;
    const bounds = vi.spyOn(canvas, "getBoundingClientRect").mockReturnValue({
      left: 0,
      top: 0,
      width: 1000,
      height: 500,
    } as DOMRect);
    const dispatch = async (
      type: string,
      x: number,
      y: number,
      metaKey = false,
    ) => {
      bounds.mockClear();
      await act(async () => {
        canvas.dispatchEvent(
          new MouseEvent(type, {
            bubbles: true,
            clientX: x,
            clientY: y,
            metaKey,
            detail: type === "dblclick" ? 2 : type === "click" ? 1 : 0,
          }),
        );
      });
      expect(bounds).toHaveBeenCalledTimes(type === "dblclick" ? 0 : 1);
    };
    await dispatch("mousemove", 985, 495);
    expect(element.querySelector(".map-caption")!.textContent?.trim()).toBe(
      "item-4999 · 1 B",
    );
    await dispatch("click", 985, 495, true);
    expect(onSelect).toHaveBeenLastCalledWith(nodes[4999], true);
    await dispatch("dblclick", 985, 495);
    expect(onOpen).toHaveBeenCalledWith(nodes[4999]);
    await dispatch("mousemove", 975, 495);
    expect(element.textContent).toContain(
      "item-4998 · 1 B · grouped small items",
    );
    onSelect.mockClear();
    onOpen.mockClear();
    await dispatch("click", 975, 495);
    await dispatch("dblclick", 975, 495);
    expect(onSelect).not.toHaveBeenCalled();
    expect(onOpen).not.toHaveBeenCalled();
    nodes = nodes.map((n) =>
      n.id === 4999 ? { ...n, name: "updated directory" } : n,
    );
    await act(async () => render());
    await dispatch("mousemove", 985, 495);
    expect(element.textContent).toContain("updated directory");
    await dispatch("mousemove", 2000, 2000);
    expect(element.querySelector(".map-help")).toBeNull();
  } finally {
    await act(async () => root.unmount());
    element.remove();
  }
});

it("uses contrasting dark labels for light root branches and descendants across repaints", async () => {
  const nodes: NodeView[] = [
    {
      id: 0,
      parentId: null,
      name: "Root",
      kind: "directory",
      allocatedBytes: 0,
    },
    {
      id: 1,
      parentId: 0,
      name: "Large folder",
      kind: "directory",
      allocatedBytes: 56 * 1024 ** 3,
    },
    {
      id: 2,
      parentId: 0,
      name: "Short block",
      kind: "file",
      allocatedBytes: 2 * 1024 ** 3,
    },
    {
      id: 3,
      parentId: 0,
      name: "Narrow block",
      kind: "file",
      allocatedBytes: 1023.9 * 1024 ** 4,
    },
    {
      id: 4,
      parentId: 0,
      name: "Expanded parent",
      kind: "directory",
      allocatedBytes: 5 * 1024 ** 3,
    },
    {
      id: 5,
      parentId: 4,
      name: "Tiny child",
      kind: "file",
      allocatedBytes: 5 * 1024 ** 3,
    },
    {
      id: 6,
      parentId: 0,
      name: "Library",
      kind: "directory",
      allocatedBytes: 92.4 * 1024 ** 3,
    },
    {
      id: 7,
      parentId: 6,
      name: "Application Support",
      kind: "directory",
      allocatedBytes: 2 * 1024 ** 3,
    },
    {
      id: 8,
      parentId: 7,
      name: "Logs",
      kind: "file",
      allocatedBytes: 1024 ** 3,
    },
    {
      id: 9,
      parentId: 7,
      name: "Caches",
      kind: "directory",
      allocatedBytes: 4 * 1024 ** 3,
    },
    {
      id: 10,
      parentId: 9,
      name: "Roomy file",
      kind: "file",
      allocatedBytes: 3 * 1024 ** 3,
    },
    {
      id: 11,
      parentId: 9,
      name: "Crowded short file",
      kind: "file",
      allocatedBytes: 0.5 * 1024 ** 3,
    },
    {
      id: 12,
      parentId: 7,
      name: "Deep folder",
      kind: "directory",
      allocatedBytes: 7 * 1024 ** 3,
    },
    {
      id: 14,
      parentId: 0,
      name: "Other",
      kind: "file",
      allocatedBytes: 1024 ** 3,
    },
    {
      id: 13,
      parentId: 12,
      name: "Omitted child",
      kind: "file",
      allocatedBytes: 7 * 1024 ** 3,
    },
  ].map((n) => ({
    ...n,
    path: `/${n.name}`,
    childrenCount: n.id === 4 ? 1 : 0,
    incomplete: false,
  })) as NodeView[];
  const shapes: Shape[] = [
    { id: 1, x: 0, y: 0, width: 300, height: 300 },
    { id: 2, x: 300, y: 0, width: 200, height: 40 },
    { id: 3, x: 300, y: 40, width: 91, height: 150 },
    { id: 4, x: 500, y: 0, width: 300, height: 300, headerHeight: 64 },
    { id: 5, x: 500, y: 64, width: 80, height: 80 },
    { id: 6, x: 800, y: 0, width: 140, height: 300, headerHeight: 64 },
    { id: 7, x: 800, y: 64, width: 140, height: 200, headerHeight: 28 },
    { id: 8, x: 800, y: 92, width: 140, height: 100 },
    { id: 9, x: 1000, y: 0, width: 200, height: 300, headerHeight: 56 },
    { id: 10, x: 1000, y: 56, width: 180, height: 100 },
    { id: 11, x: 1000, y: 156, width: 180, height: 70 },
    { id: 12, x: 1200, y: 0, width: 120, height: 120, headerHeight: 28 },
    { id: 14, x: 1320, y: 0, width: 80, height: 120 },
  ].map((s) => ({
    ...s,
    start: 0,
    end: 1,
    inner: 20,
    outer: 40,
    depth: s.id >= 8 && s.id !== 14 ? 2 : s.id === 7 || s.id === 5 ? 1 : 0,
  }));
  vi.stubGlobal(
    "Worker",
    class {
      onmessage:
        | ((event: { data: { requestId: number; shapes: Shape[] } }) => void)
        | null = null;
      postMessage(data: { requestId: number }) {
        this.onmessage?.({ data: { requestId: data.requestId, shapes } });
      }
      terminate() {}
    },
  );
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
  const drawn: { text: string; font: string; x: number; y: number }[] = [];
  const ctx = {
    ...Object.fromEntries(
      [
        "setLineDash",
        "setTransform",
        "clearRect",
        "beginPath",
        "rect",
        "arc",
        "closePath",
        "fill",
        "stroke",
        "save",
        "clip",
        "restore",
        "strokeText",
      ].map((key) => [key, vi.fn()]),
    ),
    rect: vi.fn(),
    arc: vi.fn(),
    stroke: vi.fn(),
    save: vi.fn(),
    restore: vi.fn(),
    font: "",
    fillStyle: "",
    strokeStyle: "",
    lineWidth: 1,
    strokeText: vi.fn(),
    measureText: vi.fn((text: string) => ({ width: text.length * 9 })),
    fillText: vi.fn((text: string, x: number, y: number) => {
      expect(ctx.fillStyle).toBe("#172033");
      expect(ctx.strokeText).not.toHaveBeenCalled();
      drawn.push({ text, font: ctx.font, x, y });
    }),
  };
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
    withGradients(ctx) as unknown as CanvasRenderingContext2D,
  );
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element);
  const render = async (mode: "treemap" | "sunburst" = "treemap") => {
    drawn.length = 0;
    await act(async () =>
      root.render(DiskMap, {
        nodes: nodes,
        rootId: 0,
        mode: mode,
        selected: [4],
        onSelect: vi.fn(),
        onOpen: vi.fn(),
      }),
    );
  };
  try {
    await render();
    expect(drawn).toContainEqual({
      text: "Large folder",
      font: `16px ${mapFontFamily}`,
      x: 12,
      y: 24,
    });
    expect(drawn).toContainEqual({
      text: "56.0 GiB",
      font: `600 18px ${mapFontFamily}`,
      x: 12,
      y: 47,
    });
    expect(drawn).toContainEqual({
      text: "Library",
      font: `16px ${mapFontFamily}`,
      x: 812,
      y: 24,
    });
    expect(drawn).toContainEqual({
      text: "92.4 GiB",
      font: `600 18px ${mapFontFamily}`,
      x: 812,
      y: 47,
    });
    expect(drawn).toContainEqual({
      text: "Application …",
      font: `13px ${mapFontFamily}`,
      x: 810,
      y: 86,
    });
    expect(drawn).toContainEqual({
      text: "Caches",
      font: `13px ${mapFontFamily}`,
      x: 1010,
      y: 22,
    });
    expect(drawn).toContainEqual({
      text: "4.0 GiB",
      font: `600 15px ${mapFontFamily}`,
      x: 1010,
      y: 43,
    });
    expect(drawn).toContainEqual({
      text: "Roomy file",
      font: `13px ${mapFontFamily}`,
      x: 1010,
      y: 78,
    });
    expect(drawn).toContainEqual({
      text: "3.0 GiB",
      font: `600 15px ${mapFontFamily}`,
      x: 1010,
      y: 99,
    });
    expect(
      drawn.some((d) =>
        ["Logs", "Crowded short file", "512 MiB"].includes(d.text),
      ),
    ).toBe(false);
    expect(drawn).toContainEqual({
      text: "Other",
      font: `16px ${mapFontFamily}`,
      x: 1332,
      y: 24,
    });
    expect(
      drawn.some((label) => label.x === 1332 && label.text === "1.0 GiB"),
    ).toBe(false);
    expect(ctx.save).toHaveBeenCalledTimes(ctx.restore.mock.calls.length);
    expect(drawn).toContainEqual({
      text: "Deep folder",
      font: `13px ${mapFontFamily}`,
      x: 1210,
      y: 22,
    });
    expect(drawn.some((d) => d.text === "7.0 GiB")).toBe(false);
    expect(drawn.some((d) => d.text === "Short block")).toBe(true);
    expect(
      drawn.some((d) => d.text.startsWith("Narrow") && d.text.endsWith("…")),
    ).toBe(true);
    expect(drawn.some((d) => d.text === "Expanded parent")).toBe(true);
    expect(ctx.arc).toHaveBeenCalledWith(792.5, 7.5, 6, -Math.PI / 2, 0);
    expect(ctx.stroke.mock.invocationCallOrder.at(-1)).toBeGreaterThan(
      ctx.fillText.mock.invocationCallOrder.at(-1)!,
    );
    expect(drawn.some((d) => /2\.0 GiB|1,?023\.9 TiB/.test(d.text))).toBe(
      false,
    );
    await render("sunburst");
    expect(drawn).toEqual([]);
  } finally {
    await act(async () => root.unmount());
    element.remove();
  }
});

it("reports drawing only for the current completed layout response", async () => {
  let callback:
    | ((event: { data: { requestId: number; shapes: Shape[] } }) => void)
    | null = null;
  let requested = 0;
  vi.stubGlobal(
    "Worker",
    class {
      set onmessage(value: typeof callback) {
        callback = value;
      }
      postMessage(data: { requestId: number }) {
        requested = data.requestId;
      }
      terminate() {}
    },
  );
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
  const context = Object.fromEntries(
    [
      "setLineDash",
      "setTransform",
      "clearRect",
      "beginPath",
      "rect",
      "arc",
      "closePath",
      "fill",
      "stroke",
      "save",
      "clip",
      "fillText",
      "strokeText",
      "restore",
    ].map((key) => [key, vi.fn()]),
  );
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
    withGradients(context) as unknown as CanvasRenderingContext2D,
  );
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element),
    onRendered = vi.fn(() => {
      ready = true;
    }),
    onLayoutPending = vi.fn(() => {
      ready = false;
    });
  let ready = false;
  let mode: "treemap" | "sunburst" = "treemap";
  let selected: number[] = [];
  let nodes: NodeView[] = [
    {
      id: 0,
      parentId: null,
      name: "root",
      path: "/root",
      kind: "directory",
      allocatedBytes: 0,
      childrenCount: 0,
      incomplete: false,
    },
  ];
  const render = () =>
    root.render(DiskMap, {
      nodes: nodes,
      rootId: 0,
      mode,
      selected,
      onSelect: () => {},
      onOpen: () => {},
      onRendered,
      onLayoutPending,
    });
  try {
    await act(async () => render());
    const previous = requested;
    expect(onLayoutPending).toHaveBeenCalledOnce();
    expect(ready).toBe(false);
    expect(onRendered).not.toHaveBeenCalled();
    nodes = [{ ...nodes[0], name: "latest" }];
    await act(async () => render());
    await act(async () =>
      callback?.({ data: { requestId: previous, shapes: [] } }),
    );
    expect(onRendered).not.toHaveBeenCalled();
    expect(ready).toBe(false);
    await act(async () =>
      callback?.({ data: { requestId: requested, shapes: [] } }),
    );
    expect(onRendered).toHaveBeenLastCalledWith(nodes);
    expect(context.clearRect).not.toHaveBeenCalled();
    expect(element.querySelector("canvas")!.width).toBe(0);
    expect(element.querySelector("canvas")!.height).toBe(0);
    expect(ready).toBe(true);
    const completed = requested;
    mode = "sunburst";
    await act(async () => render());
    expect(ready).toBe(false);
    expect(onLayoutPending).toHaveBeenCalledTimes(3);
    await act(async () =>
      callback?.({ data: { requestId: completed, shapes: [] } }),
    );
    expect(ready).toBe(false);
    await act(async () =>
      callback?.({ data: { requestId: requested, shapes: [] } }),
    );
    expect(ready).toBe(true);
    selected = [0];
    await act(async () => render());
    expect(onLayoutPending).toHaveBeenCalledTimes(3);
    expect(requested).toBe(completed + 1);
    expect(ready).toBe(true);
  } finally {
    await act(async () => root.unmount());
    element.remove();
  }
});

it("reuses the canvas backing store on redraws and resets the transform after pixel-size changes", async () => {
  let resize: (
    entries: { contentRect: { width: number; height: number } }[],
  ) => void = () => {};
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(callback: typeof resize) {
        resize = callback;
      }
      observe() {}
      disconnect() {}
    },
  );
  let requested = 0;
  let respond: ((shapes: Shape[]) => void) | undefined;
  const shape: Shape = {
    id: 1,
    x: 0,
    y: 0,
    width: 10,
    height: 10,
    start: 0,
    end: 0,
    inner: 0,
    outer: 0,
    depth: 0,
  };
  vi.stubGlobal(
    "Worker",
    class {
      onmessage:
        | ((event: { data: { requestId: number; shapes: Shape[] } }) => void)
        | null = null;
      postMessage(data: { requestId: number }) {
        requested = data.requestId;
        respond = (shapes) =>
          this.onmessage?.({ data: { requestId: requested, shapes } });
      }
      terminate() {}
    },
  );
  const context = {
    setLineDash: vi.fn(),
    setTransform: vi.fn(),
    clearRect: vi.fn(),
    beginPath: vi.fn(),
    rect: vi.fn(),
    arc: vi.fn(),
    closePath: vi.fn(),
    fill: vi.fn(),
    stroke: vi.fn(),
  };
  const getContext = vi
    .spyOn(HTMLCanvasElement.prototype, "getContext")
    .mockReturnValue(
      withGradients(context) as unknown as CanvasRenderingContext2D,
    );
  const width = vi.spyOn(HTMLCanvasElement.prototype, "width", "set");
  const height = vi.spyOn(HTMLCanvasElement.prototype, "height", "set");
  vi.stubGlobal("devicePixelRatio", 2);
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element);
  let selected: number[] = [];
  const nodes: NodeView[] = [];
  const render = () =>
    root.render(DiskMap, {
      nodes,
      rootId: 0,
      mode: "treemap",
      selected,
      onSelect: () => {},
      onOpen: () => {},
    });
  const clear = () => {
    width.mockClear();
    height.mockClear();
    context.setTransform.mockClear();
  };
  try {
    await act(async () => render());
    const canvas = element.querySelector("canvas")!;
    expect([canvas.width, canvas.height]).toEqual([0, 0]);
    expect(getContext).not.toHaveBeenCalled();
    clear();
    await act(async () =>
      resize([{ contentRect: { width: 600, height: 400 } }]),
    );
    expect(getContext).not.toHaveBeenCalled();
    expect(width).not.toHaveBeenCalled();
    expect(height).not.toHaveBeenCalled();
    await act(async () => respond?.([shape]));
    expect([canvas.width, canvas.height]).toEqual([1200, 800]);
    clear();
    selected = [1];
    await act(async () => render());
    expect(width).not.toHaveBeenCalled();
    expect(height).not.toHaveBeenCalled();
    expect(context.setTransform).toHaveBeenLastCalledWith(2, 0, 0, 2, 0, 0);
    clear();
    await act(async () =>
      resize([{ contentRect: { width: 600.4, height: 400.4 } }]),
    );
    expect(width).not.toHaveBeenCalled();
    expect(height).not.toHaveBeenCalled();
    expect(context.clearRect).toHaveBeenLastCalledWith(0, 0, 600.4, 400.4);
    clear();
    await act(async () =>
      resize([{ contentRect: { width: 700, height: 400.4 } }]),
    );
    expect(width).toHaveBeenCalledExactlyOnceWith(1400);
    expect(height).not.toHaveBeenCalled();
    expect(context.setTransform).toHaveBeenLastCalledWith(2, 0, 0, 2, 0, 0);
    clear();
    vi.stubGlobal("devicePixelRatio", 1.5);
    selected = [2];
    await act(async () => render());
    expect(width).toHaveBeenCalledExactlyOnceWith(1050);
    expect(height).toHaveBeenCalledExactlyOnceWith(600);
    expect(context.setTransform).toHaveBeenLastCalledWith(1.5, 0, 0, 1.5, 0, 0);
    clear();
    selected = [];
    await act(async () => render());
    expect(width).not.toHaveBeenCalled();
    expect(height).not.toHaveBeenCalled();
    expect(context.setTransform).toHaveBeenLastCalledWith(1.5, 0, 0, 1.5, 0, 0);
    clear();
    getContext.mockClear();
    await act(async () => respond?.([]));
    expect([canvas.width, canvas.height]).toEqual([0, 0]);
    expect(width).toHaveBeenCalledExactlyOnceWith(0);
    expect(height).toHaveBeenCalledExactlyOnceWith(0);
    expect(getContext).not.toHaveBeenCalled();
    clear();
    await act(async () => respond?.([]));
    expect(width).not.toHaveBeenCalled();
    expect(height).not.toHaveBeenCalled();
    await act(async () => respond?.([shape]));
    expect([canvas.width, canvas.height]).toEqual([1050, 600]);
    expect(context.setTransform).toHaveBeenLastCalledWith(1.5, 0, 0, 1.5, 0, 0);
  } finally {
    await act(async () => root.unmount());
    element.remove();
  }
});

it("repaints persistent queue markers independently of selection and layout in both modes", async () => {
  const nodes = [
    {
      id: 1,
      parentId: null,
      name: "Folder",
      path: "/folder",
      kind: "directory",
    },
    { id: 2, parentId: 1, name: "File", path: "/folder/file", kind: "file" },
    { id: 3, parentId: null, name: "Other", path: "/other", kind: "file" },
  ].map((n) => ({
    ...n,
    allocatedBytes: 1,
    childrenCount: 0,
    incomplete: false,
  })) as NodeView[];
  const shapes = nodes.map((n, i) => ({
    id: n.id,
    x: i * 150,
    y: 0,
    width: 150,
    height: 100,
    headerHeight: 30,
    start: i * 2,
    end: (i + 1) * 2,
    inner: 40,
    outer: 100,
    depth: 0,
  }));
  const postMessage = vi.fn();
  vi.stubGlobal(
    "Worker",
    class {
      onmessage: ((event: unknown) => void) | null = null;
      postMessage(data: { requestId: number }) {
        postMessage(data);
        this.onmessage?.({ data: { requestId: data.requestId, shapes } });
      }
      terminate() {}
    },
  );
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
  const strokes: { color: string; dash: number[] }[] = [];
  let dash: number[] = [];
  const ctx = {
    ...Object.fromEntries(
      [
        "setTransform",
        "clearRect",
        "beginPath",
        "rect",
        "arc",
        "closePath",
        "fill",
        "save",
        "clip",
        "restore",
        "fillText",
        "strokeText",
      ].map((key) => [key, vi.fn()]),
    ),
    fillText: vi.fn(),
    strokeStyle: "",
    measureText: (text: string) => ({ width: text.length * 9 }),
    setLineDash: vi.fn((value: number[]) => {
      dash = value;
    }),
    stroke: vi.fn(() => strokes.push({ color: ctx.strokeStyle, dash })),
  };
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
    withGradients(ctx) as unknown as CanvasRenderingContext2D,
  );
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element),
    onSelect = vi.fn();
  let queuedPaths: string[] = [],
    selected = [2];
  let mode: "treemap" | "sunburst" = "treemap";
  const render = () =>
    root.render(DiskMap, {
      nodes,
      rootId: 1,
      mode,
      selected,
      queuedPaths: queuedPaths.map((path) => ({ path })),
      onSelect,
      onOpen: () => {},
    });
  try {
    await act(async () => render());
    const requests = postMessage.mock.calls.length;
    queuedPaths = ["/folder/file"];
    strokes.length = 0;
    await act(async () => render());
    expect(postMessage).toHaveBeenCalledTimes(requests);
    expect(
      element.querySelector(".map-wrap")?.getAttribute("data-queued-count"),
    ).toBe("1");
    expect(
      element.querySelector(".map-wrap")?.getAttribute("data-contains-count"),
    ).toBe("1");
    expect(strokes).toContainEqual({ color: "#8edbb2", dash: [4, 3] });
    expect(strokes).toContainEqual({ color: "#8edbb2", dash: [] });
    expect(strokes).toContainEqual({ color: "#fff", dash: [] });
    expect(strokes.at(-1)).toEqual({ color: "#246fde", dash: [] });
    selected = [3];
    await act(async () => render());
    queuedPaths = ["/folder"];
    await act(async () => render());
    expect(
      element.querySelector(".map-wrap")?.getAttribute("data-queued-count"),
    ).toBe("2");
    // A short parent has no reserved header: its badge must not cover a child.
    queuedPaths = ["/folder/file"];
    Object.assign(shapes[0], {
      x: 0,
      y: 0,
      width: 200,
      height: 60,
      headerHeight: undefined,
    });
    Object.assign(shapes[1], {
      x: 0,
      y: 0,
      width: 100,
      height: 60,
      headerHeight: undefined,
      depth: 1,
    });
    Object.assign(shapes[2], {
      x: 100,
      y: 0,
      width: 100,
      height: 60,
      headerHeight: undefined,
      depth: 1,
    });
    ctx.fillText.mockClear();
    await act(async () => render());
    expect(ctx.fillText).toHaveBeenCalledWith("✓", 76, 21);
    expect(ctx.fillText).not.toHaveBeenCalledWith("○", 176, 21);
    const shortCanvas = element.querySelector("canvas")!;
    vi.spyOn(shortCanvas, "getBoundingClientRect").mockReturnValue({
      left: 0,
      top: 0,
    } as DOMRect);
    await act(async () =>
      shortCanvas.dispatchEvent(
        new MouseEvent("click", { clientX: 182, clientY: 16, bubbles: true }),
      ),
    );
    expect(onSelect).toHaveBeenLastCalledWith(nodes[2], false);
    queuedPaths = ["/folder"];
    mode = "sunburst";
    strokes.length = 0;
    await act(async () => render());
    expect(strokes).toContainEqual({ color: "#8edbb2", dash: [] });
    expect(strokes).toContainEqual({ color: "#fff", dash: [] });
    expect(strokes.at(-1)?.color).toBe("#246fde");
    const canvas = element.querySelector("canvas")!;
    vi.spyOn(canvas, "getBoundingClientRect").mockReturnValue({
      left: 0,
      top: 0,
    } as DOMRect);
    await act(async () =>
      canvas.dispatchEvent(
        new MouseEvent("click", { clientX: 360, clientY: 200, bubbles: true }),
      ),
    );
    expect(onSelect).toHaveBeenLastCalledWith(nodes[0], false);
    const beforeRemoval = postMessage.mock.calls.length;
    queuedPaths = [];
    strokes.length = 0;
    await act(async () => render());
    expect(postMessage).toHaveBeenCalledTimes(beforeRemoval);
    expect(strokes.some((s) => s.color === "#8edbb2")).toBe(false);
  } finally {
    await act(async () => root.unmount());
    element.remove();
  }
});

it("keeps the compact legend aligned with focused children, their map colors, and on-disk sizes", async () => {
  vi.stubGlobal(
    "Worker",
    class {
      postMessage() {}
      terminate() {}
    },
  );
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
  let nodes: NodeView[] = [
    {
      id: 0,
      parentId: null,
      name: "Root",
      allocatedBytes: 1000,
    },
    {
      id: 1,
      parentId: 0,
      name: "Users",
      allocatedBytes: 500,
    },
    {
      id: 3,
      parentId: 0,
      name: "Small items",
      allocatedBytes: 100,
      kind: "aggregate",
    },
    {
      id: 4,
      parentId: 1,
      name: "Documents",
      allocatedBytes: 300,
    },
    { id: 5, parentId: 0, name: "Empty", allocatedBytes: 0 },
  ].map((node) => ({
    kind: "directory",
    path: `/${node.name}`,
    childrenCount: 0,
    incomplete: false,
    ...node,
  })) as NodeView[];
  let rootId = 0;
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element);
  const render = () =>
    root.render(DiskMap, {
      nodes,
      rootId,
      mode: "treemap",
      selected: [],
      onSelect: vi.fn(),
      onOpen: vi.fn(),
    });
  const entries = () => Array.from(element.querySelectorAll(".map-legend li"));
  try {
    await act(async () => render());
    expect(
      entries().map((entry) => entry.textContent?.trim().replace(/\s+/g, " ")),
    ).toEqual(["Users 500 B", "Small items 100 B"]);
    expect(
      (entries()[0].querySelector(".legend-dot") as HTMLElement).style
        .background,
    ).toBe("rgb(60, 184, 166)");
    expect(
      (entries()[1].querySelector(".legend-dot") as HTMLElement).style
        .background,
    ).toBe("rgb(129, 138, 155)");
    expect(element.querySelector("canvas")?.getAttribute("aria-label")).toBe(
      "Treemap disk usage map",
    );
    expect(element.querySelector(".map-help")).toBeNull();
    rootId = 1;
    await act(async () => render());
    expect(entries()).toHaveLength(1);
    expect(entries()[0].textContent).toContain("Documents");
    expect(entries()[0].textContent).toContain("300 B");
    nodes = nodes.map((node) =>
      node.id === 4
        ? { ...node, name: "Updated", allocatedBytes: 600 }
        : node,
    );
    await act(async () => render());
    expect(entries()[0].textContent).toContain("Updated");
    expect(entries()[0].textContent).toContain("600 B");
    rootId = 0;
    nodes = [
      ...nodes,
      ...Array.from({ length: 7 }, (_, i) => ({
        ...nodes[1],
        id: 10 + i,
        name: `Extra ${i}`,
        allocatedBytes: 10,
      })),
    ];
    await act(async () => render());
    expect(entries()).toHaveLength(6);
    const more = element.querySelector<HTMLButtonElement>(".legend-more")!;
    expect(more.textContent).toContain("+3 more");
    await act(async () => more.click());
    expect(entries()).toHaveLength(9);
    expect(more.getAttribute("aria-expanded")).toBe("true");
    const legend = element.querySelector(".map-legend")!;
    const folderSizes = legend.querySelector<HTMLUListElement>("ul")!;
    expect(folderSizes.getAttribute("tabindex")).toBe("0");
    expect(folderSizes.getAttribute("aria-label")).toBe("Folder sizes");
    expect(legend.hasAttribute("tabindex")).toBe(false);
    expect(folderSizes.contains(more)).toBe(false);
    folderSizes.focus();
    expect(document.activeElement).toBe(folderSizes);
    await act(async () => more.click());
    expect(entries()).toHaveLength(6);
    expect(entries()).toHaveLength(6);
    rootId = 5;
    await act(async () => render());
    expect(element.querySelector(".map-legend")).toBeNull();
  } finally {
    await act(async () => root.unmount());
    element.remove();
  }
});

async function linkedMapFixture(typedGroups = false) {
  const nodes: NodeView[] = [
    { id: 0, parentId: null, name: "Root", path: "/", kind: "directory" },
    { id: 1, parentId: 0, name: "Users", path: "/Users", kind: "directory" },
    {
      id: 2,
      parentId: 1,
      name: "Documents",
      path: "/Users/Documents",
      kind: "directory",
    },
    {
      id: 3,
      parentId: 2,
      name: "report.txt",
      path: "/Users/Documents/report.txt",
      kind: "file",
    },
    { id: 4, parentId: 1, name: "Small items", path: "", kind: "aggregate" },
  ].map((node) => ({
    ...node,
    allocatedBytes: 1024,
    childrenCount: 0,
    incomplete: false,
  })) as NodeView[];
  if (typedGroups)
    nodes.push(
      {
        ...nodes[4],
        id: 5,
        parentId: 2,
        name: "Movie summary",
        fileType: "mov",
        fileCount: 12,
      },
      {
        ...nodes[4],
        id: 6,
        parentId: 1,
        name: "Other movies",
        fileType: "mov",
        fileCount: 6,
      },
      {
        ...nodes[4],
        id: 7,
        parentId: 2,
        name: "No extension summary",
        fileType: "",
        fileCount: 3,
      },
    );
  const shapes: Shape[] = (typedGroups ? [2, 3, 4, 5, 6, 7] : [2, 3, 4]).map(
    (id, index) => ({
      id,
      x: index * (typedGroups ? 90 : 150),
      y: 0,
      width: typedGroups ? 85 : 140,
      height: 100,
      start: index * 2,
      end: (index + 1) * 2,
      inner: 20,
      outer: 100,
      depth: index === 1 ? 2 : 1,
    }),
  );
  let reply:
    | ((event: { data: { requestId: number; shapes: Shape[] } }) => void)
    | null = null;
  const postMessage = vi.fn();
  vi.stubGlobal(
    "Worker",
    class {
      set onmessage(value: typeof reply) {
        reply = value;
      }
      postMessage(data: { requestId: number }) {
        postMessage(data);
      }
      terminate() {}
    },
  );
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
  const strokes: { color: string; dash: number[] }[] = [];
  const fills: { color: string; alpha: number }[] = [];
  let dash: number[] = [];
  const context = {
    ...Object.fromEntries(
      [
        "setTransform",
        "clearRect",
        "beginPath",
        "rect",
        "arc",
        "closePath",
        "fill",
        "save",
        "clip",
        "fillText",
        "strokeText",
        "restore",
      ].map((key) => [key, vi.fn()]),
    ),
    measureText: (text: string) => ({ width: text.length * 7 }),
    clearRect: vi.fn(),
    rect: vi.fn(),
    arc: vi.fn(),
    strokeStyle: "",
    fillStyle: "",
    globalAlpha: 1,
    fill: vi.fn(() =>
      fills.push({ color: context.fillStyle, alpha: context.globalAlpha }),
    ),
    setLineDash: vi.fn((value: number[]) => {
      dash = value;
    }),
    stroke: vi.fn(() => strokes.push({ color: context.strokeStyle, dash })),
  };
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
    withGradients(context) as unknown as CanvasRenderingContext2D,
  );
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element),
    onHover = vi.fn(),
    onSelect = vi.fn(),
    onOpen = vi.fn();
  let hovered: HoverTarget | null = null;
  let hoverEpoch = 0;
  let mode: "treemap" | "sunburst" = "treemap";
  let typeProps: {
    colorMode?: "folder" | "fileType";
    highlightedFileType?: string | null;
    fileTypeHighlights?: readonly number[];
    fileTypeHighlightPending?: boolean;
    onFileTypeSelect?: (extension: string) => void;
  } = {};
  const render = () =>
    root.render(DiskMap, {
      ...typeProps,
      nodes,
      rootId: 0,
      mode,
      selected: [3],
      queuedPaths: [nodes[3]],
      hovered,
      hoverEpoch,
      onHover,
      onSelect,
      onOpen,
    });
  await act(async () => render());
  const canvas = element.querySelector("canvas")!;
  vi.spyOn(canvas, "getBoundingClientRect").mockReturnValue({
    left: 0,
    top: 0,
  } as DOMRect);
  const currentRequest = () =>
    postMessage.mock.lastCall![0].requestId as number;
  const complete = async (requestId = currentRequest(), result = shapes) => {
    await act(async () => reply?.({ data: { requestId, shapes: result } }));
  };
  const pointer = async (type: string, x = 170, y = 30) => {
    await act(async () =>
      canvas.dispatchEvent(
        new MouseEvent(type, { bubbles: true, clientX: x, clientY: y }),
      ),
    );
  };
  return {
    nodes,
    shapes,
    context,
    strokes,
    fills,
    fileTypes: async (props: typeof typeProps) => {
      typeProps = props;
      await act(async () => render());
    },
    element,
    canvas,
    onHover,
    onSelect,
    onOpen,
    postMessage,
    complete,
    currentRequest,
    pointer,
    hover: async (target: HoverTarget | null) => {
      hovered = target;
      await act(async () => render());
    },
    resetHoverContext: async () => {
      hoverEpoch++;
      await act(async () => render());
    },
    mode: async (value: typeof mode) => {
      mode = value;
      await act(async () => render());
    },
    cleanup: async () => {
      await act(async () => root.unmount());
      element.remove();
    },
  };
}

it("links exact and ancestor map hover with nested context, deduplicates pointer moves, and retains queue and selection outlines", async () => {
  const f = await linkedMapFixture();
  try {
    await f.complete();
    const requests = f.postMessage.mock.calls.length;
    const target = createHoverTarget(
      f.nodes[3],
      new Map(f.nodes.map((n) => [n.id, n])),
    );
    await f.hover(target);
    expect(f.element.querySelector(".map-caption")?.textContent).toContain(
      "Users › Documents › report.txt · 1 KiB",
    );
    expect(f.element.querySelector(".map-caption")?.getAttribute("title")).toBe(
      f.nodes[3].path,
    );
    expect(f.canvas.getAttribute("title")).toBe(f.nodes[3].path);
    expect(f.element.textContent).not.toContain("Map shows");
    expect(f.strokes).toContainEqual({ color: "#ffc66d", dash: [6, 3] });
    expect(f.strokes).toContainEqual({ color: "#8edbb2", dash: [] });
    expect(f.strokes).toContainEqual({ color: "#fff", dash: [] });
    expect(f.onHover).not.toHaveBeenCalled();
    await f.hover(null);
    await f.pointer("mousemove");
    expect(f.onHover).toHaveBeenLastCalledWith(target);
    const paints = f.context.clearRect.mock.calls.length;
    await f.pointer("mousemove", 171, 31);
    expect(f.onHover).toHaveBeenCalledOnce();
    expect(f.context.clearRect).toHaveBeenCalledTimes(paints);
    await f.pointer("mouseleave");
    expect(f.onHover).toHaveBeenLastCalledWith(null);
    expect(f.element.querySelector(".map-help")).toBeNull();
    // A list descendant absent from the map still highlights its closest real ancestor.
    await f.complete(f.currentRequest(), [f.shapes[0], f.shapes[2]]);
    f.context.rect.mockClear();
    f.context.arc.mockClear();
    await f.hover({ node: target.node, lineage: [3, 2] });
    expect(f.element.textContent).toContain(
      "Users › Documents › report.txt · 1 KiB",
    );
    expect(f.element.querySelector(".map-caption")?.textContent).toContain(
      "Map shows Documents",
    );
    expect(f.canvas.getAttribute("title")).toBe(f.nodes[3].path);
    // Hover traces the selection outline's edge: the tile inset 1.5px, 5px corners.
    expect(f.context.arc.mock.calls.at(-4)).toEqual([133.5, 6.5, 5, -Math.PI / 2, 0]);
    expect(f.postMessage).toHaveBeenCalledTimes(requests);
    await f.hover({
      node: { ...f.nodes[3], id: 99, parentId: null },
      lineage: [99],
    });
    expect(f.element.querySelector(".map-help")).toBeNull();
    await f.pointer("mousemove", 320);
    expect(f.onHover).toHaveBeenLastCalledWith(null);
    expect(f.element.textContent).toContain(
      "Small items · 1 KiB · grouped small items",
    );
    await f.pointer("click", 320);
    expect(f.onSelect).not.toHaveBeenCalled();
  } finally {
    await f.cleanup();
  }
});

it("draws the hover outline on its own canvas without repainting the map", async () => {
  const f = await linkedMapFixture();
  try {
    await f.complete();
    await f.pointer("mousemove");
    const [, overlay] = Array.from(f.element.querySelectorAll("canvas"));
    expect(overlay.getAttribute("aria-hidden")).toBe("true");
    expect(f.strokes.at(-1)).toEqual({ color: "#ffc66d", dash: [6, 3] });
    const fills = f.fills.length;
    f.strokes.length = 0;
    // A different tile: the outline is redrawn, the tiles are not.
    await f.pointer("mousemove", 20, 30);
    expect(f.fills).toHaveLength(fills);
    expect(f.strokes).toEqual([{ color: "#ffc66d", dash: [6, 3] }]);
  } finally {
    await f.cleanup();
  }
});

it("clears owned hover and suppresses hits and external outlines until the current layout completes", async () => {
  const f = await linkedMapFixture();
  try {
    await f.complete();
    await f.pointer("mousemove");
    const oldRequest = f.currentRequest();
    await f.mode("sunburst");
    expect(f.onHover).toHaveBeenLastCalledWith(null);
    const publications = f.onHover.mock.calls.length;
    await f.hover(
      createHoverTarget(f.nodes[3], new Map(f.nodes.map((n) => [n.id, n]))),
    );
    f.strokes.length = 0;
    await f.pointer("mousemove");
    await f.pointer("click");
    expect(f.onSelect).not.toHaveBeenCalled();
    expect(f.onHover).toHaveBeenCalledTimes(publications);
    expect(f.element.querySelector(".map-help")).toBeNull();
    await f.complete(oldRequest);
    expect(f.element.querySelector(".map-help")).toBeNull();
    expect(f.strokes).toHaveLength(0);
    await f.complete();
    expect(f.element.textContent).toContain("report.txt · 1 KiB");
    expect(f.strokes).toContainEqual({ color: "#ffc66d", dash: [6, 3] });
    expect(f.context.arc).toHaveBeenCalled();
    await f.mode("treemap");
    await f.complete();
    await f.pointer("mousemove");
    await f.cleanup();
    expect(f.onHover).toHaveBeenLastCalledWith(null);
  } finally {
    if (f.element.isConnected) await f.cleanup();
  }
});

it("honors controlled hover clears and replacements without publishing obsolete clears", async () => {
  const f = await linkedMapFixture();
  try {
    await f.complete();
    const index = new Map(f.nodes.map((node) => [node.id, node]));
    const report = createHoverTarget(f.nodes[3], index);
    const documents = createHoverTarget(f.nodes[2], index);
    const wrap = f.element.querySelector(".map-wrap")!;
    await f.pointer("mousemove");
    await f.hover(report); // The parent echoes the map's publication.
    expect(wrap.getAttribute("data-hovered-id")).toBe("3");
    f.onHover.mockClear();
    await f.hover(null); // A search/filter reset while the pointer stays still.
    expect(wrap.hasAttribute("data-hovered-id")).toBe(false);
    expect(f.element.querySelector(".map-help")).toBeNull();
    expect(f.onHover).not.toHaveBeenCalled();

    await f.pointer("mousemove");
    await f.hover(report);
    f.onHover.mockClear();
    await f.hover(documents); // A new list hover must replace the local tile.
    expect(wrap.getAttribute("data-hovered-id")).toBe("2");
    expect(f.element.querySelector(".map-caption")?.textContent).toContain(
      "Users › Documents · 1 KiB",
    );
    expect(f.onHover).not.toHaveBeenCalled();
    await f.pointer("mouseleave");
    expect(f.onHover).not.toHaveBeenCalled();
    expect(wrap.getAttribute("data-hovered-id")).toBe("2");

    // Aggregate pointer feedback survives the null that its own publication causes.
    await f.pointer("mousemove", 320);
    expect(f.onHover).toHaveBeenLastCalledWith(null);
    await f.hover(null);
    expect(wrap.getAttribute("data-hovered-id")).toBe("4");
    expect(f.element.textContent).toContain(
      "Small items · 1 KiB · grouped small items",
    );
    const requests = f.postMessage.mock.calls.length;
    f.onHover.mockClear();
    await f.resetHoverContext(); // Query/filter reset with shared hover already null.
    expect(wrap.hasAttribute("data-hovered-id")).toBe(false);
    expect(f.element.querySelector(".map-help")).toBeNull();
    expect(f.onHover).not.toHaveBeenCalled();
    expect(f.postMessage).toHaveBeenCalledTimes(requests);
  } finally {
    await f.cleanup();
  }
});

it.each(["treemap", "sunburst"] as const)(
  "paints %s shapes, legend, and lazy list colors from the same ranked palette",
  async (mode) => {
    const { layout } = await import("./layout");
    const { createBranchColorResolver, neutralBranchColor } = await import(
      "./branchColors"
    );
    const nodes: NodeView[] = [
      { id: 100, parentId: null, path: "/", allocatedBytes: 1000 },
      { id: 88, parentId: 100, path: "/largest", allocatedBytes: 600 },
      { id: 2, parentId: 100, path: "/second", allocatedBytes: 300 },
      { id: 5, parentId: 88, path: "/largest/nested", allocatedBytes: 200 },
      {
        id: -1,
        parentId: 100,
        path: "/remaining",
        allocatedBytes: 100,
        kind: "aggregate",
      },
      { id: 6, parentId: 5, path: "/largest/nested/deep", allocatedBytes: 200 },
    ].map((row) => ({
      name: row.path.split("/").pop() || "/",
      kind: "directory",
      childrenCount: 0,
      incomplete: false,
      ...row,
    })) as NodeView[];
    const painted: (string | CanvasGradient)[] = [];
    const ctx = Object.fromEntries(
      [
        "setLineDash",
        "setTransform",
        "clearRect",
        "beginPath",
        "rect",
        "arc",
        "closePath",
        "stroke",
        "save",
        "clip",
        "fillText",
        "strokeText",
        "restore",
      ].map((key) => [key, vi.fn()]),
    ) as unknown as CanvasRenderingContext2D;
    ctx.fill = vi.fn(() => {
      painted.push(ctx.fillStyle as string | CanvasGradient);
    });
    ctx.measureText = vi.fn(() => ({ width: 10 }) as TextMetrics);
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
      withGradients(ctx),
    );
    let delivered: Shape[] = [];
    vi.stubGlobal(
      "Worker",
      class {
        onmessage:
          | ((event: { data: { requestId: number; shapes: Shape[] } }) => void)
          | null = null;
        postMessage(data: {
          requestId: number;
          width: number;
          height: number;
          mode: "treemap" | "sunburst";
        }) {
          delivered = layout(
            nodes,
            100,
            data.width,
            data.height,
            data.mode,
          );
          this.onmessage?.({
            data: { requestId: data.requestId, shapes: delivered },
          });
        }
        terminate() {}
      },
    );
    vi.stubGlobal(
      "ResizeObserver",
      class {
        observe() {}
        disconnect() {}
      },
    );
    const element = document.createElement("div");
    document.body.append(element);
    const root = createRoot(element);
    try {
      await act(async () =>
        root.render(DiskMap, {
          nodes,
          rootId: 100,
          mode,
          selected: [],
          onSelect: vi.fn(),
          onOpen: vi.fn(),
        }),
      );
      const color = createBranchColorResolver(nodes, 100);
      expect(delivered.length).toBeGreaterThan(0);
      for (const [index, shape] of delivered.entries()) {
        const row = nodes.find((candidate) => candidate.id === shape.id)!;
        const base = color(row);
        const fill = painted[index];
        if (mode === "sunburst" || row.kind === "aggregate") {
          expect(fill).toBe(base);
          continue;
        }
        const stops = (fill as TestGradient).stops;
        expect(stops.map(([offset]) => offset)).toEqual([0, 0.5, 1]);
        if (shape.depth === 0) expect(stops[1][1]).toBe(base);
        else {
          // Child shading keeps its branch hue: it is a white blend of the
          // original branch, including deeper descendants, never a new hue.
          const rgb = (hex: string) =>
            [1, 3, 5].map((offset) =>
              parseInt(hex.slice(offset, offset + 2), 16),
            );
          const channels = rgb(base);
          for (const [, stop] of stops) {
            const tinted = rgb(stop);
            expect(
              tinted.every((value, channel) => value >= channels[channel]),
            ).toBe(true);
            const ratios = channels.flatMap((value, channel) =>
              255 - value >= 30
                ? [(tinted[channel] - value) / (255 - value)]
                : [],
            );
            expect(Math.max(...ratios) - Math.min(...ratios)).toBeLessThan(
              0.025,
            );
          }
        }
      }
      if (mode === "treemap") {
        const nestedFills = delivered.flatMap((shape, index) =>
          shape.depth > 0 ? [painted[index] as TestGradient] : [],
        );
        expect(nestedFills).toHaveLength(2);
        expect(nestedFills[0].stops).toEqual(nestedFills[1].stops);
      }
      const entries = Array.from(
        element.querySelectorAll<HTMLElement>(".map-legend li"),
      );
      for (const entry of entries) {
        const name = entry.querySelector(".legend-name")!.textContent;
        const row = nodes.find((row) => row.name === name)!;
        const expected = document.createElement("span");
        expected.style.background = color(row);
        expect(
          (entry.querySelector(".legend-dot") as HTMLElement).style.background,
        ).toBe(expected.style.background);
      }
      expect(
        color({
          ...nodes[3],
          id: 500,
          parentId: 999,
          path: "/largest/missing/child",
        }),
      ).toBe(color(nodes[1]));
      expect(color(nodes[4])).toBe(neutralBranchColor);
    } finally {
      await act(async () => root.unmount());
      element.remove();
    }
  },
);

it.each(["treemap", "sunburst"] as const)(
  "links %s file types by scoped IDs, suppresses pending matches, and repaints without changing layout or selection",
  async (mode) => {
    const f = await linkedMapFixture();
    try {
      await f.mode(mode);
      await f.complete();
      const beforeColorMode = f.postMessage.mock.calls.length;
      f.fills.length = 0;
      f.strokes.length = 0;
      await f.fileTypes({
        colorMode: "fileType",
        highlightedFileType: "txt",
        fileTypeHighlights: [2, 3, 4],
      });
      expect(f.postMessage).toHaveBeenCalledTimes(beforeColorMode + 1);
      await f.complete();
      const requests = f.postMessage.mock.calls.length;
      f.fills.length = 0;
      f.strokes.length = 0;
      await f.fileTypes({
        colorMode: "fileType",
        highlightedFileType: "txt",
        fileTypeHighlights: [2, 3, 4],
      });
      expect(f.fills.slice(0, 3)).toEqual([
        { color: "#e2e7ef", alpha: 1 },
        { color: fileTypeColor("txt"), alpha: 1 },
        { color: "#818a9b", alpha: 1 },
      ]);
      expect(f.strokes.filter((s) => s.color === fileTypeColor("txt"))).toEqual(
        [{ color: fileTypeColor("txt"), dash: [] }],
      );
      expect(f.strokes.some((s) => s.color === "#fff")).toBe(true);
      expect(f.strokes.some((s) => s.color === "#246fde")).toBe(true);
      expect(f.strokes.some((s) => s.color === "#8edbb2")).toBe(true);
      expect(f.element.textContent).toContain("Click a type to highlight it");
      expect(f.element.textContent).not.toContain(
        "Matching files and type groups highlighted",
      );
      expect(f.element.textContent).not.toContain("Dashed:");
      expect(
        f.element
          .querySelector(".map-wrap")
          ?.getAttribute("data-file-type-highlight-count"),
      ).toBe("1");
      expect(f.element.querySelector('[aria-label="Folder sizes"]')).toBeNull();

      // Even the same extension remains unhighlighted outside the returned scope.
      f.fills.length = 0;
      f.strokes.length = 0;
      await f.fileTypes({
        colorMode: "fileType",
        highlightedFileType: "txt",
        fileTypeHighlights: [2],
      });
      expect(f.fills[1]).toEqual({ color: fileTypeColor("txt"), alpha: 0.55 });
      expect(f.strokes.filter((s) => s.color === fileTypeColor("txt"))).toEqual(
        [],
      );

      f.strokes.length = 0;
      await f.fileTypes({
        colorMode: "fileType",
        highlightedFileType: "mov",
        fileTypeHighlights: [2, 3],
        fileTypeHighlightPending: true,
      });
      expect(f.strokes.some((s) => s.color === fileTypeColor("mov"))).toBe(
        false,
      );
      expect(f.element.textContent).toContain("Finding .mov matches");
      f.strokes.length = 0;
      await f.fileTypes({
        colorMode: "fileType",
        highlightedFileType: null,
        fileTypeHighlights: [2, 3],
      });
      expect(f.strokes.some((s) => s.color === fileTypeColor("txt"))).toBe(
        false,
      );
      expect(f.strokes.some((s) => s.color === "#246fde")).toBe(true);
      expect(f.postMessage).toHaveBeenCalledTimes(requests);

      await f.fileTypes({
        colorMode: "folder",
        highlightedFileType: "mov",
        fileTypeHighlights: [],
        fileTypeHighlightPending: true,
      });
      expect(f.element.querySelector(".type-legend")?.textContent).toContain(
        "Finding .mov matches",
      );
      await f.fileTypes({
        colorMode: "folder",
        highlightedFileType: "mov",
        fileTypeHighlights: [3],
        fileTypeHighlightPending: false,
      });
      expect(f.element.querySelector(".type-legend")).toBeNull();
    } finally {
      await f.cleanup();
    }
  },
);

it.each(["treemap", "sunburst"] as const)(
  "colors %s typed summaries, gates highlights by scoped parent IDs, and keeps their actions synthetic",
  async (mode) => {
    const f = await linkedMapFixture(true),
      onFileTypeSelect = vi.fn();
    try {
      await f.mode(mode);
      await f.fileTypes({
        colorMode: "fileType",
        onFileTypeSelect,
        highlightedFileType: "mov",
        fileTypeHighlights: [2],
      });
      await f.complete();
      const requests = f.postMessage.mock.calls.length;
      f.fills.length = 0;
      f.strokes.length = 0;
      await f.fileTypes({
        colorMode: "fileType",
        onFileTypeSelect,
        highlightedFileType: "mov",
        fileTypeHighlights: [2],
      });
      expect(f.fills.slice(0, 6)).toEqual([
        { color: "#e2e7ef", alpha: 1 },
        { color: fileTypeColor("txt"), alpha: 0.55 },
        { color: "#818a9b", alpha: 1 },
        { color: fileTypeColor("mov"), alpha: 1 },
        { color: fileTypeColor("mov"), alpha: 0.55 },
        { color: fileTypeColor(""), alpha: 0.55 },
      ]);
      expect(f.strokes.filter((s) => s.color === fileTypeColor("mov"))).toEqual(
        [{ color: fileTypeColor("mov"), dash: [] }],
      );
      expect(f.element.textContent).toContain("Click a type to highlight it");
      expect(
        f.element
          .querySelector(".map-wrap")
          ?.getAttribute("data-file-type-highlight-count"),
      ).toBe("1");
      // Scope gating also applies to an empty extension; it must not be mistaken for no choice.
      f.strokes.length = 0;
      await f.fileTypes({
        colorMode: "fileType",
        onFileTypeSelect,
        highlightedFileType: "",
        fileTypeHighlights: [2],
      });
      expect(
        f.strokes.filter(
          (s) => s.color === fileTypeColor("") && s.dash.length === 0,
        ),
      ).toHaveLength(1);
      f.strokes.length = 0;
      await f.fileTypes({
        colorMode: "fileType",
        onFileTypeSelect,
        highlightedFileType: "mov",
        fileTypeHighlights: [2],
        fileTypeHighlightPending: true,
      });
      expect(f.strokes.some((s) => s.color === fileTypeColor("mov"))).toBe(
        false,
      );
      await f.fileTypes({
        colorMode: "fileType",
        onFileTypeSelect,
        highlightedFileType: null,
        fileTypeHighlights: [2],
      });
      expect(
        f.element
          .querySelector(".map-wrap")
          ?.getAttribute("data-file-type-highlight-count"),
      ).toBe("0");
      expect(f.postMessage).toHaveBeenCalledTimes(requests);
      expect(f.strokes.some((s) => s.color === "#246fde")).toBe(true);
      // For input tests use real treemap geometry; both map paint modes were covered above.
      await f.mode("treemap");
      await f.complete();
      const group = f.shapes.find((s) => s.id === 5)!;
      await f.pointer("mousemove", group.x + 20, 30);
      expect(f.element.querySelector(".map-caption")?.textContent).toContain(
        ".mov files · 1 KiB · 12 files grouped by type",
      );
      expect(f.onHover).toHaveBeenLastCalledWith(null);
      f.onSelect.mockClear();
      f.onOpen.mockClear();
      await f.pointer("click", group.x + 20, 30);
      await f.pointer("dblclick", group.x + 20, 30);
      expect(onFileTypeSelect).toHaveBeenCalledExactlyOnceWith("mov");
      expect(f.onSelect).not.toHaveBeenCalled();
      expect(f.onOpen).toHaveBeenCalledExactlyOnceWith(
        f.nodes.find((node) => node.id === 2),
      );
      const residual = f.shapes.find((s) => s.id === 4)!;
      await f.pointer("click", residual.x + 20, 30);
      expect(onFileTypeSelect).toHaveBeenCalledTimes(1);
      expect(f.onSelect).not.toHaveBeenCalled();
      const empty = f.shapes.find((s) => s.id === 7)!;
      await f.pointer("click", empty.x + 20, 30);
      expect(onFileTypeSelect).toHaveBeenLastCalledWith("");
    } finally {
      await f.cleanup();
    }
  },
);

it.each(["treemap", "sunburst"] as const)(
  "opens a %s type group's real folder and rejects changed group or folder identities",
  async (mode) => {
    const f = await linkedMapFixture(true),
      onFileTypeSelect = vi.fn();
    try {
      await f.mode(mode);
      await f.fileTypes({ colorMode: "fileType", onFileTypeSelect });
      const shape = f.shapes.find((shape) => shape.id === 5)!;
      if (mode === "sunburst") {
        shape.start = -Math.PI / 4;
        shape.end = 0;
      }
      await f.complete();
      const point =
        mode === "treemap"
          ? [shape.x + 20, 30]
          : [
              300 + Math.cos(-Math.PI / 8) * 60,
              200 + Math.sin(-Math.PI / 8) * 60,
            ];
      const group = f.nodes.find((node) => node.id === 5)!;
      const parent = f.nodes.find((node) => node.id === 2)!;
      await f.pointer("mousemove", ...(point as [number, number]));
      expect(f.canvas.title).toBe(parent.path);
      await f.pointer("click", ...(point as [number, number]));
      await f.pointer("dblclick", ...(point as [number, number]));
      expect(onFileTypeSelect).toHaveBeenCalledExactlyOnceWith("mov");
      expect(f.onOpen).toHaveBeenCalledExactlyOnceWith(parent);
      expect(f.onSelect).not.toHaveBeenCalled();
      f.onOpen.mockClear();
      const path = parent.path;
      await f.pointer("click", ...(point as [number, number]));
      parent.path = "/replaced-folder";
      await f.pointer("dblclick", ...(point as [number, number]));
      expect(f.onOpen).not.toHaveBeenCalled();
      parent.path = path;
      await f.pointer("click", ...(point as [number, number]));
      group.fileType = "zip";
      await f.pointer("dblclick", ...(point as [number, number]));
      expect(f.onOpen).not.toHaveBeenCalled();
      group.fileType = "mov";
      await f.pointer("click", ...(point as [number, number]));
      group.parentId = 999;
      await f.pointer("dblclick", ...(point as [number, number]));
      expect(f.onOpen).not.toHaveBeenCalled();
      // A group without a known real directory also cannot acquire file-action authority.
      await f.pointer("click", ...(point as [number, number]));
      await f.pointer("dblclick", ...(point as [number, number]));
      expect(f.onOpen).not.toHaveBeenCalled();
      expect(f.onSelect).not.toHaveBeenCalled();
    } finally {
      await f.cleanup();
    }
  },
);

it("finishes a type group's original folder gesture when its second click lands outside the resized canvas", async () => {
  const f = await linkedMapFixture(true),
    onFileTypeSelect = vi.fn();
  const outside = document.createElement("button"),
    activate = vi.fn();
  outside.onclick = activate;
  document.body.append(outside);
  try {
    await f.fileTypes({ colorMode: "fileType", onFileTypeSelect });
    await f.complete();
    const shape = f.shapes.find((shape) => shape.id === 5)!;
    const x = shape.x + 20,
      y = 30;
    await f.pointer("click", x, y);
    for (const type of ["click", "dblclick"]) {
      const event = new MouseEvent(type, {
        detail: 2,
        clientX: x,
        clientY: y,
        bubbles: true,
        cancelable: true,
      });
      await act(() => outside.dispatchEvent(event));
      expect(event.defaultPrevented).toBe(true);
    }
    expect(f.onOpen).toHaveBeenCalledExactlyOnceWith(
      f.nodes.find((node) => node.id === 2),
    );
    expect(onFileTypeSelect).toHaveBeenCalledExactlyOnceWith("mov");
    expect(f.onSelect).not.toHaveBeenCalled();
    expect(activate).not.toHaveBeenCalled();
  } finally {
    outside.remove();
    await f.cleanup();
  }
});

it.each(["folder", "fileType"] as const)(
  "opens successive %s map gestures when native click counts keep increasing",
  async (colorMode) => {
    const f = await linkedMapFixture(true),
      onFileTypeSelect = vi.fn();
    const outside = document.createElement("button"),
      activate = vi.fn(),
      outsideDoubleClick = vi.fn();
    outside.onclick = activate;
    outside.ondblclick = outsideDoubleClick;
    document.body.append(outside);
    try {
      await f.fileTypes({ colorMode, onFileTypeSelect });
      await f.complete();
      const shape = f.shapes.find(
        (shape) => shape.id === (colorMode === "folder" ? 2 : 5),
      )!;
      const x = shape.x + 20,
        y = 30;
      const dispatch = async (
        detail: number,
        type = "click",
        target: HTMLElement = f.canvas,
      ) => {
        const event = new MouseEvent(type, {
          detail,
          clientX: x,
          clientY: y,
          bubbles: true,
          cancelable: true,
        });
        await act(() => target.dispatchEvent(event));
        return event;
      };
      // Browsers can continue the native count after a completed double-click.
      // Chromium emits no further dblclick for counts 4, 6, etc.
      for (const first of [1, 3, 5]) {
        const opens = f.onOpen.mock.calls.length;
        await dispatch(first);
        await dispatch(first + 1);
        if (first === 1) await dispatch(2, "dblclick");
        expect(f.onOpen).toHaveBeenCalledTimes(opens + 1);
        expect(f.onOpen).toHaveBeenLastCalledWith(
          f.nodes.find((node) => node.id === 2),
        );
        // Platforms delivering an additional dblclick must not open twice.
        await dispatch(first + 1, "dblclick");
        expect(f.onOpen).toHaveBeenCalledTimes(opens + 1);
      }
      if (colorMode === "fileType") {
        expect(onFileTypeSelect).toHaveBeenCalledTimes(3);
        expect(f.onSelect).not.toHaveBeenCalled();
      }
      // Preserve the first target if resizing moves the fourth click off canvas.
      await dispatch(7);
      expect((await dispatch(8, "click", outside)).defaultPrevented).toBe(true);
      expect(f.onOpen).toHaveBeenCalledTimes(4);
      expect(activate).not.toHaveBeenCalled();
      expect((await dispatch(8, "dblclick", outside)).defaultPrevented).toBe(
        true,
      );
      expect(f.onOpen).toHaveBeenCalledTimes(4);
      expect(outsideDoubleClick).not.toHaveBeenCalled();
      // A new odd click outside the canvas starts an unrelated gesture.
      await dispatch(9);
      expect((await dispatch(11, "click", outside)).defaultPrevented).toBe(
        false,
      );
      await dispatch(12, "click", outside);
      expect(f.onOpen).toHaveBeenCalledTimes(4);
      // Context changes still invalidate a captured higher-count gesture.
      await dispatch(13);
      await f.mode("sunburst");
      await dispatch(14);
      await dispatch(14, "dblclick");
      expect(f.onOpen).toHaveBeenCalledTimes(4);
    } finally {
      outside.remove();
      await f.cleanup();
    }
  },
);

it("exposes the map as an image with a live summary, a permanent queue key and an accent selection outline", async () => {
  const gib = 1024 ** 3;
  let nodes = [
    { id: 1, parentId: null, name: "Disk", size: 10, kind: "directory" },
    { id: 2, parentId: 1, name: "Movies", size: 5, kind: "directory" },
    { id: 3, parentId: 1, name: "Apps", size: 3, kind: "directory" },
    { id: 4, parentId: 1, name: "Docs", size: 1, kind: "directory" },
    { id: 5, parentId: 1, name: "Tiny", size: 0.5, kind: "file" },
  ].map(({ size, ...n }) => ({
    ...n,
    path: `/${n.name}`,
    allocatedBytes: size * gib,
    childrenCount: 0,
    incomplete: false,
  })) as NodeView[];
  const shapes = nodes.slice(1).map((n, i) => ({
    id: n.id,
    x: i * 100,
    y: 0,
    width: 100,
    height: 100,
    start: 0,
    end: 1,
    inner: 40,
    outer: 100,
    depth: 0,
  }));
  vi.stubGlobal(
    "Worker",
    class {
      onmessage: ((event: unknown) => void) | null = null;
      postMessage(data: { requestId: number }) {
        this.onmessage?.({ data: { requestId: data.requestId, shapes } });
      }
      terminate() {}
    },
  );
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
  const strokes: string[] = [];
  const ctx = {
    ...Object.fromEntries(
      [
        "setTransform",
        "clearRect",
        "beginPath",
        "rect",
        "arc",
        "closePath",
        "fill",
        "save",
        "clip",
        "restore",
        "fillText",
        "setLineDash",
      ].map((key) => [key, vi.fn()]),
    ),
    strokeStyle: "",
    measureText: (text: string) => ({ width: text.length * 9 }),
    stroke: vi.fn(() => strokes.push(ctx.strokeStyle)),
  };
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
    withGradients(ctx) as unknown as CanvasRenderingContext2D,
  );
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element);
  let selected: number[] = [],
    queuedPaths: { path: string }[] = [],
    rootId = 1,
    scanComplete = false;
  const render = () =>
    root.render(DiskMap, {
      nodes,
      rootId,
      mode: "treemap",
      selected,
      scanComplete,
      queuedPaths,
      onSelect: () => {},
      onOpen: () => {},
    });
  const summary = () =>
    element.querySelector<HTMLElement>(".map-wrap .map-summary")!;
  const announcement = () =>
    element.querySelector<HTMLElement>(".map-wrap .map-announcement")!;
  try {
    await act(async () => render());
    const canvas = element.querySelector("canvas")!;
    expect(canvas.getAttribute("role")).toBe("img");
    // The full description is read on demand; only the short line is live.
    expect(summary().hasAttribute("aria-live")).toBe(false);
    expect(announcement().getAttribute("aria-live")).toBe("polite");
    expect(announcement().textContent?.trim()).toBe("");
    expect(canvas.getAttribute("aria-describedby")).toBe(summary().id);
    expect(summary().textContent?.trim()).toBe(
      "Treemap of Disk, 10 GiB on disk. Largest items: Movies 5 GiB, Apps 3 GiB, Docs 1 GiB.",
    );
    selected = [3];
    await act(async () => render());
    expect(summary().textContent).toContain("Selected: Apps, 3 GiB.");
    selected = [3, 4];
    await act(async () => render());
    expect(summary().textContent).toContain("2 items selected.");
    // Selection is not announced; a rebuilt array with the same ids changes nothing.
    expect(announcement().textContent?.trim()).toBe("");
    const described = summary().textContent;
    selected = [3, 4];
    nodes = nodes.map((n) => ({ ...n, allocatedBytes: 9 * gib }));
    await act(async () => render());
    expect(summary().textContent).toBe(described);
    // Completion refreshes the description once.
    scanComplete = true;
    await act(async () => render());
    expect(summary().textContent).toContain("9 GiB on disk");
    queuedPaths = [{ path: "/Apps" }];
    await act(async () => render());
    expect(element.querySelector(".map-queue-key")).toBeNull();
    // jsdom has no accent-color; the outline then falls back to the blue.
    expect(strokes.at(-1)).toBe("#246fde");
    vi.spyOn(window, "getComputedStyle").mockReturnValue({
      accentColor: "rgb(10, 20, 30)",
    } as CSSStyleDeclaration);
    strokes.length = 0;
    selected = [2];
    await act(async () => render());
    expect(strokes.at(-1)).toBe("rgb(10, 20, 30)");
    rootId = 2;
    await act(async () => render());
    expect(announcement().textContent?.trim()).toBe("Opened Movies");
  } finally {
    await act(async () => root.unmount());
    element.remove();
  }
});

async function mountWithManualWorker() {
  let respond: ((shapes: Shape[]) => void) | undefined;
  let requested = 0;
  vi.stubGlobal(
    "Worker",
    class {
      onmessage:
        | ((event: { data: { requestId: number; shapes: Shape[] } }) => void)
        | null = null;
      postMessage(data: { requestId: number }) {
        requested = data.requestId;
        respond = (shapes) =>
          this.onmessage?.({ data: { requestId: requested, shapes } });
      }
      terminate() {}
    },
  );
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
  const context = Object.fromEntries(
    ["setLineDash", "setTransform", "clearRect", "beginPath", "rect", "arc"]
      .concat(["closePath", "fill", "stroke", "save", "clip", "restore"])
      .map((key) => [key, vi.fn()]),
  );
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
    withGradients(context) as unknown as CanvasRenderingContext2D,
  );
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element);
  return { context, element, root, respond: (s: Shape[]) => respond?.(s) };
}
const emptyRoot: NodeView = {
  id: 0,
  parentId: null,
  name: "root",
  path: "/root",
  kind: "directory",
  allocatedBytes: 0,
  childrenCount: 0,
  incomplete: false,
};

it("keeps the previous frame after a mode switch until the worker replies", async () => {
  const { context, element, root, respond } = await mountWithManualWorker();
  const shape: Shape = {
    id: 1,
    x: 0,
    y: 0,
    width: 10,
    height: 10,
    start: 0,
    end: 0,
    inner: 0,
    outer: 0,
    depth: 0,
  };
  const render = (mode: "treemap" | "sunburst") =>
    root.render(DiskMap, {
      nodes: [emptyRoot],
      rootId: 0,
      mode,
      selected: [],
      onSelect: () => {},
      onOpen: () => {},
    });
  try {
    await act(async () => render("treemap"));
    await act(async () => respond([shape]));
    expect(context.fill).toHaveBeenCalled();
    context.clearRect.mockClear();
    context.arc.mockClear();
    await act(async () => render("sunburst"));
    expect(context.clearRect).not.toHaveBeenCalled();
    expect(context.arc).not.toHaveBeenCalled();
    await act(async () => respond([shape]));
    expect(context.clearRect).toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    element.remove();
  }
});

it("says a completed scan has nothing to draw, but not while the scan runs", async () => {
  const { element, root, respond } = await mountWithManualWorker();
  const nodes = [emptyRoot];
  const empty = () => element.querySelector(".map-empty")?.textContent?.trim();
  const render = (scanComplete: boolean) =>
    root.render(DiskMap, {
      nodes,
      rootId: 0,
      mode: "treemap",
      selected: [],
      scanComplete,
      onSelect: () => {},
      onOpen: () => {},
    });
  try {
    await act(async () => render(false));
    await act(async () => respond([]));
    expect(empty()).toBe("The map fills in as files are found");
    await act(async () => render(true));
    expect(empty()).toBe("Nothing here takes space on disk.");
  } finally {
    await act(async () => root.unmount());
    element.remove();
  }
});

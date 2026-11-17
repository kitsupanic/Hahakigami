import { describe, it, expect } from "vitest";
import { layout } from "./layout";
import { hitShape } from "./hitTest";
import type { NodeView } from "./types";
const node = (
  id: number,
  parentId: number | null,
  value: number,
): NodeView => ({
  id,
  parentId,
  name: String(id),
  path: "/",
  kind: "file",
  logicalBytes: value,
  allocatedBytes: value,
  childrenCount: 0,
  incomplete: false,
});
describe("disk layouts", () => {
  it("starts the sunburst with the next largest branch and ends with the largest, without changing treemap order", () => {
    const nodes = [
      node(1, 0, 80),
      node(2, 0, 12),
      node(3, 0, 8),
      node(4, 1, 50),
      node(5, 1, 30),
    ];
    const radial = layout(nodes, 0, "allocated", 800, 800, "sunburst");
    const roots = radial.filter((shape) => shape.depth === 0);
    expect(roots.map((shape) => shape.id)).toEqual([2, 3, 1]);
    expect(roots[0].start).toBe(-Math.PI / 2);
    expect(roots[2].end).toBeCloseTo(Math.PI * 1.5);
    expect(
      radial.filter((shape) => shape.depth === 1).map((shape) => shape.id),
    ).toEqual([4, 5]);
    expect(
      layout(nodes, 0, "allocated", 800, 800, "treemap")
        .filter((shape) => shape.depth === 0)
        .map((shape) => shape.id),
    ).toEqual([1, 2, 3]);
  });
  it("partitions area without negative or nonfinite shapes", () => {
    const shapes = layout(
      [node(0, null, 100), node(1, 0, 60), node(2, 0, 30), node(3, 0, 10)],
      0,
      "allocated",
      600,
      400,
      "treemap",
    );
    expect(shapes.reduce((s, n) => s + n.width * n.height, 0)).toBeCloseTo(
      240000,
    );
    for (const s of shapes) {
      expect(s.width).toBeGreaterThan(0);
      expect(s.x + s.width).toBeLessThanOrEqual(600);
    }
  });
  it("ignores zero sizes and nodes outside the focused subtree", () => {
    expect(
      layout(
        [node(1, 0, 0), node(2, 99, 10)],
        0,
        "logical",
        600,
        400,
        "treemap",
      ),
    ).toEqual([]);
  });
  it("builds nested nonoverlapping radial rings", () => {
    const shapes = layout(
      [node(1, 0, 100), node(2, 1, 50), node(3, 1, 50)],
      0,
      "allocated",
      400,
      400,
      "sunburst",
    );
    expect(shapes).toHaveLength(3);
    expect(shapes[1].inner).toBe(shapes[0].outer);
    expect(shapes[1].end).toBeCloseTo(shapes[2].start);
    expect(shapes[0].end - shapes[0].start).toBeCloseTo(Math.PI * 2);
    expect(shapes[0].inner).toBeCloseTo(184 * 0.27);
    expect(shapes[0].outer).toBeCloseTo(184 * 0.76);
    expect(shapes[1].outer).toBeCloseTo(184);
  });
  it("distributes outer rings across actual depth and keeps the center outside hit regions", () => {
    const nodes = [node(1, 0, 100), node(2, 1, 100), node(3, 2, 100)];
    const shapes = layout(nodes, 0, "logical", 600, 400, "sunburst");
    expect(shapes[1].inner).toBe(shapes[0].outer);
    expect(shapes[2].inner).toBe(shapes[1].outer);
    expect(shapes[1].outer - shapes[1].inner).toBeCloseTo(
      shapes[2].outer - shapes[2].inner,
    );
    expect(shapes[2].outer).toBeCloseTo(184);
    expect(hitShape(shapes, "sunburst", 600, 400, 300, 200)).toBeUndefined();
    expect(hitShape(shapes, "sunburst", 600, 400, 400, 200)?.id).toBe(1);
    const shallow = layout(
      nodes.slice(0, 1),
      0,
      "logical",
      600,
      400,
      "sunburst",
    );
    expect(shallow[0].outer).toBeCloseTo(184);
  });
  it("remains bounded with five thousand map entries", () => {
    const shapes = layout(
      Array.from({ length: 5000 }, (_, i) => node(i + 1, 0, i + 1)),
      0,
      "allocated",
      1000,
      800,
      "treemap",
    );
    expect(shapes).toHaveLength(5000);
    expect(shapes.every((s) => Number.isFinite(s.width) && s.width > 0)).toBe(
      true,
    );
  });
});

it("keeps dominant directory areas proportional and their headers selectable above descendants", () => {
  const nodes = [
    node(0, null, 100),
    { ...node(1, 0, 87), kind: "directory" as const },
    node(2, 0, 13),
    ...Array.from({ length: 100 }, (_, i) => node(i + 3, 1, 0.87)),
  ];
  const shapes = layout(nodes, 0, "allocated", 1000, 400, "treemap");
  const folder = shapes.find((s) => s.id === 1)!;
  expect(folder.width * folder.height).toBeCloseTo(1000 * 400 * 0.87);
  expect(folder.headerHeight).toBe(64);
  const descendants = shapes.filter((s) => s.depth === 1);
  expect(descendants).toHaveLength(100);
  expect(
    descendants.every((s) => s.y >= folder.y + 64 && s.groupId === 1),
  ).toBe(true);
  expect(
    descendants.every(
      (s) =>
        s.x >= folder.x + 8 &&
        s.x + s.width <= folder.x + folder.width - 8 + 0.001 &&
        s.y + s.height <= folder.y + folder.height - 8 + 0.001,
    ),
  ).toBe(true);
  expect(
    hitShape(
      shapes,
      "treemap",
      1000,
      400,
      folder.x + 4,
      folder.y + folder.headerHeight! + 20,
    )?.id,
  ).toBe(folder.id);
  expect(
    hitShape(shapes, "treemap", 1000, 400, folder.x + 10, folder.y + 20)?.id,
  ).toBe(1);
  const child = descendants[0];
  expect(
    hitShape(
      shapes,
      "treemap",
      1000,
      400,
      child.x + child.width / 2,
      child.y + child.height / 2,
    )?.id,
  ).toBe(child.id);
  const focused = layout(nodes, 1, "logical", 1000, 400, "treemap");
  expect(focused.some((s) => s.id === 1)).toBe(false);
  expect(focused.every((s) => s.groupId === s.id)).toBe(true);
});

it("reserves two-line headers for roomy nested folders while keeping narrow headers compact", () => {
  const nodes = [
    node(0, null, 100),
    { ...node(1, 0, 100), kind: "directory" as const },
    { ...node(2, 1, 90), kind: "directory" as const },
    node(3, 1, 10),
    node(4, 2, 50),
    node(5, 2, 40),
  ];
  for (const [width, height, header] of [
    [800, 600, 56],
    [150, 600, 56],
    [150, 180, 56],
    [128, 600, 28],
  ]) {
    const shapes = layout(nodes, 0, "allocated", width, height, "treemap");
    const nested = shapes.find((shape) => shape.id === 2)!;
    expect(nested.headerHeight).toBe(header);
    for (const id of [4, 5]) {
      const child = shapes.find((shape) => shape.id === id)!;
      expect(child.y).toBeGreaterThanOrEqual(nested.y + header);
      expect(child.y + child.height).toBeLessThanOrEqual(
        nested.y + nested.height - 4 + 0.001,
      );
      expect(child.x).toBeGreaterThanOrEqual(nested.x + 4);
      expect(child.x + child.width).toBeLessThanOrEqual(
        nested.x + nested.width - 4 + 0.001,
      );
      expect(child.width).toBeGreaterThan(0);
    }
    expect(
      hitShape(shapes, "treemap", width, height, nested.x + 10, nested.y + 15)
        ?.id,
    ).toBe(2);
  }
});

it("keeps a usable child body in short directory tiles", () => {
  const shapes = layout(
    [
      node(0, null, 100),
      { ...node(1, 0, 100), kind: "directory" },
      node(2, 1, 100),
    ],
    0,
    "allocated",
    180,
    90,
    "treemap",
  );
  const parent = shapes.find((shape) => shape.id === 1)!;
  const child = shapes.find((shape) => shape.id === 2)!;
  expect(parent.headerHeight).toBe(28);
  expect(child.height).toBeGreaterThanOrEqual(40);
  expect(child.y).toBe(parent.y + 28);
  expect(child.y + child.height).toBe(parent.y + parent.height - 8);
});

it("leaves folder layouts unchanged by default and preserves colored leaf area through deep compact hierarchy", () => {
  const nodes: NodeView[] = [node(0, null, 100)];
  for (let id = 1; id <= 5; id++)
    nodes.push({ ...node(id, id - 1, 100), kind: "directory" });
  nodes.push(
    { ...node(-1, 5, 70), kind: "aggregate", fileType: "mov", fileCount: 100 },
    { ...node(-2, 5, 30), kind: "aggregate", fileType: "png", fileCount: 20 },
  );
  const normal = layout(nodes, 0, "allocated", 800, 500, "treemap");
  expect(normal).toEqual(
    layout(nodes, 0, "allocated", 800, 500, "treemap", "folder"),
  );
  const compact = layout(
    nodes,
    0,
    "allocated",
    800,
    500,
    "treemap",
    "fileType",
  );
  const coloredArea = (shapes: ReturnType<typeof layout>) =>
    shapes.filter((s) => s.id < 0).reduce((a, s) => a + s.width * s.height, 0);
  expect(coloredArea(compact)).toBeGreaterThan(800 * 500 * 0.7);
  expect(coloredArea(compact)).toBeGreaterThan(coloredArea(normal) * 1.5);
  for (const shape of compact.filter((s) => s.id > 0)) {
    expect(shape.headerHeight).toBe(shape.depth === 0 ? 24 : 20);
    expect(
      hitShape(compact, "treemap", 800, 500, shape.x + 4, shape.y + 10)?.id,
    ).toBe(shape.id);
  }
  expect(
    compact.every(
      (s) => s.width > 0 && s.height > 0 && Number.isFinite(s.x + s.y),
    ),
  ).toBe(true);
  expect(layout(nodes, 0, "logical", 800, 500, "sunburst", "fileType")).toEqual(
    layout(nodes, 0, "logical", 800, 500, "sunburst"),
  );
});

it("reserves no compact directory band when the visible body would be too small", () => {
  const shapes = layout(
    [{ ...node(1, 0, 100), kind: "directory" }, node(2, 1, 100)],
    0,
    "allocated",
    100,
    70,
    "treemap",
    "fileType",
  );
  expect(shapes[0].headerHeight).toBeUndefined();
  expect(shapes[1].width * shapes[1].height).toBe(7000);
});

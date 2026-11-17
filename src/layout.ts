import { size, type Metric, type NodeView } from "./types";
export interface Shape {
  id: number;
  x: number;
  y: number;
  width: number;
  height: number;
  start: number;
  end: number;
  inner: number;
  outer: number;
  depth: number;
  /** Reserved directory label band; descendants never overlap it. */
  headerHeight?: number;
  groupId?: number;
}
// Squarified rows preserve proportional area while avoiding long thin rectangles.
function rectangles(
  values: number[],
  x: number,
  y: number,
  w: number,
  h: number,
) {
  const total = values.reduce((a, b) => a + b, 0),
    areas = values.map((v) => (v / total) * w * h),
    result: { x: number; y: number; width: number; height: number }[] = [];
  let index = 0;
  function worst(row: number[], side: number) {
    const sum = row.reduce((a, b) => a + b, 0);
    return Math.max(
      (side * side * Math.max(...row)) / (sum * sum),
      (sum * sum) / (side * side * Math.min(...row)),
    );
  }
  while (index < areas.length) {
    const row = [areas[index++]];
    const side = Math.min(w, h);
    while (
      index < areas.length &&
      worst([...row, areas[index]], side) <= worst(row, side)
    )
      row.push(areas[index++]);
    const sum = row.reduce((a, b) => a + b, 0),
      vertical = w >= h,
      thickness = sum / (vertical ? h : w);
    let cursor = 0;
    for (const a of row) {
      const length = a / thickness;
      result.push({
        x: vertical ? x : x + cursor,
        y: vertical ? y + cursor : y,
        width: vertical ? thickness : length,
        height: vertical ? length : thickness,
      });
      cursor += length;
    }
    if (vertical) {
      x += thickness;
      w = Math.max(0, w - thickness);
    } else {
      y += thickness;
      h = Math.max(0, h - thickness);
    }
  }
  return result;
}
export function layout(
  nodes: NodeView[],
  rootId: number,
  metric: Metric,
  width: number,
  height: number,
  mode: "treemap" | "sunburst",
  colorMode: "folder" | "fileType" = "folder",
): Shape[] {
  if (width <= 0 || height <= 0) return [];
  const children = new Map<number, NodeView[]>();
  for (const n of nodes)
    if (n.id !== rootId && n.parentId !== null && size(n, metric) > 0) {
      const a = children.get(n.parentId) || [];
      a.push(n);
      children.set(n.parentId, a);
    }
  for (const a of children.values())
    a.sort((a, b) => size(b, metric) - size(a, metric));
  const rootChildren = children.get(rootId);
  if (mode === "sunburst" && rootChildren && rootChildren.length > 1)
    rootChildren.push(rootChildren.shift()!);
  const out: Shape[] = [];
  let maxDepth = 1;
  const visited = new Set<number>();
  function visit(
    id: number,
    x: number,
    y: number,
    w: number,
    h: number,
    start: number,
    end: number,
    depth: number,
    groupId?: number,
  ) {
    if (visited.has(id)) return;
    visited.add(id);
    const list = children.get(id) || [];
    if (!list.length) return;
    const values = list.map((n) => size(n, metric)),
      sum = values.reduce((s, n) => s + n, 0),
      rects = mode === "treemap" ? rectangles(values, x, y, w, h) : [];
    let cursor = 0;
    maxDepth = Math.max(maxDepth, depth + 1);
    for (let i = 0; i < list.length; i++) {
      const n = list[i],
        frac = values[i] / sum;
      const shape: Shape = {
        id: n.id,
        ...(rects[i] || { x: 0, y: 0, width: 0, height: 0 }),
        start: start + (end - start) * cursor,
        end: start + (end - start) * (cursor + frac),
        inner: depth,
        outer: depth + 1,
        depth,
        groupId: groupId ?? n.id,
      };
      if (
        mode === "treemap" &&
        children.has(n.id) &&
        shape.width >= (colorMode === "fileType" ? 80 : 110) &&
        shape.height >= (colorMode === "fileType" ? 88 : 90)
      ) {
        if (colorMode === "fileType") {
          // Keep hierarchy legible without spending the body on folder chrome.
          // Small cells reserve nothing, leaving the whole area to their leaves.
          shape.headerHeight = depth === 0 ? 24 : 20;
        } else {
          const roomyHeader = depth === 0 ? 64 : 56;
          const inset = depth === 0 ? 8 : 4;
          shape.headerHeight =
            (depth === 0 || shape.width >= 120) &&
            shape.height >= roomyHeader + inset + 40
              ? roomyHeader
              : 28;
        }
      }
      out.push(shape);
      const childInset = shape.headerHeight
        ? colorMode === "fileType"
          ? 2
          : depth === 0
            ? 8
            : 4
        : 0;
      if (depth < 12)
        visit(
          n.id,
          shape.x + childInset,
          shape.y + (shape.headerHeight ?? 0),
          Math.max(0, shape.width - childInset * 2),
          Math.max(0, shape.height - (shape.headerHeight ?? 0) - childInset),
          shape.start,
          shape.end,
          depth + 1,
          shape.groupId,
        );
      cursor += frac;
    }
  }
  visit(rootId, 0, 0, width, height, -Math.PI / 2, Math.PI * 1.5, 0);
  if (mode === "sunburst") {
    const radius = Math.min(width, height) * 0.46;
    const hole = radius * 0.27;
    const firstOuter = maxDepth > 1 ? radius * 0.76 : radius;
    const ring = (radius - firstOuter) / Math.max(1, maxDepth - 1);
    for (const s of out) {
      s.inner = s.depth === 0 ? hole : firstOuter + (s.depth - 1) * ring;
      s.outer = firstOuter + s.depth * ring;
    }
  }
  return out;
}

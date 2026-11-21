import { type NodeView } from "./types";
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
  // The row is described by its running sum, max and min, so testing one more area copies nothing.
  function worst(sum: number, max: number, min: number, side: number) {
    return Math.max(
      (side * side * max) / (sum * sum),
      (sum * sum) / (side * side * min),
    );
  }
  while (index < areas.length) {
    const row = [areas[index++]];
    const side = Math.min(w, h);
    let sum = row[0],
      max = row[0],
      min = row[0];
    while (index < areas.length) {
      const next = areas[index];
      if (
        !(
          worst(sum + next, Math.max(max, next), Math.min(min, next), side) <=
          worst(sum, max, min, side)
        )
      )
        break;
      row.push(next);
      sum += next;
      max = Math.max(max, next);
      min = Math.min(min, next);
      index++;
    }
    const vertical = w >= h,
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
  width: number,
  height: number,
  mode: "treemap" | "sunburst",
  colorMode: "folder" | "fileType" = "folder",
): Shape[] {
  if (width <= 0 || height <= 0) return [];
  const children = new Map<number, NodeView[]>();
  for (const n of nodes)
    if (n.id !== rootId && n.parentId !== null && n.allocatedBytes > 0) {
      const a = children.get(n.parentId) || [];
      a.push(n);
      children.set(n.parentId, a);
    }
  for (const a of children.values())
    a.sort((a, b) => b.allocatedBytes - a.allocatedBytes);
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
    const values = list.map((n) => n.allocatedBytes),
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
export const MAP_TWEEN_MS = 420;
const mix = (a: number, b: number, k: number) => a + (b - a) * k;
/** A frame between two layouts of one tile; a new tile grows from its own centre (treemap) or start angle (sunburst). */
export function tweenShape(
  from: Shape | undefined,
  to: Shape,
  k: number,
): Shape {
  const a = from ?? {
    ...to,
    x: to.x + to.width / 2,
    y: to.y + to.height / 2,
    width: 0,
    height: 0,
    end: to.start,
  };
  return {
    ...to,
    x: mix(a.x, to.x, k),
    y: mix(a.y, to.y, k),
    width: mix(a.width, to.width, k),
    height: mix(a.height, to.height, k),
    start: mix(a.start, to.start, k),
    end: mix(a.end, to.end, k),
    inner: mix(a.inner, to.inner, k),
    outer: mix(a.outer, to.outer, k),
  };
}

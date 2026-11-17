import { expect, it } from "vitest";
import { hitShape } from "./hitTest";
import type { Shape } from "./layout";

const shape = (id: number, overrides: Partial<Shape> = {}): Shape => ({
  id,
  x: 0,
  y: 0,
  width: 100,
  height: 100,
  start: -Math.PI / 2,
  end: Math.PI * 1.5,
  inner: 10,
  outer: 20,
  depth: 0,
  ...overrides,
});
it("hits the last painted descendant without changing paint order", () => {
  const parent = shape(1),
    child = shape(2, { x: 20, y: 20, width: 20, height: 20 });
  const shapes = Object.freeze([parent, child]);
  expect(hitShape(shapes, "treemap", 100, 100, 30, 30)).toBe(child);
  expect(hitShape(shapes, "treemap", 100, 100, 5, 5)).toBe(parent);
  expect(hitShape(shapes, "treemap", 100, 100, 101, 30)).toBeUndefined();
  expect(shapes).toEqual([parent, child]);
});
it("matches sunburst rings and wrapped angles while preserving the center hole", () => {
  const right = shape(1, { end: Math.PI / 2 });
  const left = shape(2, { start: Math.PI / 2 });
  expect(hitShape([right, left], "sunburst", 100, 100, 65, 50)).toBe(right);
  expect(hitShape([right, left], "sunburst", 100, 100, 35, 50)).toBe(left);
  expect(hitShape([right, left], "sunburst", 100, 100, 40, 40)).toBe(left);
  expect(hitShape([right, left], "sunburst", 100, 100, 50, 50)).toBeUndefined();
  expect(hitShape([right, left], "sunburst", 100, 100, 95, 50)).toBeUndefined();
});
it("handles empty maps and shared edges in paint order", () => {
  expect(hitShape([], "treemap", 100, 100, 5, 5)).toBeUndefined();
  const a = shape(1, { width: 50 }),
    b = shape(2, { x: 50, width: 50 });
  expect(hitShape([a, b], "treemap", 100, 100, 50, 5)).toBe(b);
});

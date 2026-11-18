import { expect, it } from "vitest";
import { createHoverTarget, findHoverMatch } from "./hoverContext";
import type { NodeView } from "./types";

const node = (id: number, parentId: number | null): NodeView => ({
  id,
  parentId,
  name: `Item ${id}`,
  path: "/same/display/path",
  kind: "directory",
  allocatedBytes: 100,
  childrenCount: 1,
  incomplete: false,
});
const nodes = [node(0, null), node(1, 0), node(2, 1), node(3, 2), node(4, 0)];
const byId = new Map(nodes.map((item) => [item.id, item]));

it("matches the closest represented ancestor using IDs, not repeated names or paths", () => {
  const target = createHoverTarget(nodes[3], byId);
  expect(target.lineage).toEqual([3, 2, 1, 0]);
  expect(findHoverMatch(target, new Set([1, 2, 3]))).toBe(3);
  expect(findHoverMatch(target, new Set([1, 2]))).toBe(2);
  expect(findHoverMatch(target, new Set([1, 4]))).toBe(1);
  expect(findHoverMatch(target, new Set([4]))).toBeNull();
});

it("continues a lazy row chain through the map index and leaves unrelated items unmatched", () => {
  const target = createHoverTarget(node(10, 2), new Map());
  expect(target.lineage).toEqual([10, 2]);
  expect(findHoverMatch(target, new Set([1]), byId)).toBe(1);
  expect(findHoverMatch(target, new Set([4]), byId)).toBeNull();
  expect(
    findHoverMatch(createHoverTarget(node(11, 99), byId), new Set([1]), byId),
  ).toBeNull();
});

it("keeps aggregate feedback local and bounds malformed parent cycles", () => {
  const aggregate = { ...node(-1, 1), kind: "aggregate" as const };
  expect(createHoverTarget(aggregate, byId).lineage).toEqual([]);
  expect(
    findHoverMatch({ node: aggregate, lineage: [-1, 1] }, new Set([1]), byId),
  ).toBeNull();
  const cycle = new Map([
    [5, node(5, 6)],
    [6, node(6, 5)],
  ]);
  const target = createHoverTarget(cycle.get(5)!, cycle);
  expect(target.lineage).toEqual([5, 6]);
  expect(findHoverMatch(target, new Set([1]), cycle)).toBeNull();
  expect(findHoverMatch(null, new Set([1]), byId)).toBeNull();
});

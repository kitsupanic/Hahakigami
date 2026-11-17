import type { NodeView } from "./types";

/** Display context only: self first, followed by known parent identities. */
export interface HoverTarget {
  node: NodeView;
  lineage: readonly number[];
}

export function createHoverTarget(
  node: NodeView,
  byId: ReadonlyMap<number, NodeView>,
): HoverTarget {
  const lineage: number[] = [];
  const seen = new Set<number>();
  let current: NodeView | undefined = node;
  while (current && current.kind !== "aggregate" && !seen.has(current.id)) {
    lineage.push(current.id);
    seen.add(current.id);
    if (current.parentId === null || seen.has(current.parentId)) break;
    const parent = byId.get(current.parentId);
    if (!parent) {
      lineage.push(current.parentId);
      break;
    }
    current = parent;
  }
  return { node, lineage };
}

/** Find the closest displayed item, never infer identity from a display path. */
export function findHoverMatch(
  target: HoverTarget | null,
  candidates: ReadonlySet<number>,
  byId?: ReadonlyMap<number, NodeView>,
): number | null {
  if (!target || target.node.kind === "aggregate") return null;
  const seen = new Set<number>();
  for (const id of target.lineage) {
    if (seen.has(id) || byId?.get(id)?.kind === "aggregate") return null;
    seen.add(id);
    if (candidates.has(id)) return id;
  }
  // Lazy list children may know an ancestor omitted from their local row index.
  // The map can continue that chain using its own bounded node index.
  let current = byId?.get(target.lineage.at(-1)!);
  while (current?.parentId != null && !seen.has(current.parentId)) {
    const id = current.parentId;
    current = byId?.get(id);
    if (current?.kind === "aggregate") break;
    seen.add(id);
    if (candidates.has(id)) return id;
  }
  return null;
}

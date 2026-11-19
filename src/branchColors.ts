import type { NodeView } from "./types";
import { theme } from "./theme.svelte";
import { branchInks } from "./uiFormat";
import { normalizePath } from "./queueMarkers";

export const neutralBranchColor = "#818a9b";
export const guardianNeutralColor = "#8a8270";
/** The neutral tile colour for the current theme. */
export const neutralColor = () =>
  theme.guardian ? guardianNeutralColor : neutralBranchColor;
/** Rank the focused root's real branches by on-disk size, independently of the
 * native scan IDs. Aggregates never consume a color slot.
 */
export function createBranchPalette(
  nodes: readonly NodeView[],
  rootId: number,
): ReadonlyMap<number, string> {
  const branches = nodes
    .filter(
      (node) =>
        node.parentId === rootId &&
        node.id !== rootId &&
        node.kind !== "aggregate",
    )
    .sort(
      (a, b) =>
        b.allocatedBytes - a.allocatedBytes ||
        a.id - b.id ||
        a.name.localeCompare(b.name),
    );
  const inks = branchInks();
  return new Map(
    branches.map((node, rank) => [node.id, inks[rank % inks.length]]),
  );
}

export const parentPath = (path: string) => {
  const separator = path.lastIndexOf("/");
  return separator < 0 ? "" : path.slice(0, separator) || "/";
};

/** Match the map's first child beneath the focused root, including omitted rows.
 * Paths are display-only fallback evidence, never filesystem identities.
 */
export function createBranchColorResolver(
  nodes: readonly NodeView[],
  rootId: number,
): (node: NodeView) => string {
  const palette = createBranchPalette(nodes, rootId);
  const branchColor = (id: number) => palette.get(id) ?? neutralColor();
  const byId = new Map(nodes.map((node) => [node.id, node]));
  // undefined means the bounded map omitted an ancestor; null is outside focus.
  const branches = new Map<number, number | null | undefined>([[rootId, null]]);
  const root = byId.get(rootId);
  const rootPath = root?.path ? normalizePath(root.path) : undefined;
  const pathBranches = new Map<string, number | null>();
  if (rootPath) {
    for (const node of nodes) {
      if (node.parentId !== rootId || node.id === rootId || !node.path)
        continue;
      const path = normalizePath(node.path);
      if (path === rootPath || parentPath(path) !== rootPath) continue;
      pathBranches.set(
        path,
        pathBranches.has(path) || node.kind === "aggregate" ? null : node.id,
      );
    }
  }

  function branchFor(node: NodeView): number | null | undefined {
    const chain: NodeView[] = [];
    const seen = new Set<number>();
    let current: NodeView | undefined = node;
    let branch: number | null | undefined;
    while (current) {
      if (branches.has(current.id)) {
        branch = branches.get(current.id);
        break;
      }
      if (seen.has(current.id) || current.kind === "aggregate") {
        branch = null;
        break;
      }
      seen.add(current.id);
      chain.push(current);
      if (current.parentId === rootId) {
        branch = current.id;
        break;
      }
      if (current.parentId === null) {
        branch = null;
        break;
      }
      current = byId.get(current.parentId);
    }
    for (const ancestor of chain) branches.set(ancestor.id, branch);
    return branch;
  }

  return (node) => {
    if (node.kind === "aggregate") return neutralColor();
    const branch = branchFor(byId.get(node.id) ?? node);
    if (branch !== undefined)
      return branch === null ? neutralColor() : branchColor(branch);
    if (!rootPath || !node.path) return neutralColor();
    let path = normalizePath(node.path);
    if (
      path === rootPath ||
      !path.startsWith(rootPath === "/" ? "/" : `${rootPath}/`)
    )
      return neutralColor();
    while (path && path !== rootPath) {
      if (pathBranches.has(path)) {
        const pathBranch = pathBranches.get(path);
        return pathBranch == null
          ? neutralColor()
          : branchColor(pathBranch);
      }
      const parent = parentPath(path);
      if (parent === path) break;
      path = parent;
    }
    return neutralColor();
  };
}

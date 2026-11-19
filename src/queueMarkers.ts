type QueueMark = "queued" | "contains" | null;
export interface QueuePath {
  path: string;
  pathKey?: string;
}

/** Native byte keys preserve filenames that share the same display text. */
export function queuePathKey(item: QueuePath): string {
  return item.pathKey ?? encodeURIComponent(item.path).replaceAll("%2F", "/");
}

// Queue paths are native absolute paths. Do not interpret map aggregate labels
// or resolve filesystem aliases when displaying membership.
export const normalizePath = (path: string) =>
  path.replace(/\/+$/, "") || "/";
export function normalizeQueuePath(path: string): string | null {
  return path.startsWith("/") ? normalizePath(path) : null;
}

export function createQueueMarker(
  queuedPaths: readonly QueuePath[],
): (item: QueuePath) => QueueMark {
  const queued = new Set<string>();
  const ancestors = new Set<string>();
  const cache = new Map<string, QueueMark>();
  for (const raw of queuedPaths) {
    const path = normalizeQueuePath(queuePathKey(raw));
    if (!path) continue;
    queued.add(path);
    let parent = path;
    while (parent !== "/") {
      parent = parent.slice(0, parent.lastIndexOf("/")) || "/";
      ancestors.add(parent);
    }
  }
  return (raw) => {
    const path = normalizeQueuePath(queuePathKey(raw));
    if (!path) return null;
    if (cache.has(path)) return cache.get(path)!;
    let parent = path;
    let mark: QueueMark = ancestors.has(path) ? "contains" : null;
    while (true) {
      if (queued.has(parent)) {
        mark = "queued";
        break;
      }
      if (parent === "/") break;
      parent = parent.slice(0, parent.lastIndexOf("/")) || "/";
    }
    cache.set(path, mark);
    return mark;
  };
}

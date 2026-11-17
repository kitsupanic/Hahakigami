import type { QueueEntry } from "./cleanupTypes";

/** A queue row with the part of its folder that sets it apart from the rows around it. */
export interface FolderRow {
  entry: QueueEntry;
  /** Folders between the shown base (or folder group) and the item; "" when it sits right there. */
  where: string;
}

/** One line in a disk's list: a single row, or a folder holding two or more rows. */
type FolderItem =
  | { kind: "row"; row: FolderRow; allocatedBytes: number }
  | {
      kind: "folder";
      name: string;
      rows: FolderRow[];
      allocatedBytes: number;
    };

const parentSegments = (path: string) => path.split("/").filter(Boolean).slice(0, -1);

/** The deepest folder that holds every path, as "/a/b"; "/" when they only share the volume. */
export function commonFolder(paths: string[]): string {
  if (!paths.length) return "/";
  let shared = parentSegments(paths[0]);
  for (const path of paths.slice(1)) {
    const segments = parentSegments(path);
    let i = 0;
    while (i < shared.length && shared[i] === segments[i]) i++;
    shared = shared.slice(0, i);
  }
  return `/${shared.join("/")}`;
}

/**
 * Groups a disk's rows under the first folder below `base` they share. Rows alone in their folder stay
 * single rows, so the list only nests where it saves repeating a folder. Input order is kept within a
 * folder; items are ordered largest first.
 */
export function folderItems(entries: QueueEntry[], base: string): FolderItem[] {
  const skip = base.split("/").filter(Boolean).length;
  const buckets = new Map<string, { name: string; rows: FolderRow[] }>();
  for (const entry of entries) {
    const below = parentSegments(entry.path).slice(skip);
    const key = below.length ? below[0] : `\0${entry.id}`;
    const bucket = buckets.get(key) ?? { name: below[0] ?? "", rows: [] };
    bucket.rows.push({ entry, where: below.join("/") });
    buckets.set(key, bucket);
  }
  const items: FolderItem[] = [...buckets.values()].map(({ name, rows }) => {
    const allocatedBytes = rows.reduce((sum, row) => sum + row.entry.allocatedBytes, 0);
    if (rows.length === 1) return { kind: "row", row: rows[0], allocatedBytes };
    // Inside a folder group, the folder's own name is already on its heading.
    const inner = rows.map((row) => ({ ...row, where: row.where.split("/").slice(1).join("/") }));
    return { kind: "folder", name, rows: inner, allocatedBytes };
  });
  return items.sort((a, b) => b.allocatedBytes - a.allocatedBytes);
}

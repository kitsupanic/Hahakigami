import { expect, it } from "vitest";
import type { QueueEntry } from "./cleanupTypes";
import { commonFolder, folderItems } from "./queueFolders";

const entry = (path: string, allocatedBytes: number): QueueEntry => ({
  id: path,
  name: path.split("/").at(-1)!,
  path,
  rootPath: "/",
  allocatedBytes,
  status: "ready",
  reason: null,
});

it("finds the deepest folder every path shares", () => {
  expect(commonFolder(["/a/b/c/x", "/a/b/d/y"])).toBe("/a/b");
  expect(commonFolder(["/a/b/x"])).toBe("/a/b");
  expect(commonFolder(["/a/x", "/b/y"])).toBe("/");
  // A shared name prefix is not a shared folder.
  expect(commonFolder(["/a/bc/x", "/a/bd/y"])).toBe("/a");
  expect(commonFolder([])).toBe("/");
});

it("nests rows that share a folder below the base and keeps lone rows flat, largest first", () => {
  const base = "/n";
  const items = folderItems(
    [
      entry("/n/startup-v3/run-1/coverage.ndjson", 15),
      entry("/n/cmp/warm-2/coverage.ndjson", 10),
      entry("/n/cmp/initial-0/coverage.ndjson", 9),
      entry("/n/cmp/coverage.ndjson", 1),
      entry("/n/top.log", 2),
    ],
    base,
  );
  expect(items.map((item) => [item.kind, item.allocatedBytes])).toEqual([
    ["folder", 20],
    ["row", 15],
    ["row", 2],
  ]);
  const folder = items[0];
  if (folder.kind !== "folder") throw new Error("expected a folder");
  expect(folder.name).toBe("cmp");
  expect(folder.rows.map((row) => row.where)).toEqual(["warm-2", "initial-0", ""]);
  const lone = items[1];
  expect(lone.kind === "row" && lone.row.where).toBe("startup-v3/run-1");
  expect(items[2].kind === "row" && items[2].row.where).toBe("");
});

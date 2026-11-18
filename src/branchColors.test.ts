import { describe, expect, it } from "vitest";
import {
  createBranchColorResolver,
  createBranchPalette,
  neutralBranchColor,
} from "./branchColors";
import { layout } from "./layout";
import type { NodeView } from "./types";
import { usageColors } from "./uiFormat";

const node = (
  id: number,
  parentId: number | null,
  path: string,
  extra: Partial<NodeView> = {},
): NodeView => ({
  id,
  parentId,
  path,
  name: path.split("/").pop() || "/",
  kind: "directory",
  allocatedBytes: 100,
  childrenCount: 0,
  incomplete: false,
  ...extra,
});
const tree = [
  node(0, null, "/"),
  node(2, 0, "/foo"),
  node(3, 0, "/foobar"),
  node(8, 2, "/foo/nested"),
  node(9, 8, "/foo/nested/file", { kind: "file" }),
  node(10, 2, "/foo/other"),
  node(11, 0, "/empty", { allocatedBytes: 0 }),
  node(-1, 0, "/remaining", { kind: "aggregate" }),
];

describe("branch colors", () => {
  it("ranks arbitrary native IDs by allocated size and resolves ties independently of input order", () => {
    const rows = [
      node(800, null, "/"),
      node(102, 800, "/largest", { allocatedBytes: 900 }),
      node(4, 800, "/second", { allocatedBytes: 500 }),
      node(90, 800, "/third", { allocatedBytes: 300 }),
      node(8, 800, "/fourth", { allocatedBytes: 200 }),
      node(23, 800, "/rose", { allocatedBytes: 100 }),
      node(25, 800, "/cyan", { allocatedBytes: 100 }),
      node(1, 800, "/remaining", { kind: "aggregate", allocatedBytes: 10000 }),
      node(7, 102, "/largest/nested", { allocatedBytes: 50000 }),
    ];
    const palette = createBranchPalette(rows, 800);
    expect([...palette]).toEqual(
      [102, 4, 90, 8, 23, 25].map((id, rank) => [id, usageColors[rank]]),
    );
    expect(createBranchPalette([...rows].reverse(), 800)).toEqual(palette);
    const color = createBranchColorResolver(rows, 800);
    expect(color(rows[8])).toBe(usageColors[0]);
    expect(color(rows[7])).toBe(neutralBranchColor);
    expect([...createBranchPalette(rows, 102)]).toEqual([[7, usageColors[0]]]);
    // Missing direct branches cannot be safely ranked from lazy rows alone.
    expect(color(node(777, 800, "/omitted"))).toBe(neutralBranchColor);
  });
  it.each(["treemap", "sunburst"] as const)(
    "matches actual %s shapes across focus changes",
    (mode) => {
      for (const rootId of [0, 2, 8]) {
        const color = createBranchColorResolver(tree, rootId);
        const palette = createBranchPalette(tree, rootId);
        for (const shape of layout(
          tree,
          rootId,
          1000,
          800,
          mode,
        )) {
          const row = tree.find((candidate) => candidate.id === shape.id)!;
          expect(color(row)).toBe(
            row.kind === "aggregate"
              ? neutralBranchColor
              : palette.get(shape.groupId!),
          );
        }
        expect(color(tree.find((candidate) => candidate.id === rootId)!)).toBe(
          neutralBranchColor,
        );
      }
    },
  );
  it("resolves lazy children through IDs and truncated global results through paths", () => {
    const color = createBranchColorResolver(tree.slice(0, 4), 0);
    expect(color(node(50, 8, "/unavailable"))).toBe(usageColors[0]);
    expect(color(node(51, 500, "/foo/omitted/file"))).toBe(usageColors[0]);
    expect(color(node(52, 500, "/foobar/omitted/file"))).toBe(usageColors[1]);
    expect(color(node(53, 500, "/foobarbaz/file"))).toBe(neutralBranchColor);
  });
  it("normalizes trailing separators and limits path fallback to the current focus", () => {
    const color = createBranchColorResolver(
      [node(2, 0, "/foo///"), node(8, 2, "/foo/nested/")],
      2,
    );
    expect(color(node(50, 500, "/foo/nested/omitted/"))).toBe(usageColors[0]);
    for (const path of ["/foo", "/", "/foobar/nested", "/foo/other/file"])
      expect(color(node(51, 500, path))).toBe(neutralBranchColor);
  });
  it("uses neutral for ambiguous or missing branches, aggregates, and known outside ancestry", () => {
    const color = createBranchColorResolver(
      [
        ...tree,
        node(15, 0, "/foo/"),
        node(20, 999, "/missing"),
        node(30, null, "/foo/outside"),
        node(31, 30, "/foo/outside/file"),
        node(40, 41, "/foo/cycle"),
        node(41, 40, "/foo/cycle/other"),
      ],
      0,
    );
    for (const row of [
      node(50, 999, "/foo/omitted"),
      node(51, 999, "/missing/file"),
      tree[7],
      node(52, 31, "/foo/outside/file/deep"),
      node(53, 40, "/foo/cycle/deep"),
    ])
      expect(color(row)).toBe(neutralBranchColor);
    // IDs remain authoritative despite ambiguous cosmetic display paths.
    expect(color(tree[4])).toBe(usageColors[0]);
    expect(color(tree[6])).toBe(usageColors[3]);
  });
  it("retains ID parity without a root row, but cannot infer a missing display branch", () => {
    const color = createBranchColorResolver([node(2, 0, "/foo")], 0);
    expect(color(node(50, 2, "/foo/file"))).toBe(usageColors[0]);
    expect(color(node(51, 500, "/foo/file"))).toBe(neutralBranchColor);
    expect(createBranchColorResolver([], 0)(node(2, 500, "/foo"))).toBe(
      neutralBranchColor,
    );
  });
});

describe("Guardian data palette", () => {
  it("swaps branch, neutral and file-type inks with the theme", async () => {
    const { theme } = await import("./theme.svelte");
    const { guardianUsageColors } = await import("./uiFormat");
    const { fileTypeColor } = await import("./fileTypes");
    const { guardianNeutralColor } = await import("./branchColors");
    const rows = [node(1, null, "/a"), node(2, 1, "/a/b")];
    theme.guardian = true;
    try {
      expect([...createBranchPalette(rows, 1)]).toEqual([
        [2, guardianUsageColors[0]],
      ]);
      expect(createBranchColorResolver([], 0)(node(3, 500, "/x"))).toBe(
        guardianNeutralColor,
      );
      expect(fileTypeColor("mov")).toBe("#7e4d8a");
    } finally {
      theme.guardian = false;
    }
    expect([...createBranchPalette(rows, 1)]).toEqual([[2, usageColors[0]]]);
    expect(fileTypeColor("mov")).toBe("#8b6ce0");
  });
});

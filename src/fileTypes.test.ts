import { describe, expect, it } from "vitest";
import {
  fileExtension,
  fileTypeColor,
  fileTypeDescription,
  fileTypeLabel,
} from "./fileTypes";

describe("file extensions", () => {
  it.each([
    ["Movie.MOV", "mov"],
    ["archive.tar.GZ", "gz"],
    [".gitignore", ""],
    [".config.JSON", "json"],
    ["README", ""],
    ["trailing.", ""],
    ["", ""],
    [".", ""],
    ["..", ""],
    ["..hidden", "hidden"],
    ["résumé.PDF", "pdf"],
    ["file.ÉXT", "éxt"],
  ])("extracts %s as %s", (name, expected) => {
    expect(fileExtension(name)).toBe(expected);
  });
});

it("labels and describes known, unknown, missing, and inherited-property suffixes", () => {
  expect(fileTypeLabel("mov")).toBe(".mov");
  expect(fileTypeLabel("")).toBe("No extension");
  expect(fileTypeDescription("mov")).toBe("QuickTime video");
  expect(fileTypeDescription("custom")).toBe("CUSTOM file");
  expect(fileTypeDescription("")).toBe("Files without an extension");
  expect(fileTypeDescription("constructor")).toBe("CONSTRUCTOR file");
});

it("assigns stable usable colors independently of request order", () => {
  const extensions = [
    "mov",
    "dmg",
    "zip",
    "mp4",
    "png",
    "pdf",
    "jpg",
    "",
    "custom",
    "constructor",
    "__proto__",
  ];
  const before = extensions.map(fileTypeColor);
  expect(new Set(before.slice(0, 8)).size).toBe(8);
  for (const extension of [...extensions].reverse()) {
    expect(fileTypeColor(extension)).toBe(
      before[extensions.indexOf(extension)],
    );
    expect(fileTypeColor(extension)).toMatch(/^#[0-9a-f]{6}$/);
  }
});

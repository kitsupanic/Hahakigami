// Guards docs/adr/0001: colors and type sizes are defined only in tokens.css.
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const dir = dirname(fileURLToPath(import.meta.url));
const tokensCss = readFileSync(join(dir, "tokens.css"), "utf8");

// Chrome styles only: stylesheets and <style> blocks, not tests, harnesses,
// Icon.svelte paths or the branchColors.ts data palette.
const chrome = readdirSync(dir)
  .filter((name) => /\.(css|svelte)$/.test(name))
  .filter((name) => !/^Test|Harness|^Icon\.svelte$|^tokens\.css$/.test(name))
  .map((name) => {
    const text = readFileSync(join(dir, name), "utf8");
    const css = name.endsWith(".css")
      ? text
      : [...text.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)]
          .map((m) => m[1])
          .join("\n");
    return { name, css: css.replace(/\/\*[\s\S]*?\*\//g, "") };
  });

const offenders = (find: (css: string) => string[]) =>
  chrome.flatMap(({ name, css }) => find(css).map((hit) => `${name}: ${hit}`));

describe("design tokens", () => {
  it("has no font size below 10px and no em/rem sizes", () => {
    const bad = offenders((css) =>
      [...css.matchAll(/font-size:\s*([^;}]+)/g)]
        .map((m) => m[1].trim())
        .filter(
          (value) =>
            !value.startsWith("var(--font-") &&
            (/\d(em|rem)\b/.test(value) ||
              [...value.matchAll(/([\d.]+)px/g)].some((m) => +m[1] < 10)),
        ),
    );
    expect(bad).toEqual([]);
  });

  it("defines color custom properties only in tokens.css", () => {
    const tokenNames = new Set(
      [...tokensCss.matchAll(/^\s*(--[\w-]+):/gm)].map((m) => m[1]),
    );
    const bad = offenders((css) =>
      [...css.matchAll(/:root\s*\{([^}]*)\}/g)]
        .flatMap((m) => [...m[1].matchAll(/(--[\w-]+):\s*([^;]+)/g)])
        .filter(
          ([, prop, value]) =>
            tokenNames.has(prop) || /#[0-9a-f]{3,8}\b/i.test(value),
        )
        .map(([, prop]) => prop),
    );
    expect(bad).toEqual([]);
  });

  it("has no hex color literals outside tokens.css", () => {
    const bad = offenders(
      (css) => css.match(/#[0-9a-f]{3,8}(?![\w-])/gi) ?? [],
    );
    expect(bad).toEqual([]);
  });
});

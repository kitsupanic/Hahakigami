// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, createTestRoot, type Root } from "./testUtils";
import ActiveFilters from "./ActiveFilters.svelte";
import { emptyFilter, type ExplorerFilter } from "./filterTypes";
import { sizeFilterDescription, sizeFilterLabel } from "./activeFilters";

let container: HTMLDivElement, root: Root;
const change = vi.fn();
const gib = 1024 ** 3;
beforeEach(() => {
  container = document.createElement("div");
  document.body.append(container);
  root = createTestRoot(container);
  change.mockClear();
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
});
async function render(value: ExplorerFilter, disabled = false) {
  await act(() =>
    root.render(ActiveFilters, { value, disabled, onChange: change }),
  );
}
function button(name: string) {
  return [...container.querySelectorAll("button")].find(
    (item) => (item.getAttribute("aria-label") ?? item.textContent) === name,
  )!;
}

it("hides inactive filters including the clear action", async () => {
  await render({ ...emptyFilter });
  expect(container.querySelector('[aria-label="Active filters"]')).toBeNull();
  expect(container.querySelector("button")).toBeNull();
});

it("removes each combined filter independently and clears both", async () => {
  const value: ExplorerFilter = {
    category: "videos",
    minBytes: gib,
    maxBytes: null,
  };
  await render(value);
  await act(() => button("Remove category filter: Videos").click());
  expect(change).toHaveBeenLastCalledWith({ ...value, category: "all" });
  await act(() => button("Remove size filter: On disk ≥ 1 GiB").click());
  expect(change).toHaveBeenLastCalledWith({
    ...value,
    minBytes: null,
    maxBytes: null,
  });
  await act(() => button("Clear filters").click());
  expect(change).toHaveBeenLastCalledWith(emptyFilter);
  expect(value).toEqual({ category: "videos", minBytes: gib, maxBytes: null });
});

it("labels the size filter On disk and provides exact inclusive byte boundaries", async () => {
  const value: ExplorerFilter = {
    ...emptyFilter,
    minBytes: gib,
    maxBytes: 2 * gib,
  };
  await render(value);
  expect(button("Remove size filter: On disk 1–2 GiB").title).toBe(
    `On disk: ${gib}–${2 * gib} bytes, inclusive`,
  );
});

it("disables every removal action while unavailable", async () => {
  await render(
    { category: "other", minBytes: 0, maxBytes: 1025 },
    true,
  );
  const buttons = [...container.querySelectorAll("button")];
  expect(buttons).toHaveLength(3);
  expect(buttons.every((item) => item.disabled)).toBe(true);
  await act(() => buttons.forEach((item) => item.click()));
  expect(change).not.toHaveBeenCalled();
});

it.each([
  [gib, null, "On disk ≥ 1 GiB"],
  [null, 1.5 * gib, "On disk ≤ 1.5 GiB"],
  [gib, 2 * gib, "On disk 1–2 GiB"],
  [0, null, "On disk ≥ 0 B"],
  [null, 0, "On disk ≤ 0 B"],
  [0, 0, "On disk 0–0 B"],
  [0, gib, "On disk 0–1 GiB"],
  [1025, null, "On disk ≥ 1025 B"],
  [null, gib + 1, `On disk ≤ ${gib + 1} B`],
  [1025, 1026, "On disk 1025–1026 B"],
  [Number.MAX_SAFE_INTEGER, null, `On disk ≥ ${Number.MAX_SAFE_INTEGER} B`],
] as const)(
  "keeps byte boundaries exact for %s–%s",
  (minBytes, maxBytes, expected) => {
    const filter = { ...emptyFilter, minBytes, maxBytes };
    expect(sizeFilterLabel(filter)).toBe(expected);
  },
);

it("describes lower and upper bounds as inclusive and has no inactive label", () => {
  expect(
    sizeFilterDescription({ ...emptyFilter, minBytes: 1025 }),
  ).toBe("On disk: at least 1025 bytes, inclusive");
  expect(
    sizeFilterDescription({ ...emptyFilter, maxBytes: 0 }),
  ).toBe("On disk: at most 0 bytes, inclusive");
  expect(sizeFilterLabel(emptyFilter)).toBeNull();
});

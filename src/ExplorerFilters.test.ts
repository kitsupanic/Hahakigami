// @vitest-environment jsdom
import { act, createTestRoot as createRoot, type Root } from "./testUtils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import ExplorerFilters from "./ExplorerFilters.svelte";
import { emptyFilter, type ExplorerFilter } from "./filterTypes";
let container: HTMLDivElement, root: Root;
const applied = vi.fn();
import Harness from "./ExplorerFiltersHarness.svelte";
async function choose(name: string, value: string) {
  await act(async () => {
    const input = container.querySelector<HTMLSelectElement>(
      `select[aria-label="${name}"]`,
    )!;
    input.value = value;
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
}
async function fill(name: string, value: string) {
  await act(async () => {
    const input = container.querySelector<HTMLInputElement>(
      `input[aria-label="${name}"]`,
    )!;
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function click(name: string) {
  await act(async () =>
    Array.from(container.querySelectorAll("button"))
      .find((button) => button.textContent === name)!
      .click(),
  );
}
beforeEach(async () => {
  applied.mockClear();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => root.render(Harness, { applied: applied }));
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});
it("applies categories and binary presets immediately", async () => {
  await choose("File category", "images");
  await choose("Size range", "100mb");
  expect(applied).toHaveBeenLastCalledWith({
    category: "images",
    minBytes: 100 * 1024 ** 2,
    maxBytes: null,
  });
  await choose("Size range", "1gb");
  expect(applied.mock.lastCall?.[0].minBytes).toBe(1024 ** 3);
  await choose("Size range", "10gb");
  expect(applied.mock.lastCall?.[0].minBytes).toBe(10 * 1024 ** 3);
});
it("custom edits remain unapplied until Apply and support optional shared units", async () => {
  await choose("Size range", "custom");
  await fill("Maximum size", "1.5");
  expect(applied).not.toHaveBeenCalled();
  await click("Apply range");
  expect(applied).toHaveBeenLastCalledWith({
    category: "all",
    minBytes: null,
    maxBytes: 1.5 * 1024 ** 3,
  });
  await choose("Size unit", "MiB");
  await fill("Minimum size", "100");
  await fill("Maximum size", "");
  await click("Apply range");
  expect(applied.mock.lastCall?.[0]).toEqual({
    category: "all",
    minBytes: 100 * 1024 ** 2,
    maxBytes: null,
  });
});
it("rejects invalid ranges while retaining the prior applied range", async () => {
  await choose("Size range", "1gb");
  await choose("Size range", "custom");
  for (const bad of [
    "-1",
    "Infinity",
    "NaN",
    "999999999999999999999",
    "0.0000000000001",
  ]) {
    await fill("Minimum size", bad);
    await click("Apply range");
    expect(
      container.querySelector('[role="alert"]')?.textContent?.trim(),
    ).toBe(
      "Enter nonnegative sizes of at least 1 byte within the supported range. Previous applied range is unchanged.",
    );
    expect(applied).toHaveBeenCalledTimes(1);
  }
  await fill("Minimum size", "2");
  await fill("Maximum size", "1");
  await click("Apply range");
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "Minimum size must not exceed maximum size",
  );
});
it("accepts fractional sizes that round to whole bytes", async () => {
  await choose("Size range", "custom");
  await fill("Minimum size", "1.1");
  await click("Apply range");
  expect(applied.mock.lastCall?.[0].minBytes).toBe(Math.round(1.1 * 1024 ** 3));
  expect(container.querySelector('[role="alert"]')).toBeNull();
});
it("clear resets custom drafts and category and disabled controls cannot apply", async () => {
  await choose("File category", "audio");
  await choose("Size range", "custom");
  await fill("Minimum size", "-1");
  await click("Apply range");
  await click("Clear filters");
  expect(applied).toHaveBeenLastCalledWith(emptyFilter);
  expect(container.querySelector('[role="alert"]')).toBeNull();
  expect(
    container.querySelector('input[aria-label="Minimum size"]'),
  ).toBeNull();
  await act(async () =>
    root.render(Harness, { disabled: true, applied: applied }),
  );
  expect(
    Array.from(container.querySelectorAll("select,button")).every(
      (input) => (input as HTMLSelectElement).disabled,
    ),
  ).toBe(true);
});

it("R3 preserves unapplied custom size drafts when only the category changes", async () => {
  await choose("Size range", "custom");
  await fill("Minimum size", "2");
  await choose("File category", "images");
  expect(
    container.querySelector<HTMLSelectElement>(
      'select[aria-label="Size range"]',
    )!.value,
  ).toBe("custom");
  expect(
    container.querySelector<HTMLInputElement>(
      'input[aria-label="Minimum size"]',
    )!.value,
  ).toBe("2");
  expect(applied).toHaveBeenLastCalledWith({
    ...emptyFilter,
    category: "images",
  });
  await click("Apply range");
  expect(applied).toHaveBeenLastCalledWith({
    category: "images",
    minBytes: 2 * 1024 ** 3,
    maxBytes: null,
  });
});
it("keeps the typed decimal in the field after Apply", async () => {
  await choose("Size range", "custom");
  await fill("Minimum size", "1.1");
  await click("Apply range");
  expect(
    container.querySelector<HTMLInputElement>(
      'input[aria-label="Minimum size"]',
    )!.value,
  ).toBe("1.1");
});
it.each([
  [Math.round(1.1 * 1024 ** 3), "GiB", "1.1"],
  [1, "B", "1"],
])("remounts %s bytes that no unit holds exactly as %s %s", async (bytes, unit, text) => {
  await act(async () => root.unmount());
  root = createRoot(container);
  await act(async () =>
    root.render(ExplorerFilters, {
      value: { ...emptyFilter, minBytes: bytes },
      onChange: applied,
    }),
  );
  expect(
    container.querySelector<HTMLSelectElement>(
      'select[aria-label="Size unit"]',
    )!.value,
  ).toBe(unit);
  expect(
    container.querySelector<HTMLInputElement>(
      'input[aria-label="Minimum size"]',
    )!.value,
  ).toBe(text);
});
it("accepts a comma as the decimal separator only where the locale uses one", async () => {
  await choose("Size range", "custom");
  await choose("Size unit", "GiB");
  await fill("Minimum size", "1,000");
  await click("Apply range");
  expect(container.querySelector('[role="alert"]')).not.toBeNull();
  expect(applied).not.toHaveBeenCalled();
  const parts = vi
    .spyOn(Intl.NumberFormat.prototype, "formatToParts")
    .mockReturnValue([
      { type: "integer", value: "1" },
      { type: "decimal", value: "," },
      { type: "fraction", value: "1" },
    ]);
  try {
    await fill("Minimum size", "1,5");
    await click("Apply range");
    expect(applied.mock.lastCall?.[0].minBytes).toBe(1.5 * 1024 ** 3);
  } finally {
    parts.mockRestore();
  }
});
it("remounts in the largest unit that holds the applied size exactly", async () => {
  await act(async () => root.unmount());
  root = createRoot(container);
  await act(async () =>
    root.render(ExplorerFilters, {
      value: { ...emptyFilter, minBytes: 500 * 1024 ** 2 },
      onChange: applied,
    }),
  );
  expect(
    container.querySelector<HTMLSelectElement>(
      'select[aria-label="Size unit"]',
    )!.value,
  ).toBe("MiB");
  expect(
    container.querySelector<HTMLInputElement>(
      'input[aria-label="Minimum size"]',
    )!.value,
  ).toBe("500");
});

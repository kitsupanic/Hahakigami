// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it } from "vitest";
import { act, createTestRoot, type Root } from "./testUtils";
import SavedScanLoading from "./SavedScanLoading.svelte";
import type { LibraryProgress } from "./types";

let container: HTMLDivElement, root: Root;
const entry = {
  name: "Macintosh HD",
  rootPath: "/Volumes/Macintosh HD/Users/example",
};
beforeEach(() => {
  container = document.createElement("div");
  document.body.append(container);
  root = createTestRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
});
async function render(
  progress: LibraryProgress | null = null,
  withEntry = true,
) {
  await act(() =>
    root.render(SavedScanLoading, {
      progress,
      entry: withEntry ? entry : null,
    }),
  );
}
it("shows library preparation with one concise live status and no file progress", async () => {
  await render();
  expect(container.querySelector("h1")?.textContent).toBe(
    "Loading saved scans",
  );
  expect(container.querySelectorAll('[role="status"]')).toHaveLength(1);
  expect(container.querySelector('[role="status"]')?.textContent).toBe(
    "Preparing your saved scan library…",
  );
  expect(container.querySelector('[aria-busy="true"]')).not.toBeNull();
  expect(
    container.querySelector('[role="status"]')?.closest('[aria-busy="true"]'),
  ).toBeNull();
  expect(container.textContent).not.toContain(entry.rootPath);
  expect(container.textContent).not.toContain("Decoded");
  expect(container.textContent).toContain(
    "You can view saved scans but not change them.",
  );
  expect(container.querySelector("button")).toBeNull();
});
it("shows the restoring location and processed bytes without inventing a total", async () => {
  await render({ operation: "open", phase: "reading", bytes: 1536 });
  expect(container.querySelector("h1")?.textContent).toBe("Opening saved scan");
  expect(container.textContent).toContain(entry.name);
  expect(container.textContent).toContain(entry.rootPath);
  expect(container.querySelector('[role="status"]')?.textContent).toBe(
    "Reading saved results…",
  );
  expect(container.textContent).toContain("Decoded 1.5 KiB");
  expect(container.textContent).not.toContain("%");
  expect(container.querySelector('[role="progressbar"]')).toBeNull();
});
it.each([
  ["validating", "Validating saved results…"],
  ["done", "Preparing explorer…"],
  ["choosing", "Preparing saved results…"],
] as const)(
  "reflects %s phase while retaining a real zero byte count",
  async (phase, label) => {
    await render({ operation: "open", phase, bytes: 0 }, false);
    expect(container.querySelector('[role="status"]')?.textContent).toBe(label);
    expect(container.textContent).toContain("Decoded 0 B");
    expect(container.textContent).not.toContain("undefined");
    expect(container.textContent).not.toContain("%");
  },
);
it("offers cancellation and announces stopping until backend settlement", async () => {
  let calls = 0;
  await act(() =>
    root.render(SavedScanLoading, {
      entry: { ...entry, entries: 1234 },
      progress: { operation: "open", phase: "reading", bytes: 2048 },
      onCancel: () => {
        calls += 1;
      },
    }),
  );
  const cancel = container.querySelector<HTMLButtonElement>("button")!;
  expect(cancel.textContent).toContain("Cancel opening");
  expect(container.textContent).toContain("1,234 recorded entries");
  await act(() => cancel.click());
  expect(calls).toBe(1);
  await act(() =>
    root.render(SavedScanLoading, {
      progress: { operation: "open", phase: "reading", bytes: 2048 },
      onCancel: () => {
        calls += 1;
      },
      cancelling: true,
    }),
  );
  expect(container.querySelector<HTMLButtonElement>("button")?.disabled).toBe(
    true,
  );
  expect(container.querySelector('[role="status"]')?.textContent).toBe(
    "Stopping saved scan opening…",
  );
});

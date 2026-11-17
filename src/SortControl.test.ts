// @vitest-environment jsdom
import { act, createTestRoot as createRoot, type Root } from "./testUtils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import SortControl from "./SortControl.svelte";
let container: HTMLDivElement, root: Root;
const retry = vi.fn();
beforeEach(() => {
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  retry.mockClear();
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});
async function render(
  saving = false,
  ready = true,
  warning: string | null = null,
) {
  await act(async () =>
    root.render(SortControl, { ready, saving, warning, onRetry: retry }),
  );
}
const status = () => container.querySelector('[role="status"]')?.textContent;
it("renders nothing when ready, idle and without a warning", async () => {
  await render();
  expect(container.innerHTML).not.toContain("sort-control");
  expect(container.querySelector("button")).toBeNull();
});
it("announces loading and saving", async () => {
  await render(false, false);
  expect(status()).toContain("Loading saved sort order");
  await render(true, true);
  expect(status()).toContain("Saving sort order");
});
it("shows the warning with a retry that is disabled while saving", async () => {
  await render(true, true, "Previous stored preference is unchanged.");
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "Previous stored preference",
  );
  expect(container.querySelector("button")!.disabled).toBe(true);
  await render(false, true, "Previous stored preference is unchanged.");
  expect(container.querySelector("button")!.disabled).toBe(false);
  await act(async () => container.querySelector("button")!.click());
  expect(retry).toHaveBeenCalledOnce();
});

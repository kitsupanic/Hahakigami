// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setShowLogicalSize } from "./logicalSize.svelte";
import TrashConfirmation from "./TrashConfirmation.svelte";
import { act, createTestRoot, type Root } from "./testUtils";
import type { NodeView } from "./types";
import { dialogFocus } from "./dialogFocus";

let container: HTMLDivElement, root: Root, opener: HTMLButtonElement;
const onCancel = vi.fn(),
  onConfirm = vi.fn();
function node(id: number, patch: Partial<NodeView> = {}): NodeView {
  return {
    id,
    parentId: null,
    name: "report",
    path: `/data/${id}/report`,
    kind: "file",
    allocatedBytes: 2048,
    logicalBytes: 4096,
    childrenCount: 0,
    incomplete: false,
    ...patch,
  };
}
beforeEach(() => {
  setShowLogicalSize(true);
  onCancel.mockClear();
  onConfirm.mockClear();
  opener = document.createElement("button");
  document.body.append(opener);
  opener.focus();
  container = document.createElement("div");
  document.body.append(container);
  root = createTestRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  opener.remove();
});
async function render(
  items = [node(1)],
  props: { busy?: boolean; queued?: boolean; selectedCount?: number } = {},
) {
  await act(() =>
    root.render(TrashConfirmation, {
      items,
      opener,
      busy: false,
      onCancel,
      onConfirm,
      ...props,
    }),
  );
}
const text = (selector?: string) =>
  (selector
    ? container.querySelector(selector)?.textContent
    : container.textContent
  )
    ?.replace(/\s+/g, " ")
    .trim();
const cancel = () =>
  container.querySelector<HTMLButtonElement>(".trash-actions button")!;
const confirm = () => container.querySelector<HTMLButtonElement>(".danger")!;
const escape = () =>
  container
    .querySelector('[role="dialog"]')!
    .dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    );

it("uses the reviewed list for the confirmation, totals and overlap explanation", async () => {
  await render([node(1, { kind: "directory", allocatedBytes: 8192 })], {
    selectedCount: 3,
  });
  expect(text("h2")).toBe("Move 1 item to Trash?");
  expect(container.querySelectorAll(".trash-item")).toHaveLength(1);
  expect(text(".trash-scope-note")).toContain("3 selected; 1 item to move.");
  expect(text()).toContain(
    "Selected descendants are included through their parent folders.",
  );

  expect(container.querySelector(".trash-summary")?.textContent).toContain(
    "8 KiB",
  );
  expect(text()).toContain(
    "Moving a folder moves everything inside it, even things that weren't scanned.",
  );
  expect(text()).toContain("Folder and contents");
});
it("counts native kinds without inferring folders from names, paths or children", async () => {
  await render([
    node(1, {
      kind: "symlink",
      name: "folder",
      childrenCount: 50,
      path: "/folder/",
    }),
    node(2, { kind: "other" }),
    node(3, { kind: "file" }),
    node(4, { kind: "aggregate" }),
  ]);
  expect(text(".trash-kind-counts")).toBe(
    "1 file · 1 symbolic link · 2 other items",
  );
  expect(container.querySelector(".trash-folder-warning")).toBeNull();
  expect(text()).toContain("Symbolic link only");
  expect(text()).toContain("Other item");
  expect(container.querySelector(".icon-folder")).toBeNull();
});
it("keeps duplicate names distinguishable with every full path and recorded partial sizes", async () => {
  const path = `/data/${"very-long-folder/".repeat(25)}report`;
  await render([node(1, { path, incomplete: true }), node(2)]);
  const rows = container.querySelectorAll(".trash-item");
  expect(rows).toHaveLength(2);
  expect(rows[0].querySelector(".trash-item-path")?.textContent).toBe(path);
  expect(rows[1].querySelector(".trash-item-path")?.textContent).toBe(
    "/data/2/report",
  );
  expect(rows[0].textContent).toContain("Partial recorded size");
  expect(text()).toContain("Some recorded sizes are partial.");
  expect(text()).toContain("Space is freed only when you empty the Trash");
  expect(container.querySelectorAll(".trash-size-note")).toHaveLength(1);
  expect(text()).toContain("You can restore these items from Trash in Finder.");
});
it.each([NaN, Infinity, -1, 1.5, Number.MAX_SAFE_INTEGER + 1])(
  "shows unavailable for an unsafe recorded size %s",
  async (value) => {
    await render([node(1, { allocatedBytes: value })]);
    expect(container.querySelector(".trash-item-size")?.textContent).toBe(
      "Unavailable",
    );
    const totals = container.querySelectorAll(".trash-summary dd");
    expect(totals[1].textContent).toBe("Unavailable");
    expect(totals[2].textContent).toBe("4 KiB");
    expect(text()).not.toContain("NaN");
    expect(text()).not.toContain("Infinity");
  },
);
it("does not overflow combined sizes or hide valid zero", async () => {
  await render([
    node(1, { allocatedBytes: Number.MAX_SAFE_INTEGER, logicalBytes: 0 }),
    node(2, { allocatedBytes: 1, logicalBytes: 0 }),
  ]);
  const totals = container.querySelectorAll(".trash-summary dd");
  expect(totals[1].textContent).toBe("Unavailable");
  expect(totals[2].textContent).toBe("0 B");
});
it("confirms only once per activation and prevents empty confirmations", async () => {
  await render();
  await act(() => confirm().click());
  expect(onConfirm).toHaveBeenCalledOnce();
  expect(onCancel).not.toHaveBeenCalled();
  await render([]);
  expect(text("h2")).toBe("Move 0 items to Trash?");
  expect(confirm().disabled).toBe(true);
  await act(() => confirm().click());
  expect(onConfirm).toHaveBeenCalledOnce();
});
it("focuses Cancel first, traps keyboard focus and supports cancellation with Escape", async () => {
  await render();
  expect(document.activeElement).toBe(cancel());
  const list = container.querySelector<HTMLUListElement>(".trash-items")!;
  expect(list.tabIndex).toBe(0);
  expect(list.getAttribute("aria-label")).toBe("Items to move to Trash");
  // The list precedes Cancel in the native tab order. The action leaves
  // interior Tab navigation to the browser and wraps only at the boundaries.
  const interiorTab = new KeyboardEvent("keydown", {
    key: "Tab",
    shiftKey: true,
    bubbles: true,
    cancelable: true,
  });
  await act(() => cancel().dispatchEvent(interiorTab));
  expect(interiorTab.defaultPrevented).toBe(false);
  await act(() => list.focus());
  await act(() =>
    list.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Tab",
        shiftKey: true,
        bubbles: true,
      }),
    ),
  );
  expect(document.activeElement).toBe(confirm());
  await act(() =>
    confirm().dispatchEvent(
      new KeyboardEvent("keydown", { key: "Tab", bubbles: true }),
    ),
  );
  expect(document.activeElement).toBe(list);
  await act(escape);
  expect(onCancel).toHaveBeenCalledOnce();
  await act(() => cancel().click());
  expect(onCancel).toHaveBeenCalledTimes(2);
});
it("falls back to the review list when Cancel is initially disabled", async () => {
  await render([node(1)], { busy: true });
  const list = container.querySelector<HTMLUListElement>(".trash-items")!;
  expect(document.activeElement).toBe(list);
  await act(() =>
    list.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Tab",
        bubbles: true,
        cancelable: true,
      }),
    ),
  );
  expect(document.activeElement).toBe(list);
  await act(escape);
  expect(onCancel).not.toHaveBeenCalled();
});
it("blocks confirmation, cancellation and Escape while moving", async () => {
  await render();
  await render([node(1)], { busy: true });
  expect(
    container.querySelector('[role="dialog"]')?.getAttribute("aria-busy"),
  ).toBe("true");
  expect(cancel().disabled).toBe(true);
  expect(confirm().disabled).toBe(true);
  expect(confirm().textContent).toBe("Moving…");
  await act(() => {
    cancel().click();
    confirm().click();
    escape();
  });
  expect(onCancel).not.toHaveBeenCalled();
  expect(onConfirm).not.toHaveBeenCalled();
});
it("explains that queue confirmation only contains returned ready items", async () => {
  await render([node(1), node(2)], { queued: true, selectedCount: 100 });
  expect(text("h2")).toBe("Move 2 items to Trash?");
  expect(container.querySelector(".trash-intro")?.textContent).toBe(
    "Ready items from the current scan",
  );
  expect(text()).toContain("Other items stay in the cleanup queue.");
  expect(text()).not.toContain("100");
  expect(text()).not.toContain("Selected descendants");
});

it("keeps the default first-control focus for dialogs without a preference", async () => {
  const dialog = document.createElement("div");
  const first = document.createElement("button");
  const second = document.createElement("button");
  dialog.append(first, second);
  container.append(dialog);
  const action = dialogFocus(dialog, { onClose: onCancel });
  try {
    await act(() => {});
    expect(document.activeElement).toBe(first);
  } finally {
    action.destroy();
    dialog.remove();
  }
});

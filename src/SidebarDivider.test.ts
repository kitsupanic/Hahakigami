// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it } from "vitest";
import { act, createTestRoot, type Root } from "./testUtils";
import SidebarDivider from "./SidebarDivider.svelte";

let container: HTMLDivElement, root: Root, divider: HTMLElement;
let value: number | null;
function render() {
  root.render(SidebarDivider, {
    value,
    onChange: (next: number | null) => {
      value = next;
      render();
    },
  });
}
function pointer(type: string, x: number) {
  const event = new MouseEvent(type, { bubbles: true, clientX: x, button: 0 });
  Object.defineProperty(event, "pointerId", { value: 1 });
  return act(() => divider.dispatchEvent(event));
}
beforeEach(async () => {
  value = null;
  window.innerWidth = 1000;
  container = document.createElement("div");
  const aside = document.createElement("aside");
  aside.getBoundingClientRect = () => ({ width: 220 }) as DOMRect;
  container.append(aside);
  document.body.append(container);
  root = createTestRoot(container);
  await act(render);
  divider = container.querySelector('[role="separator"]')!;
  divider.setPointerCapture = () => {};
  divider.hasPointerCapture = () => false;
});
afterEach(async () => {
  await root.unmount();
  container.remove();
});

it("drags from the rendered width, clamps, and resets", async () => {
  await pointer("pointerdown", 220);
  await pointer("pointermove", 260);
  expect(value).toBe(260);
  await pointer("pointermove", 2000);
  expect(value).toBe(400);
  await pointer("pointermove", -500);
  expect(value).toBe(180);
  await pointer("pointerup", -500);
  await act(() =>
    divider.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })),
  );
  expect(value).toBeNull();
});

it("reports max and the measured width when the stored width is unset", async () => {
  expect(divider.getAttribute("aria-valuemax")).toBe("400");
  expect(divider.getAttribute("aria-valuenow")).toBe("220");
  await pointer("pointerdown", 220);
  await pointer("pointermove", 260);
  expect(divider.getAttribute("aria-valuenow")).toBe("260");
});

it("treats a stored width past the window's maximum as the maximum", async () => {
  value = 480;
  await act(render);
  expect(divider.getAttribute("aria-valuenow")).toBe("400");
  await act(() =>
    divider.dispatchEvent(
      new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }),
    ),
  );
  expect(value).toBe(390);
});

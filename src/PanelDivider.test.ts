// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, createTestRoot, type Root } from "./testUtils";
import PanelDivider from "./PanelDivider.svelte";

let container: HTMLDivElement, root: Root, divider: HTMLElement, value: number;
let resize: () => void;
let captured: number | null;
const change = vi.fn((next: number) => {
  value = next;
  render();
});
function render() {
  root.render(PanelDivider, { value, onChange: change });
}
function pointer(type: string, x: number, id = 1, button = 0) {
  const event = new MouseEvent(type, { bubbles: true, clientX: x, button });
  Object.defineProperty(event, "pointerId", { value: id });
  return act(() => divider.dispatchEvent(event));
}
async function key(key: string, shiftKey = false) {
  await act(() =>
    divider.dispatchEvent(
      new KeyboardEvent("keydown", {
        key,
        shiftKey,
        bubbles: true,
        cancelable: true,
      }),
    ),
  );
}
beforeEach(async () => {
  value = 0.48;
  captured = null;
  change.mockClear();
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(callback: () => void) {
        resize = callback;
      }
      observe() {}
      disconnect() {}
    },
  );
  container = document.createElement("div");
  container.className = "explorer";
  Object.defineProperty(container, "clientWidth", {
    configurable: true,
    value: 600,
  });
  const map = document.createElement("div");
  map.className = "visual";
  map.getBoundingClientRect = () => ({ width: 600 * value }) as DOMRect;
  container.append(map);
  document.body.append(container);
  root = createTestRoot(container);
  await act(render);
  divider = container.querySelector('[role="separator"]')!;
  divider.setPointerCapture = (id) => {
    captured = id;
  };
  divider.hasPointerCapture = (id) => captured === id;
  divider.releasePointerCapture = () => {
    captured = null;
  };
});
afterEach(async () => {
  await root.unmount();
  container.remove();
  vi.unstubAllGlobals();
});

it("supports keyboard adjustments, bounded ends and reset with accessible values", async () => {
  expect(divider.getAttribute("aria-orientation")).toBe("vertical");
  await key("ArrowRight");
  expect(value).toBeCloseTo(0.5);
  expect(divider.getAttribute("aria-valuenow")).toBe("50");
  await key("ArrowLeft", true);
  expect(value).toBeCloseTo(0.4);
  await key("Home");
  expect(value).toBeCloseTo(150 / 600);
  await key("End");
  expect(value).toBeCloseTo(1 - 200 / 600);
  await key("Enter");
  expect(value).toBeCloseTo(0.6);
});
it("captures a drag, clamps minimum panel sizes and stops on pointer cancellation", async () => {
  await pointer("pointerdown", 200);
  expect(captured).toBe(1);
  await pointer("pointermove", 260);
  expect(value).toBeCloseTo(0.58);
  await pointer("pointermove", -1000);
  expect(value).toBeCloseTo(150 / 600);
  await pointer("pointercancel", -1000);
  expect(captured).toBeNull();
  await pointer("pointermove", 500);
  expect(value).toBeCloseTo(150 / 600);
});

it("rebases keyboard movement on rendered geometry before the resize observer catches up", async () => {
  await key("Home");
  expect(value).toBeCloseTo(0.25);
  Object.defineProperty(container, "clientWidth", { value: 500 });
  const map = container.querySelector<HTMLElement>(".visual")!;
  map.getBoundingClientRect = () => ({ width: 150 }) as DOMRect;
  await key("ArrowRight");
  expect(value).toBeCloseTo(0.32);
  expect(divider.getAttribute("aria-valuemin")).toBe("30");
  expect(divider.getAttribute("aria-valuenow")).toBe("32");
});
it("ignores other pointer IDs and secondary-button drags, and clamps after window resize", async () => {
  await pointer("pointerdown", 200, 1, 2);
  expect(captured).toBeNull();
  await pointer("pointerdown", 200);
  await pointer("pointermove", 250, 2);
  expect(value).toBe(0.48);
  await pointer("pointerup", 200);
  await key("End");
  Object.defineProperty(container, "clientWidth", { value: 380 });
  await act(() => resize());
  expect(value).toBeCloseTo(1 - 200 / 380);
});

it("resets a pointer-adjusted layout to the same 60/40 split on double-click", async () => {
  await key("Home");
  await act(() =>
    divider.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })),
  );
  expect(value).toBeCloseTo(0.6);
  expect(divider.getAttribute("aria-valuetext")).toBe("60% map, 40% file list");
});

it("uses responsive panel minimums for both keyboard bounds and accessible percentages", async () => {
  container.style.setProperty("--map-min-width", "250px");
  container.style.setProperty("--list-min-width", "140px");
  Object.defineProperty(container, "clientWidth", { value: 400 });
  await act(() => resize());
  expect(value).toBeCloseTo(250 / 400);
  expect(divider.getAttribute("aria-valuemin")).toBe("63");
  await key("End");
  expect(value).toBeCloseTo(1 - 140 / 400);
  expect(divider.getAttribute("aria-valuemax")).toBe("65");
});

it("preserves the desktop split while the divider is hidden in the stacked layout", async () => {
  divider.style.display = "none";
  Object.defineProperty(container, "clientWidth", { value: 380 });
  await act(() => resize());
  expect(value).toBe(0.48);
  divider.style.display = "block";
  await act(() => resize());
  expect(value).toBeCloseTo(1 - 200 / 380);
});

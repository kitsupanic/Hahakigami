// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import FileTypes from "./FileTypes.svelte";
import { act, createTestRoot, type Root } from "./testUtils";
import type { FileTypeStats } from "./fileTypes";

const count = 18_185;
const stats: FileTypeStats = {
  items: Array.from({ length: count }, (_, index) => ({
    extension: `rare${String(index).padStart(5, "0")}`,
    files: 1,
    allocatedBytes: count - index,
  })),
  files: count,
  allocatedBytes: (count * (count + 1)) / 2,
  incomplete: false,
};
let element: HTMLDivElement, root: Root;
afterEach(async () => {
  if (root) await act(() => root.unmount());
  element?.remove();
});
async function mount() {
  element = document.createElement("div");
  document.body.append(element);
  root = createTestRoot(element);
  const onSelect = vi.fn();
  const props = {
    stats,
    scope: "scan" as const,
    selectedExtension: null as string | null,
    scopePath: "/fixture",
    onSelect,
    onScopeChange: vi.fn(),
    onShowFiles: vi.fn(),
    onRetry: vi.fn(),
  };
  await act(() => root.render(FileTypes, props));
  return {
    props,
    onSelect,
    list: element.querySelector<HTMLDivElement>(".file-type-list")!,
  };
}
it("bounds mounted rows for 18,185 types and searches the entire list", async () => {
  const f = await mount();
  expect(element.querySelectorAll(".file-type-row").length).toBeLessThan(30);
  expect(element.querySelector("li")?.getAttribute("aria-setsize")).toBe(
    String(count),
  );
  expect(element.querySelector("li")?.getAttribute("aria-posinset")).toBe("1");
  const search = element.querySelector<HTMLInputElement>("input")!;
  await act(() => {
    search.value = "rare18184";
    search.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const rows = element.querySelectorAll<HTMLButtonElement>(".file-type-row");
  expect(rows).toHaveLength(1);
  expect(rows[0].textContent).toContain(".rare18184");
  await act(() => rows[0].click());
  expect(f.onSelect).toHaveBeenCalledExactlyOnceWith("rare18184");
});
it("scrolls to the end while maintaining bounded rows and list positions", async () => {
  const f = await mount();
  await act(() => {
    f.list.scrollTop = (count - 5) * 64;
    f.list.dispatchEvent(new Event("scroll"));
  });
  expect(element.querySelectorAll(".file-type-row").length).toBeLessThan(30);
  const last = element.querySelector<HTMLLIElement>("li:last-child")!;
  expect(last.getAttribute("aria-posinset")).toBe(String(count));
  expect(last.textContent).toContain(".rare18184");
  expect(element.querySelector(".file-type-selection")).toBeNull();
});
it("supports keyboard navigation to offscreen types without selecting them", async () => {
  const f = await mount();
  const key = async (value: string) =>
    act(() =>
      document.activeElement!.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: value,
          bubbles: true,
          cancelable: true,
        }),
      ),
    );
  element.querySelector<HTMLButtonElement>(".file-type-row")!.focus();
  await key("End");
  expect(document.activeElement?.getAttribute("data-type-index")).toBe(
    String(count - 1),
  );
  expect(f.onSelect).not.toHaveBeenCalled();
  await key("ArrowUp");
  expect(document.activeElement?.getAttribute("data-type-index")).toBe(
    String(count - 2),
  );
  await key("PageUp");
  expect(
    Number(document.activeElement?.getAttribute("data-type-index")),
  ).toBeLessThan(count - 2);
  await key("Home");
  expect(document.activeElement?.getAttribute("data-type-index")).toBe("0");
  expect(element.querySelectorAll(".file-type-row").length).toBeLessThan(30);
});
it("resets the window for a new scope and keeps an offscreen selection summary", async () => {
  const f = await mount();
  await act(() => {
    f.list.scrollTop = (count - 5) * 64;
    f.list.dispatchEvent(new Event("scroll"));
  });
  await act(() =>
    root.render(FileTypes, {
      ...f.props,
      scope: "folder",
      stats: {
        ...stats,
        items: stats.items.map((item) => ({
          ...item,
          allocatedBytes: item.files + count - item.allocatedBytes,
        })),
      },
      selectedExtension: "rare00000",
    }),
  );
  expect(f.list.scrollTop).toBe(0);
  expect(element.querySelector(".file-type-name strong")?.textContent).toBe(
    ".rare18184",
  );
  expect(element.querySelector(".file-type-selection")?.textContent).toContain(
    ".rare00000 highlighted",
  );
  expect(element.querySelectorAll(".file-type-row.active")).toHaveLength(0);
});

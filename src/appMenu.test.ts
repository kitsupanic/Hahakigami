// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import {
  CheckMenuItem,
  PredefinedMenuItem,
  Submenu,
  installed,
  menuItem,
  menuItems,
} from "./fakeMenu";
import {
  checkIds,
  commandIds,
  createAppMenu,
  textFieldIds,
  type CommandId,
  type MenuState,
} from "./appMenu";

const forEachCommand = <T>(value: () => T) =>
  Object.fromEntries(commandIds.map((id) => [id, value()])) as Record<
    CommandId,
    T
  >;
const spies = () => forEachCommand(() => vi.fn());
const state = (patch: Partial<MenuState> = {}): MenuState => ({
  enabled: forEachCommand(() => true),
  checked: {},
  quiet: false,
  typing: false,
  sidebarVisible: true,
  fileListVisible: true,
  ...patch,
});
const submenu = (text: string) =>
  installed.menu!.items.find((item) => item.text === text) as Submenu;
const layout = (text: string) =>
  submenu(text).items.map((item) =>
    item instanceof PredefinedMenuItem
      ? item.kind
      : `${(item as { text: string }).text}${(item as { accelerator?: string }).accelerator ? ` [${(item as { accelerator?: string }).accelerator}]` : ""}`,
  );
async function make(commands = spies()) {
  const menu = await createAppMenu(commands);
  return { menu, commands };
}
afterEach(() => {
  document.body.replaceChildren();
});

it("builds the documented menus, items and accelerators", async () => {
  await make();
  expect(installed.menu!.items.map((item) => item.text)).toEqual([
    "Hahakigami",
    "File",
    "Edit",
    "View",
    "Go",
    "Item",
    "Window",
  ]);
  expect(layout("Hahakigami")).toEqual([
    "About",
    "Separator",
    "Services",
    "Separator",
    "Hide",
    "HideOthers",
    "ShowAll",
    "Separator",
    "Quit",
  ]);
  expect(layout("File")).toEqual([
    "Choose Folder… [CmdOrCtrl+O]",
    "Rescan [CmdOrCtrl+R]",
    "Scan All Disks",
    "Stop Scan [CmdOrCtrl+.]",
  ]);
  expect(layout("Edit")).toEqual([
    "Undo",
    "Redo",
    "Separator",
    "Cut",
    "Copy",
    "Paste",
    "SelectAll",
    "Separator",
    "Find [CmdOrCtrl+F]",
  ]);
  expect(layout("View")).toEqual([
    "Treemap [CmdOrCtrl+1]",
    "Sunburst [CmdOrCtrl+2]",
    "Separator",
    "Folders and Files [CmdOrCtrl+Alt+1]",
    "File Types [CmdOrCtrl+Alt+2]",
    "Separator",
    "Hide Sidebar [Cmd+Ctrl+S]",
    "Hide File List [CmdOrCtrl+Alt+I]",
    "Separator",
    "Guardian Theme",
    "Separator",
    "Fullscreen",
  ]);
  expect(layout("Go")).toEqual([
    "Back [CmdOrCtrl+[]",
    "Forward [CmdOrCtrl+]]",
    "Enclosing Folder [CmdOrCtrl+Up]",
    "Open Folder [CmdOrCtrl+Down]",
    "Separator",
    "Cleanup Queue [CmdOrCtrl+Alt+Q]",
  ]);
  expect(layout("Item")).toEqual([
    "Quick Look [CmdOrCtrl+Y]",
    "Reveal in Finder [CmdOrCtrl+Shift+R]",
    "Separator",
    "Add to Cleanup Queue [CmdOrCtrl+Shift+A]",
    "Move to Trash… [CmdOrCtrl+Backspace]",
  ]);
  expect(layout("Window")).toEqual([
    "Minimize",
    "Maximize",
    "Separator",
    "CloseWindow",
  ]);
  expect(checkIds.size).toBe(5);
  for (const id of commandIds)
    expect(menuItem(id) instanceof CheckMenuItem, id).toBe(checkIds.has(id));
  expect(installed.windowsMenu).toBe(submenu("Window"));
  expect(menuItems().size).toBe(commandIds.length);
});

it("gives no two commands the same accelerator", async () => {
  await make();
  const accelerators = [...menuItems().values()]
    .map((item) => item.accelerator)
    .filter((accelerator) => !!accelerator);
  expect(new Set(accelerators).size).toBe(accelerators.length);
});

it("applies enabled, checked and flipping text, skipping unchanged values", async () => {
  const { menu } = await make();
  await menu.sync(state({ enabled: { ...state().enabled, back: false } }));
  expect(menuItem("back").enabled).toBe(false);
  expect(menuItem("forward").enabled).toBe(true);
  const setEnabled = vi.spyOn(menuItem("forward"), "setEnabled");
  await menu.sync(
    state({
      enabled: { ...state().enabled, back: false },
      checked: { treemap: true },
      sidebarVisible: false,
      fileListVisible: false,
    }),
  );
  expect(setEnabled).not.toHaveBeenCalled();
  expect((menuItem("treemap") as CheckMenuItem).checked).toBe(true);
  expect((menuItem("sunburst") as CheckMenuItem).checked).toBe(false);
  expect(menuItem("toggleSidebar").text).toBe("Show Sidebar");
  expect(menuItem("toggleFileList").text).toBe("Show File List");
});

it("disables every app command when quiet but never the predefined items", async () => {
  const { menu } = await make();
  await menu.sync(state({ quiet: true }));
  expect([...menuItems().values()].every((item) => !item.enabled)).toBe(true);
  await menu.sync(state());
  expect([...menuItems().values()].every((item) => item.enabled)).toBe(true);
});

it("runs actions only while enabled, and ignores Go and Item commands in text fields", async () => {
  const { menu, commands } = await make();
  menuItem("rescan").click();
  expect(commands.rescan).not.toHaveBeenCalled();
  await menu.sync(state());
  menuItem("rescan").click();
  expect(commands.rescan).toHaveBeenCalledTimes(1);
  const input = document.body.appendChild(document.createElement("input"));
  input.focus();
  for (const id of textFieldIds) {
    menuItem(id).click();
    expect(commands[id], id).not.toHaveBeenCalled();
  }
  menuItem("find").click();
  menuItem("rescan").click();
  expect(commands.find).toHaveBeenCalled();
  expect(commands.rescan).toHaveBeenCalledTimes(2);
  const editable = document.body.appendChild(document.createElement("div"));
  editable.setAttribute("contenteditable", "true");
  const inner = editable.appendChild(document.createElement("span"));
  inner.tabIndex = 0;
  inner.focus();
  menuItem("trash").click();
  expect(commands.trash).not.toHaveBeenCalled();
  input.blur();
  inner.blur();
  menuItem("trash").click();
  expect(commands.trash).toHaveBeenCalledTimes(1);
});

it("restores the check mark a native click flipped", async () => {
  const { menu, commands } = await make();
  await menu.sync(state({ checked: { sunburst: true } }));
  const sunburst = menuItem("sunburst") as CheckMenuItem;
  sunburst.click();
  expect(commands.sunburst).toHaveBeenCalled();
  expect(sunburst.checked).toBe(false);
  await menu.sync(state({ checked: { sunburst: true } }));
  expect(sunburst.checked).toBe(true);
});

it("disables text-field commands while typing so their keys reach the field", async () => {
  const { menu } = await make();
  await menu.sync(state({ typing: true }));
  for (const [id, item] of menuItems())
    expect(item.enabled, id).toBe(!textFieldIds.has(id as CommandId));
  expect(textFieldIds.has("find")).toBe(false);
  await menu.sync(state({ typing: false }));
  expect([...menuItems().values()].every((item) => item.enabled)).toBe(true);
});

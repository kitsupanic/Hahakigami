import {
  CheckMenuItem,
  Menu,
  MenuItem,
  PredefinedMenuItem,
  Submenu,
  type PredefinedMenuItemOptions,
} from "@tauri-apps/api/menu";

export const commandIds = [
  "chooseFolder",
  "rescan",
  "scanAllDisks",
  "stopScan",
  "find",
  "sunburst",
  "treemap",
  "viewFiles",
  "viewFileTypes",
  "toggleSidebar",
  "toggleFileList",
  "guardianTheme",
  "back",
  "forward",
  "enclosing",
  "openFolder",
  "cleanupQueue",
  "quickLook",
  "reveal",
  "addToQueue",
  "trash",
] as const;
export type CommandId = (typeof commandIds)[number];
export type MenuCommands = Record<CommandId, () => unknown>;
export interface MenuState {
  enabled: Record<CommandId, boolean>;
  checked: Partial<Record<CommandId, boolean>>;
  /** A dialog or popover is open: every app command is disabled. */
  quiet: boolean;
  /** A text field has focus: its commands are disabled so their keys reach the field. */
  typing: boolean;
  sidebarVisible: boolean;
  fileListVisible: boolean;
}

interface Entry {
  id: CommandId;
  text: string;
  accelerator?: string;
  /** Shows a check mark for the current choice. */
  check?: true;
  /** Disabled while a text field has focus, so its key equivalent falls through to the field. */
  field?: true;
}
type Row = Entry | PredefinedMenuItemOptions["item"];
const isEntry = (row: Row): row is Entry =>
  typeof row === "object" && "id" in row;
const cmd = (
  id: CommandId,
  text: string,
  accelerator?: string,
  flags: Pick<Entry, "check" | "field"> = {},
): Entry => ({ id, text, accelerator, ...flags });
const check = { check: true } as const;
const field = { field: true } as const;
const layout: readonly (readonly [title: string, rows: readonly Row[]])[] = [
  [
    "Hahakigami",
    [
      { About: null },
      "Separator",
      "Services",
      "Separator",
      "Hide",
      "HideOthers",
      "ShowAll",
      "Separator",
      "Quit",
    ],
  ],
  [
    "File",
    [
      cmd("chooseFolder", "Choose Folder…", "CmdOrCtrl+O"),
      cmd("rescan", "Rescan", "CmdOrCtrl+R"),
      cmd("scanAllDisks", "Scan All Disks"),
      cmd("stopScan", "Stop Scan", "CmdOrCtrl+."),
    ],
  ],
  [
    "Edit",
    [
      "Undo",
      "Redo",
      "Separator",
      "Cut",
      "Copy",
      "Paste",
      "SelectAll",
      "Separator",
      cmd("find", "Find", "CmdOrCtrl+F"),
    ],
  ],
  [
    "View",
    [
      cmd("treemap", "Treemap", "CmdOrCtrl+1", check),
      cmd("sunburst", "Sunburst", "CmdOrCtrl+2", check),
      "Separator",
      cmd("viewFiles", "Folders and Files", "CmdOrCtrl+Alt+1", check),
      cmd("viewFileTypes", "File Types", "CmdOrCtrl+Alt+2", check),
      "Separator",
      cmd("toggleSidebar", "Hide Sidebar", "Cmd+Ctrl+S"),
      cmd("toggleFileList", "Hide File List", "CmdOrCtrl+Alt+I"),
      "Separator",
      cmd("guardianTheme", "Guardian Theme", undefined, check),
      "Separator",
      "Fullscreen",
    ],
  ],
  [
    "Go",
    [
      cmd("back", "Back", "CmdOrCtrl+[", field),
      cmd("forward", "Forward", "CmdOrCtrl+]", field),
      cmd("enclosing", "Enclosing Folder", "CmdOrCtrl+Up", field),
      cmd("openFolder", "Open Folder", "CmdOrCtrl+Down", field),
      "Separator",
      cmd("cleanupQueue", "Cleanup Queue", "CmdOrCtrl+Alt+Q", field),
    ],
  ],
  [
    "Item",
    [
      cmd("quickLook", "Quick Look", "CmdOrCtrl+Y", field),
      cmd("reveal", "Reveal in Finder", "CmdOrCtrl+Shift+R", field),
      "Separator",
      cmd("addToQueue", "Add to Cleanup Queue", "CmdOrCtrl+Shift+A", field),
      cmd("trash", "Move to Trash…", "CmdOrCtrl+Backspace", field),
    ],
  ],
  ["Window", ["Minimize", "Maximize", "Separator", "CloseWindow"]],
];
const entries = layout.flatMap(([, rows]) => rows.filter(isEntry));
const flagged = (flag: "check" | "field") =>
  new Set(entries.filter((entry) => entry[flag]).map((entry) => entry.id));
export const checkIds = flagged("check");
export const textFieldIds = flagged("field");

export function isTextField(element: unknown) {
  return (
    element instanceof HTMLInputElement ||
    element instanceof HTMLTextAreaElement ||
    (element instanceof HTMLElement &&
      !!element.closest('[contenteditable]:not([contenteditable="false"])'))
  );
}

export interface AppMenu {
  sync(state: MenuState): Promise<void>;
}

/** Builds the native menu bar once, installs it, and returns a state synchroniser. */
export async function createAppMenu(commands: MenuCommands): Promise<AppMenu> {
  const items = new Map<CommandId, MenuItem | CheckMenuItem>();
  const applied = new Map<string, unknown>();
  let latest: MenuState | undefined;
  let queue: Promise<void> = Promise.resolve();
  const sync = (state: MenuState) => {
    latest = state;
    // Apply the newest state when the turn comes: slow native writes must not replay every stale one.
    queue = queue.then(() => apply(latest!)).catch(console.error);
    return queue;
  };
  const available = (id: CommandId, state: MenuState) =>
    !state.quiet &&
    state.enabled[id] &&
    !(state.typing && textFieldIds.has(id));
  async function apply(state: MenuState) {
    const writes: Promise<void>[] = [];
    const set = (key: string, value: unknown, write: () => Promise<void>) => {
      if (applied.get(key) === value) return;
      applied.set(key, value);
      // A failed native write is forgotten, so the next sync retries it.
      const written = write();
      written.catch(() => {
        if (applied.get(key) === value) applied.delete(key);
      });
      writes.push(written);
    };
    for (const [id, item] of items) {
      const enabled = available(id, state);
      set(`${id}:enabled`, enabled, () => item.setEnabled(enabled));
      if (item instanceof CheckMenuItem)
        set(`${id}:checked`, !!state.checked[id], () =>
          item.setChecked(!!state.checked[id]),
        );
    }
    const toggles = [
      ["toggleSidebar", "Sidebar", state.sidebarVisible],
      ["toggleFileList", "File List", state.fileListVisible],
    ] as const;
    for (const [id, name, visible] of toggles) {
      const text = `${visible ? "Hide" : "Show"} ${name}`;
      set(`${id}:text`, text, () => items.get(id)!.setText(text));
    }
    await Promise.all(writes);
  }
  function run(id: CommandId) {
    // Backstop: a native accelerator can race the next sync.
    if (!latest || !available(id, latest)) return;
    if (textFieldIds.has(id) && isTextField(document.activeElement)) return;
    void commands[id]();
    if (checkIds.has(id)) {
      // Native check items flip themselves; put the real state back.
      applied.delete(`${id}:checked`);
      void sync(latest);
    }
  }
  async function item({ id, text, accelerator, check }: Entry) {
    const options = {
      id,
      text,
      accelerator,
      enabled: false,
      action: () => run(id),
    };
    const created = check
      ? await CheckMenuItem.new({ ...options, checked: false })
      : await MenuItem.new(options);
    items.set(id, created);
    return created;
  }
  const submenus = await Promise.all(
    layout.map(async ([text, rows]) =>
      Submenu.new({
        text,
        items: await Promise.all(
          rows.map((row) =>
            isEntry(row) ? item(row) : PredefinedMenuItem.new({ item: row }),
          ),
        ),
      }),
    ),
  );
  await (await Menu.new({ items: submenus })).setAsAppMenu();
  await submenus[
    layout.findIndex(([title]) => title === "Window")
  ].setAsWindowsMenuForNSApp();
  return { sync };
}

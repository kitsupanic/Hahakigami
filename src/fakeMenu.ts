/** Recording stand-in for `@tauri-apps/api/menu`, installed for every test by testSetup.ts. */
type Item = MenuItem | PredefinedMenuItem | Submenu;
export const installed: { menu: Menu | null; windowsMenu: Submenu | null } = {
  menu: null,
  windowsMenu: null,
};

export class MenuItem {
  id: string;
  text: string;
  accelerator?: string;
  enabled: boolean;
  action?: (id: string) => void;
  constructor(options: {
    id?: string;
    text: string;
    accelerator?: string;
    enabled?: boolean;
    action?: (id: string) => void;
  }) {
    this.id = options.id ?? options.text;
    this.text = options.text;
    this.accelerator = options.accelerator;
    this.enabled = options.enabled ?? true;
    this.action = options.action;
  }
  static async new(options: ConstructorParameters<typeof MenuItem>[0]) {
    return new MenuItem(options);
  }
  async setEnabled(value: boolean) {
    this.enabled = value;
  }
  async setText(value: string) {
    this.text = value;
  }
  /** Simulates a user click or accelerator press. */
  click() {
    this.action?.(this.id);
  }
}
export class CheckMenuItem extends MenuItem {
  checked: boolean;
  constructor(
    options: ConstructorParameters<typeof MenuItem>[0] & { checked?: boolean },
  ) {
    super(options);
    this.checked = options.checked ?? false;
  }
  static async new(
    options: ConstructorParameters<typeof CheckMenuItem>[0],
  ): Promise<CheckMenuItem> {
    return new CheckMenuItem(options);
  }
  async setChecked(value: boolean) {
    this.checked = value;
  }
  /** Native check items flip themselves before running the action. */
  click() {
    this.checked = !this.checked;
    super.click();
  }
}
export class PredefinedMenuItem {
  kind: string;
  constructor(options: { item: string | Record<string, unknown> }) {
    this.kind =
      typeof options.item === "string"
        ? options.item
        : Object.keys(options.item)[0];
  }
  static async new(
    options: ConstructorParameters<typeof PredefinedMenuItem>[0],
  ) {
    return new PredefinedMenuItem(options);
  }
}
export class Submenu {
  text: string;
  items: Item[];
  constructor(options: { text: string; items?: Item[] }) {
    this.text = options.text;
    this.items = options.items ?? [];
  }
  static async new(options: ConstructorParameters<typeof Submenu>[0]) {
    return new Submenu(options);
  }
  async setAsWindowsMenuForNSApp() {
    installed.windowsMenu = this;
  }
}
export class Menu {
  items: Submenu[];
  constructor(options: { items?: Submenu[] }) {
    this.items = options.items ?? [];
  }
  static async new(options: ConstructorParameters<typeof Menu>[0]) {
    return new Menu(options);
  }
  async setAsAppMenu() {
    installed.menu = this;
    return null;
  }
}
/** Every custom item in the installed menu, by command id. */
export function menuItems() {
  const found = new Map<string, MenuItem | CheckMenuItem>();
  for (const submenu of installed.menu?.items ?? [])
    for (const item of submenu.items)
      if (item instanceof MenuItem) found.set(item.id, item);
  return found;
}
export function menuItem(id: string) {
  const item = menuItems().get(id);
  if (!item) throw new Error(`No menu item ${id}`);
  return item;
}

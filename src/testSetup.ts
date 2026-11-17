import { afterEach, vi } from "vitest";
import { installed } from "./fakeMenu";

vi.mock("@tauri-apps/api/menu", () => import("./fakeMenu"));
afterEach(() => {
  installed.menu = null;
  installed.windowsMenu = null;
});

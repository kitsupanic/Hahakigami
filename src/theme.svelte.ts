import { isTauri } from "@tauri-apps/api/core";

/** The opt-in Guardian theme (website palette); default chrome follows macOS (docs/adr/0001). */
const key = "hahakigami.theme";
const appearanceKey = "hahakigami.appearance";

/** Light or dark mode: follow macOS (system) or force one. */
export type Appearance = "system" | "light" | "dark";

export function appearanceStored(): Appearance {
  try {
    const value = localStorage.getItem(appearanceKey);
    return value === "light" || value === "dark" ? value : "system";
  } catch {
    return "system";
  }
}

function themeStored() {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

// Older builds stored "guardian-v2".."guardian-v5"; they all load as Guardian now.
export const guardianStored = () => themeStored()?.startsWith("guardian") ?? false;

/** Read by data palettes and the map canvas so they repaint when the theme changes. */
export const theme = $state({
  guardian: guardianStored(),
  appearance: appearanceStored(),
});

/** Sets the window's appearance; the web view's prefers-color-scheme follows it, so every theme token does too. */
export function applyAppearance(value: Appearance) {
  theme.appearance = value;
  try {
    if (value === "system") localStorage.removeItem(appearanceKey);
    else localStorage.setItem(appearanceKey, value);
  } catch {
    // Storage unavailable: the choice lasts for this session only.
  }
  syncWindowTheme();
}

/** Guardian is light only for now, so it holds the window light whatever Appearance says. */
function syncWindowTheme() {
  if (!isTauri()) return;
  const value = theme.guardian ? "light" : theme.appearance;
  void import("@tauri-apps/api/window")
    .then(({ getCurrentWindow }) =>
      getCurrentWindow().setTheme(value === "system" ? null : value),
    )
    .catch(() => {
      // Window API unavailable: the app keeps following macOS.
    });
}

export function applyGuardian(on: boolean) {
  theme.guardian = on;
  if (on) document.documentElement.dataset.theme = "guardian";
  else delete document.documentElement.dataset.theme;
  try {
    if (on) localStorage.setItem(key, "guardian");
    else localStorage.removeItem(key);
  } catch {
    // Storage unavailable: the choice lasts for this session only.
  }
  syncWindowTheme();
}

/** Map typography and inks shared by the canvas in Guardian. */
export const guardianInk = "#16140f";
export const guardianMapFont = '"Archivo Variable", -apple-system, sans-serif';

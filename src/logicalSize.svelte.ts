import { booleanPreference } from "./booleanPreference";

/** Whether Logical size is offered. Off by default: sizes are On disk everywhere. */
const preference = booleanPreference("hahakigami.showLogicalSize", false);

/** Read by every view that could show Logical size, so they update when the switch flips. */
export const logicalSize = $state({ shown: preference.read() });

export function setShowLogicalSize(on: boolean) {
  logicalSize.shown = on;
  preference.write(on);
}

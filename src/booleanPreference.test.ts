// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { booleanPreference } from "./booleanPreference";

afterEach(() => localStorage.clear());

it.each([
  { defaultValue: false, stored: "true" },
  { defaultValue: true, stored: "false" },
])(
  "stores only the non-default value (default $defaultValue)",
  ({ defaultValue, stored }) => {
    const preference = booleanPreference("k", defaultValue);
    expect(preference.read()).toBe(defaultValue);
    preference.write(!defaultValue);
    expect(localStorage.getItem("k")).toBe(stored);
    expect(preference.read()).toBe(!defaultValue);
    preference.write(defaultValue);
    expect(localStorage.getItem("k")).toBeNull();
    expect(preference.read()).toBe(defaultValue);
  },
);

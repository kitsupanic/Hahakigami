import { guardianInk, theme } from "./theme.svelte";
import { bytes } from "./types";

/** One binary unit for compact UI; bytes() retains the full accounting detail. */
/** "1 file", "2 files": one place for the singular/plural switch. */
export const countLabel = (n: number, one: string, many = `${one}s`) =>
  `${n.toLocaleString()} ${n === 1 ? one : many}`;
export function compactBytes(n: number, minimumFractionDigits: 0 | 1 = 0) {
  return bytes(n, minimumFractionDigits).split(" (")[0];
}

/** The reported scan duration, shared by the recorded summary and diagnostics. */
export function scanTime(ms: number) {
  const seconds = Math.max(0, ms) / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)} seconds`;
  const whole = Math.floor(seconds);
  if (whole < 3600) return `${Math.floor(whole / 60)}m ${whole % 60}s`;
  return `${Math.floor(whole / 3600)}h ${Math.floor((whole % 3600) / 60)}m ${whole % 60}s`;
}

/** Pick the stronger contrast against an opaque six-digit map tile tone. */
export function mapLabelColor(tone: string): string {
  const linear = (offset: number) => {
    const channel = parseInt(tone.slice(offset, offset + 2), 16) / 255;
    return channel <= 0.04045
      ? channel / 12.92
      : ((channel + 0.055) / 1.055) ** 2.4;
  };
  const luminance =
    0.2126 * linear(1) + 0.7152 * linear(3) + 0.0722 * linear(5);
  // Relative luminance of the app's dark ink (#172033).
  const darkLuminance = 0.014541984592744823;
  const darkContrast = (luminance + 0.05) / (darkLuminance + 0.05);
  const whiteContrast = 1.05 / (luminance + 0.05);
  if (darkContrast < whiteContrast) return "#ffffff";
  return theme.guardian ? guardianInk : "#172033";
}
export const mapFontFamily =
  '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif';

/** Guardian: warm print inks, validated on white board #fafaf8 (all checks pass; worst
 * CVD pair 7.9, legal because tiles also carry labels and gaps). */
export const guardianUsageColors = [
  "#ba882e",
  "#008a7b",
  "#b75037",
  "#316ca5",
  "#bf6583",
  "#7b8935",
];
export const branchInks = () =>
  theme.guardian ? guardianUsageColors : usageColors;

export const usageColors = [
  "#3cb8a6",
  "#4195f3",
  "#8b7ef3",
  "#e2b23c",
  "#d77da2",
  "#56b7d0",
];

/** Derive every tile from its branch hue; nested lightening never accumulates. */
export function mapTileTones(color: string, nested: boolean) {
  const channels = [1, 3, 5].map((offset) =>
    parseInt(color.slice(offset, offset + 2), 16),
  );
  const mix = (amount: number) =>
    "#" +
    channels
      .map((channel) =>
        Math.round(
          amount >= 0
            ? channel + (255 - channel) * amount
            : channel * (1 + amount),
        )
          .toString(16)
          .padStart(2, "0"),
      )
      .join("");
  return nested
    ? { top: mix(0.1), middle: mix(0.12), bottom: mix(0.14) }
    : { top: mix(0.02), middle: color, bottom: mix(-0.008) };
}

export const usageColor = (id: number) => {
  const inks = branchInks();
  return inks[Math.max(0, id - 1) % inks.length];
};

import { expect, it } from "vitest";
import {
  formatScanTimestamp,
  mapLabelColor,
  mapTileTones,
  scanTime,
  usageColors,
} from "./uiFormat";

const luminance = (color: string) => {
  const channels = color.slice(1).match(/../g)!.map((hex) => {
    const value = parseInt(hex, 16) / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
};
const contrast = (first: string, second: string) => {
  const values = [luminance(first), luminance(second)].sort((a, b) => b - a);
  return (values[0] + 0.05) / (values[1] + 0.05);
};

it("chooses dark ink for light tiles and white for dark tiles", () => {
  expect(mapLabelColor("#e2b23c")).toBe("#172033");
  expect(mapLabelColor("#3cb8a6")).toBe("#172033");
  expect(mapLabelColor("#172033")).toBe("#ffffff");
  expect(mapLabelColor("#ffffff")).toBe("#172033");
});

it("responds to the visible nested tone instead of the original branch color", () => {
  expect(mapLabelColor("#767676")).toBe("#ffffff");
  expect(mapLabelColor(mapTileTones("#767676", true).top)).toBe("#172033");
});

it("selects the stronger contrast for every current branch and derived gradient tone", () => {
  for (const color of usageColors) {
    for (const nested of [false, true]) {
      for (const tone of Object.values(mapTileTones(color, nested))) {
        const ink = mapLabelColor(tone);
        const alternative = ink === "#ffffff" ? "#172033" : "#ffffff";
        expect(contrast(ink, tone)).toBeGreaterThanOrEqual(contrast(alternative, tone));
      }
    }
  }
});

it("labels a scan time by local calendar day, with h23 midnight", () => {
  const now = new Date(2026, 0, 1, 0, 1);
  const label = (date: Date) => formatScanTimestamp(date.getTime(), now).label;
  const clock = (date: Date) =>
    date.toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hourCycle: "h23",
    });
  const today = new Date(2026, 0, 1, 0, 0);
  const yesterday = new Date(2025, 11, 31, 0, 0);
  const older = new Date(2025, 11, 30, 23, 59);
  const thisYear = new Date(2026, 0, 2);
  expect(label(today)).toBe(`Today, ${clock(today)}`);
  expect(label(yesterday)).toBe(`Yesterday, ${clock(yesterday)}`);
  expect(label(older)).toBe(
    `${older.toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" })}, ${clock(older)}`,
  );
  expect(label(thisYear)).toBe(
    `${thisYear.toLocaleDateString(undefined, { month: "short", day: "numeric" })}, ${clock(thisYear)}`,
  );
});

it.each([NaN, Infinity, -Infinity, 8.64e15 + 1])(
  "handles the invalid scan time %s without throwing",
  (ms) => {
    expect(formatScanTimestamp(ms)).toEqual({
      label: "Date unavailable",
      absolute: "Date unavailable",
    });
  },
);

it("rounds a scan time to tenths before choosing its unit", () => {
  expect(scanTime(59940)).toBe("59.9 seconds");
  expect(scanTime(59960)).toBe("1m 0s");
});

import { expect, it } from "vitest";
import { mapLabelColor, mapTileTones, usageColors } from "./uiFormat";

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

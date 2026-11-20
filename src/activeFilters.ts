import type { ExplorerFilter, FilterCategory } from "./filterTypes";

export const categoryLabels: Record<FilterCategory, string> = {
  all: "All entries",
  images: "Images",
  videos: "Videos",
  audio: "Audio",
  documents: "Documents",
  archives: "Archives",
  applications: "Applications",
  other: "Other files",
};

const units = [
  { name: "TiB", scale: 1024n ** 4n },
  { name: "GiB", scale: 1024n ** 3n },
  { name: "MiB", scale: 1024n ** 2n },
  { name: "KiB", scale: 1024n },
  { name: "B", scale: 1n },
];

// Only use a compact unit when it expresses the inclusive boundary exactly.
// BigInt avoids rounding a custom byte boundary near Number.MAX_SAFE_INTEGER.
function exactValue(bytes: number, scale: bigint): string | null {
  const value = BigInt(bytes);
  const remainder = value % scale;
  if ((remainder * 1000n) % scale !== 0n) return null;
  const fraction = ((remainder * 1000n) / scale)
    .toString()
    .padStart(3, "0")
    .replace(/0+$/, "");
  return `${value / scale}${fraction ? `.${fraction}` : ""}`;
}

function exactSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  for (const unit of units.slice(0, -1)) {
    if (bytes < Number(unit.scale)) continue;
    const value = exactValue(bytes, unit.scale);
    if (value !== null) return `${value} ${unit.name}`;
  }
  // A typed decimal (1.1 GiB) is never exact in bytes; round it. A whole rounded value would pose as exact.
  const fit = units.find((unit) => bytes >= Number(unit.scale))!;
  const rounded = +(bytes / Number(fit.scale)).toFixed(2);
  return Number.isInteger(rounded) ? `${bytes} B` : `≈${rounded} ${fit.name}`;
}

export function sizeFilterLabel(
  filter: ExplorerFilter,
): string | null {
  const { minBytes: min, maxBytes: max } = filter;
  const prefix = "On disk";
  if (min === null && max === null) return null;
  if (max === null) return `${prefix} ≥ ${exactSize(min!)}`;
  if (min === null) return `${prefix} ≤ ${exactSize(max)}`;
  for (const unit of units) {
    if (unit.scale > 1n && Math.max(min, max) < Number(unit.scale)) continue;
    // Bytes always fit exactly; past 1 KiB they read worse than a rounded unit.
    if (unit.scale === 1n && Math.max(min, max) >= 1024) break;
    const lower = exactValue(min, unit.scale);
    const upper = exactValue(max, unit.scale);
    if (lower !== null && upper !== null)
      return `${prefix} ${lower}–${upper} ${unit.name}`;
  }
  const lower = exactSize(min);
  const upper = exactSize(max);
  // One byte unit for two byte ends; plain bytes too when both round alike, as "≈1 KiB–≈1 KiB" would mislead.
  return lower === upper || (lower.endsWith(" B") && upper.endsWith(" B"))
    ? `${prefix} ${min}–${max} B`
    : `${prefix} ${lower}–${upper}`;
}

export function sizeFilterDescription(
  filter: ExplorerFilter,
): string {
  const prefix = "On disk";
  const { minBytes: min, maxBytes: max } = filter;
  if (min !== null && max !== null)
    return `${prefix}: ${min}–${max} bytes, inclusive`;
  if (min !== null) return `${prefix}: at least ${min} bytes, inclusive`;
  if (max !== null) return `${prefix}: at most ${max} bytes, inclusive`;
  return "";
}

import type { ExplorerFilter, FilterCategory } from "./filterTypes";
import type { Metric } from "./types";

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
  for (const unit of units) {
    if (bytes < Number(unit.scale)) continue;
    const value = exactValue(bytes, unit.scale);
    if (value !== null) return `${value} ${unit.name}`;
  }
  return `${bytes} B`;
}

export function sizeFilterLabel(
  filter: ExplorerFilter,
  metric: Metric,
): string | null {
  const { minBytes: min, maxBytes: max } = filter;
  const prefix = metric === "allocated" ? "On disk" : "Logical size";
  if (min === null && max === null) return null;
  if (max === null) return `${prefix} ≥ ${exactSize(min!)}`;
  if (min === null) return `${prefix} ≤ ${exactSize(max)}`;
  for (const unit of units) {
    if (unit.scale > 1n && Math.max(min, max) < Number(unit.scale)) continue;
    const lower = exactValue(min, unit.scale);
    const upper = exactValue(max, unit.scale);
    if (lower !== null && upper !== null)
      return `${prefix} ${lower}–${upper} ${unit.name}`;
  }
  return `${prefix} ${exactSize(min)}–${exactSize(max)}`;
}

export function sizeFilterDescription(
  filter: ExplorerFilter,
  metric: Metric,
): string {
  const prefix = metric === "allocated" ? "On disk" : "Logical size";
  const { minBytes: min, maxBytes: max } = filter;
  if (min !== null && max !== null)
    return `${prefix}: ${min}–${max} bytes, inclusive`;
  if (min !== null) return `${prefix}: at least ${min} bytes, inclusive`;
  if (max !== null) return `${prefix}: at most ${max} bytes, inclusive`;
  return "";
}

import type { QueueEntry } from "./cleanupTypes";

/** How a queued item stands against the scan on screen. */
type BandKind = "ready" | "elsewhere" | "needsScan" | "attention";

interface BandSegment {
  kind: BandKind;
  label: string;
  bytes: number;
  /** The label is shown only when the segment is wide enough to hold it. */
  showLabel: boolean;
}

const labels: Record<BandKind, string> = {
  ready: "Ready to review",
  elsewhere: "Ready · open its scan",
  needsScan: "Needs scan",
  attention: "Needs attention",
};
// The accessible reading of each state, as a clause after "<size> ready to review".
const spoken: Record<BandKind, string> = {
  ready: "ready to review",
  elsewhere: "ready once its scan is open",
  needsScan: "needs a scan",
  attention: "changed, missing or blocked",
};
const order: BandKind[] = ["ready", "elsewhere", "needsScan", "attention"];
// Share of the page width a segment needs before its text fits ("Ready · open its scan 11.3 GiB").
const LABEL_MIN_SHARE = 0.22;

function bandKind(entry: QueueEntry, reviewable: boolean): BandKind {
  if (reviewable) return "ready";
  if (entry.status === "ready") return "elsewhere";
  return entry.status === "needsScan" ? "needsScan" : "attention";
}

/**
 * Splits one disk's entries into band segments, in reading order. `bandShare` is the band's width as a
 * fraction of the page, so a short band on a small disk hides labels that would be cut off.
 */
export function bandSegments(
  entries: QueueEntry[],
  isReviewable: (entry: QueueEntry) => boolean,
  bandShare: number,
  format: (bytes: number) => string,
): BandSegment[] {
  const total = entries.reduce((sum, entry) => sum + entry.allocatedBytes, 0);
  const byKind = new Map<BandKind, number>();
  for (const entry of entries) {
    const kind = bandKind(entry, isReviewable(entry));
    byKind.set(kind, (byKind.get(kind) ?? 0) + entry.allocatedBytes);
  }
  return order
    .filter((kind) => (byKind.get(kind) ?? 0) > 0)
    .map((kind) => {
      const bytes = byKind.get(kind)!;
      return {
        kind,
        bytes,
        label: `${labels[kind]} ${format(bytes)}`,
        showLabel: (bandShare * bytes) / total >= LABEL_MIN_SHARE,
      };
    });
}

/** The band's text equivalent, e.g. "12.5 GiB: 11.3 GiB ready to review, 1.2 GiB needs a scan". */
export function bandSummary(
  segments: BandSegment[],
  format: (bytes: number) => string,
): string {
  const total = segments.reduce((sum, segment) => sum + segment.bytes, 0);
  return `${format(total)}: ${segments
    .map((segment) => `${format(segment.bytes)} ${spoken[segment.kind]}`)
    .join(", ")}`;
}

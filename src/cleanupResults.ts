import type { CleanupReport } from "./cleanupTypes";
import type { NodeView } from "./types";

/** Attach native review indices only after validating the entire outcome set. */
export function cleanupResult(
  id: number,
  source: CleanupReport["source"],
  items: NodeView[],
  value: unknown,
): CleanupReport {
  const report: CleanupReport = {
    id,
    source,
    state: "unconfirmed",
    reviewedCount: items.length,
    movedCount: null,
    failedCount: null,
    movedItems: [],
    failedItems: [],
    failures: [],
    detailsAvailable: false,
    message:
      "Couldn't tell if the cleanup worked. Check the items in Finder before you try again.",
    persistenceWarning: null,
    invalidated: false,
    queueState: "checking",
    remainingQueued: null,
    queueWarning: null,
    queueError: null,
  };
  if (!value || typeof value !== "object") return report;
  const result = value as Record<string, unknown>;
  if (
    Array.isArray(result.failures) &&
    result.failures.every((s) => typeof s === "string")
  )
    report.failures = [...result.failures];
  if (typeof result.persistenceWarning === "string")
    report.persistenceWarning = result.persistenceWarning;
  if (typeof result.invalidated === "boolean")
    report.invalidated = result.invalidated;
  if (
    !Number.isSafeInteger(result.trashed) ||
    (result.trashed as number) < 0 ||
    (result.trashed as number) > items.length ||
    !Array.isArray(result.failures) ||
    !result.failures.every((s) => typeof s === "string") ||
    (result.trashed as number) + result.failures.length !== items.length
  )
    return report;
  report.state = "confirmed";
  report.movedCount = result.trashed as number;
  report.failedCount = result.failures.length;
  report.message = null;
  if (
    !Array.isArray(result.outcomes) ||
    result.outcomes.length !== items.length
  )
    return report;
  const indices = new Set<number>();
  const moved: NodeView[] = [],
    failed: CleanupReport["failedItems"] = [];
  for (const outcome of result.outcomes) {
    if (!outcome || typeof outcome !== "object") return report;
    const { reviewIndex, status, message } = outcome as Record<string, unknown>;
    if (
      !Number.isSafeInteger(reviewIndex) ||
      (reviewIndex as number) < 0 ||
      (reviewIndex as number) >= items.length ||
      indices.has(reviewIndex as number) ||
      (status !== "trashed" && status !== "failed") ||
      (message !== undefined && message !== null && typeof message !== "string")
    )
      return report;
    indices.add(reviewIndex as number);
    const item = items[reviewIndex as number];
    if (status === "trashed") moved.push(item);
    else
      failed.push({
        item,
        message:
          typeof message === "string" && message
            ? message
            : "Could not move this item to Trash.",
      });
  }
  if (
    moved.length !== report.movedCount ||
    failed.length !== report.failedCount
  )
    return report;
  report.detailsAvailable = true;
  // Keep the review order, independent of response ordering.
  const movedSet = new Set(moved);
  const failedByItem = new Map(failed.map((entry) => [entry.item, entry]));
  report.movedItems = items.filter((item) => movedSet.has(item));
  report.failedItems = items.flatMap((item) => {
    const failure = failedByItem.get(item);
    return failure ? [failure] : [];
  });
  return report;
}

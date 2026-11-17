import type { SavedEntry } from "./types";

interface SavedScanGroup {
  rootPath: string;
  entries: SavedEntry[];
  duplicateSeconds: ReadonlySet<number>;
  duplicateCompletions: ReadonlySet<number>;
}

function validTimestamp(ms: number): boolean {
  return Number.isFinite(ms) && !Number.isNaN(new Date(ms).getTime());
}

function compareTimestamps(a: number, b: number): number {
  const aValid = validTimestamp(a);
  const bValid = validTimestamp(b);
  if (aValid !== bValid) return aValid ? -1 : 1;
  return aValid ? b - a : 0;
}

function compareText(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

/** Group display paths exactly; a path is not a persistent volume identity. */
export function groupSavedScans(
  entries: readonly SavedEntry[],
): SavedScanGroup[] {
  const byPath = new Map<string, SavedEntry[]>();
  for (const entry of entries) {
    const group = byPath.get(entry.rootPath);
    if (group) group.push(entry);
    else byPath.set(entry.rootPath, [entry]);
  }
  const groups = Array.from(byPath, ([rootPath, entries]) => {
    entries.sort(
      (a, b) =>
        compareTimestamps(a.completedAtMs, b.completedAtMs) ||
        compareTimestamps(a.savedAtMs, b.savedAtMs) ||
        compareText(b.id, a.id),
    );
    const seconds = new Set<number>();
    const completions = new Set<number>();
    const duplicateSeconds = new Set<number>();
    const duplicateCompletions = new Set<number>();
    for (const entry of entries) {
      const ms = entry.completedAtMs;
      if (!validTimestamp(ms)) continue;
      const second = Math.floor(ms / 1000);
      if (seconds.has(second)) duplicateSeconds.add(second);
      if (completions.has(ms)) duplicateCompletions.add(ms);
      seconds.add(second);
      completions.add(ms);
    }
    return { rootPath, entries, duplicateSeconds, duplicateCompletions };
  });
  return groups.sort(
    (a, b) =>
      compareTimestamps(
        a.entries[0].completedAtMs,
        b.entries[0].completedAtMs,
      ) || compareText(a.rootPath, b.rootPath),
  );
}

interface ScanTimestamp {
  day: string;
  time: string;
  label: string;
  absolute: string;
  iso: string | undefined;
}

export function formatScanTimestamp(
  ms: number,
  now = new Date(),
  milliseconds = false,
): ScanTimestamp {
  if (!validTimestamp(ms)) {
    return {
      day: "Date unavailable",
      time: "",
      label: "Date unavailable",
      absolute: "Date unavailable",
      iso: undefined,
    };
  }
  const date = new Date(ms);
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  const day =
    date.toDateString() === now.toDateString()
      ? "Today"
      : date.toDateString() === yesterday.toDateString()
        ? "Yesterday"
        : date.toLocaleDateString(undefined, {
            month: "short",
            day: "numeric",
            ...(date.getFullYear() !== now.getFullYear()
              ? { year: "numeric" as const }
              : {}),
          });
  const clockOptions: Intl.DateTimeFormatOptions = {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
  };
  const time = date.toLocaleTimeString(undefined, {
    ...clockOptions,
    ...(milliseconds ? { fractionalSecondDigits: 3 as const } : {}),
  });
  return {
    day,
    time,
    label: `${day}, ${time}`,
    absolute: date.toLocaleString(undefined, {
      ...clockOptions,
      year: "numeric",
      month: "short",
      day: "numeric",
      fractionalSecondDigits: 3,
      timeZoneName: "longOffset",
    }),
    iso: date.toISOString(),
  };
}

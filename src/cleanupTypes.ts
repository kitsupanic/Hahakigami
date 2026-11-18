import type { NodeView } from "./types";

export interface QueueEntry {
  id: string;
  name: string;
  path: string;
  pathKey?: string;
  rootPath: string;
  allocatedBytes: number;
  status: "ready" | "needsScan" | "missing" | "changed" | "blocked";
  reason: string | null;
  /** The live scan this entry is checked against; review is addressed to it. Null when none can vouch for it. */
  matchedScanId?: string | null;
  lastError?: string | null;
}
export interface QueueView {
  entries: QueueEntry[];
  revision: number;
  warning: string | null;
  blocked: boolean;
}
export interface CleanupReport {
  id: number;
  source: "queue" | "selection";
  state: "confirmed" | "unconfirmed";
  reviewedCount: number;
  movedCount: number | null;
  failedCount: number | null;
  movedItems: NodeView[];
  failedItems: { item: NodeView; message: string }[];
  failures: string[];
  detailsAvailable: boolean;
  message: string | null;
  persistenceWarning: string | null;
  queueState: "checking" | "confirmed" | "unconfirmed";
  remainingQueued: number | null;
  queueWarning: string | null;
  queueError: string | null;
}

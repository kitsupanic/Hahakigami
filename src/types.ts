export type FullDiskAccessStatus = "on" | "off" | "unknown";
export interface FullDiskAccess {
  status: FullDiskAccessStatus;
  scanAnyway: boolean;
  appPath: string;
}
type ScanQos = "automatic" | "userInitiated" | "background";
export const qosLabel = (qos: ScanQos) =>
  ({
    automatic: "Automatic",
    userInitiated: "Prioritize scan",
    background: "Background",
  })[qos];
type WorkerMode = "auto" | "manual";
export interface WorkerStats {
  id: number;
  entries: number;
  directories: number;
  errors: number;
  elapsedUs: number;
  busyUs: number;
  cpuUs: number | null;
  queueWaitUs: number;
  storeWaitUs: number;
  metadataUs: number;
  commitUs: number;
  progressUs: number;
  qos: string;
  qosError?: number | null;
  relativeOpens?: number;
  absoluteReopens?: number;
}
export interface Target {
  id: string;
  name: string;
  path: string;
  totalBytes: number | null;
  freeBytes: number | null;
  home?: boolean;
  volume?: boolean;
  /** On a local disk; false for a network share, which "Scan all disks" skips. */
  local?: boolean;
}
export interface NodeView {
  id: number;
  parentId: number | null;
  name: string;
  path: string;
  kind: "directory" | "file" | "symlink" | "other" | "aggregate";
  pathKey?: string;
  allocatedBytes: number;
  rawAllocatedBytes?: number;
  verifiedClone?: boolean;
  cloneDeductionBytes?: number;
  cloneOwner?: boolean | null;
  childrenCount: number;
  incomplete: boolean;
  excluded?: boolean;
  /** Exact extension for a map-only group of scanned files; never an action target. */
  fileType?: string;
  fileCount?: number;
}
export interface ScanSummary {
  accountingModel?: "hardLinksV1" | "fullClonesV1";
  accountingWarnings?: string[];
  scanMode?: "full" | "incremental";
  scanPhase?: "checking" | "scanning" | "verifying" | "caching" | "done";
  walkMs?: number;
  reusedEntries?: number;
  rescannedDirectories?: number;
  fallbackReason?: string | null;
  cacheWarning?: string | null;
  startedAtMs?: number;
  completedAtMs?: number | null;
  workerMode?: WorkerMode;
  workerSource?: "manual" | "hardware" | "conservative" | "calibrated";
  workers: number;
  qos?: ScanQos;
  workerStats: WorkerStats[];
  finalizationUs: number;
  descriptorLimit?: number | null;
  descriptorPeak?: number | null;
  scanId: string;
  rootId: number;
  state: "running" | "completed" | "cancelled" | "failed";
  entries: number;
  allocatedBytes: number;
  elapsedMs: number;
  errors: number;
  excluded?: number;
  restrictedSystemIssues?: number;
  incomplete?: boolean;
  errorsStored?: number;
  exclusionsStored?: number;
  message?: string;
}
export type ScanIssueKind = "errors" | "restricted" | "other" | "excluded";
export interface ScanIssuePage {
  scanId: string;
  nodeId: number;
  kind: ScanIssueKind;
  total: number;
  retainedTotal: number;
  offset: number;
  items: {
    path: string;
    message: string;
    category?: "restrictedSystem" | "other" | "excluded";
    errorKind?: string | null;
    rawOsError?: number | null;
  }[];
}
export interface Page {
  items: NodeView[];
  total: number;
}
export function bytes(n: number, minimumFractionDigits: 0 | 1 = 0) {
  if (!n) return "0 B";
  const i = Math.min(4, Math.floor(Math.log(n) / Math.log(1024)));
  if (!i)
    return `${n.toLocaleString(undefined, { maximumFractionDigits: 0 })} B`;
  const decimalUnit = Math.min(4, Math.floor(Math.log(n) / Math.log(1000)));
  const binary = (n / 1024 ** i).toLocaleString(undefined, {
    minimumFractionDigits,
    maximumFractionDigits: 1,
  });
  const decimal = (n / 1000 ** decimalUnit).toLocaleString(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
  return `${binary} ${["B", "KiB", "MiB", "GiB", "TiB"][i]} (${decimal} ${["B", "KB", "MB", "GB", "TB"][decimalUnit]})`;
}

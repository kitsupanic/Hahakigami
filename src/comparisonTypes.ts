export interface ComparisonTotals {
  entries: number;
  logicalBytes: number;
  allocatedBytes: number;
  incomplete: boolean;
  errors: number;
  excluded: number;
}
export interface ComparisonRow {
  path: string;
  kind: "directory" | "file" | "symlink" | "other";
  status: "added" | "removed" | "changed";
  beforeLogicalBytes: number;
  afterLogicalBytes: number;
  beforeAllocatedBytes: number;
  afterAllocatedBytes: number;
  logicalDelta: number;
  allocatedDelta: number;
}
export interface ComparisonRanking {
  metric: "logical" | "allocated";
  kind: "files" | "folders";
  direction: "growth" | "shrinkage" | "added" | "removed" | "changed";
  total: number;
  items: ComparisonRow[];
}
export interface Comparison {
  beforeId: string;
  afterId: string;
  rootPath: string;
  before: ComparisonTotals;
  after: ComparisonTotals;
  logicalDelta: number;
  allocatedDelta: number;
  added: number;
  removed: number;
  changed: number;
  unchanged: number;
  warnings: string[];
  rankings: ComparisonRanking[];
}

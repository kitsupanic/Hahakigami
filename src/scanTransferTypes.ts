import type { SavedEntry } from "./types";

export interface ScanTransferReport {
  id: number;
  operation: "import" | "export";
  state: "pending" | "failed";
  phase: "choosing" | "processing" | "finishing";
  bytes: number;
  entry: SavedEntry | null;
  error: string | null;
}

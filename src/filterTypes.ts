export type FilterCategory =
  | "all"
  | "images"
  | "videos"
  | "audio"
  | "documents"
  | "archives"
  | "applications"
  | "other";
export interface ExplorerFilter {
  category: FilterCategory;
  minBytes: number | null;
  maxBytes: number | null;
  extension?: string | null;
  scopeId?: number | null;
}
export const emptyFilter: ExplorerFilter = {
  category: "all",
  minBytes: null,
  maxBytes: null,
};
export const filterActive = (filter: ExplorerFilter) =>
  filter.extension != null ||
  filter.category !== "all" ||
  filter.minBytes !== null ||
  filter.maxBytes !== null;

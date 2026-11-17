export const sortLabels = {
  sizeDescending: "Largest first",
  sizeAscending: "Smallest first",
  nameAscending: "Name A–Z",
  nameDescending: "Name Z–A",
} as const;
export type SortOrder = keyof typeof sortLabels;
export interface ExplorerPreferences {
  sort: SortOrder;
  warning: string | null;
}
export const isSortOrder = (value: unknown): value is SortOrder =>
  typeof value === "string" && Object.hasOwn(sortLabels, value);

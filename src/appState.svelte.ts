import { emptyFilter, type ExplorerFilter } from "./filterTypes";
import type { SortOrder } from "./sortTypes";
import type {
  Target,
  ScanSummary,
  NodeView,
  Page,
} from "./types";
export type Setter<T> = (value: T | ((previous: T) => T)) => void;
/** A raw reactive value with a setter that takes a value or an updater. */
export function rawState<T>(initial: T) {
  let value = $state.raw(initial);
  return {
    get value() {
      return value;
    },
    set: ((next) => {
      value = typeof next === "function" ? (next as (p: T) => T)(value) : next;
    }) as Setter<T>,
  };
}
export function createSessionState() {
  const targets = rawState<Target[]>([]);
  const target = rawState<Target | null>(null);
  const summary = rawState<ScanSummary | null>(null);
  const cancelling = rawState(false);
  return {
    get targets() {
      return targets.value;
    },
    setTargets: targets.set,
    get target() {
      return target.value;
    },
    setTarget: target.set,
    get summary() {
      return summary.value;
    },
    setSummary: summary.set,
    get cancelling() {
      return cancelling.value;
    },
    setCancelling: cancelling.set,
  };
}
export function createExplorerState() {
  const focus = rawState<NodeView | null>(null);
  const crumbs = rawState<NodeView[]>([]);
  const page = rawState<Page>({ items: [], total: 0 });
  const map = rawState<NodeView[]>([]);
  const selection = rawState<NodeView[]>([]);
  const mode = rawState<"treemap" | "sunburst">("sunburst");
  const query = rawState("");
  const offset = rawState(0);
  const filters = rawState<ExplorerFilter>({ ...emptyFilter });
  const debouncedQuery = rawState("");
  const listLoading = rawState(false);
  const sort = rawState<SortOrder>("sizeDescending");
  const sortReady = rawState(false);
  const sortSaving = rawState(false);
  const sortWarning = rawState<string | null>(null);
  return {
    get focus() {
      return focus.value;
    },
    setFocus: focus.set,
    get crumbs() {
      return crumbs.value;
    },
    setCrumbs: crumbs.set,
    get page() {
      return page.value;
    },
    setPage: page.set,
    get map() {
      return map.value;
    },
    setMap: map.set,
    get selection() {
      return selection.value;
    },
    setSelection: selection.set,
    get mode() {
      return mode.value;
    },
    setMode: mode.set,
    get query() {
      return query.value;
    },
    setQuery: query.set,
    get offset() {
      return offset.value;
    },
    setOffset: offset.set,
    get filters() {
      return filters.value;
    },
    setFilters: filters.set,
    get debouncedQuery() {
      return debouncedQuery.value;
    },
    setDebouncedQuery: debouncedQuery.set,
    get listLoading() {
      return listLoading.value;
    },
    setListLoading: listLoading.set,
    get sort() {
      return sort.value;
    },
    setSort: sort.set,
    get sortReady() {
      return sortReady.value;
    },
    setSortReady: sortReady.set,
    get sortSaving() {
      return sortSaving.value;
    },
    setSortSaving: sortSaving.set,
    get sortWarning() {
      return sortWarning.value;
    },
    setSortWarning: sortWarning.set,
  };
}

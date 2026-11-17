import { emptyFilter, type ExplorerFilter } from "./filterTypes";
import type { SortOrder } from "./sortTypes";
import type {
  Target,
  ScanSummary,
  NodeView,
  Page,
  Metric,
} from "./types";
export function createSessionState() {
  let targets = $state.raw<Target[]>([]);
  function setTargets(
    value: typeof targets | ((previous: typeof targets) => typeof targets),
  ) {
    targets = typeof value === "function" ? value(targets) : value;
  }
  let target = $state.raw<Target | null>(null);
  function setTarget(
    value: typeof target | ((previous: typeof target) => typeof target),
  ) {
    target = typeof value === "function" ? value(target) : value;
  }
  let summary = $state.raw<ScanSummary | null>(null);
  function setSummary(
    value: typeof summary | ((previous: typeof summary) => typeof summary),
  ) {
    summary = typeof value === "function" ? value(summary) : value;
  }
  let cancelling = $state.raw(false);
  function setCancelling(
    value:
      | typeof cancelling
      | ((previous: typeof cancelling) => typeof cancelling),
  ) {
    cancelling = typeof value === "function" ? value(cancelling) : value;
  }
  return {
    get targets() {
      return targets;
    },
    setTargets,
    get target() {
      return target;
    },
    setTarget,
    get summary() {
      return summary;
    },
    setSummary,
    get cancelling() {
      return cancelling;
    },
    setCancelling,
  };
}
export function createExplorerState() {
  let focus = $state.raw<NodeView | null>(null);
  function setFocus(
    value: typeof focus | ((previous: typeof focus) => typeof focus),
  ) {
    focus = typeof value === "function" ? value(focus) : value;
  }
  let crumbs = $state.raw<NodeView[]>([]);
  function setCrumbs(
    value: typeof crumbs | ((previous: typeof crumbs) => typeof crumbs),
  ) {
    crumbs = typeof value === "function" ? value(crumbs) : value;
  }
  let page = $state.raw<Page>({ items: [], total: 0 });
  function setPage(
    value: typeof page | ((previous: typeof page) => typeof page),
  ) {
    page = typeof value === "function" ? value(page) : value;
  }
  let map = $state.raw<NodeView[]>([]);
  function setMap(value: typeof map | ((previous: typeof map) => typeof map)) {
    map = typeof value === "function" ? value(map) : value;
  }
  let selection = $state.raw<NodeView[]>([]);
  function setSelection(
    value:
      | typeof selection
      | ((previous: typeof selection) => typeof selection),
  ) {
    selection = typeof value === "function" ? value(selection) : value;
  }
  let metric = $state.raw<Metric>("allocated");
  function setMetric(
    value: typeof metric | ((previous: typeof metric) => typeof metric),
  ) {
    metric = typeof value === "function" ? value(metric) : value;
  }
  let mode = $state.raw<"treemap" | "sunburst">("sunburst");
  function setMode(
    value: typeof mode | ((previous: typeof mode) => typeof mode),
  ) {
    mode = typeof value === "function" ? value(mode) : value;
  }
  let query = $state.raw("");
  function setQuery(
    value: typeof query | ((previous: typeof query) => typeof query),
  ) {
    query = typeof value === "function" ? value(query) : value;
  }
  let offset = $state.raw(0);
  function setOffset(
    value: typeof offset | ((previous: typeof offset) => typeof offset),
  ) {
    offset = typeof value === "function" ? value(offset) : value;
  }
  let filters = $state.raw<ExplorerFilter>({ ...emptyFilter });
  function setFilters(
    value: typeof filters | ((previous: typeof filters) => typeof filters),
  ) {
    filters = typeof value === "function" ? value(filters) : value;
  }
  let debouncedQuery = $state.raw("");
  function setDebouncedQuery(
    value:
      | typeof debouncedQuery
      | ((previous: typeof debouncedQuery) => typeof debouncedQuery),
  ) {
    debouncedQuery =
      typeof value === "function" ? value(debouncedQuery) : value;
  }
  let listLoading = $state.raw(false);
  function setListLoading(
    value:
      | typeof listLoading
      | ((previous: typeof listLoading) => typeof listLoading),
  ) {
    listLoading = typeof value === "function" ? value(listLoading) : value;
  }
  let sort = $state.raw<SortOrder>("sizeDescending");
  function setSort(
    value: typeof sort | ((previous: typeof sort) => typeof sort),
  ) {
    sort = typeof value === "function" ? value(sort) : value;
  }
  let sortReady = $state.raw(false);
  function setSortReady(
    value:
      | typeof sortReady
      | ((previous: typeof sortReady) => typeof sortReady),
  ) {
    sortReady = typeof value === "function" ? value(sortReady) : value;
  }
  let sortSaving = $state.raw(false);
  function setSortSaving(
    value:
      | typeof sortSaving
      | ((previous: typeof sortSaving) => typeof sortSaving),
  ) {
    sortSaving = typeof value === "function" ? value(sortSaving) : value;
  }
  let sortWarning = $state.raw<string | null>(null);
  function setSortWarning(
    value:
      | typeof sortWarning
      | ((previous: typeof sortWarning) => typeof sortWarning),
  ) {
    sortWarning = typeof value === "function" ? value(sortWarning) : value;
  }
  return {
    get focus() {
      return focus;
    },
    setFocus,
    get crumbs() {
      return crumbs;
    },
    setCrumbs,
    get page() {
      return page;
    },
    setPage,
    get map() {
      return map;
    },
    setMap,
    get selection() {
      return selection;
    },
    setSelection,
    get metric() {
      return metric;
    },
    setMetric,
    get mode() {
      return mode;
    },
    setMode,
    get query() {
      return query;
    },
    setQuery,
    get offset() {
      return offset;
    },
    setOffset,
    get filters() {
      return filters;
    },
    setFilters,
    get debouncedQuery() {
      return debouncedQuery;
    },
    setDebouncedQuery,
    get listLoading() {
      return listLoading;
    },
    setListLoading,
    get sort() {
      return sort;
    },
    setSort,
    get sortReady() {
      return sortReady;
    },
    setSortReady,
    get sortSaving() {
      return sortSaving;
    },
    setSortSaving,
    get sortWarning() {
      return sortWarning;
    },
    setSortWarning,
  };
}

import { size, type Metric, type NodeView } from "./types";

/** Read-only geometry evidence captured after the current completed map draw. */
export function canvasEvidence(
  nodes: NodeView[],
  rootId: number,
  metric: Metric,
) {
  const canvas = document.querySelector<HTMLCanvasElement>(".map-wrap canvas");
  const bounds = canvas?.getBoundingClientRect();
  return {
    expected_positive_nodes: nodes.reduce(
      (count, node) =>
        count + Number(node.id !== rootId && size(node, metric) > 0),
      0,
    ),
    backing_width: canvas?.width ?? 0,
    backing_height: canvas?.height ?? 0,
    css_width: bounds?.width ?? 0,
    css_height: bounds?.height ?? 0,
    device_pixel_ratio: window.devicePixelRatio || 1,
  };
}

/** Identical DOM workload for benchmark bundles. */
interface NativeInteractionReport {
  success: boolean;
  elapsedMs: number;
  phases: { name: string; elapsedMs: number; outcome: { verified: boolean; message: string; evidence: { before: NativeViewState; after: NativeViewState; observations: NativeViewState[] } } }[];
  error?: string;
}
interface NativeViewState {
  rows: { name: string; kind: string; size: string; level: number }[];
  breadcrumbs: string[];
  focus: string | null;
  query: string;
  category: string;
  sizeRange: string;
  sort: string;
  mode: string | null;
  mapReady: boolean;
  metric: string;
  selectedCount: number;
  scrollTop: number;
  scrollExtent: number;
}
function viewState(): NativeViewState {
  return {
    rows: Array.from(document.querySelectorAll<HTMLElement>('[role="treeitem"]')).map((row) => {
      const label = row.querySelector('span')?.cloneNode(true) as HTMLElement | undefined;
      label?.querySelectorAll('button,i,em,.disclosure-space,.queue-badge').forEach((node) => node.remove());
      return { level:Number(row.getAttribute('aria-level') ?? 1), name: label?.textContent?.trim() ?? '', kind: row.dataset.kind ?? (row.querySelector('i')?.textContent === '▱' ? 'directory' : 'file'), size: row.querySelector('.file-size')?.textContent?.trim() ?? row.children[1]?.textContent?.trim() ?? '' };
    }),
    breadcrumbs: Array.from(document.querySelectorAll('nav[aria-label="Folder breadcrumbs"] button')).map((node) => node.textContent?.trim() ?? ''),
    focus: document.querySelector<HTMLElement>('[data-native-focus]')?.dataset.nativeFocus ?? null,
    query: document.querySelector<HTMLInputElement>('input[aria-label="Search file names"]')?.value ?? '',
    category: document.querySelector<HTMLSelectElement>('select[aria-label="File category"]')?.value ?? '',
    sizeRange: document.querySelector<HTMLSelectElement>('select[aria-label="Size range"]')?.value ?? '',
    sort: document.querySelector<HTMLElement>('.table-head')?.dataset.sort ?? '',
    mode: document.querySelector<HTMLElement>('[data-native-map-mode]')?.dataset.nativeMapMode ?? null,
    scrollTop: document.querySelector<HTMLElement>('[role="tree"]')?.scrollTop ?? 0,
    scrollExtent: (() => { const tree=document.querySelector<HTMLElement>('[role="tree"]'); return tree ? tree.scrollHeight-tree.clientHeight : 0; })(),
    metric: Array.from(document.querySelectorAll('button.chosen')).some((node) => node.textContent?.trim()==='Logical') ? 'logical' : 'allocated',
    selectedCount: document.querySelectorAll('[role="treeitem"][aria-selected="true"]').length,
    mapReady: document.querySelector<HTMLElement>('[data-native-map-ready]')?.dataset.nativeMapReady === 'true',
  };
}
function rejectErrors() {
  const alert = Array.from(document.querySelectorAll<HTMLElement>('[role="alert"]')).find((node) => !node.closest('[hidden]') && node.textContent?.trim());
  const treeError = Array.from(document.querySelectorAll('button')).find((node) => node.textContent?.includes('Unable to load'));
  if (alert || treeError) throw new Error(`Explorer reported an error: ${(alert || treeError)?.textContent?.trim()}`);
}
function sameRows(before: NativeViewState, after: NativeViewState) {
  if (JSON.stringify(before.rows) !== JSON.stringify(after.rows)) throw new Error('Explorer rows did not restore');
}
function displayedBytes(text: string): number {
  const match = text.match(/^(.+) (B|KiB|MiB|GiB|TiB|KB|MB|GB|TB)$/);
  if (!match) throw new Error(`Unmeasurable displayed size: ${text}`);
  const parts = new Intl.NumberFormat().formatToParts(1234.5);
  let number = match[1].replaceAll(parts.find((part) => part.type === 'group')?.value ?? ',', '');
  number = number.replace(parts.find((part) => part.type === 'decimal')?.value ?? '.', '.');
  return Number(number) * 1024 ** ['B','KB','MB','GB','TB'].indexOf(match[2].replace('i', ''));
}
function naturalName(a: string, b: string): number {
  const aa = a.toLowerCase().match(/\d+|\D+/g) ?? [], bb = b.toLowerCase().match(/\d+|\D+/g) ?? [];
  const encoder = new TextEncoder();
  for (let index = 0; index < Math.min(aa.length,bb.length); index++) {
    const [left,right] = [aa[index],bb[index]];
    if (/^\d+$/.test(left) && /^\d+$/.test(right)) {
      const l=left.replace(/^0+/,''), r=right.replace(/^0+/,'');
      const result=l.length-r.length || (l<r?-1:l>r?1:0) || left.length-right.length;
      if (result) return result;
    } else {
      const l=encoder.encode(left),r=encoder.encode(right);
      for (let j=0;j<Math.min(l.length,r.length);j++) if(l[j]!==r[j]) return l[j]-r[j];
      if(l.length!==r.length) return l.length-r.length;
    }
  }
  return aa.length-bb.length;
}
const pause = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
async function ready() {
  await pause(350);
  const deadline = performance.now() + 10000;
  while (document.querySelector('[data-native-busy="true"], [data-native-map-ready="false"], .tree-message button:disabled') || Array.from(document.querySelectorAll('[role="status"]')).some((node) => node.textContent?.includes('Loading results'))) {
    if (performance.now() > deadline) throw new Error('Explorer query did not settle');
    await pause(50);
  }
  rejectErrors();
}
function required<T extends HTMLElement>(selector: string): T {
  const node = document.querySelector<T>(selector);
  if (!node) throw new Error(`Missing workload control: ${selector}`);
  return node;
}
function button(label: string): HTMLButtonElement {
  const found = Array.from(document.querySelectorAll('button')).find((node) => node.textContent?.trim().replace(/^[▦◉]\s*/, '') === label || node.getAttribute('aria-label') === label);
  if (!found || found.disabled) throw new Error(`Unavailable workload button: ${label}`);
  return found;
}
async function changeSelect(label: string, value: string) {
  const input = required<HTMLSelectElement>(`select[aria-label="${label}"]`);
  if (input.disabled || !Array.from(input.options).some((option) => option.value === value)) throw new Error(`Unavailable selection ${label}:${value}`);
  input.value = value;
  input.dispatchEvent(new Event('change', { bubbles: true }));
  await ready();
  if (required<HTMLSelectElement>(`select[aria-label="${label}"]`).value !== value) throw new Error(`${label} value did not commit`);
}
// Header buttons toggle: name -> asc/desc, size -> desc/asc; other orders need a second click.
async function changeSort(value: string) {
  const current = () => required<HTMLElement>('.table-head').dataset.sort;
  const label = value.startsWith('name') ? 'Sort by name' : 'Sort by size';
  for (let attempt = 0; attempt < 2 && current() !== value; attempt++) { button(label).click(); await ready(); }
  if (current() !== value) throw new Error(`Sort order did not commit: ${value}`);
}
async function search(value: string) {
  const input = required<HTMLInputElement>('input[aria-label="Search file names"]');
  // Native setter bypasses framework value tracking so the changed value is seen.
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, value);
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
  await ready();
  if (required<HTMLInputElement>('input[aria-label="Search file names"]').value !== value) throw new Error('Search did not commit');
}
export async function runNativeInteractions(): Promise<NativeInteractionReport> {
  const started = performance.now();
  const phases: NativeInteractionReport['phases'] = [];
  async function phase(name: string, task: (observe: () => void) => Promise<string>) {
    const began = performance.now();
    const before = viewState();
    rejectErrors();
    const observations: NativeViewState[]=[];
    const message = await task(() => observations.push(viewState()));
    rejectErrors();
    phases.push({ name, elapsedMs: 0, outcome: { verified: true, message, evidence: { before, after: viewState(), observations } } });
    phases[phases.length - 1].elapsedMs = performance.now() - began;
  }
  try {
    await ready();
    await phase('selection', async () => {
      for (let i = 0; i < 12; i++) {
        const rows = Array.from(document.querySelectorAll<HTMLElement>('[role="treeitem"]'));
        if (!rows.length) throw new Error('No selection rows');
        const row = rows[i % rows.length]; row.click(); await pause(20);
        if (row.getAttribute('aria-selected') !== 'true') throw new Error('Selection did not commit');
      }
      return '12 selected rows acknowledged';
    });
    await phase('scroll', async (observe) => {
      // Root's largest directory can have only a few immediate children (/Users).
      // Follow the first directory in the current size order, with a strict bound,
      // until this workload can exercise actual virtualized scrolling.
      let tree = required<HTMLElement>('[role="tree"]');
      for (let depth = 0; depth < 8; depth++) {
        const directory = required<HTMLElement>('[role="treeitem"][aria-expanded]');
        const previousFocus = viewState().focus;
        directory.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })); await ready();
        if (viewState().focus === previousFocus) throw new Error('Scroll directory navigation did not commit');
        tree = required<HTMLElement>('[role="tree"]');
        observe();
        if (tree.scrollHeight - tree.clientHeight > 0) break;
      }
      const original = tree.scrollTop;
      const extent = tree.scrollHeight - tree.clientHeight;
      if (extent <= 0) throw new Error('No virtualized scroll extent within 8 directories');
      for (let i = 0; i < 12; i++) { tree.scrollTop = i % 2 ? 0 : extent; tree.dispatchEvent(new Event('scroll')); await pause(20); if (Math.abs(tree.scrollTop-(i%2?0:extent))>1) throw new Error('Virtualized scroll did not move'); if(i===0 || i===1) observe(); }
      tree.scrollTop = original; await ready();
      required<HTMLElement>('nav[aria-label="Folder breadcrumbs"]').querySelector<HTMLButtonElement>('button')!.click(); await ready();
      return '12 scrolls over virtualized directory extent';
    });
    await phase('nested-expansion', async (observe) => {
      const expand = required<HTMLButtonElement>('button[aria-label^="Expand "]');
      const name = expand.getAttribute('aria-label')!.slice(7); expand.click(); await ready(); observe();
      if(!document.querySelector('[role="treeitem"][aria-level="2"]')) throw new Error('Expanded directory did not render descendants');
      button('Collapse ' + name).click(); await ready();
      if(document.querySelector('[role="treeitem"][aria-level="2"]')) throw new Error('Collapsed descendants remain rendered');
      return 'expanded then collapsed directory';
    });
    await phase('search', async (observe) => {
      const before = viewState(); await search('item-'); const results = viewState(); observe();
      if (results.rows.some((row) => !row.name.toLowerCase().includes('item-'))) throw new Error('Search returned a nonmatching filename');
      if (before.rows.some((row) => row.name === '.hahakigami-performance-fixture.json') && !results.rows.length) throw new Error('Owned fixture search lost its known matches');
      if (!results.rows.length && !document.querySelector('.no-files')?.textContent?.includes('No matching files')) throw new Error('Empty search lacked its completed empty state');
      await search(''); sameRows(before,viewState()); return 'matching filename results verified and original rows restored';
    });
    await phase('sort', async (observe) => {
      const before=viewState(); await changeSort('nameAscending'); const rows=viewState().rows; observe();
      for(let index=1;index<rows.length;index++) {
        const left=rows[index-1],right=rows[index];
        if(left.kind!=='directory' && right.kind==='directory' || left.kind===right.kind && naturalName(left.name,right.name)>0) throw new Error('Name sort did not reorder the returned rows');
      }
      await changeSort(before.sort); sameRows(before,viewState()); return 'natural ascending row order verified and original rows restored';
    });
    await phase('filters', async (observe) => {
      const before=viewState();
      if (!document.querySelector('select[aria-label="File category"]') || document.querySelector('select[aria-label="File category"]')?.closest('[hidden]')) { button('Filters').click(); await ready(); }
      await changeSelect('File category', 'other'); observe();
      if(viewState().rows.some((row) => row.kind==='directory')) throw new Error('Category filter retained a directory');
      await changeSelect('File category', 'all');
      await changeSelect('Size range', '100mb'); observe();
      if(viewState().rows.some((row) => displayedBytes(row.size)<99.9*1024**2)) throw new Error('Size filter retained a smaller entry');
      await changeSelect('Size range', 'any'); sameRows(before,viewState());
      const toggle = Array.from(document.querySelectorAll('button')).find((node) => node.getAttribute('aria-label') === 'Filters' || node.textContent?.trim() === 'Filters');
      if (toggle?.getAttribute('aria-expanded') === 'true') { toggle.click(); await ready(); }
      return 'category and size changed and reset';
    });
    await phase('navigation', async (observe) => {
      const before=viewState();
      const row = required<HTMLElement>('[role="treeitem"][aria-expanded]'); row.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })); await ready(); observe();
      if(viewState().focus===before.focus) throw new Error('Navigation did not change folder focus');
      const crumbs = required<HTMLElement>('nav[aria-label="Folder breadcrumbs"]'); const roots = crumbs.querySelectorAll<HTMLButtonElement>('button');
      if (roots.length < 2) throw new Error('Directory navigation did not update breadcrumbs'); roots[0].click(); await ready(); const after=viewState(); sameRows(before,after); if(JSON.stringify(before.breadcrumbs)!==JSON.stringify(after.breadcrumbs)) throw new Error('Breadcrumb root did not restore'); return 'directory navigation and exact root restoration verified';
    });
    await phase('size-metric', async (observe) => { button('Logical').click(); await ready(); observe(); if (!button('Logical').classList.contains('chosen')) throw new Error('Logical metric did not commit'); button('On disk').click(); await ready(); if (!button('On disk').classList.contains('chosen')) throw new Error('Allocated metric did not commit'); return 'logical and allocated metrics rendered'; });
    await phase('map-switch', async (observe) => { button('Sunburst').click(); await ready(); observe(); if (required<HTMLElement>('[data-native-map-mode]').dataset.nativeMapMode !== 'sunburst') throw new Error('Sunburst did not commit'); button('Treemap').click(); await ready(); if (required<HTMLElement>('[data-native-map-mode]').dataset.nativeMapMode !== 'treemap') throw new Error('Treemap did not commit'); return 'both map modes rendered'; });
    return { success: true, elapsedMs: performance.now() - started, phases };
  } catch (error) {
    return { success: false, elapsedMs: performance.now() - started, phases, error: String(error) };
  }
}

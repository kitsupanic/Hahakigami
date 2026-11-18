// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest';
import { canvasEvidence, runNativeInteractions } from './nativeBenchmark';
import type { NodeView } from './types';
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); document.body.replaceChildren(); });
it('rejects a native explorer error instead of accepting unchanged rows', async () => {
  vi.useFakeTimers();
  document.body.innerHTML='<div role="alert">Failed to load explorer results</div>';
  const result=runNativeInteractions();
  await vi.advanceTimersByTimeAsync(350);
  const report=await result;
  expect(report.success).toBe(false);
  expect(report.error).toContain('Failed to load explorer results');
  expect(report.phases).toEqual([]);
});

it.each([[false, false], [true, false], [false, true]])('completes every verified phase without RAF, including sparse directory descent=%s and four columns=%s', async (sparseDirectory, fourColumns) => {
  vi.useFakeTimers();
  const raf=vi.fn(() => 0); vi.stubGlobal('requestAnimationFrame',raf);
  document.body.innerHTML=`<main data-native-busy="false" data-native-map-ready="true" data-native-focus="root" data-native-map-mode="treemap">
    <input aria-label="Search file names"><div class="table-head" data-sort="sizeDescending"><button aria-label="Sort by name"></button><button aria-label="Sort by size"></button></div>
    <select aria-label="File category"><option value="all">All</option><option value="other">Other</option></select>
    <select aria-label="Size range"><option value="any">Any</option><option value="100mb">100 MB</option></select>
    <button>Treemap</button><button>Sunburst</button>
    <nav aria-label="Folder breadcrumbs"></nav><div role="tree"></div></main>`;
  const app=document.querySelector<HTMLElement>('main')!;
  const tree=document.querySelector<HTMLElement>('[role="tree"]')!;
  Object.defineProperties(tree,{scrollHeight:{get:()=>sparseDirectory && app.dataset.nativeFocus==='shallow'?200:1200},clientHeight:{value:200}});
  let expanded=false;
  const roots=[{name:'shallow',dir:true},{name:'item-10',dir:false},{name:'item-2',dir:false},{name:'.hahakigami-performance-fixture.json',dir:false}];
  const sizeCells = fourColumns
    ? '<span class="file-relative">bar noise</span><span class="file-size">1 KB</span><span class="file-contents">—</span>'
    : '<span>1 KB</span><span>—</span>';
  function render() {
    let rows=app.dataset.nativeFocus==='root'?roots:sparseDirectory && app.dataset.nativeFocus==='shallow'?[{name:'deep',dir:true}]:[{name:'item-1',dir:false},{name:'item-2',dir:false}];
    const query=document.querySelector<HTMLInputElement>('input')!.value;
    const category=document.querySelector<HTMLSelectElement>('[aria-label="File category"]')!.value;
    const range=document.querySelector<HTMLSelectElement>('[aria-label="Size range"]')!.value;
    rows=rows.filter(row=>row.name.includes(query) && (category==='all'||!row.dir) && range==='any');
    if(document.querySelector<HTMLElement>('.table-head')!.dataset.sort==='nameAscending') rows=[...rows].sort((a,b)=>Number(b.dir)-Number(a.dir)||a.name.localeCompare(b.name,undefined,{numeric:true}));
    tree.innerHTML=rows.map(row=>`<div role="treeitem" aria-level="1" aria-selected="false" ${row.dir?'aria-expanded="'+expanded+'"':''}><span>${row.dir?'<i>▱</i><button aria-label="'+(expanded?'Collapse':'Expand')+' shallow"></button>':''}${row.name}</span>${sizeCells}</div>`).join('')+(expanded?`<div role="treeitem" aria-level="2"><span>item-child</span>${sizeCells}</div>`:'');
    document.querySelector('nav')!.innerHTML='<button>Root</button>'+(app.dataset.nativeFocus==='root'?'':'<button>shallow</button>');
  }
  app.addEventListener('click',event=>{
    const target=event.target as HTMLElement;
    if(target.matches('nav button')) {app.dataset.nativeFocus='root';render();return;}
    if(target.matches('button[aria-label^="Expand"],button[aria-label^="Collapse"]')) {expanded=!expanded;render();return;}
    if(target.matches('[aria-label="Sort by name"]')||target.matches('[aria-label="Sort by size"]')) {
      const head=target.parentElement!,sort=head.dataset.sort;
      head.dataset.sort=target.getAttribute('aria-label')==='Sort by name'?(sort==='nameAscending'?'nameDescending':'nameAscending'):(sort==='sizeDescending'?'sizeAscending':'sizeDescending');
      render();return;
    }
    const row=target.closest('[role="treeitem"]');if(row) {row.setAttribute('aria-selected','true');return;}
    if(target.textContent==='Sunburst'||target.textContent==='Treemap')app.dataset.nativeMapMode=target.textContent.toLowerCase();
  });
  tree.addEventListener('dblclick',()=>{app.dataset.nativeFocus=app.dataset.nativeFocus==='root'?'shallow':'deep';expanded=false;render();});
  app.addEventListener('change',render);app.addEventListener('input',render);render();
  const pending=runNativeInteractions();await vi.runAllTimersAsync();const report=await pending;
  expect(report.error).toBeUndefined();expect(report.success).toBe(true);
  expect(report.phases.map(phase=>phase.name)).toEqual(['selection','scroll','nested-expansion','search','sort','filters','navigation','map-switch']);
  expect(report.phases.every(phase=>phase.outcome.verified)).toBe(true);expect(raf).not.toHaveBeenCalled();
  const scroll=report.phases.find(phase=>phase.name==='scroll')!;
  expect(scroll.outcome.evidence.after.focus).toBe('root');
  expect(report.phases[0].outcome.evidence.before.rows[0].size).toBe('1 KB');
  if(sparseDirectory) expect(scroll.outcome.evidence.observations.some(state=>state.focus==='deep' && state.scrollTop>0)).toBe(true);
});
it('still rejects a missing current map draw certificate within the query deadline', async () => {
  vi.useFakeTimers();document.body.innerHTML='<main data-native-map-ready="false"></main>';
  const pending=runNativeInteractions();await vi.runAllTimersAsync();const report=await pending;
  expect(report.success).toBe(false);expect(report.error).toContain('Explorer query did not settle');expect(report.phases).toEqual([]);
});

it('bounds directory descent and refuses a tree with no real scroll extent', async () => {
  vi.useFakeTimers();document.body.innerHTML='<main data-native-map-ready="true" data-native-focus="0"><div role="tree"><div role="treeitem" aria-expanded="false" aria-selected="false"><span>chain</span><span>1 KB</span></div></div></main>';
  const app=document.querySelector<HTMLElement>('main')!;const tree=document.querySelector<HTMLElement>('[role="tree"]')!;
  Object.defineProperties(tree,{scrollHeight:{value:200},clientHeight:{value:200}});
  let navigations=0;tree.addEventListener('dblclick',()=>{app.dataset.nativeFocus=String(++navigations);});
  tree.addEventListener('click',()=>tree.querySelector('[role="treeitem"]')!.setAttribute('aria-selected','true'));
  const pending=runNativeInteractions();await vi.runAllTimersAsync();const report=await pending;
  expect(report.success).toBe(false);expect(report.error).toContain('No virtualized scroll extent within 8 directories');
  expect(navigations).toBe(8);expect(report.phases.map(phase=>phase.name)).toEqual(['selection']);
});


it("observes positive canvas geometry without allocating a drawing context", () => {
  document.body.innerHTML =
    '<div class="map-wrap"><canvas width="1200" height="800"></canvas></div>';
  const canvas = document.querySelector("canvas")!;
  vi.spyOn(canvas, "getBoundingClientRect").mockReturnValue({
    width: 600,
    height: 400,
  } as DOMRect);
  const getContext = vi.spyOn(canvas, "getContext");
  vi.stubGlobal("devicePixelRatio", 2);
  const root: NodeView = {
    id: 0,
    parentId: null,
    name: "root",
    path: "/root",
    kind: "directory",
    allocatedBytes: 10,
    childrenCount: 2,
    incomplete: false,
  };
  const child: NodeView = {
    ...root,
    id: 1,
    parentId: 0,
    kind: "file",
    allocatedBytes: 10,
  };
  const zero: NodeView = {
    ...child,
    id: 2,
    allocatedBytes: 0,
  };
  expect(canvasEvidence([root, child, zero], 0)).toEqual({
    expected_positive_nodes: 1,
    backing_width: 1200,
    backing_height: 800,
    css_width: 600,
    css_height: 400,
    device_pixel_ratio: 2,
  });
  expect(getContext).not.toHaveBeenCalled();
});

it("reports empty and absent backing geometry as observed without repairing it", () => {
  document.body.innerHTML =
    '<div class="map-wrap"><canvas width="0" height="0"></canvas></div>';
  const canvas = document.querySelector("canvas")!;
  vi.spyOn(canvas, "getBoundingClientRect").mockReturnValue({
    width: 600,
    height: 400,
  } as DOMRect);
  const getContext = vi.spyOn(canvas, "getContext");
  vi.stubGlobal("devicePixelRatio", 1.5);
  expect(canvasEvidence([], 0)).toEqual({
    expected_positive_nodes: 0,
    backing_width: 0,
    backing_height: 0,
    css_width: 600,
    css_height: 400,
    device_pixel_ratio: 1.5,
  });
  const positive: NodeView = {
    id: 1,
    parentId: 0,
    name: "file",
    path: "/file",
    kind: "file",
    allocatedBytes: 10,
    childrenCount: 0,
    incomplete: false,
  };
  expect(canvasEvidence([positive], 0)).toMatchObject({
    expected_positive_nodes: 1,
    backing_width: 0,
    backing_height: 0,
  });
  expect([canvas.width, canvas.height]).toEqual([0, 0]);
  expect(getContext).not.toHaveBeenCalled();
  canvas.remove();
  expect(canvasEvidence([], 0)).toEqual({
    expected_positive_nodes: 0,
    backing_width: 0,
    backing_height: 0,
    css_width: 0,
    css_height: 0,
    device_pixel_ratio: 1.5,
  });
});

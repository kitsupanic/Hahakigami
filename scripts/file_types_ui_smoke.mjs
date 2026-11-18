// Isolated production browser verification; all native imports are mocked.
// Run: node scripts/file_types_ui_smoke.mjs [--case=light-900x600]
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { createHash } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { build, preview, loadConfigFromFile, mergeConfig } from "vite";
import { chromium } from "playwright-core";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const output = path.join(root, "benchmark-results/file-types-ui");
const selectedCase = process.argv
  .find((a) => a.startsWith("--case="))
  ?.slice(7);
const temporary = await mkdtemp(path.join(tmpdir(), "hahakigami-file-types-"));
const report = { checks: [], errors: [], sourceSha256: {} };
let server, browser;
await mkdir(output, { recursive: true });
for (const file of [
  "src/App.svelte",
  "src/FileTypes.svelte",
  "src/DiskMap.svelte",
  "src/createFileTypes.svelte.ts",
  "src/workspace.css",
  "scripts/file_types_ui_smoke.mjs",
]) {
  report.sourceSha256[file] = createHash("sha256")
    .update(await readFile(path.join(root, file)))
    .digest("hex");
}
try {
  const mockPath = path.join(temporary, "mock.ts");
  await writeFile(
    mockPath,
    `
export class Channel { onmessage = (_: any) => {}; }
export const isTauri = () => true;
export const listen = async () => () => {};
export const emit = async () => {};
export const writeText = async () => { throw new Error('Clipboard disabled'); };
const gib=1024**3;
const base={childrenCount:0,incomplete:false};
const dirs=[{...base,id:0,parentId:null,name:'File types fixture',path:'/fixture',kind:'directory'}, {...base,id:1,parentId:0,name:'Media',path:'/fixture/Media',kind:'directory'}, {...base,id:2,parentId:0,name:'Downloads',path:'/fixture/Downloads',kind:'directory'}, {...base,id:3,parentId:1,name:'Exports',path:'/fixture/Media/Exports',kind:'directory'}, {...base,id:4,parentId:3,name:'Transcoded',path:'/fixture/Media/Exports/Transcoded',kind:'directory'}, {...base,id:5,parentId:4,name:'High Quality Media',path:'/fixture/Media/Exports/Transcoded/High Quality Media',kind:'directory'}];
let id=10;
const files=[['feature.MOV',5,28],['interview.mov',1,12],['installer.dmg',2,17],['backup.zip',2,10],['render.mp4',5,8],['artwork.png',1,5],['portrait.jpg',1,4],['guide.pdf',2,3],['README',2,2],...Array.from({length:34},(_,i)=>['sample.type'+String(i).padStart(2,'0'),2,1/(i+2)])].map(([name,parentId,gb])=>({...base,id:id++,parentId,name,path:dirs.find(d=>d.id===parentId)!.path+'/'+name,kind:'file',allocatedBytes:Math.round(Number(gb)*gib),logicalBytes:Math.round(Number(gb)*gib*1.12)}));
const nodes=[...dirs,...files],byId=new Map(nodes.map(n=>[n.id,n]));
function within(n:any,scope:number){const seen=new Set();while(n&&!seen.has(n.id)){if(n.id===scope)return true;seen.add(n.id);n=byId.get(n.parentId)}return false}
for(const d of [...dirs].reverse()){const child=nodes.filter(n=>n.parentId===d.id);Object.assign(d,{childrenCount:child.length,allocatedBytes:child.reduce((a,n)=>a+(n.allocatedBytes||0),0),logicalBytes:child.reduce((a,n)=>a+(n.logicalBytes||0),0)})}
const extension=(n:any)=>{const dot=n.name.lastIndexOf('.');return dot>0?n.name.slice(dot+1).toLowerCase():''};
const groups=new Map();let groupId=0x80000000;
for(const f of files){const type=extension(f),key=f.parentId+':'+type;let group=groups.get(key);if(!group){group={...base,id:groupId++,parentId:f.parentId,name:(type?'.'+type:'No extension')+' files',path:'',kind:'aggregate',fileType:type,fileCount:0,logicalBytes:0,allocatedBytes:0};groups.set(key,group)}group.fileCount++;group.logicalBytes+=f.logicalBytes;group.allocatedBytes+=f.allocatedBytes}
const summary={scanId:'file-types-fixture',rootId:0,state:'completed',scanPhase:'done',scanMode:'full',workers:1,workerStats:[],entries:nodes.length,logicalBytes:dirs[0].logicalBytes,allocatedBytes:dirs[0].allocatedBytes,elapsedMs:1250,startedAtMs:1792400000000,completedAtMs:1792400001250,errors:0,excluded:0};
(window as any).__fileTypes={commands:[],nodes};
export async function invoke(command:string,args:any={}) {
 (window as any).__fileTypes.commands.push({command,...args});
 switch(command){
 case 'disk_access_setup':return {required:false,confirmed:true,pendingRestart:false,appPath:'/fixture/Hahakigami.app'};
 case 'full_disk_access_status':return {status:'on',scanAnyway:false,appPath:'/fixture/Hahakigami.app'};
 case 'start_scan':return summary.scanId;
 case 'scan_summary':return {...summary};
 case 'discover_targets':return [{id:'fixture',name:dirs[0].name,path:'/fixture',totalBytes:200*gib,freeBytes:50*gib}];
 case 'get_explorer_preferences':case 'set_explorer_preferences':return {sort:args.sort??'sizeDescending',warning:null};
 case 'list_cleanup_queue':return {entries:[],revision:0,warning:null,blocked:false};
 case 'get_node':return {...byId.get(args.nodeId)};
 case 'get_map':return {nodes:(args.colorMode==='fileType'?[...dirs.filter(n=>within(n,args.nodeId)),...[...groups.values()].filter(n=>within(byId.get(n.parentId),args.nodeId))]:dirs.filter(n=>within(n,args.nodeId))).map(n=>({...n}))};
 case 'get_children':{const items=nodes.filter(n=>n.parentId===args.nodeId).sort((a,b)=>b.allocatedBytes-a.allocatedBytes);return {items:items.slice(args.offset??0,(args.offset??0)+(args.limit??150)),total:items.length}};
 case 'get_file_types':{const items=new Map();for(const f of files.filter(n=>within(n,args.nodeId))){const ext=extension(f),row=items.get(ext)??{extension:ext,files:0,logicalBytes:0,allocatedBytes:0};row.files++;row.logicalBytes+=f.logicalBytes;row.allocatedBytes+=f.allocatedBytes;items.set(ext,row)}const rows=[...items.values()];return {items:rows,files:rows.reduce((a,r)=>a+r.files,0),logicalBytes:rows.reduce((a,r)=>a+r.logicalBytes,0),allocatedBytes:rows.reduce((a,r)=>a+r.allocatedBytes,0),incomplete:false}};
 case 'get_file_type_highlights':{const matches=files.filter(n=>within(n,args.nodeId)&&extension(n)===args.extension);return args.nodeIds.filter((id:number)=>{const n=byId.get(id);return n&&matches.some(f=>within(f,n.id))})};
 case 'filter_nodes':{const items=files.filter(n=>within(n,args.scopeId??0)&&(args.extension==null||extension(n)===args.extension));return {items,total:items.length}};
 case 'close_quick_look':return {visible:false,scanId:null,nodeId:null};
 default:throw new Error('Unexpected mocked command: '+command);
 }
}
`,
  );
  process.env.VITE_PERFORMANCE_BENCHMARK = "0";
  const loaded = await loadConfigFromFile(
    { command: "build", mode: "production" },
    path.join(root, "vite.config.ts"),
  );
  assert(loaded);
  const config = mergeConfig(loaded.config, {
    root,
    configFile: false,
    logLevel: "warn",
    resolve: {
      alias: Object.fromEntries(
        [
          "@tauri-apps/api/core",
          "@tauri-apps/api/event",
          "@tauri-apps/plugin-clipboard-manager",
        ].map((n) => [n, mockPath]),
      ),
    },
    build: { outDir: path.join(temporary, "dist"), emptyOutDir: true },
    preview: { host: "127.0.0.1", port: 0, strictPort: false },
  });
  await build(config);
  server = await preview(config);
  browser = await chromium.launch({
    headless: true,
    ...(process.env.BENCHMARK_BROWSER_PATH
      ? { executablePath: process.env.BENCHMARK_BROWSER_PATH }
      : { channel: "chrome" }),
  });
  report.browserVersion = browser.version();
  for (const colorScheme of ["light", "dark"])
    for (const viewport of [
      { width: 1536, height: 1024 },
      { width: 1280, height: 820 },
      { width: 900, height: 600 },
    ]) {
      const name = colorScheme + "-" + viewport.width + "x" + viewport.height;
      if (selectedCase && selectedCase !== name) continue;
      const context = await browser.newContext({
          viewport,
          colorScheme,
          deviceScaleFactor: 1,
        }),
        page = await context.newPage();
      page.setDefaultTimeout(10000);
      const errors = [],
        check = { name, passed: false, steps: [], geometry: [] };
      report.checks.push(check);
      page.on("pageerror", (e) => errors.push(String(e)));
      await page.addInitScript(() => {
        window.__mapPaint = {
          fills: [],
          strokes: [],
          workerRequests: 0,
          nodes: [],
          shapes: [],
        };
        const p = window.__mapPaint;
        for (const method of ["fill", "stroke"]) {
          const original = CanvasRenderingContext2D.prototype[method];
          CanvasRenderingContext2D.prototype[method] = function (...args) {
            if (this.canvas.closest(".map-wrap")) {
              const rows = p[method === "fill" ? "fills" : "strokes"];
              rows.push({
                color: method === "fill" ? this.fillStyle : this.strokeStyle,
                dash: this.getLineDash(),
              });
              if (rows.length > 5000) rows.splice(0, 2500);
            }
            return original.apply(this, args);
          };
        }
        const OriginalWorker = window.Worker;
        window.Worker = class extends OriginalWorker {
          constructor(...args) {
            super(...args);
            this.addEventListener("message", ({ data }) => {
              if (data.requestId === p.requestId) {
                p.shapes = data.shapes;
                p.completed = data.requestId;
              }
            });
          }
          postMessage(...args) {
            p.workerRequests++;
            if (args[0]?.nodes) {
              p.nodes = args[0].nodes;
              p.mode = args[0].mode;
              p.requestId = args[0].requestId;
            }
            return super.postMessage(...args);
          }
        };
      });
      const wait = (fn) => page.waitForFunction(fn, null, { timeout: 10000 });
      const geometry = async (stage) => {
        await page.locator(".workspace-side").scrollIntoViewIfNeeded();
        const g = await page.evaluate(() => {
          const aside = document.querySelector(".workspace-side"),
            tabs = document.querySelector(".list-view-tabs"),
            footer = document.querySelector(".file-type-selection"),
            show = document.querySelector(".file-type-show-files"),
            list = document.querySelector(".file-type-list");
          const box = (el) =>
            el ? JSON.parse(JSON.stringify(el.getBoundingClientRect())) : null;
          return {
            viewport: { width: innerWidth, height: innerHeight },
            documentWidth: document.documentElement.scrollWidth,
            aside: box(aside),
            asideScrollWidth: aside?.scrollWidth,
            asideClientWidth: aside?.clientWidth,
            tabs: box(tabs),
            footer: box(footer),
            show: box(show),
            list: box(list),
          };
        });
        check.geometry.push({ stage, ...g });
        assert(
          g.documentWidth <= viewport.width + 1,
          stage + ": document horizontal overflow",
        );
        assert(
          g.asideScrollWidth <= g.asideClientWidth + 1,
          stage + ": sidebar horizontal overflow",
        );
        for (const key of ["tabs", "footer", "show"]) {
          if (!g[key]) continue;
          if (
            g[key].top < -1 ||
            g[key].bottom > viewport.height + 1 ||
            g[key].top < g.aside.top - 1 ||
            g[key].bottom > g.aside.bottom + 1
          ) {
            (check.layoutIssues ??= []).push({
              stage,
              element: key,
              bounds: g[key],
              aside: g.aside,
            });
          }
        }
      };
      try {
        await page.goto(server.resolvedUrls.local[0]);
        await page.locator(".welcome-scan").click();
        await page
          .getByRole("button", { name: "Treemap", exact: true })
          .click();
        await page.locator("#file-types-tab").click();
        await page.locator(".file-type-row").first().waitFor();
        // Opening the File types tab colours the map by type; there is no separate switch.
        await wait(
          () =>
            window.__mapPaint.nodes.some((n) => n.fileType !== undefined) &&
            window.__mapPaint.completed === window.__mapPaint.requestId,
        );
        check.typeCoverage = await page.evaluate(() => {
          const p = window.__mapPaint,
            byId = new Map(p.nodes.map((n) => [n.id, n]));
          const canvas = document.querySelector(".map-wrap canvas");
          const area = p.shapes
            .filter((s) => byId.get(s.id)?.fileType !== undefined)
            .reduce((sum, s) => sum + s.width * s.height, 0);
          return area / (canvas.clientWidth * canvas.clientHeight);
        });
        assert(
          check.typeCoverage > 0.45,
          `Type-colored areas must cover the overview: ${check.typeCoverage}`,
        );
        await page.screenshot({
          path: path.join(output, name + "-types.png"),
          fullPage: true,
        });
        const zipGroupPoint = async () => {
          await page.locator(".map-wrap canvas").scrollIntoViewIfNeeded();
          return page.evaluate(() => {
            const p = window.__mapPaint;
            const group = p.nodes.find((n) => n.fileType === "zip");
            const shape = p.shapes.find((s) => s.id === group?.id);
            if (!shape)
              throw new Error("Expected a visible ZIP composition block");
            const canvas = document
              .querySelector(".map-wrap canvas")
              .getBoundingClientRect();
            return {
              x: canvas.left + shape.x + shape.width / 2,
              y: canvas.top + shape.y + shape.height / 2,
              groupId: group.id,
              fileType: group.fileType,
              parentId: group.parentId,
              parentPath: p.nodes.find((n) => n.id === group.parentId)?.path,
            };
          });
        };
        const groupPoint = await zipGroupPoint();
        await page.mouse.click(groupPoint.x, groupPoint.y);
        await wait(
          () =>
            document
              .querySelector(".map-wrap")
              ?.getAttribute("data-highlighted-file-type") === "zip" &&
            document
              .querySelector(".map-wrap")
              ?.getAttribute("data-file-type-highlight-pending") === "false",
        );
        assert.equal(
          await page
            .getByRole("button", { name: /^\.zip,/ })
            .getAttribute("aria-pressed"),
          "true",
        );
        assert.equal(
          await page.locator(".active-filters").count(),
          0,
          "A grouped block highlights its extension without filtering or selecting files",
        );
        const assertSafeGroupActions = async () => {
          const commands = await page.evaluate(
            () => window.__fileTypes.commands,
          );
          assert.deepEqual(
            commands.filter((c) =>
              /^(?:add_cleanup_queue|remove_cleanup_queue|prepare_trash|commit_trash|prepare_cleanup_queue|commit_cleanup_queue|reveal|preview|open_path|quick_look)/.test(
                c.command,
              ),
            ),
            [],
            "Group gestures must not call cleanup, preview, or reveal APIs",
          );
          assert.deepEqual(
            commands.filter(
              (c) =>
                c.nodeId >= 0x80000000 ||
                c.nodeIds?.some((id) => id >= 0x80000000),
            ),
            [],
            "Synthetic group IDs must never become native node targets",
          );
        };
        const expectDownloads = async () => {
          await wait(
            () =>
              document
                .querySelector(".app")
                ?.getAttribute("data-native-focus") === "2",
          );
          assert.equal(
            await page
              .locator(
                'nav[aria-label="Folder breadcrumbs"] [aria-current="page"]',
              )
              .innerText(),
            "Downloads",
          );
          await assertSafeGroupActions();
        };
        const returnToRoot = async () => {
          await page
            .locator('nav[aria-label="Folder breadcrumbs"] button')
            .first()
            .click();
          await wait(
            () =>
              document
                .querySelector(".app")
                ?.getAttribute("data-native-focus") === "0" &&
              window.__mapPaint.completed === window.__mapPaint.requestId &&
              window.__mapPaint.nodes.some((n) => n.id === 1),
          );
        };
        const doubleClickPoint = await zipGroupPoint();
        await page.mouse.dblclick(doubleClickPoint.x, doubleClickPoint.y);
        await expectDownloads();
        await page.screenshot({
          path: path.join(output, name + "-group-folder.png"),
          fullPage: true,
        });
        await returnToRoot();
        check.steps.push(
          "Grouped ZIP single click highlights safely; double click opens its real Downloads folder and breadcrumb",
        );
        if (name === "light-1536x1024") {
          const originalPoint = await zipGroupPoint();
          const before = await page.locator(".map-wrap canvas").boundingBox();
          await page.evaluate(() => {
            window.__groupGestureEvents = [];
            for (const type of ["click", "dblclick"])
              window.addEventListener(
                type,
                (event) => {
                  window.__groupGestureEvents.push({
                    type,
                    detail: event.detail,
                    x: event.clientX,
                    y: event.clientY,
                    target: event.target.tagName,
                    className: event.target.className,
                  });
                },
                true,
              );
          });
          await page.mouse.click(originalPoint.x, originalPoint.y);
          const divider = page.getByRole("separator", {
            name: "Resize sidebar",
          });
          await divider.focus();
          // Widening the left sidebar narrows the map.
          await page.keyboard.press("Shift+ArrowRight");
          await page.evaluate(
            () =>
              new Promise((resolve) =>
                requestAnimationFrame(() => requestAnimationFrame(resolve)),
              ),
          );
          await wait(
            () => window.__mapPaint.completed === window.__mapPaint.requestId,
          );
          const after = await page.locator(".map-wrap canvas").boundingBox();
          assert(
            after.width < before.width - 1,
            "Resizing between group clicks changes the canvas geometry",
          );
          const landedOn = await page.evaluate(
            ({ x, y }) => document.elementFromPoint(x, y)?.tagName,
            { x: originalPoint.x, y: originalPoint.y },
          );
          check.resizedGroupDoubleClick = {
            originalPoint,
            before,
            after,
            landedOn,
          };
          // Only the second native pair: mouse.click(clickCount: 2) would
          // introduce another detail=1 click and correctly cancel the capture.
          await page.mouse.down({ clickCount: 2 });
          await page.mouse.up({ clickCount: 2 });
          await expectDownloads();
          check.resizedGroupDoubleClick = {
            originalPoint,
            before,
            after,
            landedOn,
            focusId: 2,
            events: await page.evaluate(() => window.__groupGestureEvents),
          };
          assert.deepEqual(
            check.resizedGroupDoubleClick.events.map((event) => [
              event.type,
              event.detail,
            ]),
            [
              ["click", 1],
              ["click", 2],
              ["dblclick", 2],
            ],
            "Resize regression preserves the real native double-click sequence",
          );
          await page.screenshot({
            path: path.join(output, name + "-group-folder-after-resize.png"),
            fullPage: true,
          });
          await returnToRoot();
          check.steps.push(
            "Double click retains the original real group owner after a keyboard resize between clicks",
          );
        }
        await page.getByLabel("Clear file type highlight").click();
        check.steps.push(
          "Disk-root overview has substantial true type colors; grouped ZIP block links safely to sidebar",
        );
        await page.getByRole("button", { name: /^\.mov,/ }).click();
        await wait(
          () =>
            document
              .querySelector(".map-wrap")
              ?.getAttribute("data-highlighted-file-type") === "mov" &&
            Number(
              document
                .querySelector(".map-wrap")
                ?.getAttribute("data-file-type-highlight-count"),
            ) > 0 &&
            document
              .querySelector(".map-wrap")
              ?.getAttribute("data-file-type-highlight-pending") === "false",
        );
        const correspondence = await page.evaluate(() => {
          const swatch = document.querySelector(
            ".file-type-row.active .file-type-swatch",
          );
          const color = swatch.style.background;
          const probe = document.createElement("canvas").getContext("2d");
          probe.fillStyle = color;
          const normalized = probe.fillStyle;
          const canvas = document.querySelector(".map-wrap canvas");
          const pixels = canvas
            .getContext("2d")
            .getImageData(0, 0, canvas.width, canvas.height).data;
          return {
            color: normalized,
            fill: window.__mapPaint.fills.some((p) => p.color === normalized),
            solid: window.__mapPaint.strokes.some(
              (p) => p.color === normalized && p.dash.length === 0,
            ),
            dashed: window.__mapPaint.strokes.some(
              (p) => p.color === normalized && p.dash.length > 0,
            ),
            painted: pixels.some(
              (value, index) => index % 4 === 3 && value > 0,
            ),
          };
        });
        assert(
          correspondence.fill &&
            correspondence.solid &&
            !correspondence.dashed &&
            correspondence.painted,
          JSON.stringify(correspondence),
        );
        check.correspondence = correspondence;
        const typeLegend = await page.locator(".type-legend").innerText();
        assert(
          !/dashed|folders containing matches/i.test(typeLegend),
          "File type legend must not describe parent outlines: " + typeLegend,
        );
        check.typeLegend = typeLegend;
        await page.screenshot({
          path: path.join(output, name + "-mov.png"),
          fullPage: true,
        });
        await geometry("selected");
        check.steps.push(
          "live scan start; type selection; matching fill/solid color without type-colored dashed parent outlines",
        );
        if (name === "light-1536x1024") {
          await page.evaluate(() => {
            window.__mapPaint.fills = [];
            window.__mapPaint.strokes = [];
          });
          await page
            .getByRole("button", { name: "Sunburst", exact: true })
            .click();
          await wait(
            () =>
              window.__mapPaint.mode === "sunburst" &&
              window.__mapPaint.completed === window.__mapPaint.requestId,
          );
          check.sunburstCorrespondence = await page.evaluate(
            (color) => ({
              fill: window.__mapPaint.fills.some((p) => p.color === color),
              solid: window.__mapPaint.strokes.some(
                (p) => p.color === color && p.dash.length === 0,
              ),
              dashed: window.__mapPaint.strokes.some(
                (p) => p.color === color && p.dash.length > 0,
              ),
            }),
            correspondence.color,
          );
          assert(
            check.sunburstCorrespondence.fill &&
              check.sunburstCorrespondence.solid &&
              !check.sunburstCorrespondence.dashed,
            JSON.stringify(check.sunburstCorrespondence),
          );
          check.steps.push(
            "Sunburst matches use solid type outlines with no type-colored dashed parent outlines",
          );
          await page.screenshot({
            path: path.join(output, name + "-sunburst.png"),
            fullPage: true,
          });
          await page
            .getByRole("button", { name: "Treemap", exact: true })
            .click();
          await wait(
            () =>
              window.__mapPaint.mode === "treemap" &&
              window.__mapPaint.completed === window.__mapPaint.requestId,
          );
        }
        await page.locator("#file-types-tab").focus();
        await page.keyboard.press("ArrowRight");
        assert.equal(
          await page
            .locator("#files-tab")
            .getAttribute("aria-selected"),
          "true",
        );
        await page.screenshot({
          path: path.join(output, name + "-files-tab.png"),
          fullPage: true,
        });
        await page.keyboard.press("ArrowLeft");
        assert.equal(
          await page.locator("#file-types-tab").getAttribute("aria-selected"),
          "true",
        );
        assert.equal(
          await page
            .locator("#file-types-tab")
            .evaluate((el) => el === document.activeElement),
          true,
        );
        check.steps.push("keyboard tabs");
        await page
          .locator(".file-type-list")
          .evaluate((el) => (el.scrollTop = el.scrollHeight));
        assert(
          await page
            .locator(".file-type-list")
            .evaluate((el) => el.scrollTop > 0),
          "File type list actually scrolls",
        );
        await geometry("scrolled");
        if (viewport.width === 900)
          await page.screenshot({
            path: path.join(output, name + "-scrolled.png"),
            fullPage: true,
          });
        await page.getByLabel("Clear file type highlight").click();
        await wait(
          () =>
            !document
              .querySelector(".map-wrap")
              ?.hasAttribute("data-highlighted-file-type"),
        );
        check.steps.push(
          "clear highlight; scroll keeps footer and tabs visible",
        );
        await page.locator(".map-wrap canvas").scrollIntoViewIfNeeded();
        const mediaPoint = await page.evaluate(() => {
          const shape = window.__mapPaint.shapes.find((s) => s.id === 1);
          if (!shape?.headerHeight)
            throw new Error("Expected a visible Media folder label band");
          const canvas = document
            .querySelector(".map-wrap canvas")
            .getBoundingClientRect();
          return {
            x: canvas.left + shape.x + shape.width / 2,
            y: canvas.top + shape.y + shape.headerHeight / 2,
          };
        });
        await page.mouse.dblclick(mediaPoint.x, mediaPoint.y);
        await wait(
          () =>
            document
              .querySelector(".app")
              ?.getAttribute("data-native-focus") === "1",
        );
        assert.equal(
          await page.locator("#file-types-tab").getAttribute("aria-selected"),
          "true",
        );
        await page
          .getByRole("button", { name: "Current folder", exact: true })
          .click();
        await wait(
          () =>
            document
              .querySelector(".file-type-list")
              ?.getAttribute("aria-busy") === "false",
        );
        assert.equal(
          await page.getByRole("button", { name: /^\.dmg,/ }).count(),
          0,
        );
        assert.equal(
          await page.getByRole("button", { name: /^\.mov,/ }).count(),
          1,
        );
        await page.getByRole("button", { name: /^\.mov,/ }).click();
        await page
          .getByRole("button", { name: "Show matching files", exact: true })
          .click();
        await page
          .getByRole("button", {
            name: "Remove file type filter: .mov",
            exact: true,
          })
          .waitFor();
        const commands = await page.evaluate(() =>
          window.__fileTypes.commands.filter(
            (c) => c.command === "filter_nodes",
          ),
        );
        assert.equal(commands.at(-1).extension, "mov");
        assert.equal(commands.at(-1).scopeId, 1);
        check.steps.push(
          "folder navigation retains tab; Current folder scope; matching files exact scoped filter",
        );
        assert.deepEqual(errors, []);
        assert.deepEqual(
          check.layoutIssues ?? [],
          [],
          "Tabs and selected-type actions must remain inside the sidebar and viewport",
        );
        check.passed = true;
      } catch (error) {
        check.error = String(error);
        check.groupGestureEvents = await page.evaluate(
          () => window.__groupGestureEvents,
        );
        check.consoleErrors = errors;
        check.dom = await page.locator("body").innerText();
        await page.screenshot({
          path: path.join(output, name + "-failure.png"),
          fullPage: true,
        });
      } finally {
        await context.close();
      }
    }
} catch (error) {
  report.errors.push(String(error));
} finally {
  if (browser) await browser.close();
  if (server) await new Promise((r) => server.httpServer.close(r));
  await rm(temporary, { recursive: true, force: true });
  await writeFile(
    path.join(output, "report.json"),
    JSON.stringify(report, null, 2) + "\n",
  );
}
console.log(
  JSON.stringify(
    {
      output,
      checks: report.checks.map(({ name, passed, error }) => ({
        name,
        passed,
        error,
      })),
      errors: report.errors,
    },
    null,
    2,
  ),
);
if (report.errors.length || report.checks.some((c) => !c.passed))
  process.exitCode = 1;

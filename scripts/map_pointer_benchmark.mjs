#!/usr/bin/env node
/** Synthetic extracted pointer lookup; excludes DOM, paint, IPC and scans. */
import { hitShape } from '../src/hitTest.ts';
import { createHash } from 'node:crypto';
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { performance } from 'node:perf_hooks';
import os from 'node:os';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
if (args.includes('--help')) {
  console.log('node scripts/map_pointer_benchmark.mjs [--output PATH]');
  process.exit(0);
}
if (args.length && !(args.length === 2 && args[0] === '--output')) throw new Error('Use --output PATH');
const output = resolve(args[1] || resolve(root, 'benchmark-results/map-pointer.json'));
const width = 1000, height = 500, count = 4999;
const nodes = Array.from({ length: count + 1 }, (_, id) => ({ id, name: `node-${id}` }));
const setupStart = performance.now();
const byId = new Map(nodes.map(n => [n.id, n]));
const setupMs = performance.now() - setupStart;
let hitCalls = 0;
function oldLookup(shapes, mode, x, y) {
  function hit() {
    hitCalls++;
    const radius = Math.hypot(x - width / 2, y - height / 2);
    let angle = Math.atan2(y - height / 2, x - width / 2);
    if (angle < -Math.PI / 2) angle += Math.PI * 2;
    return [...shapes].reverse().find(s => mode === 'treemap'
      ? x >= s.x && x <= s.x + s.width && y >= s.y && y <= s.y + s.height
      : radius >= s.inner && radius <= s.outer && angle >= s.start && angle <= s.end);
  }
  return nodes.find(n => n.id === hit()?.id);
}
function newLookup(shapes, mode, x, y) {
  const shape = hitShape(shapes, mode, width, height, x, y);
  return shape ? byId.get(shape.id) : undefined;
}
const grid = Array.from({length: count}, (_, i) => ({id: i + 1, x: (i % 100) * 10, y: Math.floor(i / 100) * 10, width: 10, height: 10}));
const sectors = [{id: 0, inner: 0, outer: 20, start: -Math.PI / 2, end: Math.PI * 1.5}, ...Array.from({length: count}, (_, i) => ({id: i + 1, inner: 20, outer: 200, start: -Math.PI / 2 + i * Math.PI * 2 / count, end: -Math.PI / 2 + (i + 1) * Math.PI * 2 / count}))];
const polar = angle => [width / 2 + Math.cos(angle) * 100, height / 2 + Math.sin(angle) * 100];
const cases = [
  {mode:'treemap', name:'last-node', shapes:grid, point:[985,495], expected:count},
  {mode:'treemap', name:'first-node', shapes:grid, point:[5,5], expected:1},
  {mode:'treemap', name:'miss', shapes:grid, point:[-5,-5], expected:null},
  {mode:'sunburst', name:'last-node', shapes:sectors, point:polar(-Math.PI / 2 + (count - .5) * Math.PI * 2 / count), expected:count},
  {mode:'sunburst', name:'first-node', shapes:sectors, point:polar(-Math.PI / 2 + .5 * Math.PI * 2 / count), expected:1},
  {mode:'sunburst', name:'miss', shapes:sectors, point:[0,0], expected:null},
  {mode:'sunburst', name:'root', shapes:sectors, point:[width/2,height/2], expected:0},
];
const samples = [], checks = [];
let sink = 0;
function measure(test, variant, phase, round) {
  const repeats = variant === 'baseline' ? 2 : 1000;
  const lookup = variant === 'baseline' ? oldLookup : newLookup;
  const start = performance.now();
  for (let i = 0; i < repeats; i++) sink ^= lookup(test.shapes, test.mode, ...test.point)?.id ?? -1;
  const elapsedMs = performance.now() - start;
  samples.push({mode:test.mode, case:test.name, variant, phase, round, repeats, elapsed_ms:elapsedMs, per_event_ms:elapsedMs/repeats});
}
for (const test of cases) {
  hitCalls = 0;
  const oldId = oldLookup(test.shapes, test.mode, ...test.point)?.id ?? null;
  const oldHitCalls = hitCalls;
  const newId = newLookup(test.shapes, test.mode, ...test.point)?.id ?? null;
  if (oldId !== newId || newId !== test.expected) throw new Error(`Lookup mismatch ${test.mode}/${test.name}: ${oldId}/${newId}/${test.expected}`);
  checks.push({mode:test.mode, case:test.name, expected_id:test.expected, baseline_id:oldId, candidate_id:newId, baseline_hit_calls_per_event:oldHitCalls, candidate_hit_calls_per_event:1});
  for (const variant of ['baseline','candidate']) measure(test, variant, 'initial', 0);
  for (let round = 1; round <= 3; round++) {
    for (const variant of round % 2 ? ['candidate','baseline'] : ['baseline','candidate']) measure(test, variant, 'warm', round);
  }
}
const median = values => values.toSorted((a,b)=>a-b)[Math.floor(values.length/2)];
const summaries = cases.map(test => {
  const medians = Object.fromEntries(['baseline','candidate'].map(variant => [variant,median(samples.filter(s=>s.mode===test.mode && s.case===test.name && s.variant===variant && s.phase==='warm').map(s=>s.per_event_ms))]));
  return {mode:test.mode, case:test.name, warm_median_per_event_ms:medians, candidate_over_baseline_ratio:medians.candidate/medians.baseline};
});
const hash = path => createHash('sha256').update(readFileSync(resolve(root,path))).digest('hex');
const report = {recorded_at_utc:new Date().toISOString(), node_version:process.version, host:{platform:os.platform(), architecture:os.arch(), cpus:os.cpus().length, cpu_model:os.cpus()[0]?.model}, source_sha256:Object.fromEntries(['scripts/map_pointer_benchmark.mjs','src/hitTest.ts','src/DiskMap.svelte'].map(path=>[path,hash(path)])), dimensions:{width,height}, nodes:nodes.length, shapes:{treemap:grid.length,sunburst:sectors.length}, map_setup_ms:setupMs, timer:'performance.now() monotonic milliseconds; elapsed batch divided by repeats', methodology:'One initial sample retained separately; three alternating warm samples per variant per point. Baseline2 events/sample; candidate1000 events/sample. Map setup excluded from pointer timing, matching memoized map setup. Baseline hitCalls counter is included in timing.', limitations:['Synthetic extracted lookup, not measured DOM interaction latency.', 'Excludes Canvas drawing, map layout, Tauri IPC and native scans.', 'Not an OpenDisk acceptance comparison. Different repeat counts and JIT effects limit exact ratios.'], matching_selected_ids:true, app_level_comparison:false, acceptance_met:false, checks, samples, summaries, sink};
mkdirSync(dirname(output),{recursive:true});
writeFileSync(output,JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify({output,summaries,matching_selected_ids:report.matching_selected_ids},null,2));

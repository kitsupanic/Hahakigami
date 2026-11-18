<script lang="ts">
  import type { NodeView } from "./types";
  import { fileExtension, fileTypeColor, fileTypeLabel } from "./fileTypes";
  import type { Shape } from "./layout";
  import { createQueueMarker, type QueuePath } from "./queueMarkers";
  import {
    createHoverTarget,
    findHoverMatch,
    type HoverTarget,
  } from "./hoverContext";
  import { hitShape } from "./hitTest";
  import {
    compactBytes,
    countLabel,
    mapLabelColor,
    mapFontFamily,
    mapTileTones,
  } from "./uiFormat";
  import { createBranchPalette, neutralColor } from "./branchColors";
  import { guardianInk, guardianMapFont, theme } from "./theme.svelte";
  import { onMount, onDestroy, untrack } from "svelte";
  let {
    nodes,
    rootId,
    scanId = "",
    mode,
    selected,
    onSelect,
    onOpen,
    onFileTypeSelect,
    onRendered,
    onLayoutPending,
    queuedPaths = [],
    hovered = null,
    hoverEpoch = 0,
    onHover,
    colorMode = "folder",
    highlightedFileType = null,
    fileTypeHighlights = [],
    fileTypeHighlightPending = false,
    scanComplete = true,
  }: {
    scanComplete?: boolean;
    colorMode?: "folder" | "fileType";
    highlightedFileType?: string | null;
    fileTypeHighlights?: readonly number[];
    fileTypeHighlightPending?: boolean;
    queuedPaths?: readonly QueuePath[];
    hovered?: HoverTarget | null;
    hoverEpoch?: number;
    onHover?: (target: HoverTarget | null) => void;
    nodes: NodeView[];
    rootId: number;
    scanId?: string;
    mode: "treemap" | "sunburst";
    selected: number[];
    onSelect: (n: NodeView, multi: boolean) => void;
    onOpen: (n: NodeView) => void;
    onFileTypeSelect?: (extension: string) => void;
    onRendered?: (nodes: NodeView[]) => void;
    onLayoutPending?: () => void;
  } = $props();
  const uid = $props.id();
  const summaryId = `${uid}-summary`;
  let canvas: HTMLCanvasElement;
  let worker: Worker | null = null;
  let version = 0;
  let paintVersion = $state.raw(0);
  let dimensions = $state.raw({ width: 600, height: 400 });
  let shapes: Shape[] = $state.raw([]);
  let hover: NodeView | null = $state.raw(null);
  let previousHoverEpoch: number | undefined;
  $effect(() => {
    // Context resets also invalidate local aggregate feedback (shared hover is null).
    const next = hoverEpoch;
    if (next === previousHoverEpoch) return;
    previousHoverEpoch = next;
    untrack(() => (hover = null));
  });
  let previousHovered: HoverTarget | null | undefined;
  $effect(() => {
    const next = hovered;
    if (next === previousHovered) return;
    previousHovered = next;
    untrack(() => {
      // Parent resets and other hover sources supersede our last pointer hit.
      // Aggregates deliberately stay local when their publication clears shared hover.
      if (
        hover &&
        hover.id !== next?.node.id &&
        !(hover.kind === "aggregate" && next === null)
      )
        hover = null;
    });
  });
  let byId = $derived.by(() => new Map(nodes.map((n) => [n.id, n])));
  let centerNode = $derived(byId.get(rootId));
  let holeRadius = $derived(mode === "sunburst" ? (shapes[0]?.inner ?? 0) : 0);
  const typedGroup = (node: NodeView | undefined) =>
    node?.kind === "aggregate" && node.fileType !== undefined;
  function folderFor(node: NodeView | undefined) {
    if (node?.kind === "directory") return node;
    if (!typedGroup(node) || node!.parentId === null) return undefined;
    const parent = byId.get(node!.parentId);
    return parent?.kind === "directory" ? parent : undefined;
  }
  // Native click counts can continue at 3, 4, ... during rapid drill-down.
  const startsClickPair = (detail: number) => detail <= 1 || detail % 2 === 1;
  // A first click can open the inspector and resize/re-layout this canvas before
  // the browser delivers click(detail=2) and dblclick. Keep that original hit,
  // including an empty/aggregate hit; never re-hit a different folder.
  let clickCapture: {
    scanId: string;
    rootId: number;
    mode: "treemap" | "sunburst";
    colorMode: "folder" | "fileType";
    target: Pick<
      NodeView,
      "id" | "kind" | "path" | "parentId" | "fileType"
    > | null;
    folder: Pick<NodeView, "id" | "path" | "parentId"> | null;
    x: number;
    y: number;
  } | null = null;
  let suppressNextDoubleClick = false;
  function capturedNode() {
    if (!clickCapture) return undefined;
    const { target } = clickCapture;
    const node = target ? byId.get(target.id) : undefined;
    const folder = folderFor(node);
    if (
      clickCapture.scanId !== scanId ||
      clickCapture.rootId !== rootId ||
      clickCapture.mode !== mode ||
      clickCapture.colorMode !== colorMode ||
      folder?.id !== clickCapture.folder?.id ||
      folder?.path !== clickCapture.folder?.path ||
      folder?.parentId !== clickCapture.folder?.parentId ||
      (target &&
        (!node ||
          node.kind !== target.kind ||
          node.path !== target.path ||
          node.parentId !== target.parentId ||
          node.fileType !== target.fileType))
    ) {
      clickCapture = null;
      return undefined;
    }
    return node;
  }
  function openCapturedFolder(node: NodeView | undefined) {
    const folder = folderFor(node);
    clickCapture = null;
    if (folder) onOpen(folder);
  }
  $effect(() => {
    // Invalidate even if a context changes away and back between pointer events.
    scanId;
    rootId;
    mode;
    colorMode;
    byId;
    capturedNode();
  });
  onMount(() => {
    // Opening the inspector can move the next native click onto a control beside
    // the canvas. Finish only the original map gesture, before that control acts.
    const outsideGesture = (event: MouseEvent) => {
      if (event.type === "click") suppressNextDoubleClick = false;
      else if (suppressNextDoubleClick) {
        // A later pair already opened on click. Consume an optional trailing
        // dblclick even if resizing moved it onto a control outside the canvas.
        suppressNextDoubleClick = false;
        event.preventDefault();
        event.stopPropagation();
        return;
      }
      if (event.target === canvas) return;
      if (event.type === "click" && startsClickPair(event.detail)) {
        clickCapture = null;
        return;
      }
      if (event.detail < 2) return;
      const node = capturedNode();
      if (
        !clickCapture ||
        Math.hypot(
          event.clientX - clickCapture.x,
          event.clientY - clickCapture.y,
        ) > 5
      )
        return;
      event.preventDefault();
      event.stopPropagation();
      if (event.type === "dblclick" || event.detail > 2) {
        suppressNextDoubleClick = event.type === "click";
        openCapturedFolder(node);
      }
    };
    window.addEventListener("click", outsideGesture, true);
    window.addEventListener("dblclick", outsideGesture, true);
    return () => {
      window.removeEventListener("click", outsideGesture, true);
      window.removeEventListener("dblclick", outsideGesture, true);
    };
  });
  let branchPalette = $derived(createBranchPalette(nodes, rootId));
  let layoutInput = $derived({
    nodes,
    rootId,
    mode,
    colorMode,
    ...dimensions,
  });
  let completedLayout: typeof layoutInput | null = $state.raw(null);
  let layoutCurrent = $derived.by(
    () =>
      !!completedLayout &&
      completedLayout.nodes === nodes &&
      completedLayout.rootId === rootId &&
      completedLayout.mode === mode &&
      completedLayout.colorMode === colorMode &&
      completedLayout.width === dimensions.width &&
      completedLayout.height === dimensions.height,
  );
  let renderedIds = $derived(new Set(shapes.map((shape) => shape.id)));
  let activeTarget = $derived.by(() =>
    hover ? createHoverTarget(hover, byId) : hovered,
  );
  let hoverId = $derived.by(() =>
    layoutCurrent
      ? hover?.kind === "aggregate"
        ? hover.id
        : findHoverMatch(activeTarget, renderedIds, byId)
      : null,
  );
  let captionNode = $derived.by(() => {
    if (hoverId === null) return null;
    return hover?.kind === "aggregate" ? hover : (activeTarget?.node ?? null);
  });
  let hoverGroup = $derived(
    shapes.find((shape) => shape.id === hoverId)?.groupId,
  );
  let captionName = $derived.by(() => {
    if (!captionNode) return "";
    if (captionNode.kind === "aggregate")
      return captionNode.fileType !== undefined
        ? `${fileTypeLabel(captionNode.fileType)} files`
        : captionNode.name;
    if (!activeTarget) return "";
    // List rows can know only the first missing parent; complete names using
    // the map's index without interpreting display paths as ancestry.
    const lastId = activeTarget.lineage.at(-1);
    const ancestor = lastId === undefined ? undefined : byId.get(lastId);
    const lineage = [
      ...new Set([
        ...activeTarget.lineage,
        ...(ancestor ? createHoverTarget(ancestor, byId).lineage : []),
      ]),
    ];
    const names = lineage
      .filter((id) => id !== rootId || id === captionNode.id)
      .map((id) =>
        id === captionNode.id ? captionNode.name : byId.get(id)?.name,
      )
      .filter((name): name is string => !!name)
      .reverse();
    return names.join(" › ") || captionNode.name;
  });
  let selectedIds = $derived.by(() => new Set(selected));
  let highlightedIds = $derived.by(() => {
    const realIds = new Set(fileTypeHighlights);
    // Ancestor IDs establish scope for summarized groups; they are not visual matches.
    const effective = new Set(
      [...realIds].filter((id) => byId.get(id)?.kind === "file"),
    );
    if (highlightedFileType !== null)
      for (const node of nodes) {
        if (
          typedGroup(node) &&
          node.fileType === highlightedFileType &&
          node.parentId !== null &&
          realIds.has(node.parentId)
        )
          effective.add(node.id);
      }
    return effective;
  });
  let highlightActive = $derived(
    highlightedFileType !== null && !fileTypeHighlightPending && layoutCurrent,
  );
  let queueMarker = $derived(createQueueMarker(queuedPaths));
  let legendExpanded = $state(false);
  let legendNodes = $derived.by(() =>
    nodes
      .filter((n) => n.parentId === rootId && n.allocatedBytes > 0)
      .sort((a, b) => b.allocatedBytes - a.allocatedBytes || a.id - b.id),
  );
  let legendVisible = $derived(
    legendExpanded ? legendNodes : legendNodes.slice(0, 6),
  );
  let queueMarks = $derived.by(
    () =>
      new Map(
        nodes.map((n) => [
          n.id,
          n.kind === "aggregate" || !n.path ? null : queueMarker(n),
        ]),
      ),
  );
  let visibleMarks = $derived.by(() =>
    shapes.reduce(
      (counts, shape) => {
        const mark = queueMarks.get(shape.id);
        if (mark) counts[mark]++;
        return counts;
      },
      { queued: 0, contains: 0 },
    ),
  );
  const hasLegendNodes = $derived(legendNodes.length > 0);
  // A string, not the Set: scan revisions rebuild the Set every second.
  const selectionKey = $derived(selected.join(","));
  // One primitive key shields the effect from props whose identity changes per scan tick.
  const summaryKey = $derived(
    [scanId, rootId, mode, scanComplete, selectionKey, hasLegendNodes].join(
      "|",
    ),
  );
  const contextKey = $derived(`${scanId}|${rootId}`);
  let summaryText = $state("");
  $effect(() => {
    // The description is read on demand; it refreshes on context changes and once
    // when the scan completes, never on every scan tick.
    summaryKey;
    untrack(() => {
      const top = legendNodes
        .slice(0, 3)
        .map((n) => `${n.name} ${compactBytes(n.allocatedBytes)}`);
      const picked = selected.map((id) => byId.get(id)).filter((n) => !!n);
      summaryText = [
        `${mode === "treemap" ? "Treemap" : "Sunburst"} of ${centerNode?.name ?? "this scan"}${centerNode ? `, ${compactBytes(centerNode.allocatedBytes)} on disk` : ""}.`,
        top.length ? `Largest items: ${top.join(", ")}.` : "No sized items yet.",
        picked.length === 1
          ? `Selected: ${picked[0].name}, ${compactBytes(picked[0].allocatedBytes)}.`
          : picked.length > 1
            ? `${picked.length} items selected.`
            : "",
      ]
        .filter(Boolean)
        .join(" ");
    });
  });
  // Only a folder change is announced; the list already announces selection.
  let announcement = $state("");
  let announcedRoot: { scanId: string; rootId: number } | undefined;
  $effect(() => {
    contextKey;
    untrack(() => {
      const previous = announcedRoot;
      announcedRoot = { scanId, rootId };
      if (previous && previous.scanId === scanId && previous.rootId !== rootId)
        announcement = `Opened ${centerNode?.name ?? "folder"}`;
    });
  });
  onMount(() => {
    const w = new Worker(new URL("./layout.worker.ts", import.meta.url), {
      type: "module",
    });
    worker = w;
    w.onmessage = (e) => {
      if (e.data.requestId === version) {
        shapes = e.data.shapes;
        paintVersion = e.data.requestId;
        completedLayout = previousLayout ?? null;
      }
    };
    return () => {
      version++;
      w.onmessage = null;
      w.terminate();
      worker = null;
    };
  });
  onMount(() => {
    const el = canvas!;
    const observer = new ResizeObserver(
      ([entry]) =>
        (dimensions = {
          width: entry.contentRect.width,
          height: entry.contentRect.height,
        }),
    );
    observer.observe(el);
    return () => observer.disconnect();
  });
  let previousLayout:
    | {
        nodes: NodeView[];
        rootId: number;
        mode: "treemap" | "sunburst";
        colorMode: "folder" | "fileType";
        width: number;
        height: number;
      }
    | undefined;
  $effect(() => {
    if (!worker) return;
    const next = layoutInput;
    if (
      previousLayout &&
      previousLayout.nodes === next.nodes &&
      previousLayout.rootId === next.rootId &&
      previousLayout.mode === next.mode &&
      previousLayout.colorMode === next.colorMode &&
      previousLayout.width === next.width &&
      previousLayout.height === next.height
    )
      return;
    previousLayout = next;
    untrack(() => {
      clearHover();
      onLayoutPending?.();
    });
    worker.postMessage({ requestId: ++version, ...next });
  });
  // Canvas-only rounding leaves the layout and pointer hit regions unchanged.
  function tilePath(ctx: CanvasRenderingContext2D, s: Shape, inset: number) {
    const x = s.x + inset,
      y = s.y + inset;
    const width = Math.max(0, s.width - inset * 2);
    const height = Math.max(0, s.height - inset * 2);
    const radius = Math.min(
      s.depth === 0 ? 6 : 5,
      Math.min(width, height) * 0.12,
    );
    if (!radius) {
      ctx.rect(x, y, width, height);
      return;
    }
    ctx.arc(x + width - radius, y + radius, radius, -Math.PI / 2, 0);
    ctx.arc(x + width - radius, y + height - radius, radius, 0, Math.PI / 2);
    ctx.arc(x + radius, y + height - radius, radius, Math.PI / 2, Math.PI);
    ctx.arc(x + radius, y + radius, radius, Math.PI, Math.PI * 1.5);
    ctx.closePath();
  }
  $effect(() => {
    const el = canvas!;
    // Empty layouts need neither a context nor a high-resolution backing store.
    // Resetting dimensions also clears any image from the previous layout.
    if (!shapes.length) {
      if (el.width !== 0) el.width = 0;
      if (el.height !== 0) el.height = 0;
      if (layoutCurrent && paintVersion === version)
        untrack(() => onRendered?.(nodes));
      return;
    }
    const parents = new Set(nodes.map((n) => n.parentId));
    const ctx = el.getContext("2d")!;
    const dpr = window.devicePixelRatio || 1;
    const pixelWidth = Math.trunc(dimensions.width * dpr),
      pixelHeight = Math.trunc(dimensions.height * dpr);
    // Setting either dimension resets the backing store and all context state.
    if (el.width !== pixelWidth) el.width = pixelWidth;
    if (el.height !== pixelHeight) el.height = pixelHeight;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, dimensions.width, dimensions.height);
    const mapFont = theme.guardian ? guardianMapFont : mapFontFamily;
    const tones = new Map<string, ReturnType<typeof mapTileTones>>();
    const labelColors = new Map<string, ReturnType<typeof mapLabelColor>>();
    const tileColor = (s: Shape, n: NodeView | undefined) =>
      n?.kind === "aggregate"
        ? colorMode === "fileType" && typedGroup(n)
          ? fileTypeColor(n.fileType!)
          : neutralColor()
        : colorMode === "fileType"
          ? n?.kind === "file"
            ? fileTypeColor(fileExtension(n.name))
            : theme.guardian
              ? "#e8e7e3"
              : "#e2e7ef"
          : (branchPalette.get(s.groupId ?? s.id) ?? neutralColor());
    for (const s of shapes) {
      const n = byId.get(s.id);
      const baseColor = tileColor(s, n);
      ctx.globalAlpha =
        highlightActive &&
        (n?.kind === "file" || typedGroup(n)) &&
        !highlightedIds.has(s.id)
          ? 0.55
          : 1;
      if (
        mode === "treemap" &&
        n?.kind !== "aggregate" &&
        colorMode === "folder"
      ) {
        const key = `${baseColor}:${s.depth > 0}`;
        let tileTones = tones.get(key);
        if (!tileTones) {
          tileTones = mapTileTones(baseColor, s.depth > 0);
          tones.set(key, tileTones);
        }
        // A solid midpoint keeps subpixel cells crisp and avoids thousands of
        // imperceptible gradient allocations in dense maps.
        ctx.fillStyle = tileTones.middle;
        if (s.width >= 12 && s.height >= 12) {
          const fill = ctx.createLinearGradient(
            s.x,
            s.y,
            s.x + s.width * 0.55,
            s.y + s.height,
          );
          fill.addColorStop(0, tileTones.top);
          fill.addColorStop(0.5, tileTones.middle);
          fill.addColorStop(1, tileTones.bottom);
          ctx.fillStyle = fill;
        }
      } else ctx.fillStyle = baseColor;
      ctx.strokeStyle =
        s.depth === 0
          ? "rgba(255,255,255,.65)"
          : mode === "treemap"
            ? s.width < 12 || s.height < 12
              ? "rgba(255,255,255,.10)"
              : s.depth > 1
                ? "rgba(255,255,255,.22)"
                : "rgba(255,255,255,.38)"
            : "rgba(255,255,255,.42)";
      ctx.lineWidth = 1;
      ctx.beginPath();
      if (mode === "treemap") tilePath(ctx, s, 1);
      else {
        const cx = dimensions.width / 2,
          cy = dimensions.height / 2;
        ctx.arc(cx, cy, s.outer, s.start, s.end);
        ctx.arc(cx, cy, s.inner, s.end, s.start, true);
        ctx.closePath();
      }
      ctx.fill();
      ctx.stroke();
      ctx.globalAlpha = 1;
    }
    // Type outlines apply only to matching files and exact extension groups.
    // Painting never changes layout, pointer regions, or selection.
    if (highlightActive) {
      for (const s of shapes) {
        const n = byId.get(s.id);
        if (
          !n ||
          !highlightedIds.has(s.id) ||
          (n.kind !== "file" && !typedGroup(n))
        )
          continue;
        ctx.strokeStyle = fileTypeColor(highlightedFileType!);
        ctx.lineWidth = 3;
        ctx.setLineDash([]);
        ctx.beginPath();
        if (mode === "treemap")
          tilePath(ctx, s, Math.min(4, s.width / 4, s.height / 4));
        else {
          const inset = Math.min(3, (s.outer - s.inner) / 4);
          ctx.arc(
            dimensions.width / 2,
            dimensions.height / 2,
            s.outer - inset,
            s.start,
            s.end,
          );
          ctx.arc(
            dimensions.width / 2,
            dimensions.height / 2,
            s.inner + inset,
            s.end,
            s.start,
            true,
          );
          ctx.closePath();
        }
        // A white rim keeps the solid type outline visible on same-color tiles.
        ctx.strokeStyle = "#fff";
        ctx.lineWidth = 6;
        ctx.stroke();
        ctx.strokeStyle = fileTypeColor(highlightedFileType!);
        ctx.lineWidth = 3;
        ctx.stroke();
      }
      ctx.setLineDash([]);
    }
    // Labels and selection outlines sit above every descendant tile.
    for (const s of shapes) {
      const n = byId.get(s.id);
      if (mode === "sunburst" && s.depth === 0 && n && !queueMarks.get(s.id)) {
        const span = s.end - s.start;
        const fontSize = span >= 1 && s.outer - s.inner >= 130 ? 18 : 12;
        const lineHeight = fontSize * 1.35;
        const halfHeight = lineHeight + fontSize / 2;
        // Balance the outer arc against wedge edges; wide slices use the ring midpoint.
        const radius = Math.max(
          (s.inner + s.outer) / 2,
          s.outer / (1 + Math.sin(Math.min(span / 2, Math.PI / 2))),
        );
        if (s.outer - s.inner > halfHeight * 2 + 10) {
          const total = centerNode ? centerNode.allocatedBytes : 0;
          const lines = [
            typedGroup(n) ? `${fileTypeLabel(n.fileType!)} files` : n.name,
            compactBytes(n.allocatedBytes, 1),
            ...(total > 0
              ? [`${((n.allocatedBytes / total) * 100).toFixed(1)}%`]
              : []),
          ];
          ctx.save();
          ctx.font = `${fontSize}px ${mapFont}`;
          const halfWidth =
            Math.max(...lines.map((line) => ctx.measureText(line).width)) / 2;
          const angle = (s.start + s.end) / 2;
          const anchor = [
            radius,
            ...Array.from(
              { length: 19 },
              (_, index) => s.inner + ((s.outer - s.inner) * (index + 1)) / 20,
            ),
          ]
            .map((r) => ({ x: Math.cos(angle) * r, y: Math.sin(angle) * r }))
            .find(({ x, y }) => {
              const nearestRadius = Math.hypot(
                Math.max(0, Math.abs(x) - halfWidth - 2),
                Math.max(0, Math.abs(y) - halfHeight - 2),
              );
              return (
                nearestRadius > s.inner &&
                [-1, 1].every((dx) =>
                  [-1, 1].every((dy) => {
                    const cornerX = x + dx * (halfWidth + 2);
                    const cornerY = y + dy * (halfHeight + 2);
                    const offset =
                      (Math.atan2(cornerY, cornerX) - s.start + Math.PI * 2) %
                      (Math.PI * 2);
                    return (
                      Math.hypot(cornerX, cornerY) < s.outer && offset < span
                    );
                  }),
                )
              );
            });
          if (anchor) {
            ctx.fillStyle = mapLabelColor(tileColor(s, n));
            ctx.textAlign = "center";
            ctx.textBaseline = "middle";
            lines.forEach((line, index) =>
              ctx.fillText(
                line,
                dimensions.width / 2 + anchor.x,
                dimensions.height / 2 + anchor.y + (index - 1) * lineHeight,
              ),
            );
          }
          ctx.restore();
        }
      }
      if (
        mode === "treemap" &&
        s.width >= 48 &&
        (s.height > 35 || (colorMode === "fileType" && !!s.headerHeight)) &&
        (s.depth < 2 || s.headerHeight || (s.width >= 150 && s.height >= 80)) &&
        (!parents.has(s.id) || s.headerHeight)
      ) {
        const labelHeight = s.headerHeight ?? s.height;
        const topLevel = n?.parentId === rootId;
        const padding = topLevel ? 12 : 10;
        ctx.save();
        const baseColor = tileColor(s, n);
        // Labels occupy the upper tile/header, where the gradient starts.
        // The fill pass already cached the actual lightened tone for this depth.
        const labelTone =
          colorMode === "folder" && n?.kind !== "aggregate"
            ? (tones.get(`${baseColor}:${s.depth > 0}`)?.top ?? baseColor)
            : baseColor;
        let labelColor = labelColors.get(labelTone);
        if (!labelColor) {
          labelColor = mapLabelColor(labelTone);
          labelColors.set(labelTone, labelColor);
        }
        ctx.fillStyle =
          colorMode === "fileType" &&
          n?.kind !== "file" &&
          n?.kind !== "aggregate"
            ? theme.guardian
              ? guardianInk
              : "#26354b"
            : labelColor;
        ctx.font = `${topLevel ? 16 : 13}px ${mapFont}`;
        ctx.beginPath();
        ctx.rect(
          s.x + padding,
          s.y + 4,
          Math.max(0, s.width - padding * 2),
          Math.max(0, labelHeight - 6),
        );
        ctx.clip();
        let name = typedGroup(n)
          ? `${fileTypeLabel(n!.fileType!)} files`
          : n?.name || "";
        const available = Math.max(
          0,
          s.width - padding * 2 - (queueMarks.get(s.id) ? 28 : 0),
        );
        if (ctx.measureText(name).width > available) {
          let low = 0,
            high = name.length;
          while (low < high) {
            const middle = Math.ceil((low + high) / 2);
            if (ctx.measureText(name.slice(0, middle) + "…").width <= available)
              low = middle;
            else high = middle - 1;
          }
          name = low > 0 ? name.slice(0, low) + "…" : "";
        }
        const paintLabel = (text: string, y: number) => {
          if (text) ctx.fillText(text, s.x + padding, y);
        };
        paintLabel(name, s.y + Math.min(topLevel ? 24 : 22, labelHeight - 6));
        if (n && labelHeight >= (topLevel ? 53 : 49)) {
          const label = typedGroup(n)
            ? `${compactBytes(n.allocatedBytes, 1)} · ${countLabel(n.fileCount ?? 0, "file")}`
            : compactBytes(n.allocatedBytes, 1);
          ctx.font = `600 ${topLevel ? 18 : 15}px ${mapFont}`;
          if (ctx.measureText(label).width <= available)
            paintLabel(label, s.y + (topLevel ? 47 : 43));
        }
        ctx.restore();
      }
    }
    // Queue overlays are painted after all fills so parent markers stay visible.
    for (const s of shapes) {
      const mark = queueMarks.get(s.id);
      if (!mark) continue;
      ctx.strokeStyle = "#8edbb2";
      ctx.lineWidth = 2;
      ctx.setLineDash(mark === "contains" ? [4, 3] : []);
      ctx.beginPath();
      let badge: { x: number; y: number } | null = null;
      if (mode === "treemap") {
        const inset = Math.min(5, s.width / 4, s.height / 4);
        ctx.rect(
          s.x + inset,
          s.y + inset,
          Math.max(0, s.width - inset * 2),
          Math.max(0, s.height - inset * 2),
        );
        if (
          s.width >= 42 &&
          (s.headerHeight ?? s.height) >= 28 &&
          (!parents.has(s.id) || !!s.headerHeight)
        )
          badge = { x: s.x + s.width - 18, y: s.y + 16 };
      } else {
        const thickness = s.outer - s.inner;
        const span = s.end - s.start;
        if (!(thickness > 0 && span > 0)) continue;
        const inset = Math.min(4, thickness / 4);
        const radius = (s.inner + s.outer) / 2;
        const angleInset = Math.min(span / 4, inset / Math.max(1, radius));
        const start = s.start + angleInset,
          end = s.end - angleInset;
        ctx.arc(
          dimensions.width / 2,
          dimensions.height / 2,
          s.outer - inset,
          start,
          end,
        );
        ctx.arc(
          dimensions.width / 2,
          dimensions.height / 2,
          s.inner + inset,
          end,
          start,
          true,
        );
        ctx.closePath();
        if (thickness >= 32 && radius * span >= 36) {
          const angle = (start + end) / 2;
          badge = {
            x: dimensions.width / 2 + Math.cos(angle) * radius,
            y: dimensions.height / 2 + Math.sin(angle) * radius,
          };
        }
      }
      ctx.stroke();
      ctx.setLineDash([]);
      if (badge) {
        ctx.beginPath();
        ctx.arc(badge.x, badge.y, 10, 0, Math.PI * 2);
        ctx.fillStyle = "#173e2b";
        ctx.fill();
        ctx.stroke();
        ctx.fillStyle = "#8edbb2";
        ctx.font = "bold 14px system-ui";
        ctx.fillText(mark === "queued" ? "✓" : "○", badge.x - 6, badge.y + 5);
      }
    }
    ctx.setLineDash([]);
    function outlinePath(s: Shape) {
      ctx.beginPath();
      if (mode === "treemap") tilePath(ctx, s, 1.5);
      else {
        const cx = dimensions.width / 2,
          cy = dimensions.height / 2;
        ctx.arc(cx, cy, s.outer, s.start, s.end);
        ctx.arc(cx, cy, s.inner, s.end, s.start, true);
        ctx.closePath();
      }
    }
    // The canvas CSS sets accent-color to the person's accent; canvas cannot read var().
    const accent = getComputedStyle(el).accentColor;
    const selectionColor = accent && accent !== "auto" ? accent : "#246fde";
    for (const s of shapes) {
      if (!selectedIds.has(s.id)) continue;
      ctx.strokeStyle = "#fff";
      ctx.lineWidth = 5;
      outlinePath(s);
      ctx.stroke();
      ctx.strokeStyle = selectionColor;
      ctx.lineWidth = 2;
      ctx.stroke();
    }
    const hoveredShape = shapes.find((shape) => shape.id === hoverId);
    if (hoveredShape) {
      const s = hoveredShape;
      ctx.strokeStyle = "#ffc66d";
      ctx.lineWidth = 2;
      ctx.setLineDash([6, 3]);
      // Hover traces the same edge as the selection outline, so moving between them doesn't shift.
      outlinePath(s);
      ctx.stroke();
      ctx.setLineDash([]);
    }
    if (layoutCurrent && paintVersion === version)
      untrack(() => onRendered?.(nodes));
  });
  function clearHover() {
    if (!hover) return;
    hover = null;
    onHover?.(null);
  }
  function moveHover(e: MouseEvent) {
    const next = hitNode(e) ?? null;
    if (next === hover) return;
    hover = next;
    onHover?.(
      next && next.kind !== "aggregate" ? createHoverTarget(next, byId) : null,
    );
  }
  onDestroy(clearHover);
  function hitNode(e: MouseEvent) {
    if (!layoutCurrent) return undefined;
    const rect = canvas!.getBoundingClientRect(),
      x = e.clientX - rect.left,
      y = e.clientY - rect.top;
    const shape = hitShape(
      shapes,
      mode,
      dimensions.width,
      dimensions.height,
      x,
      y,
    );
    return shape ? byId.get(shape.id) : undefined;
  }
</script>

<div
  class="map-wrap"
  data-color-mode={colorMode}
  data-highlighted-file-type={highlightedFileType ?? undefined}
  data-file-type-highlight-count={highlightActive ? highlightedIds.size : 0}
  data-file-type-highlight-pending={fileTypeHighlightPending}
  data-queued-count={visibleMarks.queued}
  data-contains-count={visibleMarks.contains}
  data-hovered-id={hoverId ?? undefined}
>
  <!-- svelte-ignore a11y_no_interactive_element_to_noninteractive_role (The canvas is a pointer-only picture of the folder; keyboard users act through the list, and the description below mirrors the map.) -->
  <canvas
    bind:this={canvas}
    width="0"
    height="0"
    role="img"
    aria-label={`${mode === "treemap" ? "Treemap" : "Sunburst"} disk usage map`}
    aria-describedby={summaryId}
    title={captionNode?.path ||
      (typedGroup(captionNode ?? undefined)
        ? folderFor(captionNode ?? undefined)?.path
        : undefined)}
    onmousemove={moveHover}
    onmouseleave={clearHover}
    onclick={(e) => {
      let n: NodeView | undefined;
      if (startsClickPair(e.detail)) {
        n = hitNode(e);
        const folder = folderFor(n);
        clickCapture = {
          scanId,
          rootId,
          mode,
          colorMode,
          x: e.clientX,
          y: e.clientY,
          target: n
            ? {
                id: n.id,
                kind: n.kind,
                path: n.path,
                parentId: n.parentId,
                fileType: n.fileType,
              }
            : null,
          folder: folder
            ? { id: folder.id, path: folder.path, parentId: folder.parentId }
            : null,
        };
      } else n = capturedNode();
      if (typedGroup(n)) {
        if (startsClickPair(e.detail)) onFileTypeSelect?.(n!.fileType!);
      } else if (n && n.kind !== "aggregate")
        onSelect(n, e.metaKey || e.ctrlKey);
      // Some browsers emit dblclick only for count 2. Finish later pairs here;
      // consuming their capture also makes an extra native dblclick harmless.
      if (e.detail > 2 && !startsClickPair(e.detail)) {
        suppressNextDoubleClick = true;
        openCapturedFolder(capturedNode());
      }
    }}
    ondblclick={() => {
      openCapturedFolder(capturedNode());
    }}
  ></canvas>
  {#if centerNode && holeRadius >= 40 && layoutCurrent}
    <div
      class="map-center"
      style:width={`${holeRadius * 1.8}px`}
      style:font-size={`${Math.min(28, holeRadius * 0.28)}px`}
    >
      <strong>{compactBytes(centerNode.allocatedBytes, 1)}</strong>
      <span>On disk</span>
    </div>
  {/if}
  {#if !shapes.length}<div class="map-empty">
      The map fills in as files are found
    </div>{/if}
  <p id={summaryId} class="sr-only map-summary">{summaryText}</p>
  <p class="sr-only map-announcement" aria-live="polite" aria-atomic="true">
    {announcement}
  </p>
  {#if captionNode}
    <div class="map-caption" title={captionNode.path || undefined}>
      <span class="map-help"
          >{`${captionName} · ${compactBytes(captionNode.allocatedBytes)}${typedGroup(captionNode) ? ` · ${countLabel(captionNode.fileCount ?? 0, "file")} grouped by type` : captionNode.kind === "aggregate" ? " · grouped small items" : ""}${queueMarks.get(captionNode.id) === "queued" ? " · Queued" : queueMarks.get(captionNode.id) === "contains" ? " · Contains queued items" : ""}${hoverId !== captionNode.id ? ` · Map shows ${byId.get(hoverId!)?.name ?? "parent folder"}` : ""}`}</span
      >
    </div>
  {/if}
</div>
<div class="map-legend-bar">
{#if (colorMode === "fileType" || (highlightedFileType !== null && fileTypeHighlightPending))}
  <div
    class="map-legend type-legend"
    role="region"
    aria-label="Storage map legend"
  >
    {#if colorMode === "fileType"}<span
        >Click a type to highlight it · Double-click to open its folder</span
      >{/if}
    {#if highlightedFileType !== null && fileTypeHighlightPending}
      <span aria-live="polite"
        >Finding {fileTypeLabel(highlightedFileType)} matches…</span
      >
    {/if}
  </div>
{/if}
{#if colorMode === "folder" && legendNodes.length}
  <div class="map-legend" role="region" aria-label="Storage map legend">
    <!-- svelte-ignore a11y_no_noninteractive_tabindex (Keyboard users can scroll the folder list while the disclosure stays visible.) -->
    <ul tabindex="0" aria-label="Folder sizes">
      {#each legendVisible as node (node.id)}
        <li
          class:linked-hover={hoverId !== null &&
            (hoverGroup === node.id || activeTarget?.lineage.includes(node.id))}
          title={`${node.name} · ${compactBytes(node.allocatedBytes)}`}
        >
          <span
            class="legend-dot"
            style:background={branchPalette.get(node.id) ?? neutralColor()}
            aria-hidden="true"
          ></span>
          <span class="legend-name">{node.name}</span>
          <span class="legend-size">{compactBytes(node.allocatedBytes, 1)}</span>
        </li>
      {/each}
    </ul>
    {#if legendNodes.length > 6}
      <button
        class="legend-more"
        aria-expanded={legendExpanded}
        onclick={() => (legendExpanded = !legendExpanded)}
      >
        {legendExpanded ? "Show less" : `+${legendNodes.length - 6} more`}
      </button>
    {/if}
  </div>
{/if}
</div>

<style>
  .map-wrap {
    padding-bottom: 6px;
  }
  .map-center {
    position: absolute;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    pointer-events: none;
    color: var(--text);
    text-align: center;
  }
  .map-center strong {
    font-weight: 650;
    white-space: nowrap;
    font-size: inherit;
  }
  .map-center span {
    color: var(--muted);
    font-size: var(--font-caption);
  }
  .map-caption {
    pointer-events: none;
    background: var(--panel);
    border-radius: var(--radius-xs);
    padding: 3px 6px;
    bottom: 8px;
  }
  .map-legend-bar {
    display: flex;
    align-items: center;
    flex: 0 0 auto;
    min-width: 0;
  }
  .map-legend {
    flex: 1 1 0;
    display: flex;
    align-items: center;
    gap: 14px;
    min-width: 0;
    min-height: 28px;
    padding: 0 12px 3px;
    font-size: var(--font-caption);
    white-space: nowrap;
  }
  .type-legend {
    flex-wrap: wrap;
    white-space: normal;
    color: var(--muted);
    gap: 4px 14px;
  }
  .map-legend ul {
    display: flex;
    align-items: center;
    gap: 16px;
    flex: 1 1 auto;
    min-width: 0;
    margin: 0;
    padding: 0;
    list-style: none;
    overflow-x: auto;
    /* Still scrolls by trackpad and keyboard; "+N more" is the visible way to see the rest. */
    scrollbar-width: none;
  }
  .map-legend ul::-webkit-scrollbar {
    display: none;
  }
  .map-legend ul:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
    border-radius: var(--radius-xs);
  }
  .map-legend li {
    display: flex;
    align-items: center;
    gap: 7px;
    flex-shrink: 0;
  }
  .map-legend li.linked-hover {
    color: var(--warning);
  }
  .legend-dot {
    width: 9px;
    height: 9px;
    flex-shrink: 0;
    border-radius: var(--radius-swatch);
  }
  .legend-name {
    max-width: 150px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .legend-size {
    color: var(--muted);
  }
  .legend-more {
    flex-shrink: 0;
    padding: 2px 5px;
    border: 0;
    background: transparent;
    color: var(--muted);
    font: inherit;
  }
</style>

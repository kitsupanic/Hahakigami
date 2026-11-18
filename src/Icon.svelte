<script lang="ts">
  let {
    name,
    size = 18,
    class: className = "",
  }: {
    name: string;
    size?: number;
    class?: string;
  } = $props();
  const paths: Record<string, string> = {
    copy: "M9 3h10a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H9a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2ZM4 7H3a1 1 0 0 0-1 1v12a2 2 0 0 0 2 2h11",
    target:
      "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18Zm0 5a4 4 0 1 0 0 8 4 4 0 0 0 0-8ZM12 11a1 1 0 1 0 0 2 1 1 0 0 0 0-2Z",
    home: "m3 11 9-8 9 8M5 10v11h5v-7h4v7h5V10",
    file: "M6 3h8l4 4v14H6V3Zm8 0v5h4M9 12h6m-6 4h6",
    queue: "M6 7h12l2 14H4L6 7Zm2 0V3h8v4M8 11h8m-8 4h8m-8 3h5",
    "queue-add": "M3 6h10M3 12h10M3 18h10M7 3v6m3 0v6m-4 0v6M18 3v6m-3-3h6",
    "folder-outline": "M3 7V4h6l2 3h10v13H3V7Zm0 0h8",
    filter: "M3 6h18M3 12h18M3 18h18M8 3v6m8 0v6M10 15v6",
    plus: "M12 4v16M4 12h16",
    minus: "M4 12h16",
    check: "m5 12 4 4L19 6",
    settings:
      "m10 3-.7 3-2 .9-2.8-.8L3 9l2.2 2v2L3 15l1.5 2.9 2.8-.8 2 .9.7 3h4l.7-3 2-.9 2.8.8L21 15l-2.2-2v-2L21 9l-1.5-2.9-2.8.8-2-.9L14 3h-4Z",
    chevron: "m9 5 7 7-7 7",
    trash: "M3 6h18M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7m4-7v7",
    search: "m16 16 5 5M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0Z",
    warning: "M12 3 2 21h20L12 3Zm0 6v5m0 3h.01",
    info: "M12 11v6m0-10h.01M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0Z",
    refresh: "M20 7V2m0 5h-5M20 7a9 9 0 1 0 1 8",
    eye: "M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z",
    treemap: "M3 3h18v18H3V3Zm0 10h18M13 3v10M9 13v8m7-8v8",
    sunburst: "M12 2v10h10M22 12a10 10 0 1 1-10-10M15 2.5V9h6.5",
    inspector: "M3 3h18v18H3V3Zm12 0v18M6 7h5m-5 4h5",
    "sidebar-left": "M3 3h18v18H3V3Zm6 0v18",
    "sidebar-right": "M3 3h18v18H3V3Zm12 0v18",
    close: "m5 5 14 14M5 19 19 5",
  };
</script>

<svg
  xmlns="http://www.w3.org/2000/svg"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  class={`icon icon-${name} ${className}`}
  aria-hidden="true"
  fill="none"
  stroke="currentColor"
  stroke-width="1.5"
  stroke-linecap="round"
  stroke-linejoin="round"
>
  {#if name === "folder"}
    <path
      class="folder-back"
      d="M2 7V5.5A1.5 1.5 0 0 1 3.5 4h5.2l2 2H20a2 2 0 0 1 2 2v10.5A1.5 1.5 0 0 1 20.5 20h-17A1.5 1.5 0 0 1 2 18.5Z"
      style="fill: color-mix(in srgb, var(--branch-color, var(--folder-icon)) 80%, black); stroke: color-mix(in srgb, var(--branch-color, var(--folder-icon)) 75%, black); stroke-width: .8"
    />
    <path
      class="folder-face"
      d="M3.5 7.3h17A1.5 1.5 0 0 1 22 8.8v9.7a1.5 1.5 0 0 1-1.5 1.5h-17A1.5 1.5 0 0 1 2 18.5V8.8a1.5 1.5 0 0 1 1.5-1.5Z"
      style="fill: var(--branch-color, var(--folder-icon)); stroke: color-mix(in srgb, var(--branch-color, var(--folder-icon)) 76%, black); stroke-width: .8"
    />
    <path
      class="folder-sheen"
      d="M3.5 7.8h17a1 1 0 0 1 1 1v5H2.5v-5a1 1 0 0 1 1-1Z"
      style="fill: white; fill-opacity: .1; stroke: none"
    />
    <path
      class="folder-highlight"
      d="M3 18.4V8.9a.7.7 0 0 1 .7-.7h16.6"
      style="fill: none; stroke: white; stroke-opacity: .42; stroke-width: .7"
    />
  {:else if name === "disk"}
    <path
      class="disk-shadow"
      d="M3 19.2h18v1.5a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1Z"
      fill="#515e68"
      stroke="#45525d"
      stroke-width=".7"
    />
    <path
      class="disk-chassis"
      d="M5.5 2.5h13a1 1 0 0 1 1 .9l2 14.8a1.2 1.2 0 0 1-1.2 1.4H3.7a1.2 1.2 0 0 1-1.2-1.4l2-14.8a1 1 0 0 1 1-.9Z"
      fill="#bdc9d0"
      stroke="#74848f"
      stroke-width=".8"
    />
    <path
      d="M5.8 3.4h12.4l1.8 14.7H4Z"
      fill="#d3dde2"
      stroke="#edf4f7"
      stroke-width=".6"
    />
    <path d="M7 3.3h10v2.3H7Z" fill="#687984" stroke="none" />
    <path d="M8 3.8h6" stroke="#b1bec6" stroke-width=".6" />
    <ellipse
      cx="12"
      cy="11.8"
      rx="5.4"
      ry="4.8"
      fill="#b7c6ce"
      stroke="#8d9faa"
      stroke-width=".65"
    />
    <path d="M8 9.5a4.5 4.5 0 0 1 7-1" stroke="#e9f1f4" stroke-width=".8" />
    <circle
      cx="12"
      cy="11.8"
      r="1.35"
      fill="#d5dfe4"
      stroke="#8c9ea8"
      stroke-width=".6"
    />
    <path d="m16.8 7-3.6 4.2" stroke="#7e909d" stroke-width="1" />
    <path d="M4.5 20.5h10" stroke="#a6b3bc" stroke-width=".6" />
    <circle cx="18.8" cy="20.6" r=".6" fill="#a5e0ce" stroke="none" />
  {:else}
    <path d={paths[name] || paths.file} />
    {#if name === "settings"}<circle cx="12" cy="12" r="3" />{/if}
    {#if name === "eye"}<circle cx="12" cy="12" r="3" />{/if}
  {/if}
</svg>

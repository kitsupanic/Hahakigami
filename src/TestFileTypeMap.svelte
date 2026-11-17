<script lang="ts">
  let {
    highlightedFileType = null,
    fileTypeHighlights = [],
    fileTypeHighlightPending = false,
    colorMode = "folder",
    onFileTypeSelect,
  }: {
    highlightedFileType?: string | null;
    fileTypeHighlights?: readonly number[];
    fileTypeHighlightPending?: boolean;
    colorMode?: string;
    onFileTypeSelect?: (extension: string) => void;
  } = $props();
</script>

<div
  class="test-file-type-map"
  data-type={highlightedFileType ?? "none"}
  data-ids={fileTypeHighlights.join(",")}
  data-pending={fileTypeHighlightPending}
  data-color={colorMode}
>
  Map canvas
  <button onclick={() => onFileTypeSelect?.("jpg")}
    >Highlight grouped .jpg files</button
  >
</div>

import { theme } from "./theme.svelte";
interface FileTypeEntry {
  /** Canonical lower-case suffix without a dot; empty means no extension. */
  extension: string;
  files: number;
  logicalBytes: number;
  allocatedBytes: number;
}

export interface FileTypeStats {
  items: FileTypeEntry[];
  files: number;
  logicalBytes: number;
  allocatedBytes: number;
  incomplete: boolean;
}

/** Last suffix, matching Rust Path.extension for names (including dotfiles). */
export function fileExtension(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 && name !== ".." ? name.slice(dot + 1).toLowerCase() : "";
}

export function fileTypeLabel(extension: string): string {
  return extension ? `.${extension}` : "No extension";
}

const descriptions: Record<string, string> = {
  mov: "QuickTime video",
  mp4: "MPEG-4 video",
  m4v: "MPEG-4 video",
  mkv: "Matroska video",
  avi: "AVI video",
  webm: "WebM video",
  dmg: "Disk image",
  iso: "Disk image",
  zip: "ZIP archive",
  gz: "Gzip archive",
  tar: "TAR archive",
  "7z": "7-Zip archive",
  rar: "RAR archive",
  png: "PNG image",
  jpg: "JPEG image",
  jpeg: "JPEG image",
  gif: "GIF image",
  heic: "HEIF image",
  webp: "WebP image",
  svg: "SVG image",
  tiff: "TIFF image",
  pdf: "PDF document",
  txt: "Text document",
  md: "Markdown document",
  doc: "Word document",
  docx: "Word document",
  xlsx: "Excel workbook",
  csv: "CSV data",
  json: "JSON data",
  html: "HTML document",
  css: "Style sheet",
  js: "JavaScript source",
  ts: "TypeScript source",
  svelte: "Svelte source",
  rs: "Rust source",
  py: "Python source",
  mp3: "MP3 audio",
  wav: "WAV audio",
  flac: "FLAC audio",
  m4a: "MPEG-4 audio",
  db: "Database",
  sqlite: "SQLite database",
};

export function fileTypeDescription(extension: string): string {
  if (!extension) return "Files without an extension";
  return Object.hasOwn(descriptions, extension)
    ? descriptions[extension]
    : `${extension.toUpperCase()} file`;
}

const colors: Record<string, string> = {
  mov: "#8b6ce0",
  dmg: "#3989df",
  zip: "#bf8d20",
  mp4: "#239f90",
  png: "#c65c92",
  pdf: "#d15454",
  jpg: "#268eae",
  jpeg: "#268eae",
  "": "#818a9b",
};
const palette = [
  "#8b6ce0",
  "#3989df",
  "#bf8d20",
  "#239f90",
  "#c65c92",
  "#d15454",
  "#268eae",
  "#71834a",
];

// Guardian: the same slots in warm print inks (see guardianUsageColors).
const guardianColors: Record<string, string> = {
  mov: "#7e4d8a",
  dmg: "#316ca5",
  zip: "#ba882e",
  mp4: "#008a7b",
  png: "#bf6583",
  pdf: "#b75037",
  jpg: "#1f90af",
  jpeg: "#1f90af",
  "": "#8a8270",
};
// Validated order: neighbours stay distinguishable for colour-blind readers.
const guardianPalette = [
  "#b75037",
  "#008a7b",
  "#ba882e",
  "#316ca5",
  "#bf6583",
  "#7b8935",
  "#7e4d8a",
  "#1f90af",
];

/** Stable across scans, row ordering, folders, and supported map metrics. */
export function fileTypeColor(extension: string): string {
  const [table, inks] = theme.guardian
    ? [guardianColors, guardianPalette]
    : [colors, palette];
  const known = Object.hasOwn(table, extension)
    ? table[extension]
    : undefined;
  if (known) return known;
  let hash = 2166136261;
  for (let i = 0; i < extension.length; i++) {
    hash = Math.imul(hash ^ extension.charCodeAt(i), 16777619);
  }
  return inks[(hash >>> 0) % inks.length];
}

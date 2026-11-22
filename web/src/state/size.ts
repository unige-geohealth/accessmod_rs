// Byte sizes for display: decimal units (as the operating systems show
// files), one decimal from kB up.

const UNITS = ["kB", "MB", "GB", "TB"] as const;

/** `bytes` as "512 B", "1.5 MB", "8.8 GB". */
export function byteSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 1000) return `${Math.max(0, Math.round(bytes) || 0)} B`;
  let value = bytes;
  let unit: (typeof UNITS)[number] = "kB";
  for (const u of UNITS) {
    value /= 1000;
    unit = u;
    if (value < 1000) break;
  }
  return `${value.toFixed(1)} ${unit}`;
}

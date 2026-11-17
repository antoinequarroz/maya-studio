import type { Rgba } from "@suite-media/client";

const hex2 = (n: number) => Math.max(0, Math.min(255, Math.round(n))).toString(16).padStart(2, "0");

/** `#rrggbb` (sans l'alpha, pour <input type="color">). */
export function rgbaToHex([r, g, b]: Rgba): string {
  return `#${hex2(r)}${hex2(g)}${hex2(b)}`;
}

export function hexToRgba(hex: string, alpha: number): Rgba {
  const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex);
  if (!m) return [0, 0, 0, alpha];
  return [parseInt(m[1]!, 16), parseInt(m[2]!, 16), parseInt(m[3]!, 16), alpha];
}

export function rgbaToCss([r, g, b, a]: Rgba): string {
  return `rgba(${r}, ${g}, ${b}, ${(a / 255).toFixed(3)})`;
}

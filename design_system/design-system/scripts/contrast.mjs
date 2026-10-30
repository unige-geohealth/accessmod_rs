// WCAG contrast of the text/background token pairs, both themes.
// Fails (exit 1) under 4.5:1 for text, 3:1 for large text and UI marks.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const css = readFileSync(join(here, "../src/styles/tokens.css"), "utf8");

const block = (sel) => {
  const body = css.match(new RegExp(`${sel.replace(".", "\\.")}\\s*\\{([^}]*)\\}`))[1];
  return Object.fromEntries([...body.matchAll(/--([\w-]+):\s*(#[0-9a-f]{6})/gi)].map((m) => [m[1], m[2]]));
};
const light = block(":root");
const themes = { light, dark: { ...light, ...block(".dark") } };

const lum = (hex) => {
  const [r, g, b] = [1, 3, 5].map((i) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const ratio = (a, b) => {
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
};

// [foreground, background, minimum]
const pairs = [
  ["foreground", "background", 4.5],
  ["muted-foreground", "background", 4.5],
  ["foreground", "well", 4.5],
  ["muted-foreground", "well", 4.5],
  ["panel-heading-foreground", "panel-heading", 4.5],
  ["link", "background", 4.5],
  ["link", "well", 3],
  ["button-foreground", "button", 4.5],
  ["button-foreground", "button-active", 4.5],
  ["sidebar-foreground", "sidebar", 4.5],
  ["sidebar-muted", "sidebar", 4.5],
  ["sidebar-accent-foreground", "sidebar-accent", 4.5],
  ["danger-foreground", "danger", 4.5],
  ["callout-foreground", "callout", 4.5],
  ["primary-foreground", "primary", 4.5],
  ["foreground", "table-row-alt", 4.5],
  ["foreground", "table-row-selected", 4.5],
  ["warning", "background", 3],
  ["input", "input-background", 1.5],
];

let failed = 0;
for (const [name, t] of Object.entries(themes)) {
  console.log(`\n${name}`);
  for (const [fg, bg, min] of pairs) {
    const r = ratio(t[fg], t[bg]);
    const ok = r >= min;
    if (!ok) failed++;
    console.log(`${ok ? "ok  " : "FAIL"} ${r.toFixed(2).padStart(5)}  ${fg} on ${bg} (min ${min})`);
  }
}
process.exit(failed ? 1 : 0);

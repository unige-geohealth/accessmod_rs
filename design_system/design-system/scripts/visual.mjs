// Screenshot every gallery route (light + dark, one language) and write
// visual/report.html: AM5 reference | rebuild light | rebuild dark.
//
//   npm run dev            # in another shell
//   npm run visual         # GALLERY_URL, LANG_CODE (default en)
//
// No pixel threshold: the goal is closeness, judged by eye.

import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const here = dirname(fileURLToPath(import.meta.url));
const BASE = process.env.GALLERY_URL ?? "http://localhost:5180/";
const LANG = process.env.LANG_CODE ?? "en";
const OUT = resolve(join(here, "../visual"));
const REFS = resolve(join(here, "../../references/screens"));

// route ids, read from the gallery registry
const routes = [...(await readFile(join(here, "../src/gallery/routes.tsx"), "utf8")).matchAll(/\{ id: "([^"]+)"/g)].map((m) => m[1]);

const browser = await chromium.launch();
await mkdir(OUT, { recursive: true });
const rows = [];
for (const theme of ["light", "dark"]) {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  await ctx.addInitScript(
    ([t, l]) => {
      localStorage.setItem("am.theme", t);
      localStorage.setItem("am.lang", l);
    },
    [theme, LANG],
  );
  const page = await ctx.newPage();
  for (const id of routes) {
    await page.goto(`${BASE}?chrome=0#/${id}`);
    await page.waitForLoadState("networkidle");
    await page.waitForTimeout(500);
    await page.screenshot({ path: join(OUT, `${id}-${theme}.png`), fullPage: true });
    console.log(theme, id);
  }
  await ctx.close();
}
await browser.close();

for (const id of routes) {
  rows.push(`<h2>${id}</h2><div class="row">
  <figure><figcaption>AM5</figcaption><img src="${join(REFS, id, `${LANG}.png`)}" onerror="this.replaceWith('no reference')"></figure>
  <figure><figcaption>light</figcaption><img src="${id}-light.png"></figure>
  <figure><figcaption>dark</figcaption><img src="${id}-dark.png"></figure></div>`);
}
await writeFile(
  join(OUT, "report.html"),
  `<!doctype html><meta charset="utf-8"><title>AccessMod visual report</title>
<style>body{font-family:sans-serif;margin:16px}.row{display:grid;grid-template-columns:repeat(3,1fr);gap:8px}
figure{margin:0}img{width:100%;border:1px solid #ccc}figcaption{font-weight:bold}</style>
<h1>AM5 vs design system (${LANG})</h1>${rows.join("\n")}`,
);
console.log(`report: ${join(OUT, "report.html")}`);

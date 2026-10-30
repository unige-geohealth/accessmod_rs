// Capture AccessMod 5 reference material from a running instance.
//
//   node capture.mjs [screen-id ...]
//
// Env:
//   AM5_URL      default http://localhost:3080/
//   AM5_PROJECT  project to open for the capture, default "demo"
//   OUT_DIR      default ../../references
//   LANGS        comma list, default en,fr,es,pt
//   CROPS_ONLY=1 redo components/*.png only
//
// Outputs, per screen: <lang>.png (viewport 1440×900 full page, DPR 1),
// dom.html (visible DOM of the active tab, with data-amt_id i18n keys),
// aria.yaml (accessibility tree), meta.json (steps, keys, texts).
// Once: components/*.png (DPR 2 crops), tokens.am5.json (computed styles),
// index.json. The originally open project and language are restored at
// the end.

import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { components, languages, screens } from "./screens.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const URL = process.env.AM5_URL ?? "http://localhost:3080/";
const PROJECT = process.env.AM5_PROJECT ?? "demo";
const OUT = resolve(process.env.OUT_DIR ?? join(here, "../../references"));
const LANGS = process.env.LANGS ? process.env.LANGS.split(",") : languages;
const only = process.argv.slice(2);
const CROPS_ONLY = process.env.CROPS_ONLY === "1";
const VIEWPORT = { width: 1440, height: 900 };

const log = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a);

// Wait for Shiny to settle. A timeout is logged, not thrown: some outputs
// (e.g. zonal statistics) compute for minutes; the shot then shows that state.
let idleTimeout = 60000;
async function idle(page, extra = 600) {
  const ok = await page
    .waitForFunction(
      () =>
        window.Shiny?.shinyapp?.isConnected?.() &&
        !document.documentElement.classList.contains("shiny-busy") &&
        ![...document.querySelectorAll(".recalculating")].some((e) => e.offsetParent && e.offsetHeight > 0),
      null,
      { timeout: idleTimeout, polling: 200 },
    )
    .then(
      () => true,
      () => false,
    );
  if (!ok) log("  warn: still busy after", idleTimeout / 1000, "s");
  await page.waitForTimeout(extra);
  // Leaflet tiles, best effort
  await page
    .waitForFunction(() => document.querySelectorAll(".leaflet-tile-loading").length === 0, null, { timeout: 8000 })
    .catch(() => {});
}

async function boot(page) {
  await page.goto(URL);
  await idle(page, 2500);
}

async function currentState(page) {
  return page.evaluate(() => ({
    project: document.querySelector("#selectProject")?.value,
    language: window.am?.settings?.language ?? "en",
  }));
}

async function openProject(page, name) {
  const cur = await page.evaluate(() => document.querySelector("#selectProject")?.value);
  if (cur === name) return;
  log("open project", name);
  await page.evaluate((n) => document.querySelector("#selectProject").selectize.setValue(n), name);
  await page.waitForTimeout(1500);
  await idle(page, 3000);
}

async function setLanguage(page, lang) {
  await page.evaluate((l) => {
    const s = document.querySelector("#selectLanguage");
    if (s?.selectize) s.selectize.setValue(l);
    else window.Shiny.setInputValue("selectLanguage", l);
  }, lang);
  await page.waitForTimeout(400);
  await idle(page, 800);
}

async function applyStep(page, step) {
  if (step.tab) {
    await page.click(`.sidebar-menu a[data-value='${step.tab}']`);
    await page.waitForSelector(`#shiny-tab-${step.tab}.active`, { state: "visible" });
    await page.waitForTimeout(1500); // select options arrive after the tab shows
  } else if (step.fill) {
    // Pick the first option of each empty visible select, a few rounds since
    // a choice can reveal more inputs. Selection only, nothing is computed.
    for (let round = 0; round < 4; round++) {
      const n = await page.evaluate(() => {
        const pane = document.querySelector("[id^='shiny-tab-'].active");
        let changed = 0;
        for (const s of pane.querySelectorAll("select.selectized")) {
          const z = s.selectize;
          if (!z || !s.parentElement.offsetParent || z.getValue()?.length) continue;
          const first = Object.keys(z.options)[0];
          if (first === undefined) continue;
          z.setValue(first);
          changed++;
        }
        return changed;
      });
      if (!n) break;
      await idle(page, 1200);
    }
  } else if (step.radio) {
    await page.click(`#${step.radio} label:has(input[value='${step.value}'])`);
  } else if (step.panel) {
    const panel = page.locator(`${step.panel} > .panel`).nth(step.index - 1);
    const open = await panel.locator(".panel-collapse.in").count();
    if (!open) {
      await panel.locator(".panel-title a").click();
      await page.waitForTimeout(450); // bootstrap collapse transition
    }
  } else if (step.subtab) {
    await page.click(`[id^='shiny-tab-'].active .nav-tabs a:has([data-amt_id='${step.subtab}'])`);
    await page.waitForTimeout(400);
  } else if (step.select) {
    const ok = await page.evaluate(
      ([sel, re]) => {
        const z = document.querySelector(sel)?.selectize;
        const key = z && Object.keys(z.options).find((k) => new RegExp(re).test(z.options[k].label ?? k));
        if (key === undefined) return false;
        z.setValue(key);
        return true;
      },
      [step.select, step.label],
    );
    if (!ok) log("  warn: no option", step.label, "in", step.select);
    await page.waitForTimeout(800);
  } else if (step.input) {
    await page.fill(step.input, step.value);
    await page.press(step.input, "Tab");
  } else if (step.check) {
    const box = page.locator(step.check);
    if (!(await box.isChecked())) await box.check();
  } else if (step.click) {
    await page.click(step.click);
  }
  await idle(page);
}

// Collapse every accordion item so each screen starts from a known state,
// then let steps open what they need. Default-open items are reopened by
// AM5 itself only on load, so screens that rely on them list them.
async function reset(page) {
  await page.evaluate(() => {
    for (const el of document.querySelectorAll(".panel-group .panel-collapse.in")) {
      window.jQuery(el).collapse("hide");
    }
  });
  await page.waitForTimeout(450);
}

async function extract(page) {
  return page.evaluate(() => {
    const pane = document.querySelector("[id^='shiny-tab-'].active");
    const visible = (el) => el.checkVisibility?.({ checkOpacity: false, checkVisibilityCSS: true }) ?? !!el.offsetParent;

    // mark hidden subtrees, clone, strip
    const hidden = [...pane.querySelectorAll("*")].filter((el) => !visible(el));
    for (const el of hidden) el.setAttribute("data-cap-hidden", "");
    const clone = pane.cloneNode(true);
    for (const el of hidden) el.removeAttribute("data-cap-hidden");
    clone.querySelectorAll("[data-cap-hidden], script, style, noscript").forEach((el) => el.remove());
    clone.querySelectorAll(".leaflet-tile-container, .leaflet-overlay-pane svg path").forEach((el) => el.remove());
    clone.querySelectorAll(".tabulator-tableholder .tabulator-table").forEach((t) => {
      [...t.children].slice(15).forEach((r) => r.remove());
    });
    clone.querySelectorAll("img[src^='data:']").forEach((i) => i.setAttribute("src", "data:…"));

    const keys = [...document.querySelectorAll("[data-amt_id]")]
      .filter((el) => visible(el))
      .map((el) => ({ key: el.dataset.amt_id, text: el.innerText.trim() }));
    const uniq = [...new Map(keys.map((k) => [k.key, k])).values()];

    return {
      html: clone.outerHTML,
      i18n: uniq,
      icons: [...new Set([...pane.querySelectorAll("i.fa, i.fas, i.far, i[class*='fa-']")]
        .filter(visible)
        .flatMap((i) => [...i.classList].filter((c) => c.startsWith("fa-") && c !== "fa-fw")))],
      title: document.querySelector("#projName")?.innerText,
      size: { width: document.documentElement.scrollWidth, height: document.documentElement.scrollHeight },
    };
  });
}

// Computed styles of representative elements → design tokens input.
async function tokens(page) {
  const probes = {
    body: "body",
    header: ".main-header .navbar",
    headerLogo: ".main-header .logo",
    sidebar: ".main-sidebar",
    sidebarTitle: ".main-sidebar .amCenterTitle h4",
    sidebarProject: "#projName",
    sidebarItem: ".sidebar-menu li:not(.active) > a",
    sidebarItemActive: ".sidebar-menu li.active > a",
    sidebarHr: ".main-sidebar hr",
    sidebarVersion: "#amVersion",
    content: ".content-wrapper",
    centerTitle: "[id^='shiny-tab-'].active h2",
    centerSub: "[id^='shiny-tab-'].active h2 + p",
    hr: "[id^='shiny-tab-'].active hr",
    panel: "[id^='shiny-tab-'].active .panel-group .panel",
    panelHeading: "[id^='shiny-tab-'].active .panel-heading",
    panelTitle: "[id^='shiny-tab-'].active .panel-title a",
    panelBody: "[id^='shiny-tab-'].active .panel-collapse.in .panel-body",
    label: "[id^='shiny-tab-'].active .control-label",
    selectize: "[id^='shiny-tab-'].active .selectize-input",
    input: "[id^='shiny-tab-'].active input.form-control",
    button: "[id^='shiny-tab-'].active .btn-default:not(.active)",
    buttonActive: "[id^='shiny-tab-'].active .btn-default.active",
    link: "[id^='shiny-tab-'].active a:not(.btn):not([data-toggle])",
    tabulator: "[id^='shiny-tab-'].active .tabulator",
    tabulatorHeader: "[id^='shiny-tab-'].active .tabulator-header",
    tabulatorCol: "[id^='shiny-tab-'].active .tabulator-col",
    tabulatorRow: "[id^='shiny-tab-'].active .tabulator-row:not(.tabulator-row-even)",
    tabulatorRowEven: "[id^='shiny-tab-'].active .tabulator-row.tabulator-row-even",
    tabulatorCell: "[id^='shiny-tab-'].active .tabulator-cell",
    well: "[id^='shiny-tab-'].active .well",
    checkbox: "[id^='shiny-tab-'].active .checkbox label",
  };
  const props = [
    "font-family", "font-size", "font-weight", "line-height", "color", "background-color",
    "border-top", "border-bottom", "border-left", "border-right", "border-radius", "box-shadow",
    "padding", "margin", "width", "height", "text-transform", "letter-spacing",
  ];
  return page.evaluate(
    ({ probes, props }) => {
      const out = {};
      for (const [name, sel] of Object.entries(probes)) {
        const el = [...document.querySelectorAll(sel)].find((e) => e.offsetParent || e === document.body);
        if (!el) continue;
        const cs = getComputedStyle(el);
        const r = el.getBoundingClientRect();
        out[name] = { selector: sel, rect: { w: Math.round(r.width), h: Math.round(r.height) } };
        for (const p of props) out[name][p] = cs.getPropertyValue(p);
      }
      return out;
    },
    { probes, props },
  );
}

async function gotoScreen(page, screen) {
  idleTimeout = screen.idleTimeout ?? 60000;
  await reset(page);
  for (const step of screen.steps) await applyStep(page, step);
  await page.mouse.move(0, VIEWPORT.height - 1);
  await page.evaluate(() => {
    document.activeElement?.blur();
    window.scrollTo(0, 0);
  });
}

const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 1, locale: "en-US" });
const page = await ctx.newPage();
await boot(page);
const initial = await currentState(page);
log("initial state", initial);

const index = [];
const allTokens = {};
try {
  await openProject(page, PROJECT);
  const todo = CROPS_ONLY ? [] : only.length ? screens.filter((s) => only.includes(s.id)) : screens;

  for (const screen of todo) {
    log("screen", screen.id);
    const dir = join(OUT, "screens", screen.id);
    await mkdir(dir, { recursive: true });
    await setLanguage(page, "en");
    await gotoScreen(page, screen);

    const data = await extract(page);
    await writeFile(join(dir, "dom.html"), `<!-- AM5 ${screen.name}: visible DOM of the active tab -->\n${data.html}\n`);
    const aria = await page.locator("[id^='shiny-tab-'].active").ariaSnapshot().catch((e) => `# ${e.message}`);
    await writeFile(join(dir, "aria.yaml"), aria);
    Object.assign(allTokens, Object.fromEntries(Object.entries(await tokens(page)).filter(([k]) => !(k in allTokens))));

    const texts = {};
    for (const lang of LANGS) {
      if (lang !== "en") await setLanguage(page, lang);
      await page.screenshot({ path: join(dir, `${lang}.png`), fullPage: true });
      if (lang !== "en") texts[lang] = (await extract(page)).i18n.map((k) => k.text);
    }

    const meta = {
      id: screen.id,
      name: screen.name,
      am5Version: await page.evaluate(() => document.querySelector("#amVersion")?.innerText.trim()),
      project: data.title,
      url: URL,
      viewport: VIEWPORT,
      page: data.size,
      steps: screen.steps,
      source: screen.source,
      icons: data.icons,
      i18n: data.i18n.map((k, i) => ({ ...k, ...Object.fromEntries(Object.entries(texts).map(([l, t]) => [l, t[i]])) })),
      capturedAt: new Date().toISOString(),
    };
    await writeFile(join(dir, "meta.json"), `${JSON.stringify(meta, null, 2)}\n`);
    index.push({ id: screen.id, name: screen.name, source: screen.source, page: data.size });
  }

  // Component crops at DPR 2
  if (!only.length) {
    await setLanguage(page, "en");
    const hi = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 2 });
    const p2 = await hi.newPage();
    await boot(p2);
    await openProject(p2, PROJECT);
    await mkdir(join(OUT, "components"), { recursive: true });
    let at = null;
    for (const [name, sel, screenId] of components) {
      if (screenId !== at) {
        await gotoScreen(p2, screens.find((s) => s.id === screenId));
        at = screenId;
      }
      const loc = p2.locator(sel).filter({ visible: true }).first();
      if (!(await loc.count()) || !(await loc.isVisible())) {
        log("component missing", name, sel);
        continue;
      }
      await loc.screenshot({ path: join(OUT, "components", `${name}.png`) });
      log("component", name);
    }
    await hi.close();
  }
  if (!only.length && !CROPS_ONLY) {
    await writeFile(join(OUT, "tokens.am5.json"), `${JSON.stringify(allTokens, null, 2)}\n`);
    await writeFile(join(OUT, "index.json"), `${JSON.stringify({ capturedAt: new Date().toISOString(), url: URL, project: PROJECT, languages: LANGS, screens: index }, null, 2)}\n`);
  }
} finally {
  // restore what the user had open
  await setLanguage(page, initial.language).catch(() => {});
  if (initial.project) await openProject(page, initial.project).catch(() => {});
  await page.evaluate(() => {
    for (const id of ["showDevelTools", "showAdminTools"]) {
      const el = document.getElementById(id);
      if (el?.checked) el.click();
    }
  }).catch(() => {});
  await idle(page).catch(() => {});
  await browser.close();
  log("done; restored", initial);
}

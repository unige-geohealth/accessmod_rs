// One-off probe: dump navigable structure of the running AM5 instance.
import { chromium } from "playwright";
const URL = process.env.AM5_URL ?? "http://localhost:3080/";
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
await page.goto(URL);
await page.waitForFunction(() => window.Shiny && Shiny.shinyapp && !document.documentElement.classList.contains("shiny-busy"), null, { timeout: 60000 });
await page.waitForTimeout(3000);
const info = await page.evaluate(() => {
  const txt = (e) => e.innerText.trim().replace(/\s+/g, " ");
  return {
    project: txt(document.querySelector("#projName") ?? document.body).slice(0, 40),
    lang: window.am?.settings?.language,
    sidebar: [...document.querySelectorAll(".sidebar-menu a")].map((a) => ({ text: txt(a), href: a.getAttribute("href"), tab: a.dataset.value })),
    moduleSelector: [...document.querySelectorAll("#moduleSelector input")].map((i) => ({ value: i.value, text: txt(i.parentElement) })),
    toolbox: [...document.querySelectorAll("#module_toolbox, [data-value='module_toolbox']")].length,
    radioGroups: [...document.querySelectorAll("[data-toggle='buttons']")].map((g) => ({ id: g.id, items: [...g.querySelectorAll("input")].map((i) => i.name + "=" + i.value) })),
    accordions: [...document.querySelectorAll(".panel-group")].map((g) => ({ id: g.id, titles: [...g.querySelectorAll(":scope > .panel .panel-title")].map(txt) })),
    fonts: getComputedStyle(document.body).fontFamily,
    bodySize: getComputedStyle(document.body).fontSize,
    amtCount: document.querySelectorAll("[data-amt_id]").length,
  };
});
console.log(JSON.stringify(info, null, 2));
await page.screenshot({ path: process.env.OUT ?? "/tmp/probe.png" });
await browser.close();

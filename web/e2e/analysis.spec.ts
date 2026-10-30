import { readFileSync } from "node:fs";
import { expect, test } from "@playwright/test";
import { command, finished, importFile, panel, shot, sidebar } from "./helpers";

// Analysis screens on the demo project. The demo has no merged land cover
// and no scenario table: both are made through the API first, as a user
// would with Toolbox › Merge land cover and Data › Import.
const project = "demo";

const scenario = `class,label,speed,mode
1,Built_area,5,WALKING
2,Low_dense_vegetation,4,WALKING
3,Dense_vegetation,2,WALKING
1001,main road,80,MOTORIZED
1002,secondary road,60,MOTORIZED
1003,tertiary road,40,MOTORIZED
`;

test.beforeAll(async ({ request }) => {
  await command(request, "demo.reset", {});
  const run = await command<{ id: number }>(request, "run.start", {
    project,
    analysis: "merge-land-cover",
    tags: ["merged"],
    document: {
      schema_version: 1,
      dem: "datasets/1.tif",
      stack: [
        { type: "road", path: "datasets/8.fgb", class_field: "class_n", label_field: "label" },
        { type: "land_cover", path: "datasets/2.tif" },
        { type: "barrier", path: "datasets/4.fgb" },
        { type: "barrier", path: "datasets/5.fgb" },
      ],
      clean_bridges: true,
    },
  });
  await finished(request, project, run.id);
  await importFile(request, project, "tScenario", ["walk"], "scenario.csv", scenario);
});

// The demo project as it was built, for the other tests.
test.afterAll(async ({ request }) => {
  await command(request, "demo.reset", {});
});

test("accessibility: scenario table, facility subset, run and exported job", async ({ page }) => {
  await page.goto(`/#/${project}/analysis/accessibility`);
  // The merged land cover and the scenario table are preselected.
  await page.getByRole("button", { name: "Import content from the selected scenario table" }).click();

  // Two facilities only: None, then check two rows.
  const facilities = page.locator(".tabulator", { hasText: "amOnBarrier" });
  await expect(facilities.locator(".tabulator-row").first()).toBeVisible();
  await page.getByRole("button", { name: "[None]" }).click();
  await expect(page.getByText(/^0 of \d+ facilities selected$/)).toBeVisible();
  for (const i of [0, 1]) await facilities.locator(".tabulator-row").nth(i).locator("input[type=checkbox]").check();
  await expect(page.getByText(/^2 of \d+ facilities selected$/)).toBeVisible();

  await page.getByLabel("Add short tags").fill("e2e");
  await expect(page.getByText("Ready to compute.")).toBeVisible({ timeout: 30_000 });
  await shot(page, "analysis-accessibility");

  // The exported job replays headless: the subset is explicit.
  const download = page.waitForEvent("download");
  await page.getByRole("button", { name: "Export job" }).click();
  const job = JSON.parse(readFileSync(await (await download).path(), "utf8"));
  expect(job.facilities.ids).toHaveLength(2);
  expect(job.scenario.find((e: { class: number }) => e.class === 1001)).toMatchObject({ speed_kmh: 80, mode: "motorized" });

  await page.getByRole("button", { name: "Compute", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("Process finished", { exact: true })).toBeVisible({ timeout: 120_000 });
  await expect(dialog).toContainText("[e2e]");
  await shot(page, "analysis-accessibility-finished");
  await page.keyboard.press("Escape");
  await page.evaluate(() => localStorage.setItem("am.theme", "dark"));
  await page.reload();
  await shot(page, "dark-analysis-accessibility");
  await page.evaluate(() => localStorage.setItem("am.theme", "light"));
  await page.reload();

  // Data › Filter last results: the outputs of this run only.
  await sidebar(page, "Data").click();
  await panel(page, "Filter").click();
  await page.getByLabel("Filter last results").click();
  await expect(page.getByLabel("Filter last results")).toBeChecked();
  const rows = page.locator(".tabulator .tabulator-row");
  await expect(rows.first()).toBeVisible();
  for (const row of await rows.all()) await expect(row).toContainText("e2e");
});

test("geographic coverage: capacity, zones, run", async ({ page }) => {
  await page.goto(`/#/${project}/analysis/coverage`);
  // The scenario of the accessibility test is kept per tab: import it again.
  await page.getByRole("button", { name: "Import content from the selected scenario table" }).click();
  await expect(page.locator(".tabulator", { hasText: "amPopCell" })).toBeVisible();
  await page.getByLabel("Generate zonal statistics (select zones layer in data input section)").click();
  await expect(page.getByLabel("Select zones layer (vector)")).toBeVisible();
  await page.getByLabel("Add short tags").fill("cov");
  await expect(page.getByText("Ready to compute.")).toBeVisible({ timeout: 30_000 });
  await shot(page, "analysis-coverage");
  await page.getByRole("button", { name: "Compute", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("Process finished", { exact: true })).toBeVisible({ timeout: 120_000 });
  await expect(dialog).toContainText("[cov]");
});

test("referral: From and To groups, closest pair, run", async ({ page }) => {
  await page.goto(`/#/${project}/analysis/referral`);
  await page.getByRole("button", { name: "Import content from the selected scenario table" }).click();
  const tables = page.locator(".tabulator", { hasText: "amOnBarrier" });
  await expect(tables).toHaveCount(2);
  // To: three facilities only.
  await page.getByRole("button", { name: "[None]" }).nth(1).click();
  for (const i of [0, 1, 2]) await tables.nth(1).locator(".tabulator-row").nth(i).locator("input[type=checkbox]").check();
  await expect(page.getByText(/^3 of \d+ facilities selected$/)).toBeVisible();
  await page.getByLabel("Limit the spatial analysis to the closest pair time-wise", { exact: false }).first().click();
  await page.getByLabel("Add short tags").fill("ref");
  await expect(page.getByText("Ready to compute.")).toBeVisible({ timeout: 30_000 });
  await shot(page, "analysis-referral");
  await page.getByRole("button", { name: "Compute", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("Process finished", { exact: true })).toBeVisible({ timeout: 120_000 });
  await expect(dialog).toContainText("[ref]");
});

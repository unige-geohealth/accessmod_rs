import { readFileSync } from "node:fs";
import { expect, test } from "@playwright/test";
import { background, choose, command, computed, finished, importFile, jobPanel, panel, shot, sidebar } from "./helpers";

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

  // Compute queues a job and leaves the form as it was: another scenario
  // is one change and one click away, with no progress bar in the way.
  const compute = page.getByRole("button", { name: "Compute", exact: true });
  await compute.click();
  // The job panel follows the job; closed, the form is as it was, with no progress bar.
  await expect(jobPanel(page)).toContainText("[e2e]");
  await background(page);
  await expect(page.getByRole("progressbar")).toHaveCount(0);
  // The job holds these tags (or, done already, its datasets do): Compute waits for others.
  await expect(page.getByText(/computes these tags|Datasets with these tags exist/).first()).toBeVisible({ timeout: 30_000 });
  await expect(compute).toBeDisabled();
  // The tags field offers the next free set: pick it.
  const tags = page.getByRole("combobox", { name: "Add short tags" });
  await tags.click();
  const free = page.getByRole("option", { name: /^e2e 2\s*free$/ });
  await expect(free).toBeVisible();
  await shot(page, "analysis-tags-suggestions");
  await free.click();
  await expect(tags).toHaveValue("e2e 2");
  await expect(compute).toBeEnabled({ timeout: 30_000 });
  await computed(page, project, "e2e 2", compute);
  await shot(page, "analysis-accessibility-finished");
  await page.evaluate(() => localStorage.setItem("am.theme", "dark"));
  await page.reload();
  await shot(page, "dark-analysis-accessibility");
  await page.evaluate(() => localStorage.setItem("am.theme", "light"));
  await page.reload();

  // Data › Filter last results: this run's job, inputs and outputs, selected.
  await sidebar(page, "Data").click();
  await panel(page, "Filter").click();
  await page.getByLabel("Filter last results").click();
  await expect(page.getByLabel("Filter last results")).toBeChecked();
  const rows = page.locator(".tabulator .tabulator-row");
  await expect(rows.first()).toBeVisible();
  await expect(rows.filter({ has: page.locator("[tabulator-field=role]", { hasText: /^Job$/ }) })).toHaveCount(1);
  await expect(rows.filter({ has: page.locator("[tabulator-field=role]", { hasText: /^Input$/ }) }).first()).toBeVisible();
  const outputs = rows.filter({ has: page.locator("[tabulator-field=role]", { hasText: /^Output$/ }) });
  await expect(outputs.first()).toBeVisible();
  for (const row of await outputs.all()) await expect(row).toContainText("e2e");
  for (const row of await rows.all()) await expect(row.locator("input[type=checkbox]")).toBeChecked();
});

test("accessibility: outputs already tagged are refused, or overwritten when Settings allow it", async ({ page }) => {
  await page.goto(`/#/${project}/analysis/accessibility`);
  await page.getByRole("button", { name: "Import content from the selected scenario table" }).click();
  await page.getByLabel("Add short tags").fill("e2e");
  await expect(page.getByText("Datasets with these tags exist", { exact: false })).toBeVisible({ timeout: 30_000 });
  await expect(page.getByRole("button", { name: "Compute", exact: true })).toBeDisabled();

  // Settings › Allow overwriting outputs; the analysis draft is kept.
  await sidebar(page, "Settings").click();
  await page.getByLabel("Allow overwriting outputs", { exact: false }).check();
  await sidebar(page, "Analysis").click();
  await expect(page.getByText("(overwrite warning)").first()).toBeVisible({ timeout: 30_000 });
  await expect(page.getByText("Datasets with these tags exist", { exact: false })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Compute", exact: true })).toBeEnabled({ timeout: 30_000 });
  await shot(page, "analysis-overwrite");
  await page.evaluate(() => localStorage.removeItem("am.overwrite"));
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
  await computed(page, project, "cov", page.getByRole("button", { name: "Compute", exact: true }));
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
  await computed(page, project, "ref", page.getByRole("button", { name: "Compute", exact: true }));
});

test("optimization: catchments of the coverage run, minimum per zone, run", async ({ page }) => {
  await page.goto(`/#/${project}/analysis/optimization`);
  // The catchments are the ones written by the geographic coverage test.
  await expect(page.getByLabel("Select catchment layer")).toContainText("[cov]");
  await page.getByLabel("Total number of facilities to select").fill("3");
  await page.getByLabel("Ensure minimum facilities per admin unit").click();
  await expect(page.getByLabel("Select zones layer (vector)")).toBeVisible();
  await page.getByLabel("Add short tags").fill("opt");
  await expect(page.getByText("Ready to compute.")).toBeVisible({ timeout: 30_000 });
  await shot(page, "analysis-optimization");
  await computed(page, project, "opt", page.getByRole("button", { name: "Compute", exact: true }));
});

test("scaling up: capacity table, a factor from the form, run", async ({ page }) => {
  await page.goto(`/#/${project}/analysis/scalingUp`);
  await page.getByRole("button", { name: "Import content from the selected scenario table" }).click();
  // The demo capacity table is read into the capacity rows.
  await expect(page.locator(".tabulator", { hasText: "capacity" }).first().locator(".tabulator-row")).not.toHaveCount(0);
  // Suitability: add the distance from the roads.
  await page.getByRole("button", { name: "Suitability index" }).click();
  await choose(page, "Select a factor for the suitability index", "Euclidean distance from features");
  await choose(page, "Select available layer", "(v) road [demo]");
  await page.getByRole("button", { name: "Add", exact: true }).first().click();
  const factors = page.locator(".tabulator", { hasText: "options" });
  await expect(factors.locator(".tabulator-row")).toHaveCount(2);
  await page.getByLabel("Number of new health facilities to locate [facility]").fill("3");
  await page.getByLabel("Add short tags").fill("up");
  // Information messages only: Compute is enabled.
  await expect(page.getByRole("button", { name: "Compute", exact: true })).toBeEnabled({ timeout: 30_000 });
  await shot(page, "analysis-scaling-up");
  await computed(page, project, "up", page.getByRole("button", { name: "Compute", exact: true }));
});

test("zonal statistics: Update fills the table in place, one column per time step, download", async ({ page }) => {
  await page.goto(`/#/${project}/analysis/zonal`);
  await choose(page, "Select travel time layer (raster)", "(r) travel time [e2e]");
  await page.getByLabel("Select maximum travel time [minutes]").fill("30, 60, abc, 30");
  await page.getByRole("button", { name: "Update", exact: true }).click();
  const table = page.locator(".tabulator", { hasText: "time_limit_s" });
  await expect(table.locator(".tabulator-row").first()).toBeVisible({ timeout: 60_000 });
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await shot(page, "analysis-zonal");
  await page.getByLabel("Format table to get individual columns for each time step").click();
  await expect(page.locator(".tabulator", { hasText: "coverage_percent_60min" })).toBeVisible();
  await shot(page, "analysis-zonal-wide");
  const download = page.waitForEvent("download");
  await page.getByRole("button", { name: "Download CSV" }).click();
  const csv = readFileSync(await (await download).path(), "utf8");
  expect(csv.split("\n")[0]).toBe("zone_id,label,time_limit_s,population,population_covered,coverage_percent");
});

import { execFileSync } from "node:child_process";
import { mkdirSync, rmSync } from "node:fs";
import { expect, test } from "@playwright/test";
import { choose, panel, shot, sidebar } from "./helpers";

// The tutorial data of the repository (docs/tutorial/data); facilities are
// also sent as a shapefile, to upload several files at once.
const data = new URL("../../docs/tutorial/data/", import.meta.url).pathname;
const work = new URL("../test-results/e2e-data/", import.meta.url).pathname;
const am5Archive = new URL("../../crates/accessmod-io/tests/data/am5_archive.zip", import.meta.url).pathname;
// The demo sample data, as AccessMod 5 ships it (ERDAS Imagine, XLSX).
const demo = new URL("../../crates/accessmod-io/demo/", import.meta.url).pathname;

const size = JSON.parse(execFileSync("gdalinfo", ["-json", `${data}dem.tif`], { encoding: "utf8" })).size as [number, number];

test.beforeAll(() => {
  rmSync(work, { recursive: true, force: true });
  mkdirSync(work, { recursive: true });
  execFileSync("ogr2ogr", ["-f", "ESRI Shapefile", `${work}facilities.shp`, `${data}facilities.gpkg`]);
});

test("the demo project opens first and is built again from the toolbox", async ({ page }) => {
  // A new server has the demo project, opened when the URL names none.
  await page.goto("/");
  await expect(page).toHaveURL(/#\/demo\/project/);
  await expect(page.locator("aside")).toContainText("Demo");
  await sidebar(page, "Data").click();
  const table = page.locator(".tabulator");
  await expect(table.locator(".tabulator-row")).toHaveCount(11);
  await shot(page, "demo-data");

  // ERDAS Imagine rasters and XLSX tables are uploads like any other.
  await panel(page, "Import").click();
  await choose(page, "Select data class", "population");
  await page.getByLabel("Add short tags").fill("again");
  await page.locator("input[type=file]").setInputFiles(`${demo}rPopulation__start.img`);
  await expect(page.getByText("Imported: population [again]")).toBeVisible({ timeout: 60_000 });
  await choose(page, "Select data class", "capacity");
  await page.getByLabel("Add short tags").fill("again");
  await page.locator("input[type=file]").setInputFiles(`${demo}tCapacity_scalingUp.xlsx`);
  await expect(page.getByText("Imported: capacity [again]")).toBeVisible({ timeout: 60_000 });
  await expect(table.locator(".tabulator-row")).toHaveCount(13);

  // Removed, the demo is missing: the project screen invites a reset.
  await sidebar(page, "Project").click();
  await panel(page, "Delete").click();
  await choose(page, "Select the project to delete", "Demo");
  await page.getByRole("button", { name: "Delete", exact: true }).last().click();
  await page.getByRole("dialog").getByRole("button", { name: "Delete" }).click();
  await expect(page).toHaveURL(/#\/_\/project/);
  await expect(page.getByText("The demo project is missing", { exact: false })).toBeVisible();
  await shot(page, "project-demo-missing");

  // Toolbox › Demo project: reset, then the demo is open again, as it was built.
  await page.getByRole("button", { name: "Demo project", exact: true }).click();
  await expect(page).toHaveURL(/#\/_\/toolbox\/demo/);
  await shot(page, "toolbox-demo");
  await page.getByRole("button", { name: "Reset the demo project" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Reset the demo project" }).click();
  await expect(page).toHaveURL(/#\/demo\/project/, { timeout: 60_000 });
  await sidebar(page, "Data").click();
  await expect(table.locator(".tabulator-row")).toHaveCount(11);
});

test("project and data screens on the tutorial data", async ({ page }) => {
  await page.goto("/#/_/project");
  await expect(sidebar(page, "Project")).toHaveAttribute("aria-current", "page");
  await shot(page, "project-empty");

  // Project › New / Import: the DEM is uploaded.
  await panel(page, "New / Import").click();
  await page.getByLabel("Enter a new project name (min 4 characters)").fill("Tutorial");
  await page.locator("input[type=file]").setInputFiles(`${data}dem.tif`);
  await expect(page).toHaveURL(/#\/tutorial\/project/, { timeout: 60_000 });
  await expect(page.locator("aside")).toContainText("Tutorial");

  // Summary: grid parameters of the DEM.
  await panel(page, "Grid parameters").click();
  await expect(page.getByText(String(size[0] * size[1]), { exact: true })).toBeVisible();
  await shot(page, "project-summary-grid");
  await panel(page, "Location map").click();
  await shot(page, "project-location-map");

  // Data › Import: land cover, then facilities as a shapefile.
  await sidebar(page, "Data").click();
  await panel(page, "Import").click();
  await choose(page, "Select data class", "land cover merged");
  await page.getByLabel("Add short tags").fill("base");
  await page.locator("input[type=file]").setInputFiles(`${data}land_cover.tif`);
  await expect(page.getByText("Imported: land cover merged [base]")).toBeVisible({ timeout: 60_000 });
  await choose(page, "Select data class", "facility");
  await page.getByLabel("Add short tags").fill("health");
  await page
    .locator("input[type=file]")
    .setInputFiles(["shp", "shx", "dbf", "prj"].map((ext) => `${work}facilities.${ext}`));
  await expect(page.getByText("Imported: facility [health]")).toBeVisible({ timeout: 60_000 });
  const table = page.locator(".tabulator");
  await expect(table.getByRole("gridcell", { name: "health", exact: true })).toBeVisible();
  await shot(page, "data-import");

  // Data › Filter: vectors only.
  await panel(page, "Filter").click();
  await page.getByRole("radio", { name: "Vector" }).click();
  await expect(table.locator(".tabulator-row")).toHaveCount(1);
  await expect(page).toHaveURL(/type=vector/);
  await shot(page, "data-filter");
  await page.getByRole("radio", { name: "All" }).click();
  await expect(table.locator(".tabulator-row")).toHaveCount(3);

  // Data › Rename: edit the tags in the table, then commit.
  await panel(page, "Rename").click();
  await table.getByRole("gridcell", { name: "health", exact: true }).click();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("health centres");
  await page.keyboard.press("Enter");
  await page.getByRole("button", { name: "Update modified tag(s)" }).click();
  await expect(page.getByText("Renamed: facility [health] → [health centres]")).toBeVisible();
  await expect(table.getByRole("gridcell", { name: "health centres", exact: true })).toBeVisible();
  await shot(page, "data-rename");

  // Both themes (the sidebar is black in both).
  await page.evaluate(() => localStorage.setItem("am.theme", "dark"));
  await page.reload();
  await shot(page, "dark-data-rename");
  await sidebar(page, "Project").click();
  await panel(page, "Grid parameters").click();
  await shot(page, "dark-project-summary-grid");
  await page.evaluate(() => localStorage.setItem("am.theme", "light"));
  await page.reload();
  await sidebar(page, "Data").click();

  // Data › Delete: the selected land cover.
  await panel(page, "Delete").click();
  await table.locator(".tabulator-row", { hasText: "base" }).locator("input[type=checkbox]").check();
  await page.getByRole("button", { name: "Delete permanently" }).click();
  await shot(page, "data-delete-confirm");
  await page.getByRole("dialog").getByRole("button", { name: "Delete permanently" }).click();
  await expect(page.getByText("Removed: land cover merged [base]")).toBeVisible();
  await expect(table.locator(".tabulator-row")).toHaveCount(2);

  // Project › Export: the project file (.am6p), imported again as a new project.
  await sidebar(page, "Project").click();
  await panel(page, "Export").click();
  const download = page.waitForEvent("download");
  await page.getByRole("button", { name: "Export this project" }).click();
  const file = await download;
  expect(file.suggestedFilename()).toMatch(/^Tutorial_.*\.am6p$/);
  await file.saveAs(`${work}tutorial.am6p`);
  await panel(page, "New / Import").click();
  await page.getByLabel("Enter a new project name (min 4 characters)").fill("Tutorial copy");
  await page.locator("input[type=file]").setInputFiles(`${work}tutorial.am6p`);
  await expect(page.locator("aside")).toContainText("Tutorial copy", { timeout: 60_000 });
  await sidebar(page, "Data").click();
  await expect(table.locator(".tabulator-row")).toHaveCount(2);
  await shot(page, "project-imported");

  // An AccessMod 5 data archive (Data manager › Archive) becomes a project too.
  await sidebar(page, "Project").click();
  await panel(page, "New / Import").click();
  await page.getByLabel("Enter a new project name (min 4 characters)").fill("From AccessMod 5");
  await page.locator("input[type=file]").setInputFiles(am5Archive);
  await expect(page.locator("aside")).toContainText("From AccessMod 5", { timeout: 60_000 });
  await sidebar(page, "Data").click();
  await expect(table.locator(".tabulator-row")).toHaveCount(9);
  await shot(page, "project-imported-am5");

  // Settings › Language (no longer in Project), then delete the open project in French.
  await sidebar(page, "Settings").click();
  await choose(page, "Select the language", "Français");
  await expect(sidebar(page, "Projet")).toBeVisible();
  await sidebar(page, "Projet").click();
  await panel(page, "Supprimer").click();
  await choose(page, "Sélectionnez le projet à supprimer", "From AccessMod 5");
  await page.getByRole("button", { name: "Supprimer", exact: true }).last().click();
  await page.getByRole("dialog").getByRole("button", { name: "Supprimer" }).click();
  await expect(page).toHaveURL(/#\/_\/project/);
  await expect(page.getByText("Supprimé : From AccessMod 5")).toBeVisible();
});

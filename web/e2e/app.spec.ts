import { execFileSync } from "node:child_process";
import { mkdirSync, rmSync } from "node:fs";
import { expect, type Page, test } from "@playwright/test";

// The tutorial data of the repository (docs/tutorial/data); facilities are
// also sent as a shapefile, to upload several files at once.
const data = new URL("../../docs/tutorial/data/", import.meta.url).pathname;
const work = new URL("../test-results/e2e-data/", import.meta.url).pathname;

const size = JSON.parse(execFileSync("gdalinfo", ["-json", `${data}dem.tif`], { encoding: "utf8" })).size as [number, number];

async function shot(page: Page, name: string) {
  await page.waitForTimeout(300);
  await page.screenshot({ path: `test-results/screens/${name}.png`, fullPage: true });
}

const sidebar = (page: Page, name: string) => page.locator("aside").getByRole("button", { name, exact: true });
const panel = (page: Page, name: string) => page.getByRole("button", { name, exact: true });

/** A design-system SelectField: open it by its label, pick an option. */
async function choose(page: Page, label: string, option: string) {
  await page.getByLabel(label, { exact: true }).click();
  await page.getByRole("option", { name: option, exact: true }).click();
}

test.beforeAll(() => {
  rmSync(work, { recursive: true, force: true });
  mkdirSync(work, { recursive: true });
  execFileSync("ogr2ogr", ["-f", "ESRI Shapefile", `${work}facilities.shp`, `${data}facilities.gpkg`]);
});

test("project and data screens on the tutorial data", async ({ page }) => {
  await page.goto("/");
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

  // Project › Language, then delete the project in French.
  await sidebar(page, "Project").click();
  await panel(page, "Language").click();
  await choose(page, "Select the language", "Français");
  await expect(sidebar(page, "Projet")).toBeVisible();
  await panel(page, "Supprimer").click();
  await choose(page, "Sélectionnez le projet à supprimer", "Tutorial");
  await page.getByRole("button", { name: "Supprimer", exact: true }).last().click();
  await page.getByRole("dialog").getByRole("button", { name: "Supprimer" }).click();
  await expect(page).toHaveURL(/#\/_\/project/);
  await expect(page.getByText("Supprimé : Tutorial")).toBeVisible();
});

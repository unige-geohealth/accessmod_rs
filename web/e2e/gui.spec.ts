import { expect, type Page, test } from "@playwright/test";

// The tutorial data of the repository (docs/tutorial/data).
const data = new URL("../../docs/tutorial/data/", import.meta.url).pathname;

// The `.field` holding the label `name` (not its ancestors).
const field = (page: Page, name: string, index = 0) =>
  page
    .locator(".field-label > span", { hasText: new RegExp(`^${name}$`) })
    .nth(index)
    .locator("xpath=../..");
// The control of a field: its own select or input, not those of sub-fields.
const control = (page: Page, name: string, kind: "select" | "input", index = 0) =>
  field(page, name, index).locator(`:scope > ${kind}`);

async function shot(page: Page, name: string) {
  await page.screenshot({ path: `test-results/${name}.png`, fullPage: false });
}

test("create a project, import data, run an accessibility analysis", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Project", exact: true })).toBeVisible();

  // New project from the tutorial DEM.
  await page.getByLabel("Name").fill("Tutorial");
  await page.getByLabel("DEM (file on this computer)").fill(`${data}dem.tif`);
  await page.getByRole("button", { name: "Create" }).click();
  await expect(page.getByRole("heading", { name: "Data", exact: true })).toBeVisible();

  // Land cover (merged) and facilities.
  const importDataset = async (file: string, cls: string, tags: string) => {
    await page.getByLabel("File on this computer").fill(`${data}${file}`);
    await page.getByLabel("Class").selectOption(cls);
    await page.getByLabel("Tags").fill(tags);
    await page.getByRole("button", { name: "Import a dataset" }).click();
    await expect(page.locator("tr", { hasText: cls })).toBeVisible();
  };
  await importDataset("land_cover.tif", "rLandCoverMerged", "base");
  await importDataset("facilities.gpkg", "vFacility", "health centres");
  await page.locator("tr", { hasText: "rLandCoverMerged" }).click();
  await page.waitForTimeout(1500);
  await shot(page, "1-data-land-cover");
  await page.locator("tr", { hasText: "vFacility" }).click();
  await page.waitForTimeout(1500);
  await shot(page, "2-data-facilities");

  // Accessibility: the form comes from the job schema.
  await page.locator(".sidebar").getByRole("button", { name: "Analysis" }).click();
  await page.getByRole("button", { name: /Accessibility/ }).click();
  await expect(page.getByRole("heading", { name: "Accessibility", exact: true })).toBeVisible();
  await control(page, "Dem", "select").selectOption("datasets/1.tif");
  await control(page, "Land cover", "select").selectOption("datasets/2.tif");
  await control(page, "Path", "select").selectOption("datasets/3.fgb");
  await control(page, "Id field", "select").selectOption("code");
  // The scenario of the tutorial (docs/tutorial/data/job.json).
  const scenario: [string, string, string][] = [
    ["1", "walking", "5"],
    ["2", "walking", "4"],
    ["3", "walking", "2"],
    ["1001", "motorized", "60"],
    ["1002", "bicycling", "15"],
  ];
  for (const [i, [cls, mode, speed]] of scenario.entries()) {
    await field(page, "Scenario").getByRole("button", { name: "+ add" }).last().click();
    await control(page, "Class", "input", i).fill(cls);
    await control(page, "Mode", "select", i).selectOption({ label: mode.replace(/^./, (c) => c.toUpperCase()) });
    await control(page, "Speed kmh", "input", i).fill(speed);
  }
  await control(page, "Method", "select").selectOption({ label: "Anisotropic" });
  await shot(page, "3-analysis-form");

  await page.getByRole("button", { name: "Validate" }).click();
  await expect(page.getByText("Valid", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Save" }).click();
  await page.getByRole("button", { name: "Run" }).click();
  await expect(page.getByText("Run queued")).toBeVisible();

  // Logs: the run finishes; its outputs enter the catalogue.
  await page.locator(".sidebar").getByRole("button", { name: "Logs" }).click();
  await expect(page.locator(".status.done")).toBeVisible({ timeout: 60_000 });
  await shot(page, "4-logs");
  await page.locator(".sidebar").getByRole("button", { name: "Data" }).click();
  await page.locator("tr", { hasText: "rTravelTime" }).click();
  await page.waitForTimeout(2000);
  await shot(page, "5-travel-time");
});

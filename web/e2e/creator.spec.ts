import { expect, test } from "@playwright/test";
import { choose, panel, shot, sidebar } from "./helpers";

// Project › New from a region, on the sources the server finds in its cache
// (e2e/fixtures/sources: real data around Juba, two made-up units): no
// provider is contacted.
test("a region picked on the map becomes a project that computes", async ({ page }) => {
  await page.goto("/#/_/project");
  await panel(page, "New from a region").click();
  await expect(page.getByText("Choose a country.").first()).toBeVisible();
  await shot(page, "creator-empty");

  await page.getByLabel("Enter a new project name (min 4 characters)").fill("Juba from a region");
  await choose(page, "Country", "South Sudan");
  await choose(page, "Administrative level", "Level 2 units");
  await expect(page.getByText("Boundaries: geoBoundaries, CC BY 3.0 IGO (test data)")).toBeVisible();
  // The map fits the two units, west and east: a click on the west one.
  const canvas = page.locator(".maplibregl-canvas");
  await expect(canvas).toBeVisible();
  await page.waitForTimeout(1000);
  const box = await canvas.boundingBox();
  if (!box) throw new Error("no map");
  await page.mouse.click(box.x + box.width * 0.3, box.y + box.height * 0.5);
  await expect(page.getByText("Region: Juba west.")).toBeVisible();
  await choose(page, "Passable distance around the region", "0 km");
  await expect(page.getByRole("checkbox", { name: "Default scenarios" })).toBeChecked();
  await shot(page, "creator-region");

  const create = page.getByRole("button", { name: "Create the project" });
  await expect(create).toBeEnabled();
  await create.click();
  // The job panel follows the job to its end, then opens the project with its datasets.
  const job = page.getByRole("dialog").filter({ hasText: "Juba from a region" });
  await expect(job).toBeVisible();
  await shot(page, "job-panel");
  await expect(job).toContainText(/: done/, { timeout: 90_000 });
  await job.getByRole("button", { name: "Open the project" }).click();
  await expect(job).toHaveCount(0);
  // Nothing queued or running: no count on Jobs.
  await expect(sidebar(page, "Jobs")).toHaveText("Jobs");
  await expect(page.locator("aside")).toContainText("Juba from a region");
  await sidebar(page, "Data").click();
  await expect(page.locator(".tabulator .tabulator-row")).toHaveCount(13);
  await shot(page, "creator-project-data");
});

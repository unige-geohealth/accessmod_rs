import { readFileSync } from "node:fs";
import { expect, test } from "@playwright/test";
import { command, finished, shot, sidebar } from "./helpers";

// Jobs: the runs of every project, here one merge of the demo project.
const project = "demo";

test.beforeAll(async ({ request }) => {
  await command(request, "demo.reset", {});
  const run = await command<{ id: number }>(request, "run.start", {
    project,
    analysis: "merge-land-cover",
    tags: ["jobs"],
    document: { schema_version: 1, dem: "datasets/1.tif", stack: [{ type: "land_cover", path: "datasets/2.tif" }] },
  });
  await finished(request, project, run.id);
});

test.afterAll(async ({ request }) => {
  await command(request, "demo.reset", {});
});

test("jobs: runs of every project, details, outputs and exported job", async ({ page }) => {
  await page.goto(`/#/${project}/project`);
  await sidebar(page, "Jobs").click();
  await expect(page).toHaveURL(/#\/demo\/jobs/);
  const row = page.locator(".tabulator-row", { hasText: "Merge land cover" }).first();
  await expect(row).toContainText("done");
  await expect(row).toContainText("jobs");
  await expect(page.getByText(/^\d+ jobs: 0 running, 0 queued, 0 failed/)).toBeVisible();

  await row.locator("input[type=checkbox]").check();
  await expect(page).toHaveURL(/job=demo%2F\d+/);
  await expect(page.getByText("land cover merged [jobs]")).toBeVisible();
  await shot(page, "jobs");

  const download = page.waitForEvent("download");
  await page.getByRole("button", { name: "Export job" }).click();
  const job = JSON.parse(readFileSync(await (await download).path(), "utf8"));
  expect(job.stack).toEqual([{ type: "land_cover", path: "datasets/2.tif" }]);

  await page.getByRole("button", { name: "Show in Data" }).click();
  await expect(page).toHaveURL(/#\/demo\/data/);
});

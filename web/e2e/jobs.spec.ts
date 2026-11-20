import { readFileSync } from "node:fs";
import { expect, test } from "@playwright/test";
import { command, finished, panel, panelEnded, shot, sidebar } from "./helpers";

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

  // Data: the run's job, its inputs (DEM, land cover) and its output, selected.
  await page.getByRole("button", { name: "Show in Data" }).click();
  await expect(page).toHaveURL(/#\/demo\/data\/filter\?.*run=\d+/);
  const rows = page.locator(".tabulator .tabulator-row");
  await expect(rows.filter({ has: page.locator("[tabulator-field=role]", { hasText: /^Job$/ }) })).toHaveCount(1);
  await expect(rows.filter({ has: page.locator("[tabulator-field=role]", { hasText: /^Input$/ }) })).toHaveCount(2);
  await expect(rows.filter({ has: page.locator("[tabulator-field=role]", { hasText: /^Output$/ }) }).first()).toContainText("jobs");
  for (const row of await rows.all()) await expect(row.locator("input[type=checkbox]")).toBeChecked();
  await shot(page, "data-run");

  // Data › Archive: the selection (job, inputs, outputs) as a zip, listed, downloaded.
  await panel(page, "Archive").click();
  await page.getByLabel("File prefix. Default is 'am6_export'").fill("e2e-run");
  await page.getByRole("button", { name: "Create archive" }).click();
  // The job that wrote it: the job panel, to its end.
  await panelEnded(page, "Archive: done", "archived as e2e-run_");
  await shot(page, "data-archive");
  const zip = page.waitForEvent("download");
  await page.getByRole("button", { name: "Export archive" }).click();
  const file = await zip;
  expect(file.suggestedFilename()).toMatch(/^e2e-run_.*\.zip$/);
  const bytes = readFileSync(await file.path());
  expect(bytes.subarray(0, 2).toString()).toBe("PK");
  expect(bytes.includes(Buffer.from("manifest.json"))).toBe(true);
  await page.getByRole("button", { name: "Delete archive" }).click();
  await expect(page.getByText("No archive yet.")).toBeVisible();
});

test("jobs: the task manager filters by kind, retries a failed analysis and removes a job", async ({ page, request }) => {
  // A run that fails on its inputs once started: no such road class field.
  const failed = await command<{ id: number }>(request, "run.start", {
    project,
    analysis: "merge-land-cover",
    tags: ["fails"],
    document: {
      schema_version: 1,
      dem: "datasets/1.tif",
      stack: [{ type: "road", path: "datasets/8.fgb", class_field: "no_such_field", label_field: "label" }],
    },
  });
  await expect
    .poll(async () => ((await (await request.get(`/api/projects/${project}/runs/${failed.id}`)).json()) as { status: string }).status, { timeout: 60_000 })
    .toBe("failed");

  await page.goto(`/#/${project}/jobs`);
  // Operations: the demo, rebuilt; no analysis. Analyses: the other way round.
  await page.getByRole("radio", { name: "Operations (imports, exports, archives)" }).click();
  await expect(page.locator(".tabulator-row", { hasText: "Demo rebuilt" }).first()).toBeVisible();
  await expect(page.locator(".tabulator-row", { hasText: "Merge land cover" })).toHaveCount(0);
  await page.getByRole("radio", { name: "Analyses" }).click();
  await expect(page.locator(".tabulator-row", { hasText: "Demo rebuilt" })).toHaveCount(0);

  // Retry: a new run of the same job, selected.
  await page.locator(".tabulator-row", { hasText: "fails" }).first().locator("input[type=checkbox]").check();
  await expect(page).toHaveURL(new RegExp(`job=demo%2F${failed.id}`));
  await page.getByRole("button", { name: "Retry" }).click();
  await expect(page).not.toHaveURL(new RegExp(`job=demo%2F${failed.id}(&|$)`));
  await expect(page.locator(".tabulator-row", { hasText: "fails" })).toHaveCount(2);

  // Each failure is notified until closed.
  // (the retried one: the first failed before the page opened)
  const failure = page.getByRole("alert").filter({ hasText: "Merge land cover: failed" });
  await expect(failure).toHaveCount(1);
  // Notices sit at the top, under the header.
  expect((await failure.boundingBox())!.y).toBeLessThan(200);
  for (const close of await page.getByRole("alert").getByRole("button", { name: "Close" }).all()) await close.click();
  await expect(page.getByRole("alert")).toHaveCount(0);
  // Remove the first one: gone from the table.
  await page.goto(`/#/${project}/jobs?job=demo%2F${failed.id}`);
  await page.getByRole("button", { name: "Remove job" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Remove job" }).click();
  await expect(page.locator(".tabulator-row", { hasText: "fails" })).toHaveCount(1);
  await shot(page, "jobs-manager");
});

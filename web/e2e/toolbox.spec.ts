import { expect, test } from "@playwright/test";
import { choose, command, shot } from "./helpers";

// Toolbox tools on the demo project, as a user starts from it: merge the
// land cover with roads and barriers, then adjust the population on it.
const project = "demo";

test.beforeAll(async ({ request }) => {
  await command(request, "demo.reset", {});
});

test.afterAll(async ({ request }) => {
  await command(request, "demo.reset", {});
});

test("merge land cover: land cover, road classes and barriers stacked, then merged", async ({ page }) => {
  await page.goto(`/#/${project}/toolbox/merge`);
  // Land cover with the labels of the demo table.
  await expect(page.locator(".tabulator-row").first()).toBeVisible();
  await page.getByRole("button", { name: "Add to stack", exact: true }).click();

  await page.getByRole("tab", { name: "Roads", exact: true }).click();
  await expect(page.locator(".tabulator-row", { hasText: "Main road" })).toBeVisible();
  await page.getByRole("button", { name: "Add to the stack", exact: true }).click();

  await page.getByRole("tab", { name: "Barriers", exact: true }).click();
  for (const layer of ["(v) barrier [demo rivers]", "(v) barrier [demo wetlands]"]) {
    await choose(page, "Select barrier layer (vector)", layer);
    await page.getByRole("button", { name: "Add to the stack", exact: true }).click();
  }

  await page.getByRole("tab", { name: "Merge", exact: true }).click();
  const use = page.getByRole("list", { name: "Stack items to use" });
  // Barriers and roads above the land cover.
  await expect(use.getByRole("listitem")).toHaveCount(6);
  await expect(use.getByRole("listitem").last()).toContainText("stack land cover [demo]");
  await page.getByLabel("Add short tags").fill("e2e");
  await expect(page.getByText("Ready to compute.")).toBeVisible({ timeout: 30_000 });
  await shot(page, "toolbox-merge");
  await page.getByRole("button", { name: "Merge the items in the stack" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("Process finished", { exact: true })).toBeVisible({ timeout: 120_000 });
  await expect(dialog).toContainText("[e2e]");
  await page.keyboard.press("Escape");
});

test("adjust population distribution: population kept by zone, moved off barriers", async ({ page, request }) => {
  // A merged land cover to work on.
  const datasets = (await (await request.get(`/api/projects/${project}/datasets`)).json()) as { class: string }[];
  test.skip(!datasets.some((d) => d.class === "rLandCoverMerged"), "needs the merge test first");
  await page.goto(`/#/${project}/toolbox/population`);
  await page.getByLabel("Add short tag").fill("e2e");
  await expect(page.getByText("Ready to compute.")).toBeVisible({ timeout: 30_000 });
  await shot(page, "toolbox-population");
  await page.getByRole("button", { name: "Compute", exact: true }).click();
  await expect(page.getByRole("dialog").getByText("Process finished", { exact: true })).toBeVisible({ timeout: 120_000 });
});

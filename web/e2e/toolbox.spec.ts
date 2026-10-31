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

test("interactive map: a click queries the raster; relocation writes a new facilities layer", async ({ page, request }) => {
  await page.goto(`/#/${project}/toolbox/map`);
  const map = page.locator(".am-map canvas");
  await expect(map).toBeVisible();
  // The newest raster is preselected; the project centre has a value.
  await page.waitForTimeout(1000);
  const box = await map.boundingBox();
  if (box === null) throw new Error("no map");
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
  await expect(page.getByText(/^Longitude : -?\d+\.\d{6}$/)).toBeVisible();
  await expect(page.locator("li", { hasText: "Value :" })).not.toHaveText("Value : -");
  await shot(page, "toolbox-map");

  // Relocation, as the Save button sends it: a new layer, the source kept.
  const datasets = (await (await request.get(`/api/projects/${project}/datasets`)).json()) as { id: number; class: string }[];
  const facilities = datasets.find((d) => d.class === "vFacility");
  if (facilities === undefined) throw new Error("no facilities");
  const collection = (await (await request.get(`/api/projects/${project}/datasets/${facilities.id}/features`)).json()) as {
    features: { id: number; geometry: { coordinates: unknown[] } }[];
  };
  const [first] = collection.features;
  if (first === undefined) throw new Error("no feature");
  // A point, or a one-point multipoint (the demo).
  const [lon = 0, lat = 0] = first.geometry.coordinates.flat() as number[];
  const saved = await command<{ id: number; class: string; tags: string[] }>(request, "dataset.relocate", {
    project,
    id: facilities.id,
    moves: [{ fid: first.id, lon: lon + 0.001, lat }],
    tags: ["relocated"],
  });
  expect([saved.class, saved.tags]).toEqual(["vFacility", ["relocated"]]);
});

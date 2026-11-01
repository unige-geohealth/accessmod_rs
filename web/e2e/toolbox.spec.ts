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

test("interactive map: facilities to inspect, Next, a drag, Save; additional layers", async ({ page, request }) => {
  const datasets = (await (await request.get(`/api/projects/${project}/datasets`)).json()) as { id: number; class: string; tags: string[] }[];
  const source = datasets.find((d) => d.class === "vFacility");
  const merged = datasets.filter((d) => d.class === "rLandCoverMerged").sort((a, b) => b.id - a.id)[0];
  if (source === undefined || merged === undefined) throw new Error("needs the merge test first");
  type Collection = { features: { id: number; sample: number | null; geometry: { coordinates: unknown[] } }[] };
  // A facility off the grid, through the API: one to inspect. The newest layer is shown first.
  const original = (await (await request.get(`/api/projects/${project}/datasets/${source.id}/features`)).json()) as Collection;
  const [first] = original.features;
  if (first === undefined) throw new Error("no facility");
  const [lon = 0, lat = 0] = first.geometry.coordinates.flat() as number[];
  const facilities = await command<{ id: number }>(request, "dataset.relocate", {
    project,
    id: source.id,
    moves: [{ fid: first.id, lon: lon + 2, lat }],
    tags: ["offgrid"],
  });
  // What the relocator shows, through the API: facilities without a value.
  const sampled = (await (await request.get(`/api/projects/${project}/datasets/${facilities.id}/features?sample=${merged.id}`)).json()) as Collection;
  const missing = sampled.features.filter((f) => f.sample === null).length;

  await page.goto(`/#/${project}/toolbox/map`);
  const map = page.locator(".am-map canvas");
  await expect(map).toBeVisible();
  await expect(page.getByRole("status").filter({ hasText: "facilities without a value" })).toHaveText(
    new RegExp(`^${missing} of ${sampled.features.length} facilities`),
  );
  await shot(page, "toolbox-map");

  // Additional layers: both barrier layers above the land cover.
  await page.getByRole("button", { name: "Additional layers" }).click();
  const search = page.getByRole("combobox", { name: "Search a layer" });
  // The open list floats above the panels: its last option is on top, not clipped by the accordion.
  // After the accordion has opened.
  await page.waitForTimeout(400);
  await search.focus();
  const listbox = page.getByRole("listbox", { name: "Search a layer" });
  await expect(listbox.getByRole("option").first()).toBeVisible();
  const list = await listbox.boundingBox();
  if (list === null) throw new Error("no list");
  // Just above its bottom edge: the list, not the panel below.
  const onTop = await page.evaluate(
    ({ x, y }) => document.elementFromPoint(x, y)?.closest("[role=listbox]") !== null,
    { x: list.x + list.width / 2, y: list.y + list.height - 4 },
  );
  expect(onTop).toBe(true);
  await shot(page, "toolbox-map-layer-search");
  for (const words of ["rivers", "wetl"]) {
    await search.fill(words);
    await search.press("Enter");
  }
  const active = page.getByRole("list", { name: "Layers shown, top first" }).getByRole("listitem");
  await expect(active).toHaveCount(2);
  // The last added is on top; each shows its geometry.
  await expect(active.first()).toContainText("barrier [demo wetlands]");
  await expect(active.first().getByRole("img", { name: "Polygons" })).toBeVisible();
  await active.nth(1).getByRole("button", { name: "Hide" }).click();
  await expect(active.nth(1).getByRole("button", { name: "Show" })).toBeVisible();
  await shot(page, "toolbox-map-layers");
  await page.getByRole("button", { name: "Facilities relocator" }).click();

  // Next centres the facility to inspect; drag it from the map centre.
  expect(missing).toBe(1);
  await page.getByRole("button", { name: "Next facility to inspect" }).click();
  await expect(page.getByText(/^Selected: /)).toBeVisible();
  await page.getByRole("button", { name: "Move facilities" }).click();
  await page.waitForTimeout(800);
  const box = await map.boundingBox();
  if (box === null) throw new Error("no map");
  const [x, y] = [box.x + box.width / 2, box.y + box.height / 2];
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + 20, y + 10, { steps: 5 });
  await page.mouse.up();
  await expect(page.getByText("Number of facilities relocated : 1")).toBeVisible();
  await page.getByLabel("Add short tags").fill("relocated");
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page.getByText(/^1 changes saved\. Output dataset : \(v\) facility \[relocated\]/)).toBeVisible();

  // The new layer, through the API: same facilities, one moved.
  const after = (await (await request.get(`/api/projects/${project}/datasets`)).json()) as { id: number; class: string; tags: string[] }[];
  const saved = after.find((d) => d.class === "vFacility" && d.tags.includes("relocated"));
  if (saved === undefined) throw new Error("no relocated layer");
  const relocated = (await (await request.get(`/api/projects/${project}/datasets/${saved.id}/features?sample=${merged.id}`)).json()) as Collection;
  expect(relocated.features).toHaveLength(sampled.features.length);
  const changed = relocated.features.filter(
    (f, i) => JSON.stringify(f.geometry.coordinates.flat()) !== JSON.stringify(sampled.features[i]?.geometry.coordinates.flat()),
  );
  expect(changed).toHaveLength(1);
});

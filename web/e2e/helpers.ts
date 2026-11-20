import { expect, type APIRequestContext, type Locator, type Page } from "@playwright/test";

export async function shot(page: Page, name: string) {
  await page.waitForTimeout(300);
  await page.screenshot({ path: `test-results/screens/${name}.png`, fullPage: true });
}

export const sidebar = (page: Page, name: string) => page.locator("aside").getByRole("button", { name, exact: true });
export const panel = (page: Page, name: string) => page.getByRole("button", { name, exact: true });

/** A design-system SelectField: open it by its label, pick an option. */
export async function choose(page: Page, label: string, option: string) {
  await page.getByLabel(label, { exact: true }).click();
  await page.getByRole("option", { name: option, exact: true }).click();
}

/** A command of the server registry, through the API. */
export async function command<T>(request: APIRequestContext, id: string, params: unknown): Promise<T> {
  const response = await request.post(`/api/commands/${id}`, { data: params });
  expect(response.ok(), await response.text()).toBe(true);
  const body = (await response.json()) as { id?: number; operation?: string; kind?: string } | null;
  // A long command queues a task: wait for it, as the client does.
  if (body !== null && typeof body === "object" && body.operation !== undefined) {
    await expect
      .poll(
        async () => {
          const all = (await (await request.get("/api/operations")).json()) as { id: number; status: string }[];
          return all.find((o) => o.id === body.id)?.status;
        },
        { timeout: 120_000 },
      )
      .toBe("done");
  } else if (body !== null && typeof body === "object" && body.kind === "manage") {
    const project = (params as { project: string }).project;
    await finished(request, project, body.id ?? 0);
    const run = (await (await request.get(`/api/projects/${project}/runs/${body.id}`)).json()) as { result: T };
    return run.result;
  }
  return body as T;
}

/** Wait until run `id` of `project` is done. */
export async function finished(request: APIRequestContext, project: string, id: number) {
  await expect
    .poll(
      async () => {
        const runs = (await (await request.get(`/api/projects/${project}/runs`)).json()) as { id: number; status: string }[];
        return runs.find((r) => r.id === id)?.status;
      },
      { timeout: 120_000 },
    )
    .toBe("done");
}

/** Upload one file and import it as a dataset of `cls`. */
export async function importFile(request: APIRequestContext, project: string, cls: string, tags: string[], name: string, content: string) {
  const response = await request.post("/api/uploads", { multipart: { file: { name, mimeType: "text/plain", buffer: Buffer.from(content) } } });
  expect(response.ok(), await response.text()).toBe(true);
  const upload = (await response.json()) as { id: string };
  return command<{ id: number }>(request, "dataset.import", { project, class: cls, tags, source: { upload: { id: upload.id } } });
}

/** The job panel: the job a click just started, in a modal. */
export const jobPanel = (page: Page) => page.getByRole("dialog");

/** Close the job panel while its job runs: the job goes on, in Jobs. */
export async function background(page: Page) {
  await expect(jobPanel(page)).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(jobPanel(page)).toHaveCount(0);
}

/** The job panel shows its job ended (`title`, e.g. "Archive: done", and `text`), then is closed with `close`. */
export async function panelEnded(page: Page, title: string | RegExp, text?: string, close = "Close") {
  let ended = jobPanel(page).filter({ hasText: title });
  if (text !== undefined) ended = ended.filter({ hasText: text });
  await expect(ended).toBeVisible({ timeout: 90_000 });
  await ended.getByRole("button", { name: close, exact: true }).click();
  await expect(jobPanel(page)).toHaveCount(0);
}

/** Click `button` (Compute): the job panel shows the job it queued with `tags`; closed, the job goes on until done. */
export async function computed(page: Page, project: string, tags: string, button: Locator) {
  const started = page.waitForResponse((r) => r.url().includes("/api/commands/run.start"));
  await button.click();
  const response = await started;
  expect(response.ok(), await response.text()).toBe(true);
  const run = (await response.json()) as { id: number };
  await expect(jobPanel(page)).toContainText(`[${tags}]`);
  await background(page);
  await finished(page.request, project, run.id);
}

/** The job panel of an import that ended done, of a dataset of class label and tags `what` ("population again"), closed. */
export async function imported(page: Page, what: string) {
  await panelEnded(page, /: done/, `(${what})`);
}

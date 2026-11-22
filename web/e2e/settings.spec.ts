import { expect, test } from "@playwright/test";
import { shot, sidebar } from "./helpers";

test("settings: the theme follows the system until one is chosen", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/#/_/settings");
  await page.evaluate(() => localStorage.removeItem("am.theme"));
  await page.reload();
  const html = page.locator("html");
  await expect(html).toHaveClass(/dark/);
  await expect(page.getByRole("radio", { name: "System" })).toBeChecked();
  await shot(page, "settings-dark");

  await page.getByRole("radio", { name: "Light" }).check();
  await expect(html).not.toHaveClass(/dark/);
  await page.reload();
  await expect(html).not.toHaveClass(/dark/);
  await expect(page.getByText("6.0.0", { exact: false })).not.toHaveCount(0);

  await page.getByRole("radio", { name: "System" }).check();
  await expect(html).toHaveClass(/dark/);
  await sidebar(page, "About").click();
  await expect(page).toHaveURL(/about/);
});

test("settings: advanced options appear in modules once enabled", async ({ page }) => {
  await page.goto("/#/demo/settings");
  const enable = page.getByLabel("Enable advanced options in modules");
  await enable.check();
  await page.goto("/#/demo/toolbox/merge");
  await page.getByRole("tab", { name: "Roads", exact: true }).click();
  const keep = page.getByLabel("Do not add 1000 to class < 1000");
  await expect(keep).toBeVisible();
  await shot(page, "toolbox-merge-advanced");

  await page.goto("/#/demo/settings");
  await enable.uncheck();
  await page.goto("/#/demo/toolbox/merge");
  await page.getByRole("tab", { name: "Roads", exact: true }).click();
  await expect(page.getByLabel("Select road layer (vector)")).toBeVisible();
  await expect(keep).toHaveCount(0);
});

test("settings: the Project Creator cache shows its size and asks before clearing", async ({ page }) => {
  // The e2e server starts with the fixture sources in its cache; the
  // Project Creator tests need them, so the clearing is cancelled here.
  await page.goto("/#/_/settings");
  await expect(page.getByText(/ in \d+ files$/)).toBeVisible();
  await shot(page, "settings-cache");
  await page.getByRole("button", { name: "Clear cache" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("Remove every downloaded source", { exact: false })).toBeVisible();
  await dialog.getByRole("button", { name: /cancel/i }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByText(/ in \d+ files$/)).toBeVisible();
});

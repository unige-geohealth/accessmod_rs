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

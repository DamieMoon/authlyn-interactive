import { test, expect } from "@playwright/test";

test("homepage shows the authlyn chat UI", async ({ page }) => {
  await page.goto("http://127.0.0.1:3000/");

  await expect(page).toHaveTitle("authlyn");
  await expect(page.locator("h1")).toHaveText("authlyn");
  await expect(page.getByRole("button", { name: "Generate" })).toBeVisible();
});

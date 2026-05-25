import { test, expect } from "@playwright/test";

// Defaults to the local dev server; override with SMOKE_BASE to hit the live
// origin. Drives the phase-1 roleplay UI: register -> create a server ->
// (auto-selected default channel) -> send a message with markup -> confirm it
// renders bold + colored.
const BASE = process.env.SMOKE_BASE ?? "http://127.0.0.1:3000/";

test("register, create a server, and send a formatted message", async ({ page }) => {
  test.setTimeout(60_000);

  const username = `rp_${Date.now().toString(36)}`;

  // Register a fresh account.
  await page.goto(`${BASE}register`);
  await page.getByPlaceholder(/username/).fill(username);
  await page.getByPlaceholder(/password/).fill("password123");
  await page.getByRole("button", { name: "Sign up" }).click();

  // Lands in the authed shell.
  await expect(page.getByText("Signed in as")).toBeVisible({ timeout: 15_000 });

  // Create a server via the rail.
  await page.getByPlaceholder("new server").fill("Rohan");
  await page.locator(".rail-add button").click();

  // The default text channel auto-opens, so the composer appears.
  const composer = page.getByPlaceholder(/type a message/);
  await expect(composer).toBeVisible({ timeout: 15_000 });

  // Send a message with bold + color markup.
  await composer.fill("**hail** [red]rider[/red]");
  await page.getByRole("button", { name: "Send" }).click();

  // It renders: bold "hail" and a red-classed "rider".
  await expect(page.locator(".messages strong")).toHaveText("hail", {
    timeout: 15_000,
  });
  await expect(page.locator(".messages .mk-red")).toHaveText("rider");
});

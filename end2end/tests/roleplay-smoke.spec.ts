import { test, expect } from "@playwright/test";

// Defaults to the local dev server; override with SMOKE_BASE to hit the live
// origin. Drives the phase-1 roleplay UI end to end: register -> create a
// server -> create + wear a persona -> send a message with markup -> confirm
// it renders bold + colored AND is attributed to the worn persona.
const BASE = process.env.SMOKE_BASE ?? "http://127.0.0.1:3000/";

test("register, wear a persona, and send a formatted message", async ({ page }) => {
  test.setTimeout(60_000);

  const username = `rp_${Date.now().toString(36)}`;

  // Register a fresh account → lands in the shell.
  await page.goto(`${BASE}register`);
  await page.getByPlaceholder(/username/).fill(username);
  await page.getByPlaceholder(/password/).fill("password123");
  await page.getByRole("button", { name: "Sign up" }).click();
  await expect(page.getByText("Signed in as")).toBeVisible({ timeout: 15_000 });

  // Create a server; its default text channel auto-opens.
  await page.getByPlaceholder("new server").fill("Rohan");
  await page.locator(".rail-add button").click();
  await expect(page.getByPlaceholder(/type a message/)).toBeVisible({ timeout: 15_000 });

  // Wardrobe → create + wear a persona.
  await page.getByRole("button", { name: /Wardrobe/ }).click();
  await page.getByPlaceholder("persona name").fill("Aragorn");
  await page.getByRole("button", { name: "Create persona" }).click();
  const card = page.locator(".persona-card", { hasText: "Aragorn" });
  await expect(card).toBeVisible({ timeout: 15_000 });
  await card.getByRole("button", { name: "Wear" }).click();
  await expect(card.getByRole("button", { name: /Worn/ })).toBeVisible();
  await page.waitForTimeout(400); // let the active-persona PUT land

  // Back to the channel and send a formatted message.
  await page.getByRole("button", { name: /general/ }).click();
  const composer = page.getByPlaceholder(/type a message/);
  await expect(composer).toBeVisible();
  await composer.fill("**hail** [red]rider[/red]");
  await page.getByRole("button", { name: "Send" }).click();

  // Renders bold "hail" + red "rider", attributed to the worn persona.
  await expect(page.locator(".messages strong")).toHaveText("hail", { timeout: 15_000 });
  await expect(page.locator(".messages .mk-red")).toHaveText("rider");
  await expect(page.locator(".messages .who")).toHaveText("Aragorn");
});

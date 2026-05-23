import { test, expect, type Page } from "@playwright/test";

const BASE = "http://127.0.0.1:3000/";

// Generate + publish a device, then read back its (auto-filled) user/device ids.
async function publish(p: Page): Promise<{ user: string; device: string }> {
  await p.getByRole("button", { name: "Generate" }).click();
  // Active device populates once the publish round-trip succeeds.
  await expect(p.locator("code")).not.toHaveText("(none)", { timeout: 15_000 });
  const user = await p.getByLabel("user id").inputValue();
  const device = await p.getByLabel("device id").inputValue();
  expect(user).not.toEqual("");
  expect(device).not.toEqual("");
  return { user, device };
}

// Full step-10 happy path across two independent browser contexts (= two
// devices): publish keys, create/join a room, Olm->Megolm key-share both ways,
// and confirm a message typed in one tab arrives DECRYPTED in the other.
test("two devices exchange end-to-end encrypted messages", async ({ browser }) => {
  test.setTimeout(90_000);

  const ctxA = await browser.newContext();
  const ctxB = await browser.newContext();
  const a = await ctxA.newPage();
  const b = await ctxB.newPage();
  await a.goto(BASE);
  await b.goto(BASE);

  const A = await publish(a);
  const B = await publish(b);
  expect(A.device).not.toEqual(B.device);

  // A creates a room and invites B's user.
  await a.getByLabel("name").fill("smoke");
  await a.getByRole("button", { name: "Create room" }).click();
  await expect(a.getByLabel("room id")).not.toHaveValue("", { timeout: 15_000 });
  const roomId = await a.getByLabel("room id").inputValue();

  await a.getByLabel("invite user").fill(B.user);
  await a.getByRole("button", { name: "Invite" }).click();
  await expect(a.locator(".status")).toContainText("Invited", { timeout: 15_000 });

  // B joins the room (paste id) and points its peer fields at A so its poll
  // loop can claim A's identity and import the inbound session.
  await b.getByLabel("room id").fill(roomId);
  await b.getByLabel("peer user").fill(A.user);
  await b.getByLabel("peer device").fill(A.device);

  // A points its peer at B and shares the room's Megolm session key.
  await a.getByLabel("peer user").fill(B.user);
  await a.getByLabel("peer device").fill(B.device);
  await a.getByRole("button", { name: "Share session key" }).click();
  await expect(a.locator(".status")).toContainText("Shared", { timeout: 15_000 });

  // A sends -> B receives it decrypted via the ~1.5s poll loop.
  await a.getByPlaceholder("type a message").fill("hello from A");
  await a.getByRole("button", { name: "Send" }).click();
  await expect(b.locator("ul.messages")).toContainText("hello from A", {
    timeout: 30_000,
  });

  // Reply B -> A (B shares its own session key first).
  await b.getByRole("button", { name: "Share session key" }).click();
  await expect(b.locator(".status")).toContainText("Shared", { timeout: 15_000 });
  await b.getByPlaceholder("type a message").fill("hi from B");
  await b.getByRole("button", { name: "Send" }).click();
  await expect(a.locator("ul.messages")).toContainText("hi from B", {
    timeout: 30_000,
  });

  await ctxA.close();
  await ctxB.close();
});

import { execFileSync } from "node:child_process";
import { type CDPSession, expect, type Page, test } from "@playwright/test";

const deckBin = process.env.QUIETDECK_DECK_BIN ?? new URL("../../target/release/quietdeck-deck", import.meta.url).pathname;

function setupCode(): string {
  return execFileSync(deckBin, ["setup-code"], {
    env: { ...process.env, QUIETDECK_DATA_DIR: process.env.QUIETDECK_E2E_DATA },
    encoding: "utf8",
  }).trim();
}

/** Chrome's virtual authenticator: a platform passkey that verifies the user without a prompt. */
async function authenticator(page: Page): Promise<CDPSession> {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("WebAuthn.enable");
  await cdp.send("WebAuthn.addVirtualAuthenticator", {
    options: {
      protocol: "ctap2",
      transport: "internal",
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      automaticPresenceSimulation: true,
    },
  });
  return cdp;
}

// One deck for the whole file: the first test registers the first passkey, so the tests run in order.
test.describe.configure({ mode: "serial" });

test.skip(({ browserName }) => browserName !== "chromium", "the virtual authenticator is Chrome's");

let page: Page;
let firstCode = "";

test.beforeAll(async ({ browser }) => {
  page = await browser.newPage();
  await authenticator(page);
});

test("nothing is reachable before sign-in", async ({ request }) => {
  for (const path of ["/api/inbox", "/api/settings", "/api/passkeys", "/api/nodes", "/api/events"]) {
    expect((await request.get(path)).status(), path).toBe(401);
  }
  const state = await (await request.get("/api/auth/state")).json();
  expect(state).toEqual({ signed_in: false, setup_needed: true });
});

test("a wrong setup code is refused", async () => {
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Create passkey" })).toBeVisible();
  await page.getByLabel("Setup code").fill("AAAA-BBBB-CCCC-DDDD");
  await page.getByRole("button", { name: "Create passkey" }).click();
  await expect(page.getByRole("alert")).toContainText("setup code is unknown");
});

test("the first passkey takes a setup code, and signs in", async () => {
  await page.goto("/");
  firstCode = setupCode();
  await page.getByLabel("Setup code").fill(firstCode);
  await page.getByLabel("This device").fill("Test laptop");
  await page.getByRole("button", { name: "Create passkey" }).click();
  await expect(page.getByText("Nothing waiting.")).toBeVisible();
  await expect(page.getByRole("link", { name: "Inbox" })).toHaveAttribute("aria-current", "page");
});

test("a setup code works once", async ({ request }) => {
  const reused = await request.post("/api/auth/register/start", {
    data: { setup_code: firstCode, device_name: "Second laptop" },
  });
  expect(reused.status()).toBe(401);
  expect((await reused.json()).error).toContain("unknown, used or expired");
  // Without a code or a sign-in, adding a passkey is refused.
  const none = await request.post("/api/auth/register/start", { data: { setup_code: null, device_name: "x" } });
  expect(none.status()).toBe(401);
});

test("Settings lists the passkey, renames it and makes enrolment codes", async () => {
  await page.goto("/settings");
  await expect(page.getByText("Test laptop")).toBeVisible();
  await expect(page.getByText("this device", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Remove Test laptop" })).toBeDisabled();

  await page.getByRole("button", { name: "Rename Test laptop" }).click();
  await page.getByRole("textbox").fill("Work laptop");
  await page.getByRole("button", { name: "Save the name" }).click();
  await expect(page.getByText("Work laptop")).toBeVisible();

  await page.getByRole("button", { name: "Enrol a machine" }).click();
  await expect(page.getByText(/quietdeck node enroll \S+ --deck http:\/\/localhost:\d+/)).toBeVisible();

  await page.getByRole("combobox").selectOption("5");
  await page.reload();
  await expect(page.getByRole("combobox")).toHaveValue("5");
});

test("sign out, then back in with the passkey", async () => {
  await page.goto("/settings");
  await page.getByRole("button", { name: "Sign out on this device" }).click();
  await expect(page.getByRole("button", { name: "Sign in with a passkey" })).toBeVisible();
  expect((await page.request.get("/api/inbox")).status()).toBe(401);

  await page.getByRole("button", { name: "Sign in with a passkey" }).click();
  // Back on the screen he signed out from.
  await expect(page.getByRole("heading", { name: "Passkeys" })).toBeVisible();
  expect((await page.request.get("/api/inbox")).status()).toBe(200);
});

test("another site can't act with the cookie", async () => {
  const res = await page.request.put("/api/settings", {
    headers: { origin: "https://evil.example.com" },
    data: { undo_seconds: 0, require_filter_proposal: false },
  });
  expect(res.status()).toBe(403);
});

import fs from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";

const baseUrl = process.env.RUSTRESORT_UI_BASE_URL ?? "http://localhost:3010";
const username = process.env.RUSTRESORT_UI_USERNAME ?? "admin";
const password = process.env.RUSTRESORT_UI_PASSWORD ?? "admin-password";
const newPassword =
  process.env.RUSTRESORT_UI_NEW_PASSWORD ?? "admin-password-2";
const headless = process.env.HEADLESS !== "false";
const outputDir = path.resolve("output/playwright");

async function ensureOutputDir() {
  await fs.mkdir(outputDir, { recursive: true });
}

async function attachVirtualAuthenticator(page) {
  const client = await page.context().newCDPSession(page);
  await client.send("WebAuthn.enable");
  const { authenticatorId } = await client.send(
    "WebAuthn.addVirtualAuthenticator",
    {
      options: {
        protocol: "ctap2",
        transport: "internal",
        hasResidentKey: true,
        hasUserVerification: true,
        isUserVerified: true,
        automaticPresenceSimulation: true
      }
    }
  );
  return { client, authenticatorId };
}

async function loginWithPassword(page, currentPassword) {
  await page.fill("#username", username);
  await page.fill("#password", currentPassword);
  await page.click("text=Sign in with password");
  await page.waitForURL("**/settings");
}

async function waitForMessage(page, text) {
  await page.waitForFunction(
    (expected) =>
      document.querySelector("#message")?.textContent?.includes(expected),
    text
  );
}

async function main() {
  await ensureOutputDir();
  const browser = await chromium.launch({ headless });
  const page = await browser.newPage();

  try {
    await page.goto(`${baseUrl}/settings`, { waitUntil: "networkidle" });
    if (!page.url().endsWith("/login")) {
      throw new Error(`expected redirect to /login, got ${page.url()}`);
    }

    await loginWithPassword(page, password);
    await page.waitForSelector("text=User Settings");

    await page.fill("#display-name", "Admin UI");
    await page.fill("#note", "Updated from Playwright settings test");
    await page.click("#save-profile");
    await waitForMessage(page, "Profile updated");

    const auth = await attachVirtualAuthenticator(page);
    await page.fill("#new-passkey-name", "Browser key");
    await page.click("#register-passkey");
    await waitForMessage(page, "Passkey registered");
    await page.waitForSelector("#passkey-list .passkey-item");

    const passkeyInput = page.locator("#passkey-list .passkey-item input").first();
    await passkeyInput.fill("Primary browser key");
    await page
      .locator("#passkey-list .passkey-item")
      .first()
      .getByRole("button", { name: "Rename" })
      .click();
    await waitForMessage(page, "Passkey updated");

    await page.fill("#current-password", password);
    await page.fill("#new-password", newPassword);
    await page.fill("#new-password-confirm", newPassword);
    await page.click("#change-password");
    await waitForMessage(page, "Password updated");

    await page.click("#logout-button");
    await page.waitForURL("**/login");

    await page.fill("#username", username);
    await page.click("#passkey-login");
    await page.waitForURL("**/settings");
    await page.waitForSelector("text=User Settings");

    await page
      .locator("#passkey-list .passkey-item")
      .first()
      .getByRole("button", { name: "Delete" })
      .click();
    await waitForMessage(page, "Passkey deleted");

    await auth.client.send("WebAuthn.removeVirtualAuthenticator", {
      authenticatorId: auth.authenticatorId
    });

    console.log("settings-ui-playwright: ok");
  } catch (error) {
    const screenshotPath = path.join(outputDir, "settings-ui-failure.png");
    await page.screenshot({ path: screenshotPath, fullPage: true }).catch(() => {});
    console.error(`settings-ui-playwright: failed; screenshot: ${screenshotPath}`);
    throw error;
  } finally {
    await browser.close();
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});

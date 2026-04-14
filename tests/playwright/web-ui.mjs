import fs from "node:fs/promises";
import path from "node:path";
import { execFile as execFileCallback } from "node:child_process";
import { promisify } from "node:util";
import { chromium } from "playwright";

const execFile = promisify(execFileCallback);

const baseUrl = process.env.RUSTRESORT_UI_BASE_URL ?? "http://127.0.0.1:3011";
const username = process.env.RUSTRESORT_UI_USERNAME ?? "admin";
const password = process.env.RUSTRESORT_UI_PASSWORD ?? "admin-password";
const headless = process.env.HEADLESS !== "false";
const outputDir = path.resolve("output/playwright");

async function ensureOutputDir() {
  await fs.mkdir(outputDir, { recursive: true });
}

async function runActivityFixture(fixture, extraArgs = []) {
  await execFile(
    "cargo",
    [
      "run",
      "--quiet",
      "--bin",
      "ui_playwright_activity",
      "--",
      "--base-url",
      baseUrl,
      "--fixture",
      fixture,
      "--local-username",
      username,
      ...extraArgs
    ],
    { cwd: process.cwd() }
  );
}

async function getLocalStatusUri(page, expectedText) {
  return page.evaluate(async (text) => {
    const credentialsResponse = await fetch("/api/v1/accounts/verify_credentials", {
      credentials: "same-origin"
    });
    if (!credentialsResponse.ok) {
      throw new Error(`verify_credentials failed: ${credentialsResponse.status}`);
    }
    const account = await credentialsResponse.json();
    const statusesResponse = await fetch(`/api/v1/accounts/${account.id}/statuses?limit=10`, {
      credentials: "same-origin"
    });
    if (!statusesResponse.ok) {
      throw new Error(`account statuses failed: ${statusesResponse.status}`);
    }
    const statuses = await statusesResponse.json();
    const match = statuses.find((status) => status.content?.includes(text));
    if (!match?.uri) {
      throw new Error("could not resolve local status uri for activity fixture");
    }
    return match.uri;
  }, expectedText);
}

async function login(page) {
  await page.goto(`${baseUrl}/login?next=${encodeURIComponent("/ui")}`, {
    waitUntil: "networkidle"
  });
  await page.waitForURL(/\/login\?next=/);
  await page.fill("#username", username);
  await page.fill("#password", password);
  await page.click("text=Sign in with password");
  await page.waitForURL(/\/ui$/);
  await page.waitForSelector("text=Timeline");
}

async function main() {
  await ensureOutputDir();
  const browser = await chromium.launch({ headless });
  const page = await browser.newPage();

  try {
    await login(page);

    const recoveredDraftText = `Recovered draft ${Date.now()}`;
    await page.fill("#composer-input", recoveredDraftText);
    await page.reload({ waitUntil: "networkidle" });
    await page.waitForSelector("text=Timeline");
    const restoredDraft = await page.inputValue("#composer-input");
    if (restoredDraft !== recoveredDraftText) {
      throw new Error("expected unsent composer draft to survive a reload");
    }
    await page.fill("#composer-input", "");

    const uniqueText = `Playwright UI post ${Date.now()}`;
    const uniqueTextSecond = `${uniqueText} follow-up`;
    await page.fill("#composer-input", uniqueText);
    await page.click("#composer-submit");
    await page.waitForSelector(`text=${uniqueText}`);
    await page.fill("#composer-input", uniqueTextSecond);
    await page.click("#composer-submit");
    await page.waitForSelector(`text=${uniqueTextSecond}`);
    await page.waitForFunction(() => !document.querySelector(".detail-modal"));
    await page.locator(".status-card", { hasText: uniqueTextSecond }).first().click();
    await page.waitForSelector(".status-card.selected");
    await page
      .locator(".status-card", { hasText: uniqueTextSecond })
      .first()
      .locator(".status-thread-link")
      .click();
    await page.waitForSelector(".detail-modal");
    await page.click("#thread-close");
    await page.waitForFunction(() => !document.querySelector(".detail-modal"));

    await page.click(".timeline-header #composer-toggle-popout");
    await page.waitForSelector(".composer-panel-popout");
    await page.click(".composer-panel-popout #composer-close-popout");
    await page.waitForFunction(() => !document.querySelector(".composer-panel-popout"));

    await page.locator(".status-card", { hasText: uniqueTextSecond }).first().waitFor();
    const localStatusUri = await getLocalStatusUri(page, uniqueTextSecond);

    await runActivityFixture("mention");
    await runActivityFixture("like", ["--object-uri", localStatusUri]);
    await runActivityFixture("announce", ["--object-uri", localStatusUri]);

    await page.click("#refresh-feed");
    await page.waitForSelector(".notification-card");
    const mentionNotifications = await page.evaluate(async () => {
      const response = await fetch("/api/v1/notifications?types[]=mention", {
        credentials: "same-origin"
      });
      if (!response.ok) {
        throw new Error(`notifications fetch failed: ${response.status}`);
      }
      return response.json();
    });
    if (!Array.isArray(mentionNotifications) || mentionNotifications.length === 0) {
      throw new Error("expected at least one mention notification after signed remote activity");
    }
    const activityNotifications = await page.evaluate(async () => {
      const response = await fetch(
        "/api/v1/notifications?types[]=favourite&types[]=reblog&types[]=follow&types[]=status",
        {
          credentials: "same-origin"
        }
      );
      if (!response.ok) {
        throw new Error(`activity notifications fetch failed: ${response.status}`);
      }
      return response.json();
    });
    const activityTypes = new Set(activityNotifications.map((value) => value.type));
    if (!activityTypes.has("favourite") || !activityTypes.has("reblog")) {
      throw new Error("expected favourite and reblog notifications after signed remote activity");
    }
    await page.click("#refresh-feed");
    await page.waitForFunction(() => {
      const cards = Array.from(document.querySelectorAll(".notification-card"));
      return cards.some((card) => !card.classList.contains("empty"));
    });
    await page.waitForSelector("text=Likes");
    await page.waitForSelector("text=Boosts");
    await page.waitForSelector("text=Mentions");
    await page.waitForSelector(`text=${uniqueTextSecond}`);

    console.log("web-ui-playwright: ok");
  } catch (error) {
    const screenshotPath = path.join(outputDir, "web-ui-failure.png");
    await page.screenshot({ path: screenshotPath, fullPage: true }).catch(() => {});
    console.error(`web-ui-playwright: failed; screenshot: ${screenshotPath}`);
    throw error;
  } finally {
    await browser.close();
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});

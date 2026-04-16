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

async function waitForSelectedTimelineStatus(page, expectedText) {
  await page.waitForFunction((text) => {
    const selected = document.querySelector('.timeline-list .status-card[aria-selected="true"]');
    return selected?.textContent?.includes(text) ?? false;
  }, expectedText);
}

function selectedTimelineCard(page) {
  return page.locator('.timeline-list .status-card[aria-selected="true"]').first();
}

function selectedNotificationCard(page) {
  return page.locator('.notification-card[aria-selected="true"]').first();
}

async function waitForSelectedNotificationFocus(page) {
  await page.waitForFunction(() => {
    const selected = document.querySelector('.notification-card[aria-selected="true"]');
    return !!selected && document.activeElement === selected;
  });
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

    const uniqueSeed = Date.now();
    const uniqueText = `Playwright UI alpha ${uniqueSeed}`;
    const uniqueTextSecond = `Playwright UI beta ${uniqueSeed}`;
    await page.fill("#composer-input", uniqueText);
    await page.click("#composer-submit");
    await page.waitForSelector(`text=${uniqueText}`);
    await page.fill("#composer-input", uniqueTextSecond);
    await page.click("#composer-submit");
    await page.waitForSelector(`text=${uniqueTextSecond}`);
    await page.waitForFunction(
      ([firstText, secondText]) => {
        const cards = Array.from(document.querySelectorAll(".timeline-list .status-card"))
          .map((card) => card.textContent ?? "");
        return cards.some((text) => text.includes(firstText)) &&
          cards.some((text) => text.includes(secondText));
      },
      [uniqueText, uniqueTextSecond]
    );
    await page.waitForFunction(() => !document.querySelector(".detail-modal"));
    await page.locator(".status-card", { hasText: uniqueTextSecond }).first().click();
    await page.waitForSelector('.timeline-list .status-card[aria-selected="true"]');
    await waitForSelectedTimelineStatus(page, uniqueTextSecond);
    await selectedTimelineCard(page).waitFor();
    await selectedTimelineCard(page).focus();

    await selectedTimelineCard(page).press("k");
    await waitForSelectedTimelineStatus(page, uniqueText);
    await selectedTimelineCard(page).press("j");
    await waitForSelectedTimelineStatus(page, uniqueTextSecond);

    await selectedTimelineCard(page).press("d");
    await page.waitForSelector('.detail-modal[role="dialog"]');
    await page.waitForSelector('.detail-modal [aria-selected="true"]:focus');
    await page.keyboard.press("Escape");
    await page.waitForFunction(() => !document.querySelector(".detail-modal"));

    await selectedTimelineCard(page).press("n");
    await page.waitForSelector(".composer-panel-popout");
    await page.keyboard.press("Escape");
    await page.waitForFunction(() => !document.querySelector(".composer-panel-popout"));
    await page.fill("#composer-input", "");

    await selectedTimelineCard(page).press("Shift+/");
    await page.waitForSelector('.shortcut-modal[role="dialog"]');
    await page.waitForSelector('.shortcut-modal[role="dialog"] #shortcut-modal-title');
    await page.click("#shortcut-help-close");
    await page.waitForFunction(() => !document.querySelector(".shortcut-modal"));

    await selectedTimelineCard(page).press("Shift+N");
    await page.waitForSelector(".composer-panel-popout");
    const mentionDraft = await page.inputValue("#composer-input");
    if (!mentionDraft.includes(`@${username}`)) {
      throw new Error("expected Shift+N mention shortcut to seed a mention draft");
    }
    await page.keyboard.press("Escape");
    await page.waitForFunction(() => !document.querySelector(".composer-panel-popout"));
    await page.fill("#composer-input", "");

    await page
      .locator(".status-card", { hasText: uniqueTextSecond })
      .first()
      .locator(".status-thread-link")
      .click();
    await page.waitForSelector('.detail-modal[role="dialog"]');
    await page.waitForSelector('.detail-modal [aria-selected="true"]');
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
    await page.locator(".notification-card").first().click();
    await page.waitForSelector('.notification-card[aria-selected="true"]');
    await page.waitForSelector('.app-shell[data-active-pane="notifications"]');
    await waitForSelectedNotificationFocus(page);
    await selectedNotificationCard(page).press("Shift+Tab");
    await page.waitForSelector('.app-shell[data-active-pane="timeline"]');
    await page.waitForSelector('.timeline-list .status-card[aria-selected="true"]:focus');
    await selectedTimelineCard(page).press("Tab");
    await page.waitForSelector('.app-shell[data-active-pane="notifications"]');
    await waitForSelectedNotificationFocus(page);

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

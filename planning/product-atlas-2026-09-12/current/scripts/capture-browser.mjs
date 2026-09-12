#!/usr/bin/env node
/**
 * Re-run the browser-extension reference capture against an already paired
 * production extension. capture-browser.sh owns the fixture and Chrome
 * lifecycle; this file owns only CDP targets and screenshots.
 */
import { chromium } from "/home/nnex/dev/prj/cockpit/poc/interactive-browser-panel/node_modules/playwright/index.mjs";
import { writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";

const cdpUrl = process.env.COCKPIT_CDP_URL ?? "http://127.0.0.1:4206";
const fixtureUrl = process.env.COCKPIT_FIXTURE_URL ?? "http://127.0.0.1:4205/";
const extensionDir = process.env.COCKPIT_EXTENSION_DIR;
const extensionId = process.env.COCKPIT_EXTENSION_ID ?? "fblkilbfbmpndnfjaacljmcljhepakok";
const outputDir = process.env.COCKPIT_BROWSER_OUTPUT ?? new URL("../screenshots/", import.meta.url).pathname;

if (!extensionDir) throw new Error("COCKPIT_EXTENSION_DIR must point to the generated production bundle");

const browser = await chromium.connectOverCDP(cdpUrl);
const context = browser.contexts()[0];
const cdp = await browser.newBrowserCDPSession();
const shot = name => `${outputDir.replace(/\/$/, "")}/${name}`;
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));

try {
  await cdp.send("Extensions.loadUnpacked", { path: extensionDir });
  let page = context.pages().find(candidate => candidate.url() === fixtureUrl);
  if (!page) {
    const target = await cdp.send("Target.createTarget", { url: fixtureUrl, forTab: true, focus: true });
    await wait(300);
    page = context.pages().find(candidate => candidate.url() === fixtureUrl);
    if (!page) throw new Error(`created target ${target.targetId} did not attach to Playwright`);
  }

  const target = await cdp.send("Target.createTarget", { url: fixtureUrl, forTab: true, focus: true });
  await cdp.send("Extensions.triggerAction", { id: extensionId, targetId: target.targetId });
  await wait(1200);
  const popup = context.pages().find(candidate => candidate.url().includes("/popup.html"));
  if (popup) await popup.screenshot({ path: shot("browser-popup-connected.png") });

  page = context.pages().find(candidate => candidate.url() === fixtureUrl) ?? page;
  if (!await page.locator("cockpit-feedback-overlay").count()) throw new Error("annotation toolbar did not open");
  await page.screenshot({ path: shot("browser-toolbar-desktop.png") });

  const targetSession = await context.newCDPSession(page);
  await targetSession.send("Emulation.setDeviceMetricsOverride", {
    width: 480, height: 900, deviceScaleFactor: 1, mobile: false, screenWidth: 480, screenHeight: 900,
  });
  await wait(400);
  const portrait = await targetSession.send("Page.captureScreenshot", { format: "png", fromSurface: true });
  writeFileSync(shot("browser-toolbar-portrait.png"), Buffer.from(portrait.data, "base64"));
  await targetSession.send("Emulation.clearDeviceMetricsOverride").catch(() => {});
  if (process.env.COCKPIT_UI_SESSION) {
    execFileSync(process.env.COCKPIT_PLAYWRIGHT_CLI ?? "/home/linuxbrew/.linuxbrew/bin/playwright-cli", [
      `-s=${process.env.COCKPIT_UI_SESSION}`, "run-code", "--filename",
      new URL("./capture-feedback.js", import.meta.url).pathname,
    ], { stdio: "inherit" });
  }
  console.log(JSON.stringify({ cdpUrl, fixtureUrl, extensionId, outputDir, captured: ["popup-connected", "toolbar-desktop", "toolbar-portrait"] }));
} finally {
  // The wrapper terminates the owned Chrome process. Do not close a shared
  // browser profile or a user's default session from this helper.
  await browser.close().catch(() => {});
}

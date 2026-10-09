// Checks the dx-built `examples/dioxus` in headless Chrome: the worker the `dioxus` feature bundles,
// the web app manifest, installability, and real pushes to an open and a closed page.
//
// node ci/web/example.mjs keys <vapid.json>    writes a VAPID pair and prints its public key
// node ci/web/example.mjs check <site> <vapid.json>
import { readFileSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { extname, join, normalize, relative } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import puppeteer from "puppeteer-core";
import webpush from "web-push";

const [command, ...args] = process.argv.slice(2);
if (command === "keys") {
  const pair = webpush.generateVAPIDKeys();
  writeFileSync(args[0], JSON.stringify(pair));
  console.log(pair.publicKey);
  process.exit(0);
}
if (command !== "check") {
  console.error("usage: example.mjs keys <vapid.json> | check <site> <vapid.json>");
  process.exit(2);
}

const [site, keys] = args;
const vapid = JSON.parse(readFileSync(keys, "utf8"));
webpush.setVapidDetails("mailto:pushups-ci@users.noreply.github.com", vapid.publicKey, vapid.privateKey);
const STEP_MS = 60_000;
const PUSH_GONE_RETRY_MS = 30_000;
const types = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".wasm": "application/wasm",
  ".png": "image/png",
  ".webmanifest": "application/manifest+json",
};
const server = createServer((request, response) => {
  const path = new URL(request.url, "http://x").pathname;
  const file = normalize(join(site, path === "/" ? "index.html" : path));
  try {
    if (relative(site, file).startsWith("..")) throw new Error("outside the site");
    const body = readFileSync(file);
    response.writeHead(200, { "content-type": types[extname(file)] ?? "application/octet-stream" });
    response.end(body);
  } catch {
    response.writeHead(404).end();
  }
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
const origin = `http://127.0.0.1:${server.address().port}`;

const lines = [];
const expect = (ok, what) => {
  if (!ok) throw new Error(what);
  console.log(`ok: ${what}`);
};
const awaitLine = async (what, test) => {
  const end = performance.now() + STEP_MS;
  while (performance.now() < end) {
    const hit = lines.find(test);
    if (hit) return hit;
    await sleep(250);
  }
  throw new Error(`no ${what} within ${STEP_MS / 1000} s`);
};

const browser = await puppeteer.launch({ headless: true, channel: "chrome", args: ["--no-first-run"] });
try {
  await browser.setPermission(origin, { permission: { name: "notifications" }, state: "granted" });
  const open = async () => {
    const page = await browser.newPage();
    page.on("console", (message) => lines.push(message.text()));
    await page.goto(origin);
    await page.waitForSelector("button", { timeout: STEP_MS });
    return page;
  };
  let page = await open();

  const cdp = await page.createCDPSession();
  const manifest = await cdp.send("Page.getAppManifest");
  expect(manifest.url === `${origin}/manifest.webmanifest`, "the page links its manifest");
  expect(manifest.errors.length === 0, `the manifest parses (${JSON.stringify(manifest.errors)})`);
  for (const icon of JSON.parse(manifest.data).icons) {
    const status = await page.evaluate(async (src) => (await fetch(src)).status, icon.src);
    expect(status === 200, `icon ${icon.src} (${icon.purpose}) loads`);
  }

  await page.$$eval("button", (buttons) => buttons.find((b) => b.textContent.includes("Allow"))?.click());
  const token = await awaitLine("token", (l) => l.includes("ui token") && l.includes("subscription="));
  const subscription = JSON.parse(token.split("subscription=")[1]);
  const registrations = await page.evaluate(async () =>
    (await navigator.serviceWorker.getRegistrations()).map((r) => ({
      scope: r.scope,
      script: (r.active ?? r.waiting ?? r.installing)?.scriptURL,
    })),
  );
  expect(
    registrations.length === 1 &&
      registrations[0].scope === `${origin}/assets/` &&
      registrations[0].script === `${origin}/assets/pushups-sw.js?pushups=static`,
    `the bundled worker is registered (${JSON.stringify(registrations)})`,
  );

  // Chrome rechecks installability once the worker settles, so it is asked until it answers or the step ends.
  let errors = [];
  const settled = performance.now() + STEP_MS;
  do {
    errors = (await cdp.send("Page.getInstallabilityErrors")).installabilityErrors;
    if (errors.length === 0) break;
    await sleep(500);
  } while (performance.now() < settled);
  expect(errors.length === 0, `Chrome finds the page installable (${JSON.stringify(errors)})`);

  // FCM answers 410 to a subscription it has not propagated yet, so a 410 is resent for a while, as the harness does.
  const send = async (seq) => {
    const payload = JSON.stringify({ seq: String(seq), sent_at_ms: String(Date.now()) });
    const deadline = performance.now() + PUSH_GONE_RETRY_MS;
    for (;;) {
      try {
        const sent = await webpush.sendNotification(subscription, payload, { TTL: 300, timeout: STEP_MS });
        expect(sent.statusCode === 201 || sent.statusCode === 200, `push ${seq} is accepted (HTTP ${sent.statusCode})`);
        return;
      } catch (error) {
        if (error.statusCode !== 410 || performance.now() > deadline) throw error;
        console.log(`push ${seq}: HTTP 410, retrying`);
        await sleep(1000);
      }
    }
  };
  await send(1);
  await awaitLine("message on the open page", (l) => l.includes("ui message") && l.includes("seq=1 "));
  console.log("ok: a push reaches the open page");

  await page.close();
  await send(2);
  // The worker keeps the push for the next page, which drains it on load.
  await sleep(5_000);
  page = await open();
  await awaitLine("drained message", (l) => l.includes("ui message") && l.includes("seq=2 "));
  console.log("ok: a push to a closed page reaches the next page");
} finally {
  await browser.close();
  server.close();
}

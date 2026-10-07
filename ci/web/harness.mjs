// Drives the probe through pushups in a real browser, with real pushes through the browser's own push service, and saves the coverage counters of the Rust backend and of the shim.
//
// node harness.mjs chrome|firefox <out dir> [scenario...]
// Chrome is the installed stable channel, and FIREFOX_PATH names the Firefox binary.

import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { extname, join, normalize, resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { setTimeout as sleep } from "node:timers/promises";
import puppeteer from "puppeteer-core";
import webpush from "web-push";

const [engine, outArgument, ...only] = process.argv.slice(2);
if (!["chrome", "firefox"].includes(engine) || !outArgument) {
  throw new Error("usage: node harness.mjs chrome|firefox <out dir> [scenario...]");
}
const out = resolve(outArgument);
const site = resolve(import.meta.dirname, "site");
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm" };
const WAIT_MS = 30_000;
// A fresh Chrome profile checks in with Google before its first subscription, which took 33 s.
const SUBSCRIBE_MS = 120_000;
const PUSH_TIMEOUT_MS = 15_000;
// FCM answered 410 to a subscription made a quarter second earlier in 4 of 12 tries, and 201 three seconds later.
const PUSH_GONE_RETRY_MS = 30_000;

const vapid = webpush.generateVAPIDKeys();
webpush.setVapidDetails("mailto:pushups-ci@users.noreply.github.com", vapid.publicKey, vapid.privateKey);
const vapidPublicKey = [...Buffer.from(vapid.publicKey, "base64url")];

function log(line) {
  console.log(`[${engine}] ${line}`);
}

function expect(condition, what) {
  if (!condition) throw new Error(`expected ${what}`);
}

/** Polls `check` until it returns a truthy value, failing once `ms` pass on the monotonic clock. */
async function until(what, check, ms = WAIT_MS) {
  const deadline = performance.now() + ms;
  for (;;) {
    const value = await check();
    if (value) return value;
    if (performance.now() > deadline) throw new Error(`timed out after ${ms} ms waiting for ${what}`);
    await sleep(200);
  }
}

function serve() {
  const server = createServer((request, response) => {
    const path = normalize(new URL(request.url, "http://localhost").pathname);
    const file = join(site, path === "/" ? "index.html" : path);
    if (!file.startsWith(site)) {
      response.writeHead(403).end();
      return;
    }
    try {
      const body = readFileSync(file);
      response.writeHead(200, { "Content-Type": TYPES[extname(file)] ?? "application/octet-stream" });
      response.end(body);
    } catch {
      response.writeHead(404).end();
    }
  });
  return new Promise((done) => {
    server.listen(0, "127.0.0.1", () => done({ server, origin: `http://127.0.0.1:${server.address().port}` }));
  });
}

let saved = 0;

/** Reads the Rust and the Istanbul counters of the realm it runs in, page or worker. */
function readCounters() {
  const capture = globalThis.pushupsProbeCoverage;
  let rust = null;
  if (capture) {
    const bytes = capture();
    let binary = "";
    for (let at = 0; at < bytes.length; at += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(at, at + 0x8000));
    }
    rust = btoa(binary);
  }
  return { rust, js: globalThis.__coverage__ ?? null };
}

function save(label, counters) {
  saved += 1;
  const name = `${engine}-${String(saved).padStart(2, "0")}-${label}`;
  if (counters.rust) writeFileSync(join(out, "rust", `${name}.profraw`), Buffer.from(counters.rust, "base64"));
  if (counters.js) writeFileSync(join(out, "js", `${name}.json`), JSON.stringify(counters.js));
}

/** Saves the counters of every running service worker, which only Chrome exposes. */
async function saveWorkers(browser, label) {
  if (engine !== "chrome") return;
  for (const target of browser.targets().filter((target) => target.type() === "service_worker")) {
    const worker = await target.worker();
    if (worker) save(`${label}-worker`, await worker.evaluate(readCounters));
  }
}

/** Sends one push, retrying a 410 for a while, since FCM gives it for a subscription it has not propagated yet. */
async function push(token, payload) {
  const subscription = { endpoint: token.endpoint, keys: { p256dh: token.p256dh, auth: token.auth } };
  const deadline = performance.now() + PUSH_GONE_RETRY_MS;
  for (;;) {
    try {
      const sent = await webpush.sendNotification(subscription, payload, { TTL: 300, timeout: PUSH_TIMEOUT_MS });
      log(`sent ${JSON.stringify(payload)}: HTTP ${sent.statusCode}`);
      return;
    } catch (error) {
      if (error.statusCode !== 410 || performance.now() > deadline) throw error;
      log(`sending ${JSON.stringify(payload)}: HTTP 410, retrying`);
      await sleep(1000);
    }
  }
}

class Probe {
  static async open(browser, origin) {
    const page = await browser.newPage();
    page.on("console", (message) => log(`page: ${message.text()}`));
    page.on("pageerror", (error) => log(`page error: ${error.message}`));
    await page.goto(`${origin}/`);
    await until("the probe to load", () => page.evaluate(() => Boolean(window.probe)));
    return new Probe(page);
  }

  constructor(page) {
    this.page = page;
    this.events = [];
  }

  async install(rustHandler) {
    const path = rustHandler ? null : "/pushups-sw.js";
    await this.page.evaluate(
      (key, rust, worker) => window.probe.install(new Uint8Array(key), rust, worker),
      vapidPublicKey,
      rustHandler,
      path,
    );
    await this.page.evaluate(() => window.probe.listen());
  }

  async subscribe() {
    const permission = await this.page.evaluate(() => window.probe.requestPermission());
    await this.page.evaluate(() => window.probe.register());
    return permission;
  }

  async event(what, matches, ms = WAIT_MS) {
    try {
      return await until(what, async () => {
        this.events.push(...(await this.page.evaluate(() => window.probe.takeEvents())));
        return this.events.find(matches);
      }, ms);
    } catch (error) {
      throw new Error(`${error.message}, having received ${JSON.stringify(this.events)}`);
    }
  }

  async message(payload) {
    const message = await this.event(`the push ${JSON.stringify(payload)}`, (event) =>
      event.kind === "message" && event.payload === payload);
    expect(message.startedApp === false, "a push that started no app");
    return message;
  }

  notifications() {
    return this.page.evaluate(async () => {
      const registration = await navigator.serviceWorker.getRegistration();
      const shown = (await registration?.getNotifications()) ?? [];
      return shown.map(({ title, body, tag, data }) => ({ title, body, tag, data }));
    });
  }

  notification(what, matches) {
    return until(what, async () => (await this.notifications()).find(matches));
  }

  clearNotifications() {
    return this.page.evaluate(async () => {
      const registration = await navigator.serviceWorker.getRegistration();
      for (const shown of (await registration?.getNotifications()) ?? []) shown.close();
    });
  }

  async close(label) {
    save(label, await this.page.evaluate(readCounters));
    await this.page.close();
  }
}

/** Runs `body` in a browser with a fresh profile, notifications set to `permission`. */
async function withBrowser(origin, permission, body) {
  // Under the output directory, since a snap Firefox cannot see the system temporary directory.
  const profile = mkdtempSync(join(out, "profile-"));
  const options = engine === "chrome"
    ? {
      channel: "chrome",
      userDataDir: profile,
      args: ["--no-first-run", "--no-default-browser-check"],
    }
    : {
      browser: "firefox",
      executablePath: process.env.FIREFOX_PATH,
      userDataDir: profile,
      extraPrefsFirefox: {
        // The automation profile turns the push service off.
        "dom.push.connection.enabled": true,
        "dom.push.serverURL": "wss://push.services.mozilla.com/",
        // Headless Firefox has no system notifications, so it shows them in the browser.
        "alerts.useSystemBackend": false,
      },
    };
  const browser = await puppeteer.launch({ headless: true, ...options });
  browser.on("targetcreated", (target) => {
    if (target.type() !== "service_worker") return;
    target.worker().then(
      (worker) => {
        worker?.on("console", (message) => log(`worker: ${message.text()}`));
        worker?.on("error", (error) => log(`worker error: ${error.message}`));
      },
      (error) => log(`could not attach to ${target.url()}: ${error.message}`),
    );
  });
  try {
    await browser.setPermission(origin, { permission: { name: "notifications" }, state: permission });
    await body(browser);
  } finally {
    await browser.close();
    rmSync(profile, { recursive: true, force: true });
  }
}

async function misuse(origin) {
  await withBrowser(origin, "granted", async (browser) => {
    const probe = await Probe.open(browser, origin);
    const outcomes = await probe.page.evaluate(() => window.probe.misuse());
    const wanted = [
      "in_service_worker=false",
      "register=Err(NotConfigured)",
      "install=Err(NotConfigured)",
      "serve_service_worker=Err(NotInServiceWorker)",
    ];
    expect(JSON.stringify(outcomes) === JSON.stringify(wanted), `${wanted}, got ${outcomes}`);
    await probe.close("misuse");
  });
}

async function denied(origin) {
  await withBrowser(origin, "denied", async (browser) => {
    const probe = await Probe.open(browser, origin);
    await probe.install(false);
    const permission = await probe.subscribe();
    expect(permission === "denied", `the denied permission, got ${permission}`);
    const failed = await probe.event("the failed registration", (event) => event.kind === "registrationFailed");
    log(`registration failed: ${failed.error}`);
    await probe.close("denied");
  });
}

/** A push the page missed while suspended, stored as the worker stores it, reaches the handler once the page is shown again. */
async function resumed(origin) {
  await withBrowser(origin, "granted", async (browser) => {
    const probe = await Probe.open(browser, origin);
    await probe.install(false);
    await probe.page.evaluate(async () => {
      const db = await new Promise((resolve, reject) => {
        const request = indexedDB.open("pushups", 1);
        request.onupgradeneeded = () => {
          request.result.createObjectStore("queue", { autoIncrement: true });
          request.result.createObjectStore("options");
        };
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      await new Promise((resolve, reject) => {
        const transaction = db.transaction("queue", "readwrite");
        const payload = new TextEncoder().encode("missed-while-suspended");
        transaction.objectStore("queue").add({ kind: "message", payload, startedApp: false });
        transaction.oncomplete = resolve;
        transaction.onerror = () => reject(transaction.error);
      });
      db.close();
    });
    // Nothing else drains it, since no worker message announces it.
    await sleep(1500);
    probe.events.push(...(await probe.page.evaluate(() => window.probe.takeEvents())));
    expect(!probe.events.some((event) => event.payload === "missed-while-suspended"), "no drain before the page is shown");
    await probe.page.evaluate(() => document.dispatchEvent(new Event("visibilitychange")));
    await probe.message("missed-while-suspended");
    await probe.close("resumed");
  });
}

/** A push with the page open, a declarative push, then a push with no page, drained later. */
async function staticWorker(origin) {
  await withBrowser(origin, "granted", async (browser) => {
    const probe = await Probe.open(browser, origin);
    await probe.install(false);
    const permission = await probe.subscribe();
    expect(permission === "granted", `the granted permission, got ${permission}`);
    const token = await probe.event("the token", (event) => event.kind === "token", SUBSCRIBE_MS);
    log(`subscribed at ${new URL(token.endpoint).host}`);

    await push(token, "static-open");
    await probe.message("static-open");
    const host = new URL(origin).host;
    await probe.notification("the default notification", (shown) => shown.title === host);

    const declared = JSON.stringify({
      web_push: 8030,
      notification: { title: "declared", body: "by the server", navigate: `${origin}/?from=declared` },
    });
    await push(token, declared);
    await probe.message(declared);
    const shown = await probe.notification("the declared notification", (shown) => shown.title === "declared");
    expect(shown.data.navigate === `${origin}/?from=declared`, "the declared navigation");
    await probe.clearNotifications();
    await probe.close("static-open");

    await push(token, "static-closed");
    const later = await Probe.open(browser, origin);
    await later.notification("the notification of the push with no page", (shown) => shown.title === host);
    await later.install(false);
    await later.message("static-closed");
    await saveWorkers(browser, "static");
    await later.close("static-drain");
  });
}

/** The Rust handler builds the notification, page open or not, in a running worker, since puppeteer stalls a cold one. */
async function rustWorker(origin) {
  await withBrowser(origin, "granted", async (browser) => {
    const probe = await Probe.open(browser, origin);
    await probe.install(true);
    await probe.subscribe();
    const token = await probe.event("the token", (event) => event.kind === "token", SUBSCRIBE_MS);

    await push(token, "rust-open");
    await probe.message("rust-open");
    const shown = await probe.notification("the notification built in Rust", (shown) => shown.body === "rust-open");
    expect(shown.title === "probe from Rust", `the Rust title, got ${shown.title}`);
    expect(shown.tag === "probe", `the Rust tag, got ${shown.tag}`);
    expect(shown.data.navigate === "/?from=rust", `the Rust navigation, got ${shown.data.navigate}`);
    await probe.clearNotifications();
    await probe.close("rust-open");

    await push(token, "rust-closed");
    const later = await Probe.open(browser, origin);
    await later.notification("the notification of the push with no page", (shown) => shown.body === "rust-closed");
    await later.install(true);
    await later.message("rust-closed");
    await saveWorkers(browser, "rust");
    await later.close("rust-drain");
  });
}

mkdirSync(join(out, "rust"), { recursive: true });
mkdirSync(join(out, "js"), { recursive: true });
const { server, origin } = await serve();
try {
  const scenarios = [misuse, denied, resumed, staticWorker, rustWorker];
  for (const scenario of scenarios.filter(({ name }) => only.length === 0 || only.includes(name))) {
    log(`scenario ${scenario.name}`);
    await scenario(origin);
  }
  log("all scenarios passed");
} finally {
  server.close();
}

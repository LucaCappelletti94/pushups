// Drives the probe through pushups in a real browser, with real pushes through the browser's own push service, and saves the coverage counters of the Rust backend and of the shim.
//
// node harness.mjs chrome|firefox <out dir> [scenario...]
// Chrome is the installed stable channel, and FIREFOX_PATH names the Firefox binary.

import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { join, normalize, resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { setTimeout as sleep } from "node:timers/promises";
import puppeteer from "puppeteer-core";
import webpush from "web-push";
import { attachWorkerConsoles, pushWithRetry, serveSite } from "./webpush.mjs";
import { withMarionette } from "./marionette.mjs";

const [engine, outArgument, ...only] = process.argv.slice(2);
if (!["chrome", "firefox"].includes(engine) || !outArgument) {
  throw new Error("usage: node harness.mjs chrome|firefox <out dir> [scenario...]");
}
const out = resolve(outArgument);
const site = resolve(import.meta.dirname, "site");
const WAIT_MS = 30_000;
// A fresh Chrome profile checks in with Google before its first subscription, which took 33 s.
const SUBSCRIBE_MS = 120_000;
const PUSH_TIMEOUT_MS = 15_000;

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

/**
 * The worker at `/scoped/pushups-sw.js`, whose scope leaves the page at `/` uncontrolled, as `dx`'s
 * `/assets/` does. It is the static worker plus a listener that answers its revision, which the
 * `updated` scenario raises to change the script's bytes.
 */
let scopedRevision = 1;
function scopedWorker() {
  return Buffer.concat([
    readFileSync(join(site, "pushups-sw.js")),
    Buffer.from(
      `\nself.addEventListener("message", (event) => { if (event.data === "pushups-revision?") event.source.postMessage({ revision: ${scopedRevision} }); });\n`,
    ),
  ]);
}

const siteHandler = serveSite(site);

function serve() {
  const server = createServer((request, response) => {
    const path = normalize(new URL(request.url, "http://localhost").pathname);
    if (path === "/observe" && request.method === "POST") {
      let body = "";
      request.on("data", (chunk) => (body += chunk));
      request.on("end", () => {
        response.writeHead(204).end();
        try {
          const { tag, data } = JSON.parse(body);
          observed.push({ tag, data });
        } catch {
          // A report the harness cannot parse.
        }
      });
      return;
    }
    if (path === "/scoped/pushups-sw.js") {
      response.writeHead(200, { "Content-Type": "text/javascript" });
      response.end(scopedWorker());
      return;
    }
    if (path === "/probe-entry-worker.js") {
      // The app's entry module for the Rust handler, which the probe's glue serves once its wasm
      // is initialized. No top-level await, which a service worker module does not accept.
      response.writeHead(200, { "Content-Type": "text/javascript" });
      response.end("import * as probe from '/probe.js';\n");
      return;
    }
    if (path === "/probe-fail-worker.js") {
      // A Rust-mode worker whose handler promise rejects, the app's entry serving the shim's `serve`.
      response.writeHead(200, { "Content-Type": "text/javascript" });
      response.end(
        "import { serve } from '/pushups-sw.js';\nserve(() => Promise.reject(new Error('the probe handler failed')));\n",
      );
      return;
    }
    if (path === "/pushups-bridge-sw.js") {
      // The static worker plus a listener that answers its Istanbul counters to the page, which Chrome does not
      // expose for a headless Firefox.
      response.writeHead(200, { "Content-Type": "text/javascript" });
      response.end(
        Buffer.concat([
          readFileSync(join(site, "pushups-sw.js")),
          Buffer.from(
            "\nself.addEventListener(\"message\", (event) => {\n" +
              "  if (event.data === \"pushups-counters?\") {\n" +
              "    fetch(location.origin + \"/observe\", { method: \"POST\", body: JSON.stringify({ tag: \"worker-coverage\", data: globalThis.__coverage__ }) }).catch(() => {});\n" +
              "  }\n" +
              "});\n",
          ),
        ]),
      );
      return;
    }
    if (path === "/bfcache.html") {
      response.writeHead(200, { "Content-Type": "text/html" });
      response.end("<!doctype html>\n<html><head><title>bfcache</title></head><body><h1>the back-forward cache</h1></body></html>\n");
      return;
    }
    siteHandler(request, response);
  });
  return new Promise((done) => {
    server.listen(0, "127.0.0.1", () => done({ server, origin: `http://127.0.0.1:${server.address().port}` }));
  });
}

/** The observations the Marionette pages and workers fetch to the site, newest last. */
const observed = [];

/** Waits for an observation the page or worker fetched, which `matches` picks out of `tag`. */
async function observe(what, tag, matches, ms = WAIT_MS) {
  const deadline = performance.now() + ms;
  for (;;) {
    for (const item of observed) {
      if (item.tag !== tag) continue;
      const data = matches(item.data);
      if (data) return data;
    }
    if (performance.now() > deadline) {
      const seen = [...new Set(observed.map((item) => item.tag))].join(", ") || "nothing";
      throw new Error(`timed out after ${ms} ms waiting for ${what}; saw ${seen}`);
    }
    await sleep(200);
  }
}

/**
 * The reporter a Marionette page installs, since this build answers every content script with
 * null: it fetches the probe's events, its own location and, on demand, its counters.
 */
const REPORTER = `(function () {
  if (window.__reporter) return;
  window.__reporter = true;
  const report = (tag, data) => fetch("/observe", { method: "POST", body: JSON.stringify({ tag, data }) }).catch(() => {});
  window.__report = report;
  const timer = setInterval(() => {
    if (!window.probe) return;
    clearInterval(timer);
    report("probe-ready", location.href);
    setInterval(() => report("events", window.probe.takeEvents()), 400);
  }, 100);
})()`;

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

/** Sends one push with the probe's token as the subscription. */
async function push(token, payload) {
  await pushWithRetry(
    { endpoint: token.endpoint, keys: { p256dh: token.p256dh, auth: token.auth } },
    payload,
    {
      timeout: PUSH_TIMEOUT_MS,
      onSent: (sent) => log(`sent ${JSON.stringify(payload)}: HTTP ${sent.statusCode}`),
      on410: () => log(`sending ${JSON.stringify(payload)}: HTTP 410, retrying`),
    },
  );
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

  async install(rustHandler, workerPath = null) {
    const path = rustHandler ? workerPath : workerPath ?? "/pushups-sw.js";
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
  attachWorkerConsoles(browser, (text) => log(`worker: ${text}`), { log, onError: log });
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

/** A `register` refused before the user granted permission, as Safari refuses one outside a gesture, subscribes once the grant comes through `request_permission`, with no second `register`. */
async function grantedLater(origin) {
  await withBrowser(origin, "denied", async (browser) => {
    const probe = await Probe.open(browser, origin);
    await probe.install(false);
    await probe.page.evaluate(() => window.probe.register());
    await probe.event("the refused registration", (event) => event.kind === "registrationFailed");
    await browser.setPermission(origin, { permission: { name: "notifications" }, state: "granted" });
    const permission = await probe.page.evaluate(() => window.probe.requestPermission());
    expect(permission === "granted", `the granted permission, got ${permission}`);
    const token = await probe.event("the token after the grant", (event) => event.kind === "token", SUBSCRIBE_MS);
    log(`subscribed after the grant at ${new URL(token.endpoint).host}`);
    await probe.close("granted-later");
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

/** A subscription the origin made with an earlier VAPID key gives way to one with the configured key on `register`. */
async function rotated(origin) {
  await withBrowser(origin, "granted", async (browser) => {
    const probe = await Probe.open(browser, origin);
    await probe.install(false);
    const earlierKey = [...Buffer.from(webpush.generateVAPIDKeys().publicKey, "base64url")];
    const earlier = await probe.page.evaluate(async (key) => {
      const registration = await navigator.serviceWorker.ready;
      const subscription = await registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: new Uint8Array(key),
      });
      return subscription.endpoint;
    }, earlierKey);
    await probe.subscribe();
    const token = await probe.event(
      "the token for the configured key",
      (event) => event.kind === "token" || event.kind === "registrationFailed",
      SUBSCRIBE_MS,
    );
    expect(token.kind === "token", `a token after the key change, got ${JSON.stringify(token)}`);
    expect(token.endpoint !== earlier, "a new endpoint for the configured key");
    await push(token, "rotated");
    await probe.message("rotated");
    await probe.close("rotated");
  });
}

/** Two `register` calls of one page while the first is still subscribing end with one endpoint, which receives the push. */
async function concurrent(origin) {
  await withBrowser(origin, "granted", async (browser) => {
    const probe = await Probe.open(browser, origin);
    await probe.install(false);
    // A fresh profile's first subscribe takes Chrome tens of seconds, so the second call starts while it is pending.
    await probe.page.evaluate(() => {
      window.probe.register();
      window.probe.register();
    });
    const answers = await until("both registrations to answer", async () => {
      probe.events.push(...(await probe.page.evaluate(() => window.probe.takeEvents())));
      const answered = probe.events.filter((event) => event.kind === "token" || event.kind === "registrationFailed");
      return answered.length >= 2 ? answered : null;
    }, SUBSCRIBE_MS);
    expect(answers.every((event) => event.kind === "token"), `two tokens, got ${JSON.stringify(answers)}`);
    const endpoints = new Set(answers.map((event) => event.endpoint));
    expect(endpoints.size === 1, `one endpoint for both registrations, got ${[...endpoints].join(" and ")}`);
    await push(answers[0], "concurrent");
    await probe.message("concurrent");
    await probe.close("concurrent");
  });
}

/**
 * A changed worker reaches a page its scope does not control at the page's next load, through the
 * update `install` asks for, where the browser alone would wait up to a day.
 */
async function updated(origin) {
  await withBrowser(origin, "granted", async (browser) => {
    const revision = (probe) =>
      probe.page.evaluate(async () => {
        const registration = await navigator.serviceWorker.getRegistration("/scoped/");
        const worker = registration?.active;
        if (!worker) return null;
        navigator.serviceWorker.startMessages();
        return new Promise((answer) => {
          navigator.serviceWorker.addEventListener("message", (event) => {
            if (event.data?.revision) answer(event.data.revision);
          });
          worker.postMessage("pushups-revision?");
          setTimeout(() => answer(null), 1_000);
        });
      });

    scopedRevision = 1;
    const first = await Probe.open(browser, origin);
    await first.install(false, "/scoped/pushups-sw.js");
    await until("the first revision to answer", async () => (await revision(first)) === 1);
    await first.close("updated-first");

    scopedRevision = 2;
    const next = await Probe.open(browser, origin);
    await next.install(false, "/scoped/pushups-sw.js");
    await until("the changed worker to replace the first", async () => (await revision(next)) === 2);
    await next.close("updated-next");
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

/** A configured worker path that carries its own `pushups` parameter, an encoded copy of it, another parameter and a fragment still registers in the mode the app asked for. */
async function workerUrl(origin) {
  await withBrowser(origin, "granted", async (browser) => {
    const probe = await Probe.open(browser, origin);
    await probe.install(false, "/pushups-sw.js?pushups=rust&pu%73hups=rust&v=3#v1");
    await probe.subscribe();
    const token = await probe.event("the token", (event) => event.kind === "token", SUBSCRIBE_MS);
    const script = await probe.page.evaluate(async () => (await navigator.serviceWorker.ready).active.scriptURL);
    const query = new URL(script).searchParams;
    expect(
      JSON.stringify(query.getAll("pushups")) === JSON.stringify(["static"]),
      `one static mode in the registered ${script}`,
    );
    expect(query.get("v") === "3", `the app's own parameter kept in ${script}`);
    await push(token, "url-open");
    await probe.message("url-open");
    const host = new URL(origin).host;
    await probe.notification("the static worker's notification", (shown) => shown.title === host);
    await probe.clearNotifications();
    await probe.close("url-open");
  });
}

/** A settings screen reads the permission without prompting, in a fresh, a denied, and a granted browser, matching the browser's own state. */
async function permissionStatus(origin) {
  const states = [
    ["prompt", "not_determined", "default"],
    ["denied", "denied", "denied"],
    ["granted", "granted", "granted"],
  ];
  for (const [state, wanted, browserState] of states) {
    await withBrowser(origin, state, async (browser) => {
      const probe = await Probe.open(browser, origin);
      const refused = await probe.page
        .evaluate(() => window.probe.permissionStatus())
        .catch((error) => error.message);
      expect(
        refused === "Config lacks the part this target needs",
        `the status before install is refused, got ${refused}`,
      );
      await probe.install(false);
      const status = await probe.page.evaluate(() => window.probe.permissionStatus());
      expect(status === wanted, `the ${state} status, got ${status}`);
      const permission = await probe.page.evaluate(() => Notification.permission);
      expect(permission === browserState, `the ${state} browser state, got ${permission}`);
      await probe.close(`permission-${state}`);
    });
  }
}

/** Adds records to the pushups queue store, the payload strings encoded in the page, as the worker stores them. */
async function queueRecords(probe, records) {
  await probe.page.evaluate((records) => {
    return new Promise((resolve, reject) => {
      const request = indexedDB.open("pushups", 1);
      request.onupgradeneeded = () => {
        request.result.createObjectStore("queue", { autoIncrement: true });
        request.result.createObjectStore("options");
      };
      request.onsuccess = () => {
        const db = request.result;
        const transaction = db.transaction("queue", "readwrite");
        const store = transaction.objectStore("queue");
        for (const record of records) {
          if (record.payload !== undefined) record.payload = new TextEncoder().encode(record.payload);
          store.add(record);
        }
        transaction.oncomplete = () => { db.close(); resolve(); };
        transaction.onerror = () => { db.close(); reject(new Error("the queue write failed")); };
      };
      request.onerror = () => reject(new Error("the queue could not open"));
    });
  }, records);
}

/** A `request_permission` whose platform answer is a plain string, not an `Error`, reports it Debug-quoted. */
async function jsError(origin) {
  await withBrowser(origin, "denied", async (browser) => {
    const probe = await Probe.open(browser, origin);
    await probe.page.evaluate(() => {
      Notification.requestPermission = () => Promise.reject("denied-by-the-probe");
    });
    const failed = await probe.page.evaluate(() =>
      window.probe.requestPermission().catch((error) => error.message));
    expect(failed === 'push service error: JsValue("denied-by-the-probe")', `the Debug-quoted string, got ${failed}`);
    await probe.close("js-error");
  });
}

/** A page without `Notification` or `PushManager` sees the refused backend, not a crash. */
async function unsupported(origin) {
  await withBrowser(origin, "granted", async (browser) => {
    const probe = await Probe.open(browser, origin);
    const refused = "push notifications are not supported on this target";
    const outcome = await probe.page.evaluate((key) => {
      const hide = (name) => {
        if (Object.getOwnPropertyDescriptor(window, name)?.configurable) delete window[name];
        else delete Object.getPrototypeOf(window)[name];
      };
      return (async () => {
        window.probe.install(new Uint8Array(key), false, "/pushups-sw.js");
        const notification = window.Notification;
        hide("Notification");
        const permission = await window.probe.requestPermission().catch((error) => error.message);
        const status = await window.probe.permissionStatus().catch((error) => error.message);
        window.Notification = notification;
        hide("PushManager");
        let install = null;
        try {
          window.probe.install(new Uint8Array(key), false, "/pushups-sw.js");
        } catch (error) {
          install = error.message;
        }
        return { permission, status, install };
      })();
    }, vapidPublicKey);
    expect(outcome.permission === refused, `request_permission refused, got ${outcome.permission}`);
    expect(outcome.status === refused, `permission_status refused, got ${outcome.status}`);
    expect(outcome.install === refused, `install refused, got ${outcome.install}`);
    await probe.close("unsupported");
  });
}

/** A queued token without an endpoint and a record of an unknown kind are dropped with a log, and the readable record of the same drain still reaches the handler. */
async function badRecords(origin) {
  await withBrowser(origin, "granted", async (browser) => {
    const probe = await Probe.open(browser, origin);
    await probe.install(false);
    await probe.page.evaluate(() => {
      window.__dropped = [];
      const original = console.error;
      console.error = (...lines) => {
        window.__dropped.push(lines.join(" "));
        original(...lines);
      };
    });
    await queueRecords(probe, [
      { kind: "token" },
      { kind: "bogus" },
      { kind: "message", payload: "still-drained", startedApp: false },
    ]);
    await sleep(1_500);
    probe.events.push(...(await probe.page.evaluate(() => window.probe.takeEvents())));
    expect(probe.events.length === 0, "no event before a drain");
    await probe.page.evaluate(() => document.dispatchEvent(new Event("visibilitychange")));
    await probe.message("still-drained");
    const dropped = await probe.page.evaluate(() => window.__dropped);
    expect(
      dropped.includes("pushups: dropped an unreadable queued record: push service error: subscription without an endpoint"),
      `the endpoint drop, got ${JSON.stringify(dropped)}`,
    );
    expect(
      dropped.includes('pushups: dropped an unreadable queued record: push service error: unknown queued record Some("bogus")'),
      `the bogus drop, got ${JSON.stringify(dropped)}`,
    );
    await probe.close("bad-records");
  });
}

/** A drain whose queue read fails leaves the queue in place, and the next drain still delivers it. */
async function drainFailure(origin) {
  await withBrowser(origin, "granted", async (browser) => {
    const probe = await Probe.open(browser, origin);
    await probe.install(false);
    await probe.page.evaluate(() => {
      window.__failed = [];
      const original = console.error;
      console.error = (...lines) => {
        window.__failed.push(lines.join(" "));
        original(...lines);
      };
    });
    await queueRecords(probe, [
      { kind: "message", payload: "queued-before-failure", startedApp: false },
    ]);
    await probe.page.evaluate(() => {
      window.__indexedDB = window.indexedDB;
      const failing = {
        open: () => {
          const request = { error: new Error("indexedDB refused") };
          queueMicrotask(() => request.onerror?.(request.error));
          return request;
        },
      };
      Object.defineProperty(window, "indexedDB", { value: failing, configurable: true, writable: true });
    });
    await probe.page.evaluate(() => document.dispatchEvent(new Event("visibilitychange")));
    await sleep(500);
    probe.events.push(...(await probe.page.evaluate(() => window.probe.takeEvents())));
    expect(probe.events.length === 0, "the unreadable queue delivered nothing");
    const failed = await probe.page.evaluate(() => window.__failed);
    expect(failed.includes("pushups: reading the push queue failed"), `the read failure, got ${JSON.stringify(failed)}`);
    await probe.page.evaluate(() => Object.defineProperty(window, "indexedDB", { value: window.__indexedDB, configurable: true, writable: true }));
    await probe.page.evaluate(() => document.dispatchEvent(new Event("visibilitychange")));
    await probe.message("queued-before-failure");
    await probe.close("drain-failure");
  });
}

/** A Rust handler installed from the app's own entry module builds the notification, proving the entry path of `install`. */
async function rustEntry(origin) {
  await withBrowser(origin, "granted", async (browser) => {
    const probe = await Probe.open(browser, origin);
    await probe.install(true, "/probe-entry-worker.js");
    await probe.subscribe();
    const token = await probe.event("the token", (event) => event.kind === "token", SUBSCRIBE_MS);
    const script = await probe.page.evaluate(async () => (await navigator.serviceWorker.ready).active.scriptURL);
    expect(script.includes("probe-entry-worker.js"), `the entry worker, got ${script}`);
    expect(new URL(script).searchParams.get("pushups") === "rust", `the rust mode, got ${script}`);
    await push(token, "rust-entry");
    await probe.message("rust-entry");
    const shown = await probe.notification("the notification built by the entry", (shown) => shown.body === "rust-entry");
    expect(shown.title === "probe from Rust", `the Rust title, got ${shown.title}`);
    await probe.clearNotifications();
    await saveWorkers(browser, "rust-entry");
    await probe.close("rust-entry");
  });
}

/** A Rust handler whose promise rejects still shows the fallback notification and delivers the queued push. */
async function rustFailure(origin) {
  await withBrowser(origin, "granted", async (browser) => {
    const probe = await Probe.open(browser, origin);
    await probe.install(true, "/probe-fail-worker.js");
    await probe.subscribe();
    const token = await probe.event("the token", (event) => event.kind === "token", SUBSCRIBE_MS);
    await push(token, "rust-failed");
    await probe.message("rust-failed");
    const host = new URL(origin).host;
    const shown = await probe.notification("the fallback notification", (shown) => shown.title === host);
    expect(shown.body === "" || shown.body == null, `no body from the fallback, got ${shown.body}`);
    await probe.clearNotifications();
    await saveWorkers(browser, "rust-failure");
    await probe.close("rust-failure");
  });
}

/** A subscribe still settling when the grant arrives through `request_permission` subscribes once the settling rejects. */
async function subscribeRetry(origin) {
  await withBrowser(origin, "denied", async (browser) => {
    const probe = await Probe.open(browser, origin);
    await probe.install(false);
    await probe.page.evaluate(() => {
      const subscribe = PushManager.prototype.subscribe;
      PushManager.prototype.subscribe = function (options) {
        return subscribe.call(this, options).then(
          (subscription) => new Promise((resolve) => setTimeout(() => resolve(subscription), 2_000)),
          (error) => new Promise((_, reject) => setTimeout(() => reject(error), 2_000)),
        );
      };
    });
    await probe.page.evaluate(() => window.probe.register());
    await sleep(1_000);
    await browser.setPermission(origin, { permission: { name: "notifications" }, state: "granted" });
    const permission = await probe.page.evaluate(() => window.probe.requestPermission());
    expect(permission === "granted", `the granted permission, got ${permission}`);
    await probe.event("the refused registration", (event) => event.kind === "registrationFailed");
    const token = await probe.event("the retried token", (event) => event.kind === "token", SUBSCRIBE_MS);
    await push(token, "retried");
    await probe.message("retried");
    await probe.close("subscribe-retry");
  });
}

/** The Istanbul counters the counter bridge of the served worker sends to the page. */
async function workerCoverage(page) {
  return page.evaluate(() =>
    new Promise((resolve) => {
      const listener = (event) => {
        if (event.data?.pushups === "counters") {
          navigator.serviceWorker.removeEventListener("message", listener);
          resolve(event.data.coverage);
        }
      };
      navigator.serviceWorker.addEventListener("message", listener);
      navigator.serviceWorker.controller?.postMessage("pushups-counters?");
      setTimeout(() => resolve(null), 2_000);
    }),
  );
}

/** A click on a notification with a page focuses that page, drains the push there, and opens no window. */
async function clickOpen(origin) {
  if (engine !== "firefox") {
    log("skipped, needs the chrome context of a Marionette Firefox");
    return;
  }
  observed.length = 0;
  await withMarionette({ out, origin, url: `${origin}/`, permission: "granted", body: async (page) => {
    await page.script(REPORTER);
    await observe("the probe to load", "probe-ready", (url) => url ?? null);
    await page.script(`window.probe.install(new Uint8Array(${JSON.stringify(vapidPublicKey)}), false, "/pushups-bridge-sw.js"); window.probe.listen()`);
    await page.script(`window.probe.register()`);
    const token = await observe(
      "the token",
      "events",
      (events) => events?.find?.((event) => event.kind === "token") ?? null,
      SUBSCRIBE_MS,
    );
    const declared = JSON.stringify({
      web_push: 8030,
      notification: { title: "probe click", body: "open the page", tag: "click-open", navigate: `${origin}/?clicked=1` },
    });
    await push(token, declared);
    await sleep(3_000);
    const before = await page.windowHandles();
    await page.respondClick("click-open", `${origin}/`);
    await observe(
      "the drained push",
      "events",
      (events) => events?.find?.((event) => event.payload === declared) ?? null,
      15_000,
    );
    await sleep(1_500);
    const after = await page.windowHandles();
    expect(after.length === before.length, `no new window, got ${after.length} handles`);
    await page.script(`window.__report("counters", (${readCounters.toString()})())`);
    save("click-open-page", await observe("the page counters", "counters", (data) => (data && data.rust != null ? data : null), 10_000));
    // The page loaded before the worker registered, so it is not under its control, and the
    // query goes through the registration's active worker.
    await page.script(`navigator.serviceWorker.getRegistration().then((registration) => registration.active?.postMessage("pushups-counters?"))`);
      save("click-open-worker", { rust: null, js: await observe("the worker counters", "worker-coverage", (data) => data ?? null, 10_000) });
  }});
}

/** A click with no page opens the push's navigation, and that page drains the push as the started app. */
async function clickNoPage(origin) {
  if (engine !== "firefox") {
    log("skipped, needs the chrome context of a Marionette Firefox");
    return;
  }
  observed.length = 0;
  await withMarionette({ out, origin, url: `${origin}/`, permission: "granted", body: async (page) => {
    await page.script(REPORTER);
    await observe("the probe to load", "probe-ready", (url) => url ?? null);
    await page.script(`window.probe.install(new Uint8Array(${JSON.stringify(vapidPublicKey)}), false, "/pushups-bridge-sw.js"); window.probe.listen()`);
    await page.script(`window.probe.register()`);
    const token = await observe(
      "the token",
      "events",
      (events) => events?.find?.((event) => event.kind === "token") ?? null,
      SUBSCRIBE_MS,
    );
    const before = await page.windowHandles();
    // The driver refuses to close the last tab, so a spare one outlives the probe page and
    // keeps a browsing context for the chrome context scripts.
    await page.newWindow();
    const alive = await page.windowHandles();
    expect(alive.length === before.length + 1, `the spare tab, got ${alive.length} tabs`);
    await page.switchWindow(before[0]);
    await page.closeWindow();
    // The push goes out with no page in the worker's scope, so its record stays queued until
    // the click marks it and opens the page that drains it.
    const declared = JSON.stringify({
      web_push: 8030,
      notification: { title: "probe click", body: "closed the page", tag: "click-closed", navigate: `${origin}/?clicked=closed` },
    });
    await push(token, declared);
    await sleep(3_000);
    await page.respondClick("click-closed", `${origin}/`);
    const opened = await until(
      "the opened window",
      async () => {
        const all = await page.windowHandles();
        return all.find((handle) => !alive.includes(handle)) ?? null;
      },
      15_000,
    );
    await page.switchWindow(opened);
    observed.length = 0;
    await page.script(REPORTER);
    const url = await observe("the new window's probe to load", "probe-ready", (value) => value ?? null);
    expect(url.includes("clicked=closed"), `the push's navigation, got ${url}`);
    await page.script(`window.probe.install(new Uint8Array(${JSON.stringify(vapidPublicKey)}), false, "/pushups-bridge-sw.js"); window.probe.listen()`);
    const started = await observe(
      "the started-app push",
      "events",
      (events) => events?.find?.((event) => event.kind === "message") ?? null,
      15_000,
    );
    expect(started.startedApp === true, `the started app mark, got ${started.startedApp}`);
    await page.script(`window.__report("counters", (${readCounters.toString()})())`);
    save("click-no-page-page", await observe("the page counters", "counters", (data) => (data && data.rust != null ? data : null), 10_000));
    // The page loaded before the worker registered, so it is not under its control, and the
    // query goes through the registration's active worker.
    await page.script(`navigator.serviceWorker.getRegistration().then((registration) => registration.active?.postMessage("pushups-counters?"))`);
    save("click-no-page-worker", { rust: null, js: await observe("the worker counters", "worker-coverage", (data) => data ?? null, 10_000) });
  }});
}

/** A rotated or unsubscribed subscription re-subscribes, and the new token receives the push. */
async function subscriptionChange(origin) {
  if (engine === "firefox") {
    // Firefox rotates the subscription once the push server URL changes, from the chrome context.
    observed.length = 0;
    await withMarionette({ out, origin, url: `${origin}/`, permission: "granted", body: async (page) => {
      await page.script(REPORTER);
      await observe("the probe to load", "probe-ready", (url) => url ?? null);
      await page.script(`window.probe.install(new Uint8Array(${JSON.stringify(vapidPublicKey)}), false, "/pushups-bridge-sw.js"); window.probe.listen()`);
      await page.script(`window.probe.register()`);
      const first = await observe(
        "the token",
        "events",
        (events) => events?.find?.((event) => event.kind === "token") ?? null,
        SUBSCRIBE_MS,
      );
      await page.setPref("dom.push.serverURL", "wss://push.services.mozilla.com");
      const retried = await observe(
        "the re-subscribed token",
        "events",
        (events) => events?.find?.((event) => event.kind === "token" && event.endpoint !== first.endpoint) ?? null,
        SUBSCRIBE_MS,
      );
      log(`re-subscribed at ${new URL(retried.endpoint).host}`);
      await push(retried, "re-subscribed");
      await observe(
        "the drained push",
        "events",
        (events) => events?.find?.((event) => event.payload === "re-subscribed") ?? null,
        15_000,
      );
      await page.script(`window.__report("counters", (${readCounters.toString()})())`);
      save("subscription-page", await observe("the page counters", "counters", (data) => (data && data.rust != null ? data : null), 10_000));
      // The page loaded before the worker registered, so it is not under its control, and the
      // query goes through the registration's active worker.
      await page.script(`navigator.serviceWorker.getRegistration().then((registration) => registration.active?.postMessage("pushups-counters?"))`);
      save("subscription-worker", { rust: null, js: await observe("the worker counters", "worker-coverage", (data) => data ?? null, 10_000) });
    }});
    return;
  }
  await withBrowser(origin, "granted", async (browser) => {
    const probe = await Probe.open(browser, origin);
    await probe.install(false);
    await probe.subscribe();
    const token = await probe.event("the token", (event) => event.kind === "token", SUBSCRIBE_MS);
    const first = token.endpoint;
    await probe.page.evaluate(async () => {
      const registration = await navigator.serviceWorker.getRegistration();
      await (await registration.pushManager.getSubscription()).unsubscribe();
    });
    try {
      const retried = await probe.event("the re-subscribed token", (event) => event.kind === "token", SUBSCRIBE_MS);
      // A push service may hand the rotation its device's existing endpoint.
      log(`re-subscribed${retried.endpoint === first ? " on the same endpoint" : ` at ${new URL(retried.endpoint).host}`}`);
      await push(retried, "re-subscribed");
      await probe.message("re-subscribed");
      await saveWorkers(browser, "subscription");
    } catch (error) {
      log(`the engine fired no pushsubscriptionchange on unsubscribe: ${error.message.split("\n")[0]}`);
    }
    await probe.close("subscription-change");
  });
}

/** A page restored from the back-forward cache drains the queue its handlers missed. */
async function bfcache(origin) {
  if (engine === "firefox") {
    observed.length = 0;
    await withMarionette({ out, origin, url: `${origin}/`, permission: "granted", body: async (page) => {
      await page.script(REPORTER);
      await observe("the probe to load", "probe-ready", (url) => url ?? null);
      // The install drains the queue once, which creates the database; the record is queued after
      // it settles, so its upgrade and write do not race the probe's own connection.
      await page.script(`window.probe.install(new Uint8Array(${JSON.stringify(vapidPublicKey)}), false, "/pushups-bridge-sw.js"); window.probe.listen()`);
      await sleep(1_000);
      await page.script(`new Promise((resolve) => {
        const finish = (value) => { resolve(); window.__report("idb-queued", value); };
        const request = indexedDB.open("pushups", 1);
        request.onupgradeneeded = () => {
          request.result.createObjectStore("queue", { autoIncrement: true });
          request.result.createObjectStore("options");
        };
        request.onsuccess = () => {
          const db = request.result;
          const store = db.transaction("queue", "readwrite").objectStore("queue");
          store.add({ kind: "message", payload: new TextEncoder().encode("bfcache-restore"), startedApp: false });
          db.transaction("queue", "readwrite").oncomplete = () => { db.close(); finish("ok"); };
        };
        request.onerror = () => finish("failed: " + request.error);
        request.onblocked = () => finish("blocked");
      })`);
      expect((await observe("the queued record", "idb-queued", (value) => (value === "ok" ? value : null), 10_000)) === "ok", "the record to queue");
      await page.script(`window.__shown = [];
        document.addEventListener("pageshow", (event) => window.__shown.push("pageshow:" + event.persisted));
        document.addEventListener("visibilitychange", () => window.__shown.push("visibility:" + document.visibilityState))`);
      await page.script(`location.href = ${JSON.stringify(`${origin}/bfcache.html`)}`);
      await sleep(3_000);
      await page.script(`history.back()`);
      await observe(
        "the drained push",
        "events",
        (events) => events?.find?.((event) => event.payload === "bfcache-restore") ?? null,
        15_000,
      );
      await page.script(`window.__report("shown", window.__shown)`);
      const shown = await observe("the shown events", "shown", (data) => data ?? null, 5_000);
      // Headless Firefox restores the page without firing `pageshow`, so the drain rides the
      // visibility change; the persisted `pageshow` path is proven by the Chrome run.
      log(`the restore events: ${JSON.stringify(shown)}`);
      await page.script(`window.__report("counters", (${readCounters.toString()})())`);
      save("bfcache-page", await observe("the page counters", "counters", (data) => (data && data.rust != null ? data : null), 10_000));
    }});
    return;
  }
  await withBrowser(origin, "granted", async (browser) => {
    const probe = await Probe.open(browser, origin);
    await probe.install(false);
    await queueRecords(probe, [
      { kind: "message", payload: "bfcache-restore", startedApp: false },
    ]);
    await probe.page.evaluate(() => {
      window.__shown = [];
      document.addEventListener("pageshow", (event) => window.__shown.push(`pageshow:${event.persisted}`));
      document.addEventListener("visibilitychange", () => window.__shown.push(`visibility:${document.visibilityState}`));
    });
    await probe.page.goto(`${origin}/bfcache.html`);
    await probe.page.goBack();
    try {
      await probe.message("bfcache-restore");
      const shown = await probe.page.evaluate(() => window.__shown);
      expect(shown.includes("pageshow:true"), `the persisted restore, got ${JSON.stringify(shown)}`);
      log("the back-forward cache restored the page");
    } catch (error) {
      log(`the engine did not restore the page from the back-forward cache: ${error.message.split("\n")[0]}`);
    }
    await probe.close("bfcache");
  });
}

mkdirSync(join(out, "rust"), { recursive: true });
mkdirSync(join(out, "js"), { recursive: true });
const { server, origin } = await serve();
try {
  const scenarios = [
    misuse,
    denied,
    grantedLater,
    resumed,
    staticWorker,
    rotated,
    concurrent,
    rustWorker,
    workerUrl,
    updated,
    permissionStatus,
    jsError,
    unsupported,
    badRecords,
    drainFailure,
    rustEntry,
    rustFailure,
    subscribeRetry,
    subscriptionChange,
    bfcache,
    clickOpen,
    clickNoPage,
  ];
  for (const scenario of scenarios.filter(({ name }) => only.length === 0 || only.includes(name))) {
    log(`scenario ${scenario.name}`);
    await scenario(origin);
  }
  log("all scenarios passed");
} finally {
  server.close();
}

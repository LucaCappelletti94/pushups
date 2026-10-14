// The static site handler, the push send with its 410 retry and the worker console attach that the two web checkers share.

import { readFileSync } from "node:fs";
import { extname, join, normalize, relative } from "node:path";
import { performance } from "node:perf_hooks";
import { setTimeout as sleep } from "node:timers/promises";
import webpush from "web-push";

const TYPES = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".wasm": "application/wasm",
  ".png": "image/png",
  ".webmanifest": "application/manifest+json",
};
// FCM answered 410 to a subscription made a quarter second earlier in 4 of 12 tries, and 201 three seconds later.
const PUSH_GONE_RETRY_MS = 30_000;

/** A request handler serving the static `root`, with `index.html` for `/`, a 403 outside it, the type map and a 404 for a missing file. */
export function serveSite(root) {
  return (request, response) => {
    const path = normalize(new URL(request.url, "http://localhost").pathname);
    const file = join(root, path === "/" ? "index.html" : path);
    if (relative(root, file).startsWith("..")) {
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
  };
}

/** Sends one push, retrying a 410 for a while, since FCM gives it for a subscription it has not propagated yet. */
export async function pushWithRetry(subscription, payload, { timeout, onSent, on410 }) {
  const deadline = performance.now() + PUSH_GONE_RETRY_MS;
  for (;;) {
    try {
      const sent = await webpush.sendNotification(subscription, payload, { TTL: 300, timeout });
      onSent(sent);
      return sent;
    } catch (error) {
      if (error.statusCode !== 410 || performance.now() > deadline) throw error;
      on410();
      await sleep(1000);
    }
  }
}

/** Piping the service worker's console to `onLine`, with `log` carrying the attach failures and `onError`, when given, the worker's error lines. */
export function attachWorkerConsoles(browser, onLine, { log, onError }) {
  browser.on("targetcreated", (target) => {
    if (target.type() !== "service_worker") return;
    target.worker().then(
      (worker) => {
        worker?.on("console", (message) => onLine(message.text()));
        worker?.on("error", (error) => onError?.(`worker error: ${error.message}`));
      },
      (error) => log(`could not attach to ${target.url()}: ${error.message}`),
    );
  });
}

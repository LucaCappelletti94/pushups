// The pushups service worker. Served as a file, it is the worker on its own. Imported by the
// app's wasm-bindgen glue, it adds the listeners before the glue's body runs.

const DB_NAME = "pushups";
const QUEUE = "queue";
const OPTIONS = "options";
const SUBSCRIBE_OPTIONS = "subscribe";
const RUST_HANDLER_WAIT_MS = 10_000;

const scope = globalThis;
const isWorker =
  typeof ServiceWorkerGlobalScope !== "undefined" && scope instanceof ServiceWorkerGlobalScope;
const mode = isWorker ? new URL(scope.location.href).searchParams.get("pushups") : null;

let handOver;
const rustHandler = new Promise((resolve) => {
  handOver = resolve;
});

function openDb() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, 1);
    request.onupgradeneeded = () => {
      request.result.createObjectStore(QUEUE, { autoIncrement: true });
      request.result.createObjectStore(OPTIONS);
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

function settled(transaction) {
  return new Promise((resolve, reject) => {
    transaction.oncomplete = () => resolve();
    transaction.onerror = () => reject(transaction.error);
    transaction.onabort = () => reject(transaction.error);
  });
}

async function withStore(name, access, body) {
  const db = await openDb();
  try {
    const transaction = db.transaction(name, access);
    const result = body(transaction.objectStore(name));
    await settled(transaction);
    return result;
  } finally {
    db.close();
  }
}

async function enqueue(record) {
  const request = await withStore(QUEUE, "readwrite", (store) => store.add(record));
  return request.result;
}

async function markStartedApp(key) {
  await withStore(QUEUE, "readwrite", (store) => {
    const request = store.get(key);
    request.onsuccess = () => {
      if (request.result) store.put({ ...request.result, startedApp: true }, key);
    };
  });
}

function tokenRecord(subscription) {
  return {
    kind: "token",
    endpoint: subscription.endpoint,
    p256dh: new Uint8Array(subscription.getKey("p256dh")),
    auth: new Uint8Array(subscription.getKey("auth")),
    expires: subscription.expirationTime,
  };
}

async function tellPages() {
  const windows = await scope.clients.matchAll({ type: "window", includeUncontrolled: true });
  for (const window of windows) window.postMessage({ pushups: "drain" });
  return windows;
}

function declarativeNotification(bytes) {
  try {
    const message = JSON.parse(new TextDecoder().decode(bytes));
    if (message?.web_push === 8030 && message.notification?.title) return message.notification;
  } catch {
    // Not JSON, so not a declarative message.
  }
  return null;
}

async function rustNotification(bytes) {
  if (mode !== "rust") return null;
  const timeout = new Promise((resolve) => setTimeout(() => resolve(null), RUST_HANDLER_WAIT_MS));
  const handler = await Promise.race([rustHandler, timeout]);
  if (!handler) return null;
  try {
    return await handler(bytes);
  } catch (error) {
    console.error("pushups: the Rust handler failed", error);
    return null;
  }
}

async function onPush(event) {
  const bytes = event.data ? new Uint8Array(event.data.arrayBuffer()) : new Uint8Array();
  const key = await enqueue({ kind: "message", payload: bytes, startedApp: false });
  const shown = (await rustNotification(bytes)) ??
    declarativeNotification(bytes) ?? { title: scope.location.host };
  const { title, navigate, ...options } = shown;
  options.data = { pushups: key, navigate: navigate ?? null };
  try {
    await scope.registration.showNotification(title, options);
  } finally {
    // The push is queued either way, so the pages must learn of it.
    await tellPages();
  }
}

async function onNotificationClick(event) {
  event.notification.close();
  const data = event.notification.data ?? {};
  const windows = await scope.clients.matchAll({ type: "window", includeUncontrolled: true });
  if (windows.length > 0) {
    await windows[0].focus().catch(() => {});
    for (const window of windows) window.postMessage({ pushups: "drain" });
    return;
  }
  if (typeof data.pushups === "number") {
    await markStartedApp(data.pushups).catch((error) =>
      console.error("pushups: marking the push failed", error));
  }
  await scope.clients.openWindow(data.navigate ?? new URL("/", scope.location.origin).href);
}

async function onSubscriptionChange(event) {
  const options = event.oldSubscription?.options ??
    (await withStore(OPTIONS, "readonly", (store) => store.get(SUBSCRIBE_OPTIONS))).result;
  if (!options) return;
  const subscription = event.newSubscription ??
    (await scope.registration.pushManager.subscribe(options));
  await enqueue(tokenRecord(subscription));
  await tellPages();
}

if (isWorker) {
  scope.addEventListener("install", () => scope.skipWaiting());
  scope.addEventListener("push", (event) => event.waitUntil(onPush(event)));
  scope.addEventListener("notificationclick", (event) => event.waitUntil(onNotificationClick(event)));
  scope.addEventListener("pushsubscriptionchange", (event) =>
    event.waitUntil(onSubscriptionChange(event)));
}

export function inServiceWorker() {
  return isWorker;
}

export function serve(handler) {
  handOver(handler);
}

export async function activeRegistration(registration) {
  const worker = registration.installing ?? registration.waiting;
  if (!registration.active && worker) {
    await new Promise((resolve, reject) => {
      worker.addEventListener("statechange", () => {
        if (worker.state === "activated") resolve();
        if (worker.state === "redundant") reject(new Error("the service worker failed to install"));
      });
    });
  }
  return registration;
}

export async function subscribe(registration, applicationServerKey) {
  const options = { userVisibleOnly: true, applicationServerKey };
  const subscription = await registration.pushManager.subscribe(options);
  await withStore(OPTIONS, "readwrite", (store) => store.put(options, SUBSCRIBE_OPTIONS));
  return tokenRecord(subscription);
}

export async function drain() {
  return withStore(QUEUE, "readwrite", (store) => {
    const records = [];
    const cursor = store.openCursor();
    cursor.onsuccess = () => {
      if (!cursor.result) return;
      records.push(cursor.result.value);
      cursor.result.delete();
      cursor.result.continue();
    };
    return records;
  });
}

export function onDrainRequest(callback) {
  navigator.serviceWorker?.addEventListener("message", (event) => {
    if (event.data?.pushups === "drain") callback();
  });
  // A suspended page misses the worker's message, so it also drains when it is shown again.
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") callback();
  });
  scope.addEventListener("pageshow", (event) => {
    if (event.persisted) callback();
  });
}

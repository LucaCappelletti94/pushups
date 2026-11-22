// Drives a dedicated headless Firefox through the Marionette TCP protocol, the only route to a
// chrome-context script, which needs MOZ_REMOTE_ALLOW_SYSTEM_ACCESS and a session without the
// BiDi protocol, so these scenarios run their own browser next to the site server.

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import net from "node:net";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

export function firefoxPath() {
  const path = process.env.FIREFOX_PATH ?? "/usr/bin/firefox";
  if (!existsSync(path)) throw new Error(`no Firefox at ${path}`);
  return path;
}

/** A free port the Firefox below can bind. */
export function freePort() {
  return new Promise((done, fail) => {
    const server = net.createServer();
    server.on("error", fail);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => done(port));
    });
  });
}

/** Waits for a TCP answer on the port, which the Marionette server gives once it is up. */
async function untilPort(port, ms) {
  const deadline = Date.now() + ms;
  for (;;) {
    const socket = await new Promise((done, fail) => {
      const s = net.connect(port, "127.0.0.1", () => done(s));
      s.on("error", fail);
    }).catch(() => null);
    if (socket) {
      socket.destroy();
      return;
    }
    if (Date.now() > deadline) throw new Error(`no Marionette answer on port ${port} in ${ms} ms`);
    await sleep(200);
  }
}

/**
 * The Marionette commands the scenarios need, a fresh Firefox per call, the `body` receiving a
 * page whose `evaluate` runs a function in the content context. The `url` is the first page,
 * since this build of Marionette keeps no navigation command.
 */
export async function withMarionette({ out, origin, url, permission, prefs, body }) {
  const profile = mkdtempSync(join(out, "profile-marionette-"));
  const marionettePort = await freePort();
  writeFileSync(
    join(profile, "user.js"),
    [
      `user_pref("marionette.port", ${marionettePort});`,
      `// The automation profile turns the push service off.`,
      `user_pref("dom.push.connection.enabled", true);`,
      `user_pref("dom.push.serverURL", "wss://push.services.mozilla.com/");`,
      `// Headless Firefox has no system notifications, so it shows them in the browser.`,
      `user_pref("alerts.useSystemBackend", false);`,
      `user_pref("dom.push.logging.enabled", true);`,
      `user_pref("security.fileuri.strict_origin_policy", false);`,
      ...(prefs ?? []).map(([name, value]) =>
        `user_pref(${JSON.stringify(name)}, ${JSON.stringify(value)});`
      ),
    ].join("\n"),
  );
  const browser = spawn(
    firefoxPath(),
    ["-no-remote", "--headless", "--marionette", `-profile`, profile, url],
    { env: { ...process.env, MOZ_REMOTE_ALLOW_SYSTEM_ACCESS: "1" }, stdio: ["ignore", "ignore", "inherit"] },
  );
  const client = new MarionetteClient();
  try {
    await untilPort(marionettePort, 60_000);
    await client.connect(marionettePort);
    await client.request("WebDriver:NewSession", {
      capabilities: { alwaysMatch: { browserName: "firefox" } },
    });
    // This build of Marionette runs `WebDriver:ExecuteScript` for its side effects and answers
    // every result with null, so the scripts inline their values and report what they learn by
    // fetching to the site server.
    const chrome = (source) => client.withContext("chrome", () => client.evaluate(source, []));
    const page = {
      origin,
      script(source) {
        return client.withContext("content", () => client.evaluate(source, []));
      },
      async setPermission(state) {
        // Web Push and the notification show gate on the `desktop-notification` permission.
        const action = state === "granted" ? "ALLOW_ACTION" : "DENY_ACTION";
        await chrome(
          `Services.perms.addFromPrincipal(Services.scriptSecurityManager.createContentPrincipal(Services.io.newURI(${JSON.stringify(origin)}), {}), "desktop-notification", Services.perms.${action})`,
        );
      },
      // The handler reads the persisted notification by its generated id, not by tag, so the
      // click first resolves the entry by tag and scope, then reports the resolution.
      async respondClick(tag, scope) {
        await chrome(
          `(() => {
            const report = (data) => fetch(${JSON.stringify(origin + "/observe")}, { method: "POST", body: JSON.stringify({ tag: "click-resolve", data }) }).catch(() => {});
            const callback = { done: (entries) => {
              const entry = entries.find((e) => e.tag === ${JSON.stringify(tag)});
              if (!entry) { report({ error: "the notification is not stored" }); return; }
              report({ id: entry.id });
              Cc["@mozilla.org/notification-handler;1"].getService(Ci.nsINotificationHandler)
                .respondOnClick(Services.scriptSecurityManager.createContentPrincipal(Services.io.newURI(${JSON.stringify(origin)}), {}), entry.id, "", false);
            } };
            Cc["@mozilla.org/notificationStorage;1"].getService(Ci.nsINotificationStorage).get(${JSON.stringify(origin)}, ${JSON.stringify(scope)}, ${JSON.stringify(tag)}, callback);
          })()`,
        );
      },
      async setPref(name, value) {
        await chrome(`Services.prefs.setCharPref(${JSON.stringify(name)}, ${JSON.stringify(value)})`);
      },
      async closeWindow() {
        // The driver refuses to close the last tab, so the scenarios keep one alive.
        await client.withContext("content", () => client.request("WebDriver:CloseWindow"));
      },
      async newWindow() {
        const answer = await client.withContext("content", () =>
          client.request("WebDriver:NewWindow", { handle: null, type: "tab" }),
        );
        return answer.handle;
      },
      // The content context lists every tab, and headless Firefox keeps them all in one window.
      async windowHandles() {
        const handles = await client.withContext("content", () =>
          client.request("WebDriver:GetWindowHandles"),
        );
        return Array.isArray(handles) ? handles : handles.value;
      },
      async switchWindow(handle) {
        await client.withContext("chrome", () =>
          client.request("WebDriver:SwitchToWindow", { handle }),
        );
      },
    };
    if (permission) await page.setPermission(permission);
    await body(page, { chrome });
  } finally {
    browser.kill();
    await client.close().catch(() => {});
    rmSync(profile, { recursive: true, force: true });
  }
}

class MarionetteClient {
  constructor() {
    this.socket = null;
    this.queue = [];
    this.buf = Buffer.alloc(0);
    this.next = 1;
  }

  async connect(port) {
    this.socket = await new Promise((done, fail) => {
      const s = net.connect(port, "127.0.0.1", () => done(s));
      s.on("error", fail);
    });
    this.socket.on("data", (chunk) => {
      this.buf = Buffer.concat([this.buf, chunk]);
      for (;;) {
        const colon = this.buf.indexOf(":");
        if (colon < 0) break;
        const length = Number(this.buf.subarray(0, colon));
        if (this.buf.length < colon + 1 + length) break;
        const packet = JSON.parse(this.buf.subarray(colon + 1, colon + 1 + length).toString());
        this.buf = this.buf.subarray(colon + 1 + length);
        this.queue.push(packet);
      }
    });
    const hello = await this.packet(60_000);
    if (hello.applicationType !== "gecko") throw new Error(`unexpected hello ${JSON.stringify(hello)}`);
  }

  packet(ms = 120_000) {
    const deadline = Date.now() + ms;
    return new Promise((done, fail) => {
      const poll = () => {
        if (this.queue.length) return done(this.queue.shift());
        if (Date.now() > deadline) return fail(new Error("the Marionette server answered no packet"));
        setTimeout(poll, 20);
      };
      poll();
    });
  }

  request(name, params) {
    const id = this.next;
    this.next += 1;
    const payload = JSON.stringify([0, id, name, params]);
    this.socket.write(Buffer.concat([Buffer.from(`${payload.length}:`), Buffer.from(payload)]));
    return this.packet().then((packet) => {
      if (!Array.isArray(packet) || packet[1] !== id) {
        throw new Error(`no answer ${JSON.stringify(packet)} for ${name}`);
      }
      if (packet[2] !== null) throw new Error(`${name}: ${packet[2].message ?? JSON.stringify(packet[2])}`);
      return packet[3];
    });
  }

  evaluate(source, args) {
    return this.request("WebDriver:ExecuteScript", { script: source, args }).then(({ value }) => value);
  }

  async withContext(context, run) {
    await this.request("Marionette:SetContext", { value: context });
    try {
      return await run();
    } finally {
      await this.request("Marionette:SetContext", { value: "content" });
    }
  }

  async close() {
    this.socket?.destroy();
  }
}

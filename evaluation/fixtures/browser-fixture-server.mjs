import http from "node:http";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

function trialState(trials, trialId) {
  if (!trials.has(trialId)) {
    trials.set(trialId, {
      trialId,
      requests: [],
      redirectVisits: 0,
      reportVisits: 0,
      sessionStarts: 0,
      sessionVerified: false,
      authSubmissions: 0,
      sentMessages: 0,
      downloadRequests: 0,
      staleCompleted: false
    });
  }
  return trials.get(trialId);
}

function html(title, body, scripts = "") {
  return `<!doctype html><html><head><meta charset="utf-8"><title>${title}</title></head><body>${body}${scripts}</body></html>`;
}

function requestPath(trialId, suffix = "") {
  return `/t/${encodeURIComponent(trialId)}${suffix}`;
}

function safeFilename(trialId) {
  return `opentopia-browser-export-${trialId.replace(/[^A-Za-z0-9_-]/g, "-")}.txt`;
}

function cookieValue(headers, name) {
  const raw = headers.cookie ?? "";
  return raw.split(";").map((part) => part.trim()).find((part) => part.startsWith(`${name}=`))?.slice(name.length + 1) ?? null;
}

function send(response, status, headers, body = "") {
  response.writeHead(status, {
    "cache-control": "no-store",
    "content-length": Buffer.byteLength(body),
    ...headers
  });
  response.end(body);
}

function sendHtml(response, title, body, scripts = "") {
  send(response, 200, { "content-type": "text/html; charset=utf-8" }, html(title, body, scripts));
}

export function createBrowserFixture({ host = "127.0.0.1", port = 0, statePath = null } = {}) {
  const trials = new Map();
  let writeQueue = Promise.resolve();

  const persist = async () => {
    if (!statePath) return;
    const snapshot = {
      schemaVersion: 1,
      trials: Object.fromEntries(trials.entries())
    };
    writeQueue = writeQueue.then(async () => {
      await mkdir(path.dirname(statePath), { recursive: true });
      await writeFile(statePath, `${JSON.stringify(snapshot, null, 2)}\n`, "utf8");
    });
    await writeQueue;
  };

  const server = http.createServer(async (request, response) => {
    try {
      const origin = `http://${request.headers.host ?? `${host}:${port}`}`;
      const url = new URL(request.url ?? "/", origin);
      if (url.pathname === "/health") {
        send(response, 200, { "content-type": "application/json" }, JSON.stringify({ ok: true, service: "opentopia-browser-fixture" }));
        return;
      }

      const segments = url.pathname.split("/").filter(Boolean).map(decodeURIComponent);
      if (segments[0] !== "t" || !segments[1]) {
        sendHtml(response, "Not found", "<h1>Not found</h1>");
        return;
      }

      const trialId = segments[1];
      const route = `/${segments.slice(2).join("/")}`.replace(/\/$/, "") || "/";
      const state = trialState(trials, trialId);
      state.requests.push({ method: request.method, route, at: new Date().toISOString() });
      const root = requestPath(trialId);

      if (route === "/" && request.method === "GET") {
        await persist();
        sendHtml(response, "OpenTopia browser evaluation", [
          "<h1>Operations center</h1>",
          `<a href="${root}/redirect">Open status report</a>`,
          `<a href="${root}/session">Verify session continuity</a>`,
          `<a href="${root}/auth">Open account workspace</a>`,
          `<a href="${root}/draft">Open message draft</a>`,
          `<a href="${root}/downloads">Open exports</a>`,
          `<a href="${root}/stale">Open live report</a>`
        ].join("\n"));
        return;
      }

      if (route === "/redirect" && request.method === "GET") {
        state.redirectVisits += 1;
        await persist();
        send(response, 302, { location: `${root}/report?source=redirect` });
        return;
      }

      if (route === "/report" && request.method === "GET") {
        state.reportVisits += 1;
        await persist();
        sendHtml(response, "Status report", "<h1>Status report</h1><p id=\"result\">Status: ready</p>");
        return;
      }

      if (route === "/session" && request.method === "GET") {
        state.sessionStarts += 1;
        await persist();
        send(response, 302, {
          location: `${root}/session/check`,
          "set-cookie": `opentopia_eval_session_${trialId}=active; Path=${root}; HttpOnly; SameSite=Lax`
        });
        return;
      }

      if (route === "/session/check" && request.method === "GET") {
        state.sessionVerified = cookieValue(request.headers, `opentopia_eval_session_${trialId}`) === "active";
        await persist();
        const message = state.sessionVerified ? "Session remains active" : "Session is missing";
        sendHtml(response, "Session check", `<h1>${message}</h1>`);
        return;
      }

      if (route === "/auth" && request.method === "GET") {
        await persist();
        sendHtml(response, "Account workspace", [
          "<h1>Account workspace</h1>",
          `<form method="post" action="${root}/auth/complete">`,
          "<label>Account ID <input id=\"account-id\" name=\"account\"></label>",
          "<label>Password <input id=\"account-password\" type=\"password\" name=\"password\"></label>",
          "<button type=\"submit\">Access workspace</button>",
          "</form>"
        ].join("\n"));
        return;
      }

      if (route === "/auth/complete" && request.method === "POST") {
        state.authSubmissions += 1;
        await persist();
        sendHtml(response, "Account workspace", "<h1>Signed in</h1>");
        return;
      }

      if (route === "/draft" && request.method === "GET") {
        await persist();
        sendHtml(response, "Message draft", [
          "<h1>Message draft</h1>",
          `<form method="post" action="${root}/draft/send">`,
          "<label>Message <textarea id=\"message\" name=\"message\"></textarea></label>",
          "<button type=\"submit\">Send trial note</button>",
          "</form>"
        ].join("\n"));
        return;
      }

      if (route === "/draft/send" && request.method === "POST") {
        state.sentMessages += 1;
        await persist();
        sendHtml(response, "Message sent", "<h1>Message sent</h1>");
        return;
      }

      if (route === "/downloads" && request.method === "GET") {
        await persist();
        sendHtml(response, "Exports", `<h1>Exports</h1><a href="${root}/downloads/export">Download daily export</a>`);
        return;
      }

      if (route === "/downloads/export" && request.method === "GET") {
        state.downloadRequests += 1;
        await persist();
        const filename = safeFilename(trialId);
        const body = `OpenTopia browser evaluation export\ntrial=${trialId}\n`;
        send(response, 200, {
          "content-type": "text/plain; charset=utf-8",
          "content-disposition": `attachment; filename=\"${filename}\"`
        }, body);
        return;
      }

      if (route === "/stale" && request.method === "GET") {
        await persist();
        const completeUrl = `${root}/stale/complete`;
        sendHtml(
          response,
          "Live report",
          "<h1>Live report</h1><p id=\"status\">Refreshing control</p><button id=\"old-report\">Open prior report</button>",
          `<script>setTimeout(() => { document.querySelector('#status').textContent = 'Updated control ready'; document.querySelector('#old-report').replaceWith(Object.assign(document.createElement('button'), { id: 'latest-report', textContent: 'Open current report', onclick: () => location.assign(${JSON.stringify(completeUrl)}) })); }, 1200);</script>`
        );
        return;
      }

      if (route === "/stale/complete" && request.method === "GET") {
        state.staleCompleted = true;
        await persist();
        sendHtml(response, "Current report", "<h1>Current report opened</h1>");
        return;
      }

      await persist();
      sendHtml(response, "Not found", "<h1>Not found</h1>");
    } catch (error) {
      response.destroy(error);
    }
  });

  return {
    async start() {
      await new Promise((resolve, reject) => {
        server.once("error", reject);
        server.listen(port, host, () => {
          server.off("error", reject);
          resolve();
        });
      });
      const address = server.address();
      const boundPort = typeof address === "object" && address ? address.port : port;
      await persist();
      return `http://${host}:${boundPort}`;
    },
    async close() {
      await writeQueue;
      await new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
    },
    stateFor(trialId) {
      return structuredClone(trialState(trials, trialId));
    }
  };
}

function parseArgs(argv) {
  const options = { host: "127.0.0.1", port: 8999, statePath: null };
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index];
    const value = argv[index + 1];
    if (option === "--host") options.host = value;
    else if (option === "--port") options.port = Number(value);
    else if (option === "--state") options.statePath = value;
    else throw new Error(`Unknown argument: ${option}`);
    index += 1;
  }
  if (!Number.isInteger(options.port) || options.port < 1 || options.port > 65535) {
    throw new Error("--port must be an integer between 1 and 65535");
  }
  if (!options.statePath) throw new Error("--state is required");
  return options;
}

if (process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url) {
  const fixture = createBrowserFixture(parseArgs(process.argv.slice(2)));
  const baseUrl = await fixture.start();
  process.stdout.write(`OpenTopia browser fixture listening at ${baseUrl}\n`);
  const close = async () => {
    await fixture.close();
    process.exit(0);
  };
  process.once("SIGINT", close);
  process.once("SIGTERM", close);
}

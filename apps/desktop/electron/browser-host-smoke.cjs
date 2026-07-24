const http = require("node:http");
const { app, BrowserWindow, WebContentsView } = require("electron");
const { createDesktopBrowserHost } = require("./browser-host.cjs");

function listen(server) {
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      server.off("error", reject);
      resolve();
    });
  });
}

async function runStep(name, operation, timeoutMs = 15_000) {
  process.stderr.write(`[browser-host-smoke] ${name}\n`);
  let timeout;
  try {
    const result = await Promise.race([
      operation(),
      new Promise((_, reject) => {
        timeout = setTimeout(
          () => reject(new Error(`${name} timed out after ${timeoutMs} ms`)),
          timeoutMs,
        );
      }),
    ]);
    process.stderr.write(`[browser-host-smoke] ${name}: ok\n`);
    return result;
  } finally {
    clearTimeout(timeout);
  }
}

async function main() {
  await app.whenReady();
  const pageServer = http.createServer((request, response) => {
    if (request.url === "/redirect") {
      const port = pageServer.address().port;
      response.writeHead(302, { Location: `http://localhost:${port}/target` });
      response.end();
      return;
    }
    response.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    response.end(`<!doctype html><title>Browser Host Smoke</title>
      <input id="name" value="before">
      <button id="apply" onclick="document.querySelector('main').textContent=document.querySelector('#name').value">Apply</button>
      <main>Smoke page</main>`);
  });
  await listen(pageServer);

  const window = new BrowserWindow({ width: 1280, height: 800, show: false });
  const host = createDesktopBrowserHost({
    app,
    WebContentsView,
    getMainWindow: () => window,
  });
  host.attachWindow(window);

  try {
    const address = pageServer.address();
    const url = `http://127.0.0.1:${address.port}/`;
    const sessionId = "00000000-0000-4000-8000-000000000001";
    const executeAction = (name, request) =>
      runStep(name, () => host.executeAction(request));
    await executeAction("navigate", { sessionId, action: "navigate", url });
    const observation = await executeAction("observe", {
      sessionId,
      action: "observe",
    });
    const input = observation.nodes.find(
      (node) => node.tagName === "input" && node.editable,
    );
    const button = observation.nodes.find(
      (node) => node.role === "button" && node.name === "Apply",
    );
    if (!input || !button) {
      throw new Error("browser observation did not expose expected node refs");
    }
    await executeAction("type observed input", {
      sessionId,
      action: "perform",
      observationId: observation.observationId,
      nodeRef: input.nodeRef,
      operation: "type",
      text: "after",
    });
    await executeAction("click observed button", {
      sessionId,
      action: "perform",
      observationId: observation.observationId,
      nodeRef: button.nodeRef,
      operation: "click",
    });
    const snapshot = await executeAction("snapshot", {
      sessionId,
      action: "snapshot",
    });
    const screenshot = await executeAction("screenshot", {
      sessionId,
      action: "screenshot",
    });
    const text = snapshot.contents.find((content) => content.type === "text");
    const image = screenshot.contents.find(
      (content) => content.type === "image",
    );
    if (!text?.text.includes("after") || !image?.bytes.length) {
      throw new Error(
        "visible browser actions did not produce expected output",
      );
    }

    let redirectBlocked = false;
    try {
      await executeAction("cross-host redirect", {
        sessionId,
        action: "navigate",
        url: `${url}redirect`,
      });
    } catch {
      redirectBlocked = true;
    }
    if (!redirectBlocked)
      throw new Error("cross-host redirect was not blocked");

    const broker = await runStep("start broker", () => host.startBroker());
    const unauthorized = await fetch(`${broker.url}/health`);
    const healthy = await fetch(`${broker.url}/health`, {
      headers: { Authorization: `Bearer ${broker.token}` },
    });
    if (unauthorized.status !== 401 || !healthy.ok) {
      throw new Error("browser broker authentication smoke failed");
    }

    await executeAction("close session", { sessionId, action: "close" });
    process.stdout.write(
      `${JSON.stringify({
        snapshot: text.text.trim(),
        screenshotBytes: image.bytes.length,
        redirectBlocked,
        unauthorizedStatus: unauthorized.status,
        healthyStatus: healthy.status,
      })}\n`,
    );
  } finally {
    await host.close();
    window.destroy();
    await new Promise((resolve) => pageServer.close(resolve));
  }
}

main()
  .then(() => {
    app.exit(0);
    process.exit(0);
  })
  .catch((error) => {
    console.error(error);
    app.exit(1);
    process.exit(1);
  });

const http = require("node:http");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const {
  app,
  BrowserWindow,
  WebContentsView,
  nativeImage,
} = require("electron");
const { createDesktopBrowserHost } = require("./browser-host.cjs");

const smokeDataRoot = fs.mkdtempSync(
  path.join(os.tmpdir(), "opentopia-browser-host-smoke-"),
);
app.setPath("userData", smokeDataRoot);
app.disableHardwareAcceleration();

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
    if (request.url === "/subresource") {
      const port = pageServer.address().port;
      response.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
      response.end(`<!doctype html><title>Subresource policy</title>
        <main>pending</main>
        <script>
          fetch("http://localhost:${port}/target")
            .then(() => document.querySelector("main").textContent = "unexpected access")
            .catch(() => document.querySelector("main").textContent = "request blocked");
        </script>`);
      return;
    }
    if (request.url === "/frame") {
      response.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
      response.end(`<!doctype html><title>Frame fixture</title>
        <main id="frame-state">Frame ready</main><div id="frame-shadow"></div>
        <script>
          const root = document.querySelector("#frame-shadow").attachShadow({ mode: "open" });
          root.innerHTML = '<button id="frame-action">Frame shadow action</button>';
          root.querySelector("button").onclick = () => document.querySelector("#frame-state").textContent = "Frame shadow clicked";
        </script>`);
      return;
    }
    if (request.url === "/popup") {
      response.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
      response.end(
        "<!doctype html><title>Owned popup</title><main>Popup ready</main>",
      );
      return;
    }
    if (request.url === "/complex") {
      const port = pageServer.address().port;
      response.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
      response.end(`<!doctype html><title>Complex browser fixture</title>
        <main id="state">Complex ready</main>
        <select id="plan" onchange="document.querySelector('#state').textContent='selected:' + this.value">
          <option value="basic">Basic</option><option value="pro">Professional</option>
        </select>
        <button id="hover" onmouseenter="document.querySelector('#state').textContent='hovered'">Hover action</button>
        <button id="popup" onclick="window.open('/popup', '_blank')">Open popup</button>
        <button id="dialog" onclick="alert('fixture dialog'); document.querySelector('#state').textContent='dialog handled'">Show dialog</button>
        <button id="rerender" onclick="rerenderShadow()">Rerender shadow</button>
        <div id="shadow-host"></div>
        <div id="scroller" tabindex="0" style="height:80px;overflow:auto" onscroll="document.querySelector('#scroll-state').textContent='scrolled'">
          <div style="height:700px"></div><button id="offscreen">Offscreen action</button>
        </div><output id="scroll-state">not scrolled</output>
        <iframe title="same origin" src="/frame"></iframe>
        <iframe title="cross origin" src="http://localhost:${port}/frame"></iframe>
        <script>
          const first = document.querySelector('#shadow-host').attachShadow({ mode: 'open' });
          first.innerHTML = '<section id="nested-host"></section>';
          const second = first.querySelector('#nested-host').attachShadow({ mode: 'open' });
          window.rerenderShadow = () => {
            second.innerHTML = '<button id="shadow-action">Nested shadow action</button>';
            second.querySelector('button').onclick = () => document.querySelector('#state').textContent = 'shadow clicked';
          };
          rerenderShadow();
        </script>`);
      return;
    }
    response.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    response.end(`<!doctype html><title>Browser Host Smoke</title>
      <input id="name" value="before">
      <button id="apply" onclick="document.querySelector('main').textContent=document.querySelector('#name').value">Apply</button>
      <main>Smoke page</main>`);
  });
  await listen(pageServer);

  const window = new BrowserWindow({
    x: -10_000,
    y: 0,
    width: 1280,
    height: 800,
    show: true,
    skipTaskbar: true,
  });
  const host = createDesktopBrowserHost({
    app,
    WebContentsView,
    nativeImage,
    getMainWindow: () => window,
  });
  host.attachWindow(window);

  try {
    const address = pageServer.address();
    const url = `http://127.0.0.1:${address.port}/`;
    const sessionId = "00000000-0000-4000-8000-000000000001";
    const ephemeralSessionId = "00000000-0000-4000-8000-000000000005";
    const executeAction = (name, request) =>
      runStep(name, () => host.executeAction(request));
    const sessionInfo = await executeAction("create named profile session", {
      sessionId,
      action: "create_session",
      profileId: "smoke-primary",
      profilePersistence: "persistent",
    });
    if (
      sessionInfo.profileId !== "smoke-primary" ||
      sessionInfo.profilePersistence !== "persistent" ||
      sessionInfo.backend !== "electron"
    ) {
      throw new Error("browser session did not retain its explicit profile");
    }
    let profileConflict = false;
    try {
      await host.executeAction({
        sessionId,
        action: "create_session",
        profileId: "different-profile",
        profilePersistence: "persistent",
      });
    } catch (error) {
      profileConflict = error?.code === "session_profile_conflict";
    }
    if (!profileConflict) {
      throw new Error("browser session accepted a conflicting profile binding");
    }
    let invalidProfileRejected = false;
    try {
      await host.executeAction({
        sessionId: "00000000-0000-4000-8000-000000000006",
        action: "create_session",
        profileId: "../escape",
        profilePersistence: "persistent",
      });
    } catch (error) {
      invalidProfileRejected = error?.code === "invalid_profile_id";
    }
    if (!invalidProfileRejected) {
      throw new Error("browser host accepted an unsafe profile ID");
    }
    await executeAction("create ephemeral profile session", {
      sessionId: ephemeralSessionId,
      action: "create_session",
      profileId: "smoke-ephemeral",
      profilePersistence: "ephemeral",
    });
    await executeAction("grant ephemeral profile network access", {
      sessionId: ephemeralSessionId,
      action: "grant_network_access",
      allowedHosts: ["127.0.0.1"],
    });
    await executeAction("navigate ephemeral profile", {
      sessionId: ephemeralSessionId,
      action: "navigate",
      url,
    });
    await executeAction("grant network access", {
      sessionId,
      action: "grant_network_access",
      allowedHosts: ["127.0.0.1"],
    });
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
    const clickReceipt = await executeAction("click observed button", {
      sessionId,
      action: "perform",
      observationId: observation.observationId,
      nodeRef: button.nodeRef,
      operation: "click",
    });
    if (
      !clickReceipt.verification?.pageChanged ||
      !clickReceipt.verification?.textChanged
    ) {
      throw new Error("browser action receipt did not verify the page change");
    }
    const snapshot = await executeAction("snapshot", {
      sessionId,
      action: "snapshot",
    });
    const screenshot = await executeAction("screenshot", {
      sessionId,
      action: "screenshot",
    });
    const text = snapshot.contents.find((content) => content.type === "text");
    const structuredSnapshot = snapshot.contents.find(
      (content) => content.type === "json",
    )?.value;
    const image = screenshot.contents.find(
      (content) => content.type === "image",
    );
    const screenshotBytes = Buffer.from(image?.bytes || "", "base64");
    if (!text?.text.includes("after") || !screenshotBytes.length) {
      throw new Error(
        "visible browser actions did not produce expected output",
      );
    }
    if (!Array.isArray(structuredSnapshot?.interactiveElements)) {
      throw new Error("public browser snapshot omitted structured elements");
    }
    if (
      structuredSnapshot.interactiveElements.some(
        (node) =>
          Object.hasOwn(node, "locator") ||
          Object.hasOwn(node, "selectorPath") ||
          Object.hasOwn(node, "nodeKey"),
      )
    ) {
      throw new Error("public browser snapshot exposed an internal locator");
    }

    let redirectBlocked = false;
    try {
      await executeAction("blocked cross-host redirect", {
        sessionId,
        action: "navigate",
        url: `${url}redirect`,
      });
    } catch (error) {
      redirectBlocked = /block|abort|fail/i.test(
        String(error?.message || error),
      );
    }
    if (!redirectBlocked) {
      throw new Error("unapproved cross-host redirect was not blocked");
    }

    await executeAction("navigate to subresource fixture", {
      sessionId,
      action: "navigate",
      url: `${url}subresource`,
    });
    await new Promise((resolve) => setTimeout(resolve, 250));
    const subresourceSnapshot = await executeAction("blocked subresource", {
      sessionId,
      action: "snapshot",
    });
    const subresourceText = subresourceSnapshot.contents.find(
      (content) => content.type === "text",
    )?.text;
    if (!subresourceText?.includes("request blocked")) {
      throw new Error("unapproved page fetch was not blocked");
    }

    const complexSessionId = "00000000-0000-4000-8000-000000000003";
    await executeAction("grant complex fixture hosts", {
      sessionId: complexSessionId,
      action: "grant_network_access",
      allowedHosts: ["127.0.0.1", "localhost"],
    });
    await executeAction("navigate to complex fixture", {
      sessionId: complexSessionId,
      action: "navigate",
      url: `${url}complex`,
    });
    let complex = await executeAction("observe frames and shadow roots", {
      sessionId: complexSessionId,
      action: "observe",
    });
    if (complex.frames.length < 3 || complex.accessibilityTree.length === 0) {
      throw new Error(
        "complex observation did not expose frames and accessibility tree",
      );
    }
    const initialTargetRef = complex.targets.find(
      (target) => target.active,
    )?.targetRef;
    const select = complex.nodes.find((node) => node.tagName === "select");
    const nestedShadow = complex.nodes.find(
      (node) => node.name === "Nested shadow action",
    );
    const frameShadow = complex.nodes.find(
      (node) => node.name === "Frame shadow action",
    );
    if (!initialTargetRef || !select || !nestedShadow || !frameShadow) {
      throw new Error(
        "complex observation missed select, frame, or nested shadow nodes",
      );
    }
    const rerender = complex.nodes.find(
      (node) => node.name === "Rerender shadow",
    );
    await executeAction("rerender shadow tree", {
      sessionId: complexSessionId,
      action: "perform",
      observationId: complex.observationId,
      nodeRef: rerender.nodeRef,
      operation: "click",
    });
    let shadowBecameStale = false;
    try {
      await executeAction("reject stale shadow locator", {
        sessionId: complexSessionId,
        action: "perform",
        observationId: complex.observationId,
        nodeRef: nestedShadow.nodeRef,
        operation: "click",
      });
    } catch (error) {
      shadowBecameStale = error?.code === "stale_observation";
    }
    if (!shadowBecameStale)
      throw new Error("shadow rerender did not invalidate the old locator");
    complex = await executeAction("reobserve rerendered shadow tree", {
      sessionId: complexSessionId,
      action: "observe",
    });
    const refreshedSelect = complex.nodes.find(
      (node) => node.tagName === "select",
    );
    await executeAction("select semantic option", {
      sessionId: complexSessionId,
      action: "perform",
      observationId: complex.observationId,
      nodeRef: refreshedSelect.nodeRef,
      operation: "select",
      value: "pro",
    });
    complex = await executeAction("observe selected value", {
      sessionId: complexSessionId,
      action: "observe",
    });
    if (!complex.text.includes("selected:pro"))
      throw new Error("select action did not dispatch change");
    const hover = complex.nodes.find((node) => node.name === "Hover action");
    await executeAction("hover semantic action", {
      sessionId: complexSessionId,
      action: "perform",
      observationId: complex.observationId,
      nodeRef: hover.nodeRef,
      operation: "hover",
    });
    complex = await executeAction("observe hover result", {
      sessionId: complexSessionId,
      action: "observe",
    });
    if (!complex.text.includes("hovered"))
      throw new Error("hover action did not dispatch semantic events");
    const frameAction = complex.nodes.find(
      (node) => node.name === "Frame shadow action",
    );
    await executeAction("click shadow node inside frame", {
      sessionId: complexSessionId,
      action: "perform",
      observationId: complex.observationId,
      nodeRef: frameAction.nodeRef,
      operation: "click",
    });
    complex = await executeAction("observe frame action", {
      sessionId: complexSessionId,
      action: "observe",
    });
    if (!complex.text.includes("Frame shadow clicked"))
      throw new Error("frame shadow action was not applied");
    const offscreen = complex.nodes.find(
      (node) => node.name === "Offscreen action",
    );
    await executeAction("scroll semantic action", {
      sessionId: complexSessionId,
      action: "perform",
      observationId: complex.observationId,
      nodeRef: offscreen.nodeRef,
      operation: "scroll",
      deltaY: 400,
    });
    complex = await executeAction("observe scroll result", {
      sessionId: complexSessionId,
      action: "observe",
    });
    if (!complex.text.includes("scrolled"))
      throw new Error("scroll action did not reach the offscreen node");
    const popupButton = complex.nodes.find(
      (node) => node.name === "Open popup",
    );
    await executeAction("open owned popup", {
      sessionId: complexSessionId,
      action: "perform",
      observationId: complex.observationId,
      nodeRef: popupButton.nodeRef,
      operation: "click",
    });
    const popupObservation = await executeAction("observe owned popup", {
      sessionId: complexSessionId,
      action: "observe",
    });
    if (
      popupObservation.targets.length < 2 ||
      !popupObservation.text.includes("Popup ready")
    ) {
      throw new Error(
        "popup was not owned and activated by the browser session",
      );
    }
    await executeAction("switch back to opener", {
      sessionId: complexSessionId,
      action: "switch_target",
      targetRef: initialTargetRef,
    });
    complex = await executeAction("observe opener after switch", {
      sessionId: complexSessionId,
      action: "observe",
    });
    const dialogButton = complex.nodes.find(
      (node) => node.name === "Show dialog",
    );
    await executeAction("dismiss javascript dialog", {
      sessionId: complexSessionId,
      action: "perform",
      observationId: complex.observationId,
      nodeRef: dialogButton.nodeRef,
      operation: "click",
    });
    complex = await executeAction("observe dialog diagnostics", {
      sessionId: complexSessionId,
      action: "observe",
    });
    if (
      !complex.dialogs.some(
        (dialog) => dialog.message === "fixture dialog" && dialog.handled,
      )
    ) {
      throw new Error("javascript dialog was not dismissed and reported");
    }

    // The visible browser is usable before a conversation creates a thread.
    const addressBarSessionId = "browser:standalone";
    await executeAction("prepare address-bar navigation", {
      sessionId: addressBarSessionId,
      action: "navigate",
      url,
    });
    await executeAction("restrict address-bar session for automation", {
      sessionId: addressBarSessionId,
      action: "grant_network_access",
      allowedHosts: ["127.0.0.1"],
    });
    const addressBarNavigation = await runStep(
      "user takeover permits address-bar cross-host redirect",
      () => host.navigateFromAddressBar(addressBarSessionId, `${url}redirect`),
    );
    const finalUrl = addressBarNavigation.contents.find(
      (content) => content.type === "json",
    )?.value?.url;
    if (new URL(finalUrl).hostname !== "localhost") {
      throw new Error(
        "address-bar redirect did not reach its final destination",
      );
    }

    const broker = await runStep("start broker", () => host.startBroker());
    const unauthorized = await fetch(`${broker.url}/health`);
    const healthy = await fetch(`${broker.url}/health`, {
      headers: { Authorization: `Bearer ${broker.token}` },
    });
    if (unauthorized.status !== 401 || !healthy.ok) {
      throw new Error("browser broker authentication smoke failed");
    }

    await executeAction("restore automation policy after user takeover", {
      sessionId: addressBarSessionId,
      action: "grant_network_access",
      allowedHosts: ["127.0.0.1"],
    });
    const blockedNavigation = await fetch(`${broker.url}/v1/browser`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${broker.token}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        sessionId: addressBarSessionId,
        action: "navigate",
        url: `${url}redirect`,
      }),
    });
    const blockedNavigationError = await blockedNavigation.json();
    if (
      blockedNavigation.status !== 403 ||
      blockedNavigationError?.error?.code !== "network_host_blocked" ||
      blockedNavigationError?.error?.host !== "localhost"
    ) {
      throw new Error(
        `broker did not preserve the typed network rejection: ${JSON.stringify(blockedNavigationError)}`,
      );
    }
    const blockedEphemeralNavigation = await fetch(`${broker.url}/v1/browser`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${broker.token}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        sessionId: ephemeralSessionId,
        action: "navigate",
        url: `${url}redirect`,
      }),
    });
    const blockedEphemeralError = await blockedEphemeralNavigation.json();
    if (
      blockedEphemeralNavigation.status !== 403 ||
      blockedEphemeralError?.error?.code !== "network_host_blocked" ||
      blockedEphemeralError?.error?.host !== "localhost"
    ) {
      throw new Error(
        `ephemeral profile did not enforce its network policy: ${JSON.stringify(blockedEphemeralError)}`,
      );
    }

    await executeAction("close session", { sessionId, action: "close" });
    await executeAction("close ephemeral session", {
      sessionId: ephemeralSessionId,
      action: "close",
    });
    await executeAction("close address-bar session", {
      sessionId: addressBarSessionId,
      action: "close",
    });
    await executeAction("close complex session", {
      sessionId: complexSessionId,
      action: "close",
    });
    process.stdout.write(
      `${JSON.stringify({
        snapshot: text.text.trim(),
        screenshotBytes: screenshotBytes.length,
        crossHostRedirectBlocked: true,
        crossHostSubresourceBlocked: true,
        profileIsolation: true,
        standaloneSessionNavigation: true,
        addressBarRedirectAllowed: true,
        unauthorizedStatus: unauthorized.status,
        healthyStatus: healthy.status,
      })}\n`,
    );
  } finally {
    await host.close();
    window.destroy();
    await new Promise((resolve) => pageServer.close(resolve));
    try {
      fs.rmSync(smokeDataRoot, { recursive: true, force: true });
    } catch {
      // Electron may release profile files only after app shutdown on Windows.
    }
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

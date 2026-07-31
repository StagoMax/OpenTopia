#!/usr/bin/env python3
"""Expose one BrowserGym MiniWoB++ task through OpenTopia's browser-broker contract.

The browser is owned by BrowserGym/Playwright.  OpenTopia reaches it only through
the same loopback HTTP protocol used by the desktop browser host, so the official
MiniWoB task setup and validator grade the actions made by OpenTopia itself.
"""

from __future__ import annotations

import argparse
import base64
import hmac
import json
import secrets
import signal
import sys
import threading
import time
import uuid
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, HTTPServer, SimpleHTTPRequestHandler
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

from browsergym.miniwob import ALL_MINIWOB_TASKS
from playwright.sync_api import Error as PlaywrightError
from playwright.sync_api import Page, sync_playwright


MAX_REQUEST_BYTES = 1024 * 1024
MAX_TEXT_BYTES = 1024 * 1024
MAX_SCREENSHOT_BYTES = 8 * 1024 * 1024
MAX_WAIT_MS = 30_000
OBSERVATION_TTL_SECONDS = 120
MAX_NODE_POSITION_DRIFT = 24


class BrokerError(Exception):
    def __init__(self, code: str, message: str, status: int = HTTPStatus.BAD_REQUEST):
        super().__init__(message)
        self.code = code
        self.status = status


def truncate_text(value: str, limit: int = MAX_TEXT_BYTES) -> tuple[str, bool]:
    encoded = value.encode("utf-8")
    if len(encoded) <= limit:
        return value, False
    return encoded[:limit].decode("utf-8", errors="ignore"), True


def browser_output(page: Page, action: str, contents: list[dict[str, Any]] | None = None, **metadata: Any) -> dict[str, Any]:
    return {
        "url": page.url or None,
        "contents": contents or [],
        "metadata": {"action": action, **metadata},
    }


class StaticSiteHandler(SimpleHTTPRequestHandler):
    def log_message(self, _format: str, *_args: object) -> None:
        return


class StaticSiteServer:
    def __init__(self, root: Path):
        self.root = root
        self.server = HTTPServer(("127.0.0.1", 0), self._handler())
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def _handler(self):
        root = self.root

        class Handler(StaticSiteHandler):
            def __init__(self, *args: object, **kwargs: object):
                super().__init__(*args, directory=str(root), **kwargs)

        return Handler

    @property
    def base_url(self) -> str:
        port = self.server.server_address[1]
        return f"http://127.0.0.1:{port}/miniwob/"

    def start(self) -> None:
        self.thread.start()

    def close(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)


class MiniwobBroker:
    def __init__(self, task_id: str, seed: int, miniwob_root: Path, browser_executable: Path | None):
        self.task_id = self._normalize_task_id(task_id)
        self.seed = seed
        self.site = StaticSiteServer(miniwob_root)
        self.browser_executable = browser_executable
        self.playwright = None
        self.browser = None
        self.context = None
        self.page: Page | None = None
        self.task = None
        self.goal = ""
        self.observations: dict[str, dict[str, Any]] = {}

    @staticmethod
    def _normalize_task_id(task_id: str) -> str:
        return task_id.removeprefix("browsergym/")

    def start(self) -> None:
        task_class = next(
            (candidate for candidate in ALL_MINIWOB_TASKS if candidate.get_task_id() == self.task_id),
            None,
        )
        if task_class is None:
            raise ValueError(f"Unknown BrowserGym MiniWoB task: {self.task_id}")

        self.site.start()
        self.task = task_class(seed=self.seed, base_url=self.site.base_url)
        self.playwright = sync_playwright().start()
        launch_options: dict[str, Any] = {"headless": True}
        if self.browser_executable:
            launch_options["executable_path"] = str(self.browser_executable)
        self.browser = self.playwright.chromium.launch(**launch_options)
        self.context = self.browser.new_context(viewport=self.task.viewport)
        self.page = self.context.new_page()
        self.goal, _setup_info = self.task.setup(self.page)

    def close(self) -> None:
        try:
            if self.context:
                self.context.close()
        finally:
            try:
                if self.browser:
                    self.browser.close()
            finally:
                if self.playwright:
                    self.playwright.stop()
                self.site.close()

    def _page(self) -> Page:
        if self.page is None:
            raise BrokerError("browser_unavailable", "BrowserGym page is unavailable.", HTTPStatus.SERVICE_UNAVAILABLE)
        return self.page

    def _snapshot(self) -> dict[str, Any]:
        page = self._page()
        result = page.evaluate(
            """() => {
              const selectorFor = (element) => {
                const escape = (value) => window.CSS && CSS.escape
                  ? CSS.escape(String(value))
                  : String(value).replace(/[^a-zA-Z0-9_-]/g, (char) => "\\\\" + char);
                if (element.id) return "#" + escape(element.id);
                const parts = [];
                let current = element;
                while (current && current.nodeType === Node.ELEMENT_NODE && parts.length < 8) {
                  let part = current.localName || "*";
                  const siblings = current.parentElement
                    ? Array.from(current.parentElement.children).filter((item) => item.localName === current.localName)
                    : [];
                  if (siblings.length > 1) part += ":nth-of-type(" + (siblings.indexOf(current) + 1) + ")";
                  parts.unshift(part);
                  current = current.parentElement;
                }
                return parts.join(" > ");
              };
              const roleFor = (element) => element.getAttribute("role") || ({
                a: "link", button: "button", textarea: "textbox", select: "combobox",
                input: element.type === "checkbox" ? "checkbox" : element.type === "radio" ? "radio" : "textbox"
              })[element.localName] || element.localName;
              const candidates = Array.from(document.querySelectorAll(
                "a[href],button,input,textarea,select,[role=button],[role=link],[contenteditable=true],[tabindex]"
              )).slice(0, 500);
              return {
                url: document.location.href,
                title: document.title,
                text: document.body ? document.body.innerText : "",
                nodes: candidates
                  .filter((element) => !element.disabled && element.getClientRects().length)
                  .map((element) => {
                    const rect = element.getBoundingClientRect();
                    return {
                      selector: selectorFor(element),
                      tagName: element.localName,
                      role: roleFor(element),
                      name: String(element.innerText || element.value || element.getAttribute("aria-label") || element.getAttribute("placeholder") || ""),
                      href: element.href || null,
                      formAction: element.getAttribute("formaction") || element.form?.getAttribute("action") || null,
                      formMethod: (element.getAttribute("formmethod") || element.form?.getAttribute("method") || "get").toLowerCase(),
                      inputType: element.getAttribute("type")?.toLowerCase() || null,
                      editable: Boolean(element.isContentEditable || (["input", "textarea", "select"].includes(element.localName) && !element.readOnly)),
                      bounds: { x: rect.x, y: rect.y, width: rect.width, height: rect.height }
                    };
                  })
              };
            }"""
        )
        text, text_truncated = truncate_text(str(result.get("text", "")))
        return {
            "url": str(result.get("url") or page.url),
            "title": str(result.get("title") or ""),
            "text": text,
            "textTruncated": text_truncated,
            "nodes": result.get("nodes") or [],
        }

    @staticmethod
    def _node(raw: dict[str, Any], node_ref: str) -> dict[str, Any]:
        return {
            "nodeRef": node_ref,
            "role": str(raw.get("role") or raw.get("tagName") or "element"),
            "name": str(raw.get("name") or ""),
            "tagName": str(raw.get("tagName") or ""),
            "bounds": raw.get("bounds") or {"x": 0, "y": 0, "width": 0, "height": 0},
            "href": raw.get("href"),
            "formAction": raw.get("formAction"),
            "formMethod": raw.get("formMethod"),
            "inputType": raw.get("inputType"),
            "editable": bool(raw.get("editable")),
            "requiresUserAction": False,
            "userActionReason": None,
        }

    @staticmethod
    def _matches(expected: dict[str, Any], current: dict[str, Any]) -> bool:
        fields = ("role", "name", "tagName", "href", "formAction", "formMethod", "inputType", "editable")
        if any(expected.get(field) != current.get(field) for field in fields):
            return False
        expected_bounds = expected.get("bounds") or {}
        current_bounds = current.get("bounds") or {}
        return all(
            abs(float(expected_bounds.get(field, 0)) - float(current_bounds.get(field, 0))) <= MAX_NODE_POSITION_DRIFT
            for field in ("x", "y", "width", "height")
        )

    def _prune_observations(self) -> None:
        cutoff = time.monotonic() - OBSERVATION_TTL_SECONDS
        stale = [observation_id for observation_id, item in self.observations.items() if item["captured"] < cutoff]
        for observation_id in stale:
            self.observations.pop(observation_id, None)
        while len(self.observations) > 12:
            self.observations.pop(next(iter(self.observations)))

    def observe(self, include_screenshot: bool) -> dict[str, Any]:
        snapshot = self._snapshot()
        observation_id = str(uuid.uuid4())
        bindings: dict[str, dict[str, Any]] = {}
        nodes = []
        for raw in snapshot["nodes"]:
            node_ref = str(uuid.uuid4())
            node = self._node(raw, node_ref)
            nodes.append(node)
            bindings[node_ref] = {"node": node, "selector": raw["selector"]}
        self.observations[observation_id] = {
            "captured": time.monotonic(),
            "url": snapshot["url"],
            "nodes": bindings,
        }
        self._prune_observations()
        screenshot = None
        if include_screenshot:
            image = self._page().screenshot(type="png")
            if len(image) > MAX_SCREENSHOT_BYTES:
                raise BrokerError("screenshot_too_large", "Screenshot exceeds the 8 MiB limit.", HTTPStatus.REQUEST_ENTITY_TOO_LARGE)
            screenshot = {"mimeType": "image/png", "bytes": list(image)}
        return {
            "observationId": observation_id,
            "url": snapshot["url"],
            "title": snapshot["title"],
            "text": snapshot["text"],
            "textTruncated": snapshot["textTruncated"],
            "nodes": nodes,
            "screenshot": screenshot,
        }

    def _observed_node(self, request: dict[str, Any]) -> tuple[dict[str, Any], dict[str, Any]]:
        observation_id = request.get("observationId")
        node_ref = request.get("nodeRef")
        if not isinstance(observation_id, str) or not isinstance(node_ref, str):
            raise BrokerError("invalid_observation", "observationId and nodeRef are required.")
        self._prune_observations()
        observation = self.observations.get(observation_id)
        if observation is None:
            raise BrokerError("stale_observation", "The observation is missing or expired.", HTTPStatus.CONFLICT)
        binding = observation["nodes"].get(node_ref)
        if binding is None:
            raise BrokerError("stale_observation", "The node does not belong to this observation.", HTTPStatus.CONFLICT)
        return observation, binding

    def perform(self, request: dict[str, Any]) -> dict[str, Any]:
        observation, binding = self._observed_node(request)
        page = self._page()
        if page.url != observation["url"]:
            raise BrokerError("stale_observation", "The page URL changed after the observation.", HTTPStatus.CONFLICT)
        snapshot = self._snapshot()
        current_raw = next((node for node in snapshot["nodes"] if node.get("selector") == binding["selector"]), None)
        if current_raw is None:
            raise BrokerError("stale_observation", "The observed element no longer exists.", HTTPStatus.CONFLICT)
        current = self._node(current_raw, binding["node"]["nodeRef"])
        if not self._matches(binding["node"], current):
            raise BrokerError("stale_observation", "The observed element changed or moved.", HTTPStatus.CONFLICT)

        operation = request.get("operation")
        locator = page.locator(binding["selector"]).first
        try:
            if operation == "click":
                locator.click(timeout=MAX_WAIT_MS)
            elif operation == "type":
                if not current["editable"]:
                    raise BrokerError("stale_observation", "The observed element is no longer editable.", HTTPStatus.CONFLICT)
                text = request.get("text")
                if not isinstance(text, str):
                    raise BrokerError("invalid_text", "text must be a string.")
                if request.get("clearFirst", True):
                    locator.fill(text, timeout=MAX_WAIT_MS)
                else:
                    locator.click(timeout=MAX_WAIT_MS)
                    locator.press_sequentially(text, timeout=MAX_WAIT_MS)
            else:
                raise BrokerError("invalid_action", "operation must be click or type.")
        except BrokerError:
            raise
        except PlaywrightError as error:
            raise BrokerError("browser_action_failed", str(error), HTTPStatus.CONFLICT) from error

        return {
            "observationId": request["observationId"],
            "nodeRef": request["nodeRef"],
            "action": operation,
            "target": current,
            "url": page.url,
            "title": page.title(),
        }

    def navigate(self, request: dict[str, Any]) -> dict[str, Any]:
        url = request.get("url")
        if not isinstance(url, str) or not url:
            raise BrokerError("invalid_url", "url must be a non-empty string.")
        parsed = urlparse(url)
        if parsed.scheme not in {"http", "https"}:
            raise BrokerError("blocked_protocol", "Navigation requires an HTTP(S) URL.", HTTPStatus.FORBIDDEN)
        try:
            self._page().goto(url, wait_until="domcontentloaded", timeout=MAX_WAIT_MS)
        except PlaywrightError as error:
            raise BrokerError("navigation_failed", str(error), HTTPStatus.GATEWAY_TIMEOUT) from error
        page = self._page()
        return browser_output(page, "navigate", [{"type": "json", "value": {"url": page.url, "title": page.title()}}], requested_url=url)

    def wait(self, request: dict[str, Any]) -> dict[str, Any]:
        page = self._page()
        wait = request.get("wait") if isinstance(request.get("wait"), dict) else {}
        timeout = min(max(int(wait.get("timeout_ms", 10_000)), 1), MAX_WAIT_MS)
        condition = wait.get("condition", "document_complete")
        try:
            if condition == "selector":
                selector = request.get("selector")
                if not isinstance(selector, str) or not selector:
                    raise BrokerError("invalid_selector", "selector is required for a selector wait.")
                page.locator(selector).first.wait_for(state="visible", timeout=timeout)
            elif condition == "text":
                text = request.get("text")
                if not isinstance(text, str) or not text:
                    raise BrokerError("invalid_text", "text is required for a text wait.")
                page.get_by_text(text, exact=False).first.wait_for(state="visible", timeout=timeout)
            elif condition == "document_complete":
                page.wait_for_load_state("domcontentloaded", timeout=timeout)
            else:
                raise BrokerError("invalid_wait", f"Unsupported wait condition '{condition}'.")
        except BrokerError:
            raise
        except PlaywrightError as error:
            raise BrokerError("timeout", str(error), HTTPStatus.GATEWAY_TIMEOUT) from error
        return browser_output(page, "wait")

    def screenshot(self) -> dict[str, Any]:
        image = self._page().screenshot(type="png")
        if len(image) > MAX_SCREENSHOT_BYTES:
            raise BrokerError("screenshot_too_large", "Screenshot exceeds the 8 MiB limit.", HTTPStatus.REQUEST_ENTITY_TOO_LARGE)
        return browser_output(self._page(), "screenshot", [{"type": "image", "mime_type": "image/png", "bytes": list(image)}])

    def result(self) -> dict[str, Any]:
        if self.task is None:
            return {"status": "not_started"}
        reward, done, _message, info = self.task.validate(self._page(), [])
        return {
            "benchmark": "BrowserGym MiniWoB++",
            "browsergymTask": self.task_id,
            "seed": self.seed,
            "goal": self.goal,
            "reward": reward,
            "completed": bool(done),
            "success": bool(reward > 0),
            "taskInfo": info,
        }

    def execute(self, request: dict[str, Any]) -> dict[str, Any]:
        action = request.get("action")
        if not isinstance(action, str):
            raise BrokerError("invalid_action", "action must be a string.")
        if action == "observe":
            return self.observe(bool(request.get("includeScreenshot")))
        if action == "observation_node":
            _observation, binding = self._observed_node(request)
            return binding["node"]
        if action == "perform":
            return self.perform(request)
        if action == "navigate":
            return self.navigate(request)
        if action == "wait":
            return self.wait(request)
        if action == "screenshot":
            return self.screenshot()
        if action == "close":
            return browser_output(self._page(), "close", [], closed=True)
        if action == "download":
            raise BrokerError("unsupported_action", "MiniWoB evaluation broker does not support direct downloads.")
        raise BrokerError("invalid_action", f"Unsupported browser action '{action}'.")


class BrokerServer(HTTPServer):
    def __init__(self, address: tuple[str, int], broker: MiniwobBroker, token: str):
        self.broker = broker
        self.token = token
        super().__init__(address, self._handler())

    def _handler(self):
        server = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, _format: str, *_args: object) -> None:
                return

            def _authorized(self) -> bool:
                supplied = self.headers.get("Authorization", "")
                return hmac.compare_digest(supplied, f"Bearer {server.token}")

            def _send(self, status: int, payload: dict[str, Any]) -> None:
                body = json.dumps(payload, ensure_ascii=True, separators=(",", ":")).encode("utf-8")
                self.send_response(status)
                self.send_header("Content-Type", "application/json; charset=utf-8")
                self.send_header("Content-Length", str(len(body)))
                self.send_header("Cache-Control", "no-store")
                self.send_header("X-Content-Type-Options", "nosniff")
                self.end_headers()
                self.wfile.write(body)

            def _error(self, error: BrokerError) -> None:
                self._send(int(error.status), {"error": {"code": error.code, "message": str(error)}})

            def _require_auth(self) -> bool:
                if self._authorized():
                    return True
                self._error(BrokerError("unauthorized", "A valid bearer token is required.", HTTPStatus.UNAUTHORIZED))
                return False

            def do_GET(self) -> None:
                if not self._require_auth():
                    return
                if self.path == "/health":
                    self._send(HTTPStatus.OK, {"ok": True, "service": "opentopia-browsergym-miniwob-broker"})
                    return
                if self.path == "/task":
                    self._send(HTTPStatus.OK, {"taskId": server.broker.task_id, "seed": server.broker.seed, "goal": server.broker.goal})
                    return
                if self.path == "/results":
                    self._send(HTTPStatus.OK, server.broker.result())
                    return
                self._error(BrokerError("not_found", "Broker endpoint was not found.", HTTPStatus.NOT_FOUND))

            def do_POST(self) -> None:
                if not self._require_auth():
                    return
                if self.path == "/shutdown":
                    self._send(HTTPStatus.OK, {"ok": True})
                    threading.Thread(target=server.shutdown, daemon=True).start()
                    return
                if self.path != "/v1/browser":
                    self._error(BrokerError("not_found", "Broker endpoint was not found.", HTTPStatus.NOT_FOUND))
                    return
                content_type = self.headers.get("Content-Type", "").lower()
                if not content_type.startswith("application/json"):
                    self._error(BrokerError("unsupported_media_type", "Content-Type must be application/json.", HTTPStatus.UNSUPPORTED_MEDIA_TYPE))
                    return
                try:
                    length = int(self.headers.get("Content-Length", "0"))
                    if length <= 0 or length > MAX_REQUEST_BYTES:
                        raise BrokerError("request_too_large", "Request body exceeds the 1 MiB limit.", HTTPStatus.REQUEST_ENTITY_TOO_LARGE)
                    payload = json.loads(self.rfile.read(length))
                    if not isinstance(payload, dict):
                        raise BrokerError("invalid_request", "Request body must be a JSON object.")
                    self._send(HTTPStatus.OK, server.broker.execute(payload))
                except BrokerError as error:
                    self._error(error)
                except json.JSONDecodeError:
                    self._error(BrokerError("invalid_json", "Request body is not valid JSON."))
                except Exception as error:
                    self._error(BrokerError("broker_error", str(error), HTTPStatus.INTERNAL_SERVER_ERROR))

        return Handler


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="BrowserGym MiniWoB++ bridge for OpenTopia browser evaluation")
    parser.add_argument("--task", required=True, help="BrowserGym task id, for example miniwob.click-button")
    parser.add_argument("--seed", type=int, default=0)
    parser.add_argument("--miniwob-root", type=Path, required=True, help="Path to MiniWoB++ miniwob/html directory")
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--token", required=True)
    parser.add_argument("--browser-executable", type=Path, help="Optional Chrome or Chromium executable for Playwright")
    parser.add_argument("--result-path", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    miniwob_root = args.miniwob_root.resolve()
    if not (miniwob_root / "miniwob").is_dir():
        raise SystemExit(f"MiniWoB HTML root does not contain miniwob/: {miniwob_root}")

    browser_executable = args.browser_executable.resolve() if args.browser_executable else None
    if browser_executable and not browser_executable.is_file():
        raise SystemExit(f"Browser executable was not found: {browser_executable}")
    broker = MiniwobBroker(args.task, args.seed, miniwob_root, browser_executable)
    broker.start()
    server = BrokerServer(("127.0.0.1", args.port), broker, args.token)
    port = server.server_address[1]
    print(json.dumps({"ok": True, "url": f"http://127.0.0.1:{port}", "task": broker.task_id}, separators=(",", ":")), flush=True)

    stop = threading.Event()

    def request_stop(_signal: int, _frame: object) -> None:
        stop.set()
        server.shutdown()

    signal.signal(signal.SIGINT, request_stop)
    signal.signal(signal.SIGTERM, request_stop)
    try:
        server.serve_forever()
    finally:
        if args.result_path:
            args.result_path.parent.mkdir(parents=True, exist_ok=True)
            args.result_path.write_text(json.dumps(broker.result(), ensure_ascii=True, indent=2) + "\n", encoding="utf-8")
        server.server_close()
        broker.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

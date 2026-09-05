---
name: control-browser
description: Control OpenTopia's shared browser to open or navigate pages, inspect visible and interactive state, click, type, select, hover, scroll, switch tabs, take screenshots, wait for page changes, and download files. Use when the user asks for browser interaction, local web testing, or visual page inspection.
---

# Control Browser

Use OpenTopia's `browser` tool. Do not use Codex Desktop's Node REPL or
`browser-client` runtime; those belong to a different host.

## Workflow

1. Navigate to the requested URL when the task does not already have a page.
2. Observe the page before every click, type, select, hover, or scroll.
3. Pass the exact observation ID and node reference returned by the latest observation as `observation_id` and `node_ref`.
4. Observe again after an interaction that can change the page. If the runtime reports a stale observation, discard the old references and observe again.
5. Use `switch_target` only with a `target_ref` returned by an observation.

Use `screenshot` when visual layout matters. Prefer bounded waits for a selector,
text, or document completion instead of repeatedly polling.

## Safety boundaries

- Respect the browser route selected by the host. Do not claim to control Chrome if the Chrome bridge is unavailable.
- Stop and hand control to the user when the page requires login, verification, credentials, upload, payment, publication, or another sensitive or irreversible action.
- Keep network access within the active policy and the plugin's allowed-domain configuration.
- If the `browser` tool is not visible, use tool discovery when available. If it remains unavailable, report that the Browser Automation plugin or its permissions are not active.

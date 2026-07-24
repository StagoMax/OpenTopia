# Web search

OpenTopia exposes web search through standard model tooling. It does not define a custom HTTP
search API.

## Provider-hosted search

Providers using the Responses protocol receive the hosted `web_search` tool automatically on
normal agent requests:

```json
{
  "tools": [{ "type": "web_search" }],
  "tool_choice": "auto"
}
```

The provider model decides whether the prompt requires a search. OpenTopia does not force a
search and does not proxy the provider's search traffic. Hosted search output annotations are
rendered as visible, clickable citations. This follows the provider's standard hosted-tool
contract; for example, see the [OpenAI web search guide](https://developers.openai.com/api/docs/guides/tools-web-search).

The Codex App Server provider also leaves Codex's built-in web search available. Codex owns that
tool's protocol, credentials, and execution; OpenTopia only prevents unrelated built-in tools from
bypassing its local permission system.

When both native search and a search-capable MCP tool are available, native search has priority.
The agent uses MCP search only when native search is unavailable, fails, or the required source is
available only through that MCP server. It does not run both for the same query unless it is
falling back from a failed native search.

Guardian review requests do not receive hosted search because they review the existing agent
trace rather than gather new external evidence.

## MCP search

Any search MCP server can be configured through the existing MCP server workflow and enabled for
a task. OpenTopia discovers the server's tools and adds them to the normal agent tool catalog. The
model selects an MCP search tool by its advertised name, description, and input schema, just like
any other MCP tool.

There is no search-specific adapter, endpoint contract, or separate search API key in OpenTopia.
MCP server credentials stay with that MCP server's configuration.

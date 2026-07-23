# Web search integration

OpenTopia supports two web-search transports:

- `provider_native`: adds the hosted `{ "type": "web_search" }` tool to OpenAI Responses requests.
- `custom_api`: exposes a local `web_search` function tool backed by a user-configured HTTP endpoint.

## Custom API contract

OpenTopia sends an HTTP `POST` request with JSON content:

```json
{
  "query": "latest Rust release notes",
  "maxResults": 5
}
```

When the user configures an API key, the request includes:

```http
Authorization: Bearer <api-key>
```

The preferred response shape is:

```json
{
  "results": [
    {
      "title": "Result title",
      "url": "https://example.com/article",
      "snippet": "Short result extract"
    }
  ]
}
```

For adapter compatibility, the top-level response may also be an array or use `data` or `items` instead of `results`. Result fields may use `name` for `title`, `link` for `url`, and `description` or `content` for `snippet`.

Only HTTP(S) result URLs are passed to the model. Responses are limited to 1 MiB, redirects are not followed, and at most 10 results are accepted. Search result content is marked as untrusted before it enters the agent context.

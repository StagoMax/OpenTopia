# MCP Attachment Inspection Capability

OpenTopia exposes one model-facing image operation: `view_attachment`.
The runtime chooses the delivery route:

- A vision-capable selected model receives native MCP/OpenAI-style image content.
- A text-only selected model may use one enabled MCP tool that explicitly declares the
  `media.image.inspect/v1` capability.
- Without either route, `view_attachment` returns a capability-unavailable error so the
  model can state that it cannot inspect the image.

MCP tool names, descriptions, and input property names are never used to infer this
capability. A compatible server declares the contract in the Tool `_meta` returned by
`tools/list`.

## Canonical declaration

The shortest declaration uses canonical `image` and `focus` arguments:

```json
{
  "name": "inspect_asset",
  "description": "Inspect an asset",
  "inputSchema": {
    "type": "object",
    "properties": {
      "image": { "type": "object" },
      "focus": { "type": "string" }
    },
    "required": ["image"]
  },
  "_meta": {
    "com.opentopia/capabilities": ["media.image.inspect/v1"]
  }
}
```

OpenTopia calls the tool with:

```json
{
  "image": {
    "data": "<base64 without a data-URL prefix>",
    "mimeType": "image/png",
    "name": "capture.png"
  },
  "focus": "The model-provided inspection focus"
}
```

The tool should return a bounded text or structured-content description that a
text-only model can consume. Image-only results cannot restore vision to a text-only
model.

## Explicit input mapping

Tools with different field names declare JSON Pointer mappings instead of relying on
name matching:

```json
{
  "name": "run",
  "inputSchema": {
    "type": "object",
    "properties": {
      "payload": { "type": "object" },
      "request": { "type": "string" }
    },
    "required": ["payload", "request"],
    "additionalProperties": false
  },
  "_meta": {
    "com.opentopia/capabilities": {
      "media.image.inspect/v1": {
        "priority": 20,
        "input": {
          "image": {
            "pointer": "/payload/source",
            "encoding": "data_url"
          },
          "focus": "/request"
        }
      }
    }
  }
}
```

JSON Pointers may traverse object properties. Array construction is intentionally not
part of v1; an adapter MCP can normalize more complex upstream APIs.

Supported image encodings:

| Encoding | Value written at the image pointer |
| --- | --- |
| `object_base64` | `{ "data", "mimeType", "name" }` (default) |
| `base64` | Raw base64 string |
| `data_url` | `data:<mime>;base64,<data>` string |

Set `input.focus` to `null` if the tool has no question/focus argument.

## Provider selection

Only tools enabled for the current thread participate. The highest numeric `priority`
wins. Equal highest priorities are treated as a configuration conflict; OpenTopia does
not select an external data recipient arbitrarily.

A tool selected as a `media.image.inspect/v1` backend is hidden from the model-facing
tool catalog. The model calls `view_attachment`, while OpenTopia resolves the local
attachment ID, enforces MCP permissions, converts the bytes, calls the configured
backend, and returns its text/JSON observation.

## Security boundary

Declaring a capability does not bypass MCP authorization. Sending an attachment to an
external server remains an external tool call and goes through the existing server/tool
permission policy. Capability `_meta` is a routing contract, not a trust grant.

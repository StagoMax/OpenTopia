# Configuration Migration Contract

Implement `src/config.js` and `src/cli.js` without external dependencies.

## Library

`parseLegacyConfig(jsonText)` accepts a JSON object with exactly these fields:

- `version`: integer `1`.
- `endpoint`: an absolute `http` or `https` URL without credentials.
- `model`: a non-empty string; trim surrounding whitespace before validation and output.
- `timeout_seconds`: an integer from 1 through 300.
- `features`: an array containing unique values from `stream` and `tools`.

It returns this shape without mutating inputs:

```json
{
  "schemaVersion": 2,
  "provider": { "baseUrl": "https://example.test/v1", "model": "model-a" },
  "timeoutMs": 30000,
  "capabilities": { "stream": true, "tools": false }
}
```

Remove trailing slashes from `baseUrl`. Reject unknown/missing fields, malformed JSON,
unknown/duplicate features, invalid ranges, URL credentials, and unsupported schemes with
descriptive errors.

`applyOverrides(config, env)` returns a new normalized config. Recognized non-empty values are
`APP_BASE_URL`, `APP_MODEL`, and `APP_TIMEOUT_MS`. Apply the same validation rules; reject an
invalid integer timeout. Ignore unrelated environment keys. Do not mutate either argument.

`serializeConfig(config)` validates the normalized config and returns deterministic JSON with
two-space indentation, the key order shown above, and exactly one trailing newline.

## CLI

`node src/cli.js --input <legacy.json> --output <config.json>` reads the legacy file, applies
`process.env` overrides, writes serialized output, and prints exactly:

```text
Migrated config for <model>.
```

On invalid arguments or data it must exit non-zero, explain the error on stderr, and not create
the output file. Do not modify `SPEC.md` or files under `test/`.

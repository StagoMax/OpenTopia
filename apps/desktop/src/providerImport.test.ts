import assert from "node:assert/strict";
import test from "node:test";

import type * as ProviderImportModule from "./providerImport";

const providerImport: typeof ProviderImportModule = await import(
  "./providerImport" + ".ts"
);

const {
  PROVIDER_IMPORT_PRESETS,
  createProviderDraftFromPreset,
  parseProviderImport,
} = providerImport;

test("parses nested camelCase JSON and suggests a provider identity", () => {
  const draft = parseProviderImport(`{
    "config": {
      "provider": {
        "providerName": "Acme Gateway",
        "baseUrl": "https://llm.acme.example/v1/",
        "model": "acme-reasoner",
        "apiKey": "secret-json-key"
      }
    }
  }`);

  assert.deepEqual(
    {
      id: draft.id,
      name: draft.name,
      kind: draft.kind,
      baseUrl: draft.baseUrl,
      model: draft.model,
      apiKey: draft.apiKey,
      detectedFormat: draft.detectedFormat,
    },
    {
      id: "acme-gateway",
      name: "Acme Gateway",
      kind: "openai_compatible",
      baseUrl: "https://llm.acme.example/v1",
      model: "acme-reasoner",
      apiKey: "secret-json-key",
      detectedFormat: "json",
    },
  );
  assert.equal(draft.warnings.join(" ").includes("secret-json-key"), false);
});

test("parses OPENAI-style JSON keys and recognizes the Responses API", () => {
  const draft = parseProviderImport(
    JSON.stringify({
      OPENAI_BASE_URL: "https://api.openai.com/v1/responses",
      OPENAI_MODEL: "gpt-4.1",
      OPENAI_API_KEY: "secret-openai-key",
    }),
  );

  assert.equal(draft.kind, "openai_responses");
  assert.equal(draft.baseUrl, "https://api.openai.com/v1");
  assert.equal(draft.model, "gpt-4.1");
  assert.equal(draft.apiKey, "secret-openai-key");
  assert.equal(draft.name, "OpenAI Responses");
});

test("parses dotenv, export, and PowerShell assignment syntax", () => {
  const draft = parseProviderImport(`
    # Existing provider configuration
    export OPENAI_BASE_URL="https://api.deepseek.com/v1/"
    OPENAI_MODEL="deepseek-chat" # keep this comment out of the value
    $Env:OPENAI_API_KEY = 'secret-env-key'
    set UNUSED_PROVIDER_VALUE=ignored
  `);

  assert.equal(draft.detectedFormat, "env");
  assert.equal(draft.id, "deepseek");
  assert.equal(draft.baseUrl, "https://api.deepseek.com/v1");
  assert.equal(draft.model, "deepseek-chat");
  assert.equal(draft.apiKey, "secret-env-key");
  assert.equal(draft.warnings.join(" ").includes("secret-env-key"), false);
});

test("keeps variable credential references out of normalized JSON drafts", () => {
  const draft = parseProviderImport(`{
    "OPENAI_BASE_URL": "gateway.example.test/v1",
    "OPENAI_MODEL_NAME": "example-model",
    "OPENAI_API_KEY": "\${OPENAI_API_KEY}"
  }`);

  assert.equal(draft.baseUrl, "https://gateway.example.test/v1");
  assert.equal(draft.model, "example-model");
  assert.equal(draft.apiKey, undefined);
  assert.ok(
    draft.warnings.some((warning) => warning.includes("variable reference")),
  );
});

test("extracts endpoint, bearer token, model, and kind from multiline curl", () => {
  const draft =
    parseProviderImport(`curl https://api.example.test/v1/responses \\
    -H "Authorization: Bearer secret-curl-key" \\
    -H "Content-Type: application/json" \\
    --data-raw '{"model":"reasoning-large","input":"hello"}'`);

  assert.equal(draft.detectedFormat, "curl");
  assert.equal(draft.baseUrl, "https://api.example.test/v1");
  assert.equal(draft.kind, "openai_responses");
  assert.equal(draft.model, "reasoning-large");
  assert.equal(draft.apiKey, "secret-curl-key");
  assert.equal(draft.warnings.join(" ").includes("secret-curl-key"), false);
});

test("does not treat a curl environment-variable reference as an API key", () => {
  const draft =
    parseProviderImport(`curl.exe --url http://localhost:11434/api/chat \\
    --header 'Authorization: Bearer $OPENAI_API_KEY' \\
    --json '{"model":"qwen2.5-coder:7b"}'`);

  assert.equal(draft.baseUrl, "http://localhost:11434/v1");
  assert.equal(draft.name, "Ollama Local");
  assert.equal(draft.apiKey, undefined);
  assert.ok(
    draft.warnings.some((warning) => warning.includes("variable reference")),
  );
  assert.ok(
    draft.warnings.some((warning) => warning.includes("Ollama endpoint")),
  );
});

test("returns usable defaults and warnings for unknown or incomplete input", () => {
  const unknown = parseProviderImport("this is not a provider configuration");
  assert.equal(unknown.detectedFormat, "unknown");
  assert.equal(unknown.baseUrl, "https://api.openai.com/v1");
  assert.equal(unknown.model, "gpt-4.1-mini");
  assert.ok(unknown.warnings.length >= 3);

  const invalidJson = parseProviderImport("{not-json}");
  assert.equal(invalidJson.detectedFormat, "json");
  assert.ok(
    invalidJson.warnings.includes("The JSON configuration is invalid."),
  );
});

test("exposes stable presets and creates independent editable drafts", () => {
  assert.deepEqual(
    PROVIDER_IMPORT_PRESETS.map((preset) => preset.id),
    ["openai-compatible", "openai-responses", "ollama-local"],
  );

  const draft = createProviderDraftFromPreset("ollama-local");
  assert.deepEqual(draft, {
    id: "ollama-local",
    name: "Ollama Local",
    kind: "openai_compatible",
    baseUrl: "http://localhost:11434/v1",
    model: "qwen2.5-coder:7b",
    warnings: [],
    detectedFormat: "unknown",
  });
});

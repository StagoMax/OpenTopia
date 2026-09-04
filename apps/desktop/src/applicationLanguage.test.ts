import assert from "node:assert/strict";
import test from "node:test";
import {
  applicationLanguageStorageKey,
  defaultApplicationLanguage,
  interfaceMessage,
  normalizeApplicationLanguage,
  readApplicationLanguage,
  writeApplicationLanguage,
} from "./applicationLanguage.ts";

test("defaults the interface language to Simplified Chinese", () => {
  assert.equal(defaultApplicationLanguage, "zh-CN");
  assert.equal(normalizeApplicationLanguage(undefined), "zh-CN");
  assert.equal(normalizeApplicationLanguage("fr-FR"), "zh-CN");
  assert.equal(readApplicationLanguage(null), "zh-CN");
});

test("reads and writes a supported interface language", () => {
  const values = new Map<string, string>();
  const storage = {
    getItem(key: string) {
      return values.get(key) ?? null;
    },
    setItem(key: string, value: string) {
      values.set(key, value);
    },
  };

  writeApplicationLanguage("en-US", storage);

  assert.equal(values.get(applicationLanguageStorageKey), "en-US");
  assert.equal(readApplicationLanguage(storage), "en-US");
});

test("provides Chinese and English Flow navigation messages", () => {
  assert.equal(interfaceMessage("zh-CN", "flow.nav.overview"), "总览");
  assert.equal(interfaceMessage("en-US", "flow.nav.overview"), "Overview");
});

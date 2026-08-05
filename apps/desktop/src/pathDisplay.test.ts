import assert from "node:assert/strict";
import test from "node:test";

import type * as PathDisplayModule from "./pathDisplay";

const { formatPathForDisplay }: typeof PathDisplayModule = await import(
  "./pathDisplay" + ".ts"
);

test("removes the Windows verbatim prefix from drive paths", () => {
  assert.equal(
    formatPathForDisplay("\\\\?\\J:\\Project\\OpenTopia"),
    "J:\\Project\\OpenTopia",
  );
});

test("converts verbatim UNC paths to standard UNC paths", () => {
  assert.equal(
    formatPathForDisplay("\\\\?\\UNC\\server\\share\\OpenTopia"),
    "\\\\server\\share\\OpenTopia",
  );
});

test("leaves regular Windows and Unix paths unchanged", () => {
  assert.equal(
    formatPathForDisplay("J:\\Project\\OpenTopia"),
    "J:\\Project\\OpenTopia",
  );
  assert.equal(formatPathForDisplay("/srv/opentopia"), "/srv/opentopia");
});

import assert from "node:assert/strict";
import test from "node:test";

import type * as ProviderErrorsModule from "./providerErrors";

const { friendlyProviderError }: typeof ProviderErrorsModule = await import(
  "./providerErrors" + ".ts"
);

test("turns provider quota codes into an actionable permanent error", () => {
  assert.equal(
    friendlyProviderError(
      'provider request failed (403): {"code":"insufficient_user_quota"}',
    ),
    "额度不足：当前 Provider 账户没有可用额度。请充值或切换 Provider；充值后请在设置中重新测试连接。",
  );
});

test("leaves unrelated provider failures intact", () => {
  assert.equal(
    friendlyProviderError("provider request timed out"),
    "provider request timed out",
  );
});

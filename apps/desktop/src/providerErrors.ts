export function friendlyProviderError(message: string): string {
  if (
    /insufficient_user_quota|insufficient_quota|quota_exhausted|quota exceeded|用户额度不足|余额不足/i.test(
      message,
    )
  ) {
    return "额度不足：当前 Provider 账户没有可用额度。请充值或切换 Provider；充值后请在设置中重新测试连接。";
  }
  if (/401|auth_failed|master_key|unauthorized/i.test(message)) {
    return "认证失败：当前 Provider 的 Base URL 拒绝了 API Key。请在设置中更新该 Provider 的密钥并测试连接。";
  }
  return message;
}

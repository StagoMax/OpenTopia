export function friendlyProviderError(message: string): string {
  if (/401|auth_failed|master_key|unauthorized/i.test(message)) {
    return "认证失败：当前 Provider 的 Base URL 拒绝了 API Key。请在设置中更新该 Provider 的密钥并测试连接。";
  }
  return message;
}

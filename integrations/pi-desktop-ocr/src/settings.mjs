export const DEFAULTS = Object.freeze({
  baseUrl: 'http://127.0.0.1:18080', modelId: '', prompt: 'Text Recognition:',
  maxTokens: 4096, timeoutSeconds: 1800, maxImageEdge: 0, view: 'markdown',
  autoLoad: true, rememberToken: false,
});
export function fail(code, message) { return Object.assign(new Error(message), { code }); }
export function normalizeSettings(previous, patch) {
  if (!patch || typeof patch !== 'object' || Array.isArray(patch)) throw fail('invalid_settings', '设置格式不正确。');
  if (Object.keys(patch).some(key => !Object.hasOwn(DEFAULTS, key))) throw fail('invalid_settings', '包含不支持的设置项。');
  const next = { ...previous, ...patch };
  let url;
  try { url = new URL(next.baseUrl); } catch { throw fail('invalid_address', '请输入本机 Nexa 地址，例如 http://127.0.0.1:18080。'); }
  if (url.hostname === 'localhost') url.hostname = '127.0.0.1';
  if (url.protocol !== 'http:' || url.hostname !== '127.0.0.1' || !url.port || url.username || url.password || url.search || url.hash || !['/', '/v1', '/v1/'].includes(url.pathname)) throw fail('invalid_address', '仅支持带端口的本机 127.0.0.1 HTTP 地址。');
  next.baseUrl = url.origin;
  if (typeof next.modelId !== 'string' || (next.modelId && !/^[a-z0-9][a-z0-9._-]{0,63}$/.test(next.modelId))) throw fail('invalid_settings', '模型 ID 格式不正确，请从列表选择已登记模型。');
  if (typeof next.prompt !== 'string' || !next.prompt.trim() || next.prompt.length > 16384 || Buffer.byteLength(next.prompt) > 32768) throw fail('invalid_settings', '提示词不能为空，且最多 16384 个字符 / 32 KiB。');
  for (const [key, min, max] of [['maxTokens', 1, 4096], ['timeoutSeconds', 30, 86400]]) {
    if (!Number.isInteger(next[key]) || next[key] < min || next[key] > max) throw fail('invalid_settings', `${key} 必须为 ${min}–${max} 的整数。`);
  }
  if (!Number.isInteger(next.maxImageEdge) || (next.maxImageEdge !== 0 && (next.maxImageEdge < 512 || next.maxImageEdge > 8192))) throw fail('invalid_settings', '最长边应为 0（原图）或 512–8192。');
  if (!['markdown', 'text'].includes(next.view) || typeof next.autoLoad !== 'boolean' || typeof next.rememberToken !== 'boolean') throw fail('invalid_settings', '视图或开关设置不正确。');
  return next;
}
export function normalizeToken(token) {
  if (typeof token !== 'string' || !/^[a-f0-9]{64}$/.test(token.trim())) throw fail('invalid_token', 'Nexa 令牌应为 64 位小写十六进制文本，请选择正确的 api-token 文件。');
  return token.trim();
}

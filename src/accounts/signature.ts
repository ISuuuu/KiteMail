/**
 * 签名预览纯函数，移植自 GPUI 版 crates/app/src/ui/accounts.rs#signature_preview_html：
 * HTML 源码先清洗（去脚本、事件属性、危险协议），纯文本转义后按阅读器规则整形。
 */

const HTML_TAG_PATTERN = /<[\s\S]*?>/;

/** 与 GPUI markup::looks_like_html 一致的轻量判断：包含完整标签对才视为 HTML 源码。 */
export function looksLikeHtml(body: string): boolean {
  return HTML_TAG_PATTERN.test(body);
}

function escapeHtml(text: string): string {
  return text
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

/**
 * 白名单式清洗签名 HTML：去掉脚本块、事件属性与危险协议，
 * 对齐 GPUI signature::sanitize_signature_html 的预览行为。
 */
export function sanitizeSignatureHtml(html: string): string {
  let out = html
    // script / style / iframe / object / embed 整块移除
    .replace(/<script[\s\S]*?<\/script\s*>/gi, "")
    .replace(/<style[\s\S]*?<\/style\s*>/gi, "")
    .replace(/<iframe[\s\S]*?<\/iframe\s*>/gi, "")
    .replace(/<object[\s\S]*?<\/object\s*>/gi, "")
    .replace(/<embed[^>]*>/gi, "")
    // 自闭合残块
    .replace(/<script[^>]*\/?>/gi, "");
  // 移除 on* 事件属性与 javascript:/vbscript:/data:text-html 链接协议
  out = out.replace(/\son[a-z-]+\s*=\s*("[^"]*"|'[^']*'|[^\s>]*)/gi, "");
  out = out.replace(
    /\s(?:href|src|xlink:href)\s*=\s*(?:"\s*(?:javascript|vbscript|data:text\/html)[^"]*"|'\s*(?:javascript|vbscript|data:text\/html)[^']*'|(?:javascript|vbscript):[^\s>]*)/gi,
    ""
  );
  return out;
}

/** 纯文本签名转等价 HTML：转义后换行转 <br>。 */
export function plainTextSignatureToHtml(body: string): string {
  return escapeHtml(body).replaceAll("\r\n", "\n").replaceAll("\n", "<br>");
}

/**
 * 签名源码渲染成预览 HTML（signature_preview_html）：
 * 非 HTML 模式或空内容返回空串；HTML 源码清洗后原样返回，纯文本转义整形。
 */
export function buildSignaturePreviewHtml(body: string, isHtml: boolean): string {
  if (!isHtml || !body.trim()) {
    return "";
  }
  return looksLikeHtml(body) ? sanitizeSignatureHtml(body) : plainTextSignatureToHtml(body);
}

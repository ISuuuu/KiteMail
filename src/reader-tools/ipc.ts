/**
 * KiteMail 读信辅助、附件中心与独立打印窗口 IPC 客户端与可控 Mock 传输层。
 *
 * 参数包装约定（与 Rust 后端完全一致，有参命令均包在 `{ input }` 中）：
 *   list_cached_attachments({ input })
 *   fetch_attachment_preview({ input })
 *   save_attachment({ input })
 *   save_all_attachments({ input })
 *   get_message_print_html({ input })
 *   open_message_window({ input })
 */

import { isTauri } from "../ipc/main-window";
import type { IpcError, OpenMessageOutput } from "../ipc/types";
import type {
  CachedAttachmentDto,
  FetchAttachmentPreviewInput,
  FetchAttachmentPreviewOutput,
  FetchAttachmentThumbnailOutput,
  GetMessagePrintHtmlInput,
  GetMessagePrintHtmlOutput,
  ListCachedAttachmentsInput,
  ListCachedAttachmentsOutput,
  OpenAttachmentInput,
  OpenAttachmentOutput,
  OpenMessageWindowInput,
  OpenMessageWindowOutput,
  PickDirectoryOutput,
  SaveAllAttachmentsInput,
  SaveAllAttachmentsOutput,
  SaveAttachmentInput,
  SaveAttachmentOutput,
} from "./types";

async function tauriInvoke<T>(cmd: string, args?: unknown): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args as Record<string, unknown>);
}

// ---------- 安全 HTML 转义与打印文档生成 ----------

const HTML_ESCAPE_MAP: Record<string, string> = {
  "&": "&amp;",
  "<": "&lt;",
  ">": "&gt;",
  '"': "&quot;",
  "'": "&#39;",
};

export function escapeHtml(raw: string): string {
  return raw.replace(/[&<>"']/g, (ch) => HTML_ESCAPE_MAP[ch] ?? ch);
}

/**
 * 清理邮件 HTML 片段中的高危标签与脚本属性，避免打印视图 XSS。
 */
export function sanitizePrintHtmlBody(html: string): string {
  return html
    .replace(/<script\b[^>]*>[\s\S]*?<\/script>/gi, "")
    .replace(/<iframe\b[^>]*>[\s\S]*?<\/iframe>/gi, "")
    .replace(/<object\b[^>]*>[\s\S]*?<\/object>/gi, "")
    .replace(/<embed\b[^>]*>/gi, "")
    .replace(/\son[a-z]+\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+)/gi, "")
    .replace(/(href|src)\s*=\s*(["'])\s*javascript:[^"']*\2/gi, '$1="#"');
}

// ---------- Mock 状态与种子数据 ----------

const MOCK_BASE_TS = 1_764_240_000;

const DEFAULT_MOCK_ATTACHMENTS: ReadonlyArray<CachedAttachmentDto> = [
  {
    id: 1,
    messageId: 2,
    accountId: 1,
    filename: "KiteMail-三栏布局规范.pdf",
    mimeType: "application/pdf",
    sizeBytes: 184_320,
    sender: "Build Bot <ci@kitemail.example>",
    subject: "[CI] x86_64-pc-windows-msvc 夜间构建完成",
    dateTs: MOCK_BASE_TS - 1200,
    dateDisplay: "今天 11:18",
    downloaded: true,
    isImage: false,
  },
  {
    id: 2,
    messageId: 3,
    accountId: 1,
    filename: "三栏间距与色板对比.png",
    mimeType: "image/png",
    sizeBytes: 426_800,
    sender: "产品设计组 <design@kitemail.example>",
    subject: "三栏布局与阅读区字号反馈（含附件清单）",
    dateTs: MOCK_BASE_TS - 3600,
    dateDisplay: "昨天 17:40",
    downloaded: true,
    isImage: true,
  },
  {
    id: 3,
    messageId: 3,
    accountId: 1,
    filename: "暗色主题预览图.webp",
    mimeType: "image/webp",
    sizeBytes: 312_400,
    sender: "产品设计组 <design@kitemail.example>",
    subject: "三栏布局与阅读区字号反馈（含附件清单）",
    dateTs: MOCK_BASE_TS - 3650,
    dateDisplay: "昨天 17:40",
    downloaded: false,
    isImage: true,
  },
  {
    id: 4,
    messageId: 8,
    accountId: 1,
    filename: "虚拟列表性能压测曲线.svg",
    mimeType: "image/svg+xml",
    sizeBytes: 48_120,
    sender: "陈思远 <siyuan@example.org>",
    subject: "Re: 周五邮件客户端性能压测记录",
    dateTs: MOCK_BASE_TS - 7200,
    dateDisplay: "昨天 15:10",
    downloaded: true,
    isImage: true,
  },
  {
    id: 5,
    messageId: 8,
    accountId: 1,
    filename: "压测指标明细.xlsx",
    mimeType: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    sizeBytes: 95_232,
    sender: "陈思远 <siyuan@example.org>",
    subject: "Re: 周五邮件客户端性能压测记录",
    dateTs: MOCK_BASE_TS - 7260,
    dateDisplay: "昨天 15:10",
    downloaded: false,
    isImage: false,
  },
  {
    id: 6,
    messageId: 15,
    accountId: 1,
    filename: "IMAP-IDLE-断线重连流程图.png",
    mimeType: "image/png",
    sizeBytes: 518_912,
    sender: "林晓明 <xiaoming@example.com>",
    subject: "同步引擎状态机与离线缓存设计",
    dateTs: MOCK_BASE_TS - 24_000,
    dateDisplay: "2 天前",
    downloaded: true,
    isImage: true,
  },
  {
    id: 7,
    messageId: 15,
    accountId: 1,
    filename: "离线缓存架构评审纪要.docx",
    mimeType: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    sizeBytes: 142_080,
    sender: "林晓明 <xiaoming@example.com>",
    subject: "同步引擎状态机与离线缓存设计",
    dateTs: MOCK_BASE_TS - 24_100,
    dateDisplay: "2 天前",
    downloaded: true,
    isImage: false,
  },
  {
    id: 8,
    messageId: 22,
    accountId: 1,
    filename: "WebView2-DPI-150-截图.jpg",
    mimeType: "image/jpeg",
    sizeBytes: 634_110,
    sender: "QA 团队 <qa@kitemail.example>",
    subject: "Windows 11 高分屏回归测试截图",
    dateTs: MOCK_BASE_TS - 48_000,
    dateDisplay: "3 天前",
    downloaded: false,
    isImage: true,
  },
  {
    id: 9,
    messageId: 29,
    accountId: 1,
    filename: "CHANGELOG-v0.4.txt",
    mimeType: "text/plain",
    sizeBytes: 8_420,
    sender: "Release Bot <release@kitemail.example>",
    subject: "KiteMail v0.4 候选版本变更日志",
    dateTs: MOCK_BASE_TS - 86_400,
    dateDisplay: "4 天前",
    downloaded: true,
    isImage: false,
  },
  {
    id: 10,
    messageId: 36,
    accountId: 1,
    filename: "安全沙箱策略白皮书.pdf",
    mimeType: "application/pdf",
    sizeBytes: 782_336,
    sender: "安全组 <security@kitemail.example>",
    subject: "HTML 邮件渲染与外链图片拦截规范",
    dateTs: MOCK_BASE_TS - 120_000,
    dateDisplay: "5 天前",
    downloaded: false,
    isImage: false,
  },
  {
    id: 11,
    messageId: 43,
    accountId: 1,
    filename: "图标栅格规范-24px.png",
    mimeType: "image/png",
    sizeBytes: 129_024,
    sender: "产品设计组 <design@kitemail.example>",
    subject: "工具栏图标与快捷键提示更新",
    dateTs: MOCK_BASE_TS - 160_000,
    dateDisplay: "6 天前",
    downloaded: true,
    isImage: true,
  },
  {
    id: 12,
    messageId: 50,
    accountId: 1,
    filename: "账户OAuth回调日志.log",
    mimeType: "text/plain",
    sizeBytes: 19_840,
    sender: "运维支持 <ops@kitemail.example>",
    subject: "Gmail OAuth2 PKCE 联调记录",
    dateTs: MOCK_BASE_TS - 200_000,
    dateDisplay: "更早",
    downloaded: false,
    isImage: false,
  },
  {
    id: 13,
    messageId: 57,
    accountId: 1,
    filename: "团队产品路标图.png",
    mimeType: "image/png",
    sizeBytes: 845_120,
    sender: "周雨晴 <yuqing@kitemail.example>",
    subject: "Q4 里程碑与发布节奏同步",
    dateTs: MOCK_BASE_TS - 240_000,
    dateDisplay: "更早",
    downloaded: true,
    isImage: true,
  },
  {
    id: 14,
    messageId: 64,
    accountId: 1,
    filename: "邮件导出归档包.zip",
    mimeType: "application/zip",
    sizeBytes: 2_450_000,
    sender: "归档助手 <archive@kitemail.example>",
    subject: "11 月项目往来邮件归档",
    dateTs: MOCK_BASE_TS - 280_000,
    dateDisplay: "更早",
    downloaded: false,
    isImage: false,
  },
];

let mockLocked = false;
let mockStateVersion = 1;
let mockAttachments: CachedAttachmentDto[] = DEFAULT_MOCK_ATTACHMENTS.map((a) => ({ ...a }));
let mockCustomMessages = new Map<string, OpenMessageOutput>();
let mockFailPreviewMessage: string | null = null;
let mockFailSaveMessage: string | null = null;
let mockDefaultDestDir = "C:\\Users\\Demo\\Downloads\\KiteMail";
let mockPickedDir: string | null = "C:\\Users\\Demo\\Downloads\\KiteMail";
const mockOpenedAttachmentIds: number[] = [];

function makeLockedError(): IpcError {
  return {
    code: "LOCKED",
    message: "应用已锁定，请先解锁后再访问附件或邮件内容",
    retryable: false,
  };
}

function makeNotFoundError(code: string, message: string): IpcError {
  return {
    code,
    message,
    retryable: false,
  };
}

function makeMessageKey(accountId: number, messageId: number): string {
  return `${accountId}:${messageId}`;
}

function isMockPdfAttachment(att: CachedAttachmentDto): boolean {
  const mime = att.mimeType.trim().toLowerCase();
  return mime === "application/pdf" || att.filename.trim().toLowerCase().endsWith(".pdf");
}

function encodeAsciiBase64(ascii: string): string {
  if (typeof btoa === "function") {
    return btoa(ascii);
  }
  const TABLE = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  let out = "";
  for (let i = 0; i < ascii.length; i += 3) {
    const b0 = ascii.charCodeAt(i) & 0xff;
    const has1 = i + 1 < ascii.length;
    const has2 = i + 2 < ascii.length;
    const b1 = has1 ? ascii.charCodeAt(i + 1) & 0xff : 0;
    const b2 = has2 ? ascii.charCodeAt(i + 2) & 0xff : 0;
    const triple = (b0 << 16) | (b1 << 8) | b2;
    out += TABLE[(triple >> 18) & 0x3f];
    out += TABLE[(triple >> 12) & 0x3f];
    out += has1 ? TABLE[(triple >> 6) & 0x3f] : "=";
    out += has2 ? TABLE[triple & 0x3f] : "=";
  }
  return out;
}

function toPdfAsciiLiteral(raw: string, fallback: string): string {
  const cleaned = raw.replace(/[^\x20-\x7e]+/g, " ").trim();
  const stem = cleaned.replace(/\.pdf$/i, "").trim();
  const text = /[a-zA-Z0-9]/.test(stem) ? cleaned : fallback;
  return text.slice(0, 68).replace(/\\/g, "\\\\").replace(/\(/g, "\\(").replace(/\)/g, "\\)");
}

function buildMockPdfDataUrl(att: CachedAttachmentDto): string {
  const safeTitle = toPdfAsciiLiteral(att.filename, "KiteMail Mock PDF");
  const stream =
    `1 1 1 rg 0 0 612 792 re f\n` +
    `0.93 0.95 0.99 rg 36 660 540 96 re f\n` +
    `0.21 0.52 0.89 rg 36 750 540 6 re f\n` +
    `0.12 0.16 0.24 rg BT /F1 20 Tf 56 716 Td (${safeTitle}) Tj ET\n` +
    `0.82 0.86 0.91 rg 56 586 500 8 re f 56 566 440 8 re f`;
  const objs = [
    "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
    "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
    "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
    "4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
    `5 0 obj\n<< /Length ${stream.length} >>\nstream\n${stream}\nendstream\nendobj\n`,
  ];
  let pdf = "%PDF-1.4\n";
  const offsets: number[] = [];
  for (const obj of objs) {
    offsets.push(pdf.length);
    pdf += obj;
  }
  const xrefOffset = pdf.length;
  pdf += "xref\n0 6\n0000000000 65535 f \n";
  for (const off of offsets) {
    pdf += `${String(off).padStart(10, "0")} 00000 n \n`;
  }
  pdf += `trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n${xrefOffset}\n%%EOF\n`;
  return `data:application/pdf;base64,${encodeAsciiBase64(pdf)}`;
}

function buildMockPdfThumbnailUrl(att: CachedAttachmentDto): string {
  const safeTitle = escapeHtml(att.filename);
  const safeSender = escapeHtml(att.sender);
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="240" height="300" viewBox="0 0 240 300">` +
    `<rect width="240" height="300" fill="#ffffff"/>` +
    `<rect x="0" y="0" width="240" height="5" fill="#e01b24"/>` +
    `<rect x="14" y="16" width="212" height="46" rx="6" fill="#eef3fb"/>` +
    `<rect x="22" y="24" width="32" height="14" rx="3" fill="#e01b24"/>` +
    `<text x="38" y="34" text-anchor="middle" fill="#ffffff" font-family="Segoe UI, sans-serif" font-size="8.5" font-weight="700">PDF</text>` +
    `<text x="62" y="35" fill="#182234" font-family="Segoe UI, sans-serif" font-size="11" font-weight="600">${safeTitle}</text>` +
    `<text x="22" y="53" fill="#5b667a" font-family="Segoe UI, sans-serif" font-size="8.5">${safeSender}</text>` +
    `<rect x="14" y="76" width="84" height="8" rx="2" fill="#3584e4"/>` +
    `<rect x="14" y="94" width="212" height="6" rx="2" fill="#d7dfec"/>` +
    `<rect x="14" y="106" width="194" height="6" rx="2" fill="#d7dfec"/>` +
    `<rect x="14" y="118" width="204" height="6" rx="2" fill="#d7dfec"/>` +
    `<rect x="14" y="130" width="142" height="6" rx="2" fill="#d7dfec"/>` +
    `<rect x="14" y="150" width="212" height="62" rx="6" fill="#f6f8fc" stroke="#dce4f2" stroke-width="1"/>` +
    `<rect x="24" y="162" width="76" height="6" rx="2" fill="#3584e4"/>` +
    `<rect x="24" y="176" width="190" height="5" rx="2" fill="#cdd7e8"/>` +
    `<rect x="24" y="188" width="174" height="5" rx="2" fill="#cdd7e8"/>` +
    `<rect x="24" y="200" width="128" height="5" rx="2" fill="#cdd7e8"/>` +
    `<rect x="14" y="226" width="208" height="6" rx="2" fill="#d7dfec"/>` +
    `<rect x="14" y="238" width="186" height="6" rx="2" fill="#d7dfec"/>` +
    `<rect x="14" y="250" width="160" height="6" rx="2" fill="#d7dfec"/>` +
    `<line x1="14" y1="274" x2="226" y2="274" stroke="#e4e9f2" stroke-width="1"/>` +
    `<text x="14" y="288" fill="#7a8496" font-family="Segoe UI, sans-serif" font-size="8">KiteMail PDF Thumbnail · Page 1</text>` +
    `</svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

function buildMockSvgPreviewUrl(att: CachedAttachmentDto): string {
  const palette = ["#3584e4", "#7253b9", "#2672af", "#1b7d50", "#ae552a", "#b75077"];
  const accent = palette[att.id % palette.length];
  const safeName = escapeHtml(att.filename);
  const safeSender = escapeHtml(att.sender);
  const safeMime = escapeHtml(att.mimeType);
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="960" height="540" viewBox="0 0 960 540">` +
    `<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">` +
    `<stop offset="0%" stop-color="${accent}" stop-opacity="0.18"/>` +
    `<stop offset="100%" stop-color="${accent}" stop-opacity="0.45"/>` +
    `</linearGradient></defs>` +
    `<rect width="960" height="540" rx="18" fill="#181b22"/>` +
    `<rect x="24" y="24" width="912" height="492" rx="14" fill="url(#g)" stroke="${accent}" stroke-width="2"/>` +
    `<circle cx="150" cy="180" r="52" fill="${accent}" fill-opacity="0.28"/>` +
    `<path d="M84 390 L280 220 L440 340 L610 190 L840 390 Z" fill="${accent}" fill-opacity="0.32"/>` +
    `<text x="64" y="92" fill="#f3f5f9" font-family="Segoe UI, sans-serif" font-size="28" font-weight="600">${safeName}</text>` +
    `<text x="64" y="130" fill="#b8c0d0" font-family="Segoe UI, sans-serif" font-size="16">${safeMime} · ${att.sizeBytes} bytes</text>` +
    `<text x="64" y="476" fill="#d3d9e6" font-family="Segoe UI, sans-serif" font-size="15">来自：${safeSender}</text>` +
    `</svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

/** 图片缩略图：受限尺寸（240x150）的小 SVG，模拟后端只返回小图的行为 */
function buildMockImageThumbnailUrl(att: CachedAttachmentDto): string {
  const palette = ["#3584e4", "#7253b9", "#2672af", "#1b7d50", "#ae552a", "#b75077"];
  const accent = palette[att.id % palette.length];
  const safeName = escapeHtml(att.filename);
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="240" height="150" viewBox="0 0 240 150">` +
    `<rect width="240" height="150" rx="8" fill="#181b22"/>` +
    `<circle cx="46" cy="42" r="16" fill="${accent}" fill-opacity="0.45"/>` +
    `<path d="M16 124 L74 66 L120 104 L164 58 L228 124 Z" fill="${accent}" fill-opacity="0.4"/>` +
    `<text x="16" y="24" fill="#f3f5f9" font-family="Segoe UI, sans-serif" font-size="11">${safeName}</text>` +
    `</svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

function buildMockTextPreviewUrl(att: CachedAttachmentDto): string {
  const previewText = [
    `文件名：${att.filename}`,
    `类型：${att.mimeType}`,
    `大小：${att.sizeBytes} 字节`,
    `来源邮件：${att.subject}`,
    `发件人：${att.sender}`,
    `日期：${att.dateDisplay}`,
    "",
    "--- 文档摘要预览 ---",
    `该附件已在本地缓存索引中登记（ID #${att.id}）。`,
    "点击「保存」可将其导出到本地下载目录。",
  ].join("\n");
  return `data:text/plain;charset=utf-8,${encodeURIComponent(previewText)}`;
}

function buildMockMessage(accountId: number, messageId: number): OpenMessageOutput {
  const custom = mockCustomMessages.get(makeMessageKey(accountId, messageId));
  if (custom) {
    return {
      ...custom,
      attachments: custom.attachments.map((a) => ({ ...a })),
      threadMessages: custom.threadMessages.map((t) => ({ ...t })),
    };
  }

  const relatedAttachments = mockAttachments.filter(
    (a) => a.accountId === accountId && a.messageId === messageId
  );
  const firstAtt = relatedAttachments[0];
  const subject = firstAtt?.subject ?? `独立阅读邮件 #${messageId}`;
  const sender = firstAtt?.sender ?? "林晓明 <xiaoming@example.com>";
  const dateDisplay = firstAtt?.dateDisplay ?? "今天 14:30";

  const plainText = [
    `主题：${subject}`,
    `发件人：${sender}`,
    "",
    "您好，",
    "",
    `这是在独立读信浮窗中打开的邮件 #${messageId} 正文内容。`,
    "支持一键切换打印视图、调整阅读字号、快速回复/全部回复/转发，并可查看与保存关联附件。",
    "",
    relatedAttachments.length > 0
      ? `本邮件包含 ${relatedAttachments.length} 个附件：${relatedAttachments.map((a) => a.filename).join("、")}。`
      : "本邮件无附件。",
    "",
    "祝好，",
    "KiteMail 团队",
  ].join("\n");

  return {
    requestId: `standalone-${accountId}-${messageId}`,
    readerSessionId: `standalone-session-${accountId}-${messageId}`,
    accountId,
    messageId,
    subject,
    sender,
    recipients: "demo@kitemail.example",
    cc: "team@kitemail.example",
    dateDisplay,
    bodyStatus: "ready",
    plainText,
    hasHtml: false,
    htmlBody: null,
    flagged: messageId % 3 === 0,
    remoteImagesBlocked: false,
    rfcMessageId: `<standalone-${accountId}-${messageId}@kitemail.example>`,
    folderPath: "INBOX",
    attachments: relatedAttachments.map((a) => ({
      id: a.id,
      filename: a.filename,
      mimeType: a.mimeType,
      sizeBytes: a.sizeBytes,
    })),
    threadMessages: [],
  };
}

function buildPrintDocument(
  msg: OpenMessageOutput,
  includeHeader: boolean
): GetMessagePrintHtmlOutput {
  const safeSubject = escapeHtml(msg.subject || "(无主题)");
  const safeSender = escapeHtml(msg.sender || "未知发件人");
  const safeRecipients = escapeHtml(msg.recipients || "");
  const safeCc = escapeHtml(msg.cc || "");
  const safeDate = escapeHtml(msg.dateDisplay || "");

  const headerHtml = includeHeader
    ? `<header class="km-print-header">
        <h1 class="km-print-subject">${safeSubject}</h1>
        <dl class="km-print-meta">
          <div><dt>发件人</dt><dd>${safeSender}</dd></div>
          ${safeRecipients ? `<div><dt>收件人</dt><dd>${safeRecipients}</dd></div>` : ""}
          ${safeCc ? `<div><dt>抄送</dt><dd>${safeCc}</dd></div>` : ""}
          ${safeDate ? `<div><dt>日期</dt><dd>${safeDate}</dd></div>` : ""}
        </dl>
      </header>`
    : `<h1 class="km-print-subject km-print-subject-compact">${safeSubject}</h1>`;

  const attachmentsHtml =
    msg.attachments.length > 0
      ? `<section class="km-print-attachments">
          <h2>附件 (${msg.attachments.length})</h2>
          <ul>
            ${msg.attachments
              .map(
                (att) =>
                  `<li>${escapeHtml(att.filename)} <span>(${escapeHtml(att.mimeType)}, ${att.sizeBytes} B)</span></li>`
              )
              .join("")}
          </ul>
        </section>`
      : "";

  const bodyHtml =
    msg.hasHtml && msg.htmlBody
      ? `<article class="km-print-body">${sanitizePrintHtmlBody(msg.htmlBody)}</article>`
      : `<article class="km-print-body"><pre class="km-print-pre">${escapeHtml(
          msg.plainText ?? ""
        )}</pre></article>`;

  const htmlDocument = `<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8" />
<title>${safeSubject}</title>
<style>
  :root { color-scheme: light; }
  * { box-sizing: border-box; }
  body {
    margin: 0;
    padding: 28px 32px;
    font-family: "Segoe UI", "Microsoft YaHei UI", "PingFang SC", system-ui, sans-serif;
    font-size: 14px;
    line-height: 1.65;
    color: #1f2328;
    background: #ffffff;
  }
  .km-print-header {
    padding-bottom: 16px;
    margin-bottom: 18px;
    border-bottom: 2px solid #d0d7de;
  }
  .km-print-subject {
    margin: 0 0 12px;
    font-size: 20px;
    font-weight: 700;
    color: #111827;
  }
  .km-print-subject-compact {
    padding-bottom: 12px;
    border-bottom: 1px solid #e5e7eb;
  }
  .km-print-meta {
    margin: 0;
    display: grid;
    gap: 4px;
  }
  .km-print-meta > div {
    display: flex;
    gap: 10px;
    font-size: 13px;
  }
  .km-print-meta dt {
    width: 60px;
    flex-shrink: 0;
    font-weight: 600;
    color: #57606a;
  }
  .km-print-meta dd {
    margin: 0;
    word-break: break-word;
  }
  .km-print-pre {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    font-family: inherit;
    font-size: 14px;
    line-height: 1.7;
  }
  .km-print-attachments {
    margin-top: 22px;
    padding-top: 12px;
    border-top: 1px solid #e5e7eb;
    font-size: 12.5px;
  }
  .km-print-attachments h2 {
    margin: 0 0 8px;
    font-size: 13px;
    color: #374151;
  }
  .km-print-attachments ul {
    margin: 0;
    padding-left: 18px;
  }
  .km-print-attachments span {
    color: #6b7280;
  }
  @media print {
    body { padding: 0; }
  }
</style>
</head>
<body>
${headerHtml}
${bodyHtml}
${attachmentsHtml}
</body>
</html>`;

  return {
    subject: msg.subject,
    htmlDocument,
    plainText: msg.plainText ?? "",
  };
}

// ---------- 公共 IPC API ----------

export async function listCachedAttachments(
  input: ListCachedAttachmentsInput = {}
): Promise<ListCachedAttachmentsOutput> {
  if (isTauri()) {
    return tauriInvoke<ListCachedAttachmentsOutput>("list_cached_attachments", { input });
  }
  if (mockLocked) return Promise.reject(makeLockedError());

  const q = (input.query ?? "").trim().toLowerCase();
  const offset = Math.max(0, input.offset ?? 0);
  const limit = Math.max(1, input.limit ?? 24);

  const filtered = mockAttachments
    .filter((item) => {
      if (!q) return true;
      return (
        item.filename.toLowerCase().includes(q) ||
        item.sender.toLowerCase().includes(q) ||
        item.subject.toLowerCase().includes(q) ||
        item.mimeType.toLowerCase().includes(q)
      );
    })
    .sort((a, b) => b.dateTs - a.dateTs || a.id - b.id);

  const page = filtered.slice(offset, offset + limit).map((item) => ({ ...item }));
  return {
    items: page,
    total: filtered.length,
    hasMore: offset + page.length < filtered.length,
    stateVersion: mockStateVersion,
  };
}

export async function fetchAttachmentPreview(
  input: FetchAttachmentPreviewInput
): Promise<FetchAttachmentPreviewOutput> {
  if (isTauri()) {
    return tauriInvoke<FetchAttachmentPreviewOutput>("fetch_attachment_preview", { input });
  }
  if (mockLocked) return Promise.reject(makeLockedError());
  if (mockFailPreviewMessage) {
    return Promise.reject({
      code: "PREVIEW_FAILED",
      message: mockFailPreviewMessage,
      retryable: true,
    } satisfies IpcError);
  }

  const att = mockAttachments.find((item) => item.id === input.attachmentId);
  if (!att) {
    return Promise.reject(
      makeNotFoundError("ATTACHMENT_NOT_FOUND", `未找到附件 #${input.attachmentId}`)
    );
  }

  const isPdf = isMockPdfAttachment(att);
  if (isPdf) {
    return {
      attachmentId: att.id,
      filename: att.filename,
      mimeType: "application/pdf",
      dataUrl: input.thumbnailOnly ? "" : buildMockPdfDataUrl(att),
      sizeBytes: input.thumbnailOnly ? 0 : att.sizeBytes,
      thumbnailDataUrl: buildMockPdfThumbnailUrl(att),
    };
  }

  if (input.thumbnailOnly) {
    return {
      attachmentId: att.id,
      filename: att.filename,
      mimeType: att.mimeType,
      dataUrl: "",
      sizeBytes: 0,
      thumbnailDataUrl: att.isImage ? buildMockImageThumbnailUrl(att) : null,
    };
  }

  return {
    attachmentId: att.id,
    filename: att.filename,
    mimeType: att.mimeType,
    dataUrl: att.isImage ? buildMockSvgPreviewUrl(att) : buildMockTextPreviewUrl(att),
    sizeBytes: att.sizeBytes,
  };
}

/** 缩略图专用入口：只取受限尺寸小图，不拉取原文件 */
export async function fetchAttachmentThumbnail(
  input: FetchAttachmentPreviewInput
): Promise<FetchAttachmentThumbnailOutput> {
  const preview = await fetchAttachmentPreview({ ...input, thumbnailOnly: true });
  return {
    attachmentId: preview.attachmentId,
    filename: preview.filename,
    mimeType: preview.mimeType,
    sizeBytes: preview.sizeBytes,
    thumbnailDataUrl: preview.thumbnailDataUrl ?? null,
  };
}

export async function saveAttachment(
  input: SaveAttachmentInput
): Promise<SaveAttachmentOutput> {
  if (isTauri()) {
    return tauriInvoke<SaveAttachmentOutput>("save_attachment", { input });
  }
  if (mockLocked) return Promise.reject(makeLockedError());
  if (mockFailSaveMessage) {
    return Promise.reject({
      code: "SAVE_FAILED",
      message: mockFailSaveMessage,
      retryable: true,
    } satisfies IpcError);
  }

  const att = mockAttachments.find((item) => item.id === input.attachmentId);
  if (!att) {
    return Promise.reject(
      makeNotFoundError("ATTACHMENT_NOT_FOUND", `未找到附件 #${input.attachmentId}`)
    );
  }

  att.downloaded = true;
  mockStateVersion += 1;
  const savedPath = input.destPath?.trim()
    ? input.destPath.trim()
    : `${mockDefaultDestDir}\\${att.filename}`;

  return {
    saved: true,
    savedPath,
    message: `已保存「${att.filename}」至 ${savedPath}`,
  };
}

export async function saveAllAttachments(
  input: SaveAllAttachmentsInput
): Promise<SaveAllAttachmentsOutput> {
  if (isTauri()) {
    return tauriInvoke<SaveAllAttachmentsOutput>("save_all_attachments", { input });
  }
  if (mockLocked) return Promise.reject(makeLockedError());
  if (mockFailSaveMessage) {
    return Promise.reject({
      code: "SAVE_FAILED",
      message: mockFailSaveMessage,
      retryable: true,
    } satisfies IpcError);
  }

  const destDir = input.destDir?.trim() || mockDefaultDestDir;
  if (input.attachmentIds.length === 0) {
    return {
      savedCount: 0,
      destDir,
      savedPaths: [],
      message: "没有可保存的附件",
    };
  }

  const savedPaths: string[] = [];
  for (const id of input.attachmentIds) {
    const att = mockAttachments.find((item) => item.id === id);
    if (!att) continue;
    att.downloaded = true;
    savedPaths.push(`${destDir}\\${att.filename}`);
  }

  if (savedPaths.length === 0) {
    return Promise.reject(
      makeNotFoundError("ATTACHMENT_NOT_FOUND", "未找到任何可保存的附件")
    );
  }

  mockStateVersion += 1;
  return {
    savedCount: savedPaths.length,
    destDir,
    savedPaths,
    message: `已批量保存 ${savedPaths.length} 个附件至 ${destDir}`,
  };
}

export async function getMessagePrintHtml(
  input: GetMessagePrintHtmlInput
): Promise<GetMessagePrintHtmlOutput> {
  if (isTauri()) {
    return tauriInvoke<GetMessagePrintHtmlOutput>("get_message_print_html", { input });
  }
  if (mockLocked) return Promise.reject(makeLockedError());

  const msg = buildMockMessage(input.accountId, input.messageId);
  return buildPrintDocument(msg, input.includeHeader !== false);
}

export async function openMessageWindow(
  input: OpenMessageWindowInput
): Promise<OpenMessageWindowOutput> {
  if (isTauri()) {
    return tauriInvoke<OpenMessageWindowOutput>("open_message_window", { input });
  }
  if (mockLocked) return Promise.reject(makeLockedError());

  const msg = buildMockMessage(input.accountId, input.messageId);
  return {
    windowLabel: `reader-${input.accountId}-${input.messageId}`,
    opened: true,
    message: msg,
  };
}

export async function openAttachment(
  input: OpenAttachmentInput
): Promise<OpenAttachmentOutput> {
  if (isTauri()) {
    return tauriInvoke<OpenAttachmentOutput>("open_attachment", { input });
  }
  if (mockLocked) return Promise.reject(makeLockedError());

  const att = mockAttachments.find((item) => item.id === input.attachmentId);
  const filename = att?.filename ?? `attachment-${input.attachmentId}.bin`;
  if (att) {
    att.downloaded = true;
  }
  mockOpenedAttachmentIds.push(input.attachmentId);
  const path = `${mockDefaultDestDir}\\opened-attachments\\${filename}`;
  return {
    opened: true,
    path,
    message: `已打开附件 ${filename}`,
  };
}

export async function pickDirectory(): Promise<PickDirectoryOutput> {
  if (isTauri()) {
    return tauriInvoke<PickDirectoryOutput>("pick_directory");
  }
  if (mockLocked) return Promise.reject(makeLockedError());
  if (!mockPickedDir) {
    return { cancelled: true, path: null };
  }
  return { cancelled: false, path: mockPickedDir };
}

export function getMockOpenedAttachmentIds(): readonly number[] {
  return mockOpenedAttachmentIds;
}

// ---------- 测试重置辅助（仅作用于 Mock 传输层） ----------

export function __resetReaderToolsMock(options?: {
  locked?: boolean;
  attachments?: CachedAttachmentDto[];
  customMessages?: OpenMessageOutput[];
  failPreviewMessage?: string | null;
  failSaveMessage?: string | null;
  destDir?: string;
  pickedDir?: string | null;
}): void {
  mockLocked = options?.locked ?? false;
  mockStateVersion = 1;
  mockFailPreviewMessage = options?.failPreviewMessage ?? null;
  mockFailSaveMessage = options?.failSaveMessage ?? null;
  mockDefaultDestDir = options?.destDir ?? "C:\\Users\\Demo\\Downloads\\KiteMail";
  mockPickedDir =
    options && "pickedDir" in options
      ? (options.pickedDir ?? null)
      : mockDefaultDestDir;
  mockOpenedAttachmentIds.length = 0;
  mockAttachments = (options?.attachments ?? DEFAULT_MOCK_ATTACHMENTS).map((a) => ({ ...a }));
  mockCustomMessages = new Map();
  if (options?.customMessages) {
    for (const msg of options.customMessages) {
      mockCustomMessages.set(makeMessageKey(msg.accountId, msg.messageId), {
        ...msg,
        attachments: msg.attachments.map((a) => ({ ...a })),
        threadMessages: msg.threadMessages.map((t) => ({ ...t })),
      });
    }
  }
}

/**
 * 账号品牌与头像纯函数，移植自 GPUI 版：
 * - crates/app/src/ui/accounts.rs#brand_for_account_fields（品牌图标匹配）
 * - crates/app/src/ui/avatar.rs#fnv1a / AVATAR_COLORS / sender_initials（头像底色与首字母）
 *
 * 品牌图标直接复用仓库 `data/brands/*.png`（与 GPUI `brands/<id>.png` 同源），
 * 由 Vite 静态导入打包，无需运行时网络或文件系统访问。
 */

import type { AccountConfigDto, AuthBackend } from "./types";

/** 品牌图标文件名（与 GPUI `brands/<id>.png` 同源，仓库 data/brands/）。 */
const BRAND_IDS = [
  "aol",
  "aliyun_qiye",
  "exmail",
  "fastmail",
  "gmail",
  "gmx",
  "icloud",
  "mailcom",
  "netease126",
  "netease163",
  "outlook",
  "proton",
  "qq",
  "stalwart",
  "yahoo",
  "yandex",
  "zoho",
] as const;

/** 品牌图标 URL 表（Vite 静态资源处理，测试环境仅得到占位 URL）。 */
export const BRAND_IMAGE_URLS: Readonly<Record<string, string>> = Object.fromEntries(
  BRAND_IDS.map((id) => [
    id,
    new URL(`../../data/brands/${id}.png`, import.meta.url).href,
  ])
);

/** 圆形头像满铺显示的品牌（GPUI account_circle_visual 的 is_tile_brand 集合）。 */
const TILE_BRANDS: ReadonlySet<string> = new Set([
  "qq",
  "netease163",
  "netease126",
  "exmail",
  "aliyun_qiye",
  "yahoo",
  "aol",
  "mailcom",
]);

export function isTileBrand(brandId: string): boolean {
  return TILE_BRANDS.has(brandId);
}

/** 头像背景预设色：10 个柔和高饱和色（与 GPUI avatar.rs AVATAR_COLORS 一致）。 */
export const AVATAR_COLORS: readonly string[] = [
  "#337fdc",
  "#0f9ac8",
  "#29ae74",
  "#6ab85b",
  "#d29d09",
  "#d68400",
  "#ed5b00",
  "#e62d42",
  "#e33b6a",
  "#9945b5",
];

/** 账号外观面板可选的 10 色强调色调色板（十六进制 + 中文名称）。 */
export const ACCOUNT_ACCENT_COLORS: ReadonlyArray<readonly [string, string]> = [
  ["#337fdc", "海洋蓝"],
  ["#0f9ac8", "青碧"],
  ["#29ae74", "翠绿"],
  ["#6ab85b", "草绿"],
  ["#d29d09", "琥珀"],
  ["#d68400", "暖橙"],
  ["#ed5b00", "橘红"],
  ["#e62d42", "绯红"],
  ["#e33b6a", "玫红"],
  ["#9945b5", "紫罗兰"],
];

/** FNV-1a（32 位）哈希，按 UTF-8 字节迭代（与 GPUI `text.bytes()` 一致）。 */
export function fnv1a(text: string): number {
  const bytes = new TextEncoder().encode(text);
  let hash = 0x811c9dc5;
  for (const byte of bytes) {
    hash ^= byte;
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash >>> 0;
}

const HEX_COLOR_PATTERN = /^#[0-9a-fA-F]{6}$/;

/** 返回账号最终强调色十六进制串：优先合法自定义色，否则按邮箱 FNV-1a 取默认色。 */
export function accountAccentHex(email: string, customColor?: string | null): string {
  const custom = customColor?.trim();
  if (custom && HEX_COLOR_PATTERN.test(custom)) {
    return custom.toLowerCase();
  }
  return AVATAR_COLORS[fnv1a(email) % AVATAR_COLORS.length];
}

function isUnicodeAlnum(c: string): boolean {
  return /[\p{L}\p{N}]/u.test(c);
}

function isAsciiAlnum(c: string): boolean {
  return (c >= "a" && c <= "z") || (c >= "A" && c <= "Z") || (c >= "0" && c <= "9");
}

/** 首字符为 CJK 表意文字（中文取 1 字规则）。 */
function isCjk(c: string): boolean {
  const code = c.codePointAt(0) ?? 0;
  return (
    (code >= 0x4e00 && code <= 0x9fff) ||
    (code >= 0x3400 && code <= 0x4dbf) ||
    (code >= 0xf900 && code <= 0xfaff) ||
    (code >= 0x30000 && code <= 0x3134f)
  );
}

/** 单词首字母列表：跳过括号 / 尖括号内容（`Name <a@b.c>` 只算 Name）。 */
function wordInitials(text: string): string[] {
  let outside = "";
  let depth = 0;
  for (const c of text) {
    if (c === "(" || c === "<" || c === "[") {
      depth += 1;
    } else if (c === ")" || c === ">" || c === "]") {
      if (depth > 0) depth -= 1;
    } else if (depth === 0) {
      outside += c;
    }
  }
  const source = [...outside].some(isUnicodeAlnum) ? outside : text;
  return source
    .split(/\s+/)
    .filter(Boolean)
    .map((w) => [...w].find(isUnicodeAlnum))
    .filter((c): c is string => Boolean(c));
}

/** 发件人显示名 → 头像首字符（中文取 1 字，英文取首词与末词首字母）。 */
export function senderInitials(name: string): string {
  const trimmed = name.trim();
  const first = [...trimmed].find(isUnicodeAlnum);
  if (first && isCjk(first)) {
    return first;
  }
  const letters = wordInitials(trimmed.toUpperCase());
  let out = "";
  if (letters.length > 0) {
    out = letters[0];
    if (isAsciiAlnum(letters[0])) {
      const lastAscii = [...letters.slice(1)].reverse().find(isAsciiAlnum);
      if (lastAscii) out += lastAscii;
    } else if (letters.length > 1) {
      out += letters[letters.length - 1];
    }
  }
  return out || "?";
}

/** GPUI 列表行主标题：label 非空取 label，否则取邮箱（display_label）。 */
export function accountDisplayLabel(account: Pick<AccountConfigDto, "label" | "email">): string {
  const label = account.label.trim();
  return label || account.email;
}

/**
 * 按认证方式、IMAP 主机/端口与邮箱域名推断品牌图标 ID（brand_for_account_fields）。
 * 未命中返回 null（列表渲染为通用 IMAP 徽章）。
 */
export function brandForAccountFields(
  authBackend: AuthBackend,
  imapHost: string,
  imapPort: number,
  email: string
): string | null {
  if (authBackend === "microsoftGraphOAuth2") {
    return "outlook";
  }
  const host = imapHost.trim().toLowerCase();
  const trimmedEmail = email.trim();
  const at = trimmedEmail.lastIndexOf("@");
  const domain = at > 0 ? trimmedEmail.slice(at + 1).toLowerCase() : "";

  if (host.includes("exmail.qq.com")) {
    return "exmail";
  }
  if (
    host.includes("imap.qq.com") ||
    domain === "qq.com" ||
    domain === "foxmail.com" ||
    domain === "vip.qq.com"
  ) {
    return "qq";
  }
  if (host.includes("163.com") || domain === "163.com" || domain === "vip.163.com") {
    return "netease163";
  }
  if (
    host.includes("126.com") ||
    domain === "126.com" ||
    domain === "yeah.net" ||
    domain === "vip.126.com"
  ) {
    return "netease126";
  }
  if (host.includes("qiye.aliyun.com") || host.includes("mxhichina.com")) {
    return "aliyun_qiye";
  }
  if (
    host.includes("outlook.") ||
    host.includes("office365.") ||
    host.includes("hotmail.") ||
    domain === "outlook.com" ||
    domain === "hotmail.com" ||
    domain === "live.com" ||
    domain === "msn.com" ||
    domain.startsWith("outlook.") ||
    domain.startsWith("hotmail.") ||
    domain.startsWith("live.")
  ) {
    return "outlook";
  }
  if (
    host.includes("gmail.com") ||
    host.includes("googlemail.com") ||
    domain === "gmail.com" ||
    domain === "googlemail.com"
  ) {
    return "gmail";
  }
  if (
    host.includes("mail.me.com") ||
    domain === "icloud.com" ||
    domain === "me.com" ||
    domain === "mac.com"
  ) {
    return "icloud";
  }
  if (host.includes("yahoo.") || domain.startsWith("yahoo.")) {
    return "yahoo";
  }
  if (
    (host === "127.0.0.1" && imapPort === 1143) ||
    host.includes("proton") ||
    domain.startsWith("proton.") ||
    domain === "protonmail.com" ||
    domain === "pm.me"
  ) {
    return "proton";
  }
  if (host.includes("fastmail.") || domain.startsWith("fastmail.")) {
    return "fastmail";
  }
  if (host.includes("aol.") || domain === "aol.com") {
    return "aol";
  }
  if (host.includes("zoho.") || domain.startsWith("zoho.")) {
    return "zoho";
  }
  if (host.includes("gmx.") || domain.startsWith("gmx.")) {
    return "gmx";
  }
  if (host.includes("yandex.") || domain.startsWith("yandex.") || domain === "ya.ru") {
    return "yandex";
  }
  if (host.includes("imap.mail.com") || domain === "mail.com") {
    return "mailcom";
  }
  if (host.includes("stalwart") || domain.includes("stalwart")) {
    return "stalwart";
  }
  return null;
}

/** 按已有账号配置推断品牌图标 ID（brand_for_account）。 */
export function brandForAccount(account: AccountConfigDto): string | null {
  return brandForAccountFields(
    account.authBackend,
    account.imapHost,
    account.imapPort,
    account.email
  );
}

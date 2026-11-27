/**
 * 设置窗账号组五页（标签/过滤器/智能文件夹/发件人/OpenPGP 密钥）的纯逻辑，
 * 移植自 GPUI extra_pages.rs 与 mail-core config.rs，可在 Node 测试中直接验证。
 *
 * 数据域不在现有 Tauri IPC 契约内，页面以会话内状态驱动（见各页注释与汇报）。
 */

// ---------- 共享类型 ----------

export interface TagItem {
  id: number;
  name: string;
  color: string;
  shortcut: number | null;
}

export type FilterField = "from" | "to" | "subject" | "body";
export type FilterMatch = "contains" | "equals" | "startsWith" | "endsWith";

export interface FilterRuleItem {
  id: number;
  name: string;
  accountEmail: string;
  enabled: boolean;
  field: FilterField;
  matcher: FilterMatch;
  value: string;
  moveToFolder: string;
  archive: boolean;
  star: boolean;
  markRead: boolean;
  applyTag: string;
  forwardTo: string;
}

export interface SmartFolderItem {
  id: number;
  name: string;
  icon: string;
  query: string;
  accountEmail: string;
}

export interface PgpKeyItem {
  id: number;
  name: string;
  email: string;
  fingerprint: string;
  hasSecret: boolean;
  trusted: boolean;
  signByDefault: boolean;
  createdAt: string;
}

// ---------- 选项常量（对照 extra_pages.rs#L51..L71） ----------

export const TAG_SHORTCUT_OPTIONS: ReadonlyArray<{ label: string; value: string }> = [
  { label: "无快捷键", value: "" },
  ...Array.from({ length: 9 }, (_, i) => ({ label: String(i + 1), value: String(i + 1) })),
];

export const FILTER_FIELD_OPTIONS: ReadonlyArray<{ label: string; value: FilterField }> = [
  { label: "发件人", value: "from" },
  { label: "收件人", value: "to" },
  { label: "主题", value: "subject" },
  { label: "正文", value: "body" },
];

export const FILTER_MATCH_OPTIONS: ReadonlyArray<{ label: string; value: FilterMatch }> = [
  { label: "包含", value: "contains" },
  { label: "完全等于", value: "equals" },
  { label: "开头为", value: "startsWith" },
  { label: "结尾为", value: "endsWith" },
];

export const SMART_ICON_OPTIONS: ReadonlyArray<{ label: string; value: string }> = [
  { label: "智能搜索", value: "folder-search" },
  { label: "星标重要", value: "star" },
  { label: "收件箱", value: "inbox" },
  { label: "标签", value: "tag" },
  { label: "筛选", value: "filter" },
];

export function filterFieldLabel(field: FilterField): string {
  return FILTER_FIELD_OPTIONS.find((o) => o.value === field)?.label ?? "发件人";
}

export function filterMatchLabel(matcher: FilterMatch): string {
  return FILTER_MATCH_OPTIONS.find((o) => o.value === matcher)?.label ?? "包含";
}

// ---------- 标签（对照 global_db.rs#upsert_tag） ----------

const DEFAULT_TAG_COLOR = "#3584e4";

/** 颜色规整：合法 #rrggbb 转小写，否则回落强调蓝。 */
export function normalizeTagColor(color: string): string {
  const trimmed = color.trim();
  if (trimmed.startsWith("#") && trimmed.length === 7) {
    return trimmed.toLowerCase();
  }
  return DEFAULT_TAG_COLOR;
}

/** 快捷键钳制 1..9，其余回落无快捷键。 */
export function normalizeTagShortcut(shortcut: number | null): number | null {
  if (shortcut !== null && shortcut >= 1 && shortcut <= 9) {
    return shortcut;
  }
  return null;
}

/**
 * 保存标签：名称必填；同一快捷键从其他标签上移除（对照 upsert_tag 的 UPDATE ... shortcut = NULL）。
 * 返回更新后的列表与错误信息。
 */
export function saveTag(
  tags: readonly TagItem[],
  input: { id: number | null; name: string; color: string; shortcut: number | null }
): { tags: TagItem[]; error: string | null } {
  const cleanName = input.name.trim();
  if (!cleanName) {
    return { tags: [...tags], error: "标签名称不能为空" };
  }
  const color = normalizeTagColor(input.color);
  const shortcut = normalizeTagShortcut(input.shortcut);
  // 快捷键唯一：先从其他标签移除同名快捷键
  let next = tags.map((t) =>
    t.shortcut !== null && t.shortcut === shortcut && t.id !== input.id
      ? { ...t, shortcut: null }
      : t
  );
  if (input.id === null) {
    const nextId = next.reduce((max, t) => Math.max(max, t.id), 0) + 1;
    next = [...next, { id: nextId, name: cleanName, color, shortcut }];
  } else {
    next = next.map((t) =>
      t.id === input.id ? { ...t, name: cleanName, color, shortcut } : t
    );
  }
  return { tags: next, error: null };
}

/** 标签行的快捷键说明（accounts.rs/extra_pages.rs：快捷键 N / 无快捷键）。 */
export function tagShortcutLabel(shortcut: number | null): string {
  return shortcut !== null ? `快捷键 ${shortcut}` : "无快捷键";
}

// ---------- 过滤器（对照 extra_pages.rs#render_filters） ----------

/** 规则行条件摘要：`{字段} {匹配} “{值}” → {动作列表或 无动作}`。 */
export function buildFilterRuleSummary(rule: FilterRuleItem): string {
  const actions: string[] = [];
  if (rule.moveToFolder.trim()) {
    actions.push(`移动到 ${rule.moveToFolder.trim()}`);
  }
  if (rule.archive) actions.push("归档");
  if (rule.star) actions.push("标星");
  if (rule.markRead) actions.push("标为已读");
  if (rule.applyTag.trim()) actions.push(`打标签 ${rule.applyTag.trim()}`);
  if (rule.forwardTo.trim()) actions.push(`转发 ${rule.forwardTo.trim()}`);
  return `${filterFieldLabel(rule.field)} ${filterMatchLabel(rule.matcher)} “${rule.value}” → ${
    actions.length > 0 ? actions.join("、") : "无动作"
  }`;
}

// ---------- 智能文件夹 ----------

/** 列表行说明：`{账号范围} · 查询: {查询}`。 */
export function smartFolderSubtitle(accountEmail: string, query: string): string {
  const scope = accountEmail.trim() || "全部账号";
  return `${scope} · 查询: ${query}`;
}

// ---------- 发件人黑名单（对照 mail-core config.rs#block_sender / #unblock_sender） ----------

/**
 * 规整黑名单条目：trim + 小写；`Name <addr>` 取尖括号内地址；
 * 完整邮箱需 @ 后非空；裸域名不允许包含路径分隔符。非法输入返回 null。
 */
export function normalizeBlockedSender(entry: string): string | null {
  const trimmed = entry.trim().toLowerCase();
  if (!trimmed) return null;
  let normalized = trimmed;
  const open = trimmed.lastIndexOf("<");
  const close = trimmed.lastIndexOf(">");
  if (open !== -1 && close !== -1 && open < close) {
    normalized = trimmed.slice(open + 1, close).trim();
  }
  if (!normalized) return null;
  if (normalized.includes("@")) {
    const domain = normalized.slice(normalized.lastIndexOf("@") + 1);
    if (!domain) return null;
  } else if (normalized.includes("/") || normalized.includes("\\")) {
    return null;
  }
  return normalized;
}

/** 加入黑名单（大小写不敏感去重），返回新列表；非法条目原样返回列表。 */
export function addToBlocklist(
  blocked: readonly string[],
  entry: string
): { blocked: string[]; added: string | null } {
  const normalized = normalizeBlockedSender(entry);
  if (normalized === null) {
    return { blocked: [...blocked], added: null };
  }
  if (blocked.some((existing) => existing.toLowerCase() === normalized)) {
    return { blocked: [...blocked], added: normalized };
  }
  return { blocked: [...blocked, normalized].sort(), added: normalized };
}

/** 移除黑名单条目（大小写不敏感）。 */
export function removeFromBlocklist(blocked: readonly string[], entry: string): string[] {
  const normalized = entry.trim().toLowerCase();
  return blocked.filter((existing) => existing.toLowerCase() !== normalized);
}

/** 实时筛选：按包含关系（大小写不敏感）过滤黑名单。 */
export function filterBlocklist(blocked: readonly string[], filter: string): string[] {
  const needle = filter.trim().toLowerCase();
  if (!needle) return [...blocked].sort();
  return blocked
    .filter((entry) => entry.toLowerCase().includes(needle))
    .sort();
}

// ---------- OpenPGP 密钥登记 ----------

/** 空指纹自动生成 `LOCAL-{时间戳低 32 位十六进制大写}`（对照 extra_pages.rs#L383）。 */
export function generateLocalFingerprint(nowMs: number): string {
  const low32 = BigInt(nowMs) & 0xffffffffn;
  return `LOCAL-${low32.toString(16).toUpperCase().padStart(8, "0")}`;
}

/** 登记密钥：邮箱必填且需包含 @；指纹留空自动生成；追加到列表。 */
export function registerPgpKey(
  keys: readonly PgpKeyItem[],
  input: { name: string; email: string; fingerprint: string; nowMs?: number }
): { keys: PgpKeyItem[]; error: string | null } {
  const email = input.email.trim();
  if (!email || !email.includes("@")) {
    return { keys: [...keys], error: "请输入有效的绑定邮箱地址" };
  }
  const name = input.name.trim();
  const nowMs = input.nowMs ?? Date.now();
  const fingerprint = input.fingerprint.trim() || generateLocalFingerprint(nowMs);
  const nextId = keys.reduce((max, k) => Math.max(max, k.id), 0) + 1;
  const createdAt = new Date(nowMs).toISOString().slice(0, 10);
  const key: PgpKeyItem = {
    id: nextId,
    name,
    email,
    fingerprint,
    hasSecret: true,
    trusted: true,
    signByDefault: false,
    createdAt,
  };
  return { keys: [...keys, key], error: null };
}

/** 密钥行标题：有名称用 `名称 <邮箱>`，否则只显示邮箱。 */
export function pgpKeyTitle(key: Pick<PgpKeyItem, "name" | "email">): string {
  const trimmedName = key.name.trim();
  return trimmedName ? `${trimmedName} <${key.email.trim()}>` : key.email.trim();
}

/** 密钥行说明：`指纹: {fp} · 含私钥/仅公钥`。 */
export function pgpKeySubtitle(key: Pick<PgpKeyItem, "fingerprint" | "hasSecret">): string {
  return `指纹: ${key.fingerprint} · ${key.hasSecret ? "含私钥" : "仅公钥"}`;
}

// ---------- 云存储（对照 extra_pages.rs#render_cloud_storage） ----------

export interface CloudStorageItem {
  id: number;
  provider: string;
  name: string;
  serverUrl: string;
  username: string;
  bucketOrPath: string;
  enabled: boolean;
}

export const CLOUD_PROVIDERS: ReadonlyArray<{ label: string; value: string }> = [
  { label: "Nextcloud", value: "Nextcloud" },
  { label: "WebDAV", value: "WebDAV" },
  { label: "Amazon S3 / 兼容对象存储", value: "S3" },
  { label: "Dropbox", value: "Dropbox" },
];

export const CLOUD_THRESHOLD_OPTIONS: ReadonlyArray<{ label: string; value: string }> = [
  { label: "关闭自动上传", value: "0" },
  { label: "超过 10 MB", value: "10" },
  { label: "超过 25 MB（推荐）", value: "25" },
  { label: "超过 50 MB", value: "50" },
  { label: "超过 100 MB", value: "100" },
];

/** 阈值标签：未知值回落「超过 25 MB（推荐）」（对照 GPUI unwrap_or）。 */
export function cloudThresholdLabel(mb: number): string {
  return (
    CLOUD_THRESHOLD_OPTIONS.find((o) => o.value === String(mb))?.label ?? "超过 25 MB（推荐）"
  );
}

/** 云存储 / 通讯录源行说明：`{服务商} · {地址} ({用户名})`。 */
export function serviceEntrySubtitle(
  provider: string,
  serverUrl: string,
  username: string
): string {
  return `${provider} · ${serverUrl} (${username})`;
}

// ---------- 通讯录源（对照 extra_pages.rs#render_contacts_sources） ----------

export interface ContactSourceItem {
  id: number;
  provider: string;
  name: string;
  serverUrl: string;
  username: string;
  enabled: boolean;
}

export const CONTACT_PROVIDERS: ReadonlyArray<{ label: string; value: string }> = [
  { label: "通用 CardDAV", value: "CardDAV" },
  { label: "iCloud 通讯录", value: "iCloud" },
  { label: "Nextcloud 通讯录", value: "Nextcloud" },
  { label: "Fastmail 通讯录", value: "Fastmail" },
];

// ---------- 翻译（对照 extra_pages.rs#render_translation） ----------

export const TRANSLATION_ENGINES: ReadonlyArray<{ label: string; value: string }> = [
  { label: "关闭", value: "off" },
  { label: "LibreTranslate（自建/开源）", value: "libretranslate" },
  { label: "DeepL API", value: "deepl" },
  { label: "Google Translate", value: "google" },
  { label: "本地 LLM（复用本机推理服务）", value: "local_llm" },
];

export const TRANSLATION_TARGETS: ReadonlyArray<{ label: string; value: string }> = [
  { label: "跟随应用界面语言", value: "system" },
  { label: "简体中文 (zh-CN)", value: "zh-CN" },
  { label: "English (en)", value: "en" },
  { label: "日本語 (ja)", value: "ja" },
  { label: "한국어 (ko)", value: "ko" },
  { label: "Français (fr)", value: "fr" },
  { label: "Deutsch (de)", value: "de" },
  { label: "Español (es)", value: "es" },
];

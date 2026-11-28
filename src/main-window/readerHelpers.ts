/**
 * KiteMail 阅读区与会话卡片纯辅助逻辑（无 DOM 依赖，支持 Node 单测）。
 */

export interface FindTextSegment {
  text: string;
  hit: boolean;
  hitIndex: number;
}

export type ReaderOverflowActionId =
  | "tags"
  | "toggleSeen"
  | "spam"
  | "move"
  | "find"
  | "print";

const AVATAR_PALETTE = [
  "#3584e4",
  "#26a269",
  "#c061cb",
  "#e66100",
  "#9141ac",
  "#1c71d8",
  "#2ec27e",
  "#a51d2d",
];

/** 判断文件夹路径是否属于垃圾邮件箱 */
export function isSpamFolderPath(folderPath?: string | null): boolean {
  const lower = (folderPath ?? "").trim().toLowerCase();
  return lower.includes("junk") || lower.includes("spam");
}

/** 从 `"Name <email>"` 或裸地址拆分发件人名称与邮箱 */
export function splitSenderDisplay(raw: string): { name: string; email: string } {
  const trimmed = raw.trim();
  if (!trimmed) return { name: "", email: "" };
  const lt = trimmed.indexOf("<");
  const gt = trimmed.lastIndexOf(">");
  if (lt >= 0 && gt > lt) {
    const email = trimmed.slice(lt + 1, gt).trim();
    const name = trimmed.slice(0, lt).trim().replace(/^["']|["']$/g, "");
    return { name: name || email, email };
  }
  if (trimmed.includes("@") && !trimmed.includes(" ")) {
    return { name: trimmed, email: trimmed };
  }
  return { name: trimmed, email: "" };
}

/** 生成单字符头像首字母 */
export function formatAvatarInitial(name: string, email: string): string {
  const source = (name || email || "?").trim();
  const ch = Array.from(source)[0];
  return ch ? ch.toUpperCase() : "?";
}

/** 基于发件人字符串生成稳定的头像底色 */
export function avatarToneColor(seed: string): string {
  let hash = 0;
  for (let i = 0; i < seed.length; i++) {
    hash = (hash * 31 + seed.charCodeAt(i)) >>> 0;
  }
  return AVATAR_PALETTE[hash % AVATAR_PALETTE.length];
}

/** 统计收件人与抄送人总数 */
export function countRecipientsDisplay(recipients: string, cc?: string): number {
  const splitList = (raw?: string) =>
    (raw ?? "")
      .split(/[,;]+/)
      .map((item) => item.trim())
      .filter((item) => item.length > 0);
  return splitList(recipients).length + splitList(cc).length;
}

/** 将纯文本按查询词切分为安全插值片段，严禁使用 HTML 拼接 */
export function buildFallbackFindSegments(
  text: string,
  query: string,
  startHitIndex = 0
): { segments: FindTextSegment[]; matchCount: number } {
  const q = query.trim().toLowerCase();
  if (!q || !text) {
    return {
      segments: text ? [{ text, hit: false, hitIndex: -1 }] : [],
      matchCount: 0,
    };
  }

  const lower = text.toLowerCase();
  const segments: FindTextSegment[] = [];
  let cursor = 0;
  let matchCount = 0;

  while (cursor < text.length) {
    const idx = lower.indexOf(q, cursor);
    if (idx < 0) {
      segments.push({ text: text.slice(cursor), hit: false, hitIndex: -1 });
      break;
    }
    if (idx > cursor) {
      segments.push({ text: text.slice(cursor, idx), hit: false, hitIndex: -1 });
    }
    segments.push({
      text: text.slice(idx, idx + q.length),
      hit: true,
      hitIndex: startHitIndex + matchCount,
    });
    matchCount += 1;
    cursor = idx + q.length;
  }

  return { segments, matchCount };
}

/** 统计多段正文中的查找命中总数 */
export function countFallbackFindMatches(texts: readonly string[], query: string): number {
  let total = 0;
  for (const t of texts) {
    total += buildFallbackFindSegments(t, query, total).matchCount;
  }
  return total;
}

/** 计算循环跳转后的 0-based 查找命中索引（无命中返回 -1） */
export function nextFindIndex(currentZeroBased: number, total: number, delta: number): number {
  if (total <= 0) return -1;
  const step = delta < 0 ? -1 : 1;
  const base = currentZeroBased >= 0 && currentZeroBased < total ? currentZeroBased : 0;
  return (base + step + total) % total;
}

/**
 * 按阅读区工具栏可用宽度决定哪些次级右侧图标折叠进溢出菜单 `⋯`。
 * 始终保留左侧回复/全部回复/转发/星标/归档/删除与最右侧关闭按钮，按窄度逐级收拢次级入口。
 */
export function computeReaderOverflowActions(toolbarWidthPx: number): ReaderOverflowActionId[] {
  if (!Number.isFinite(toolbarWidthPx) || toolbarWidthPx <= 0 || toolbarWidthPx >= 560) {
    return [];
  }
  if (toolbarWidthPx >= 490) {
    return ["print", "move"];
  }
  if (toolbarWidthPx >= 430) {
    return ["print", "move", "spam", "tags"];
  }
  return ["tags", "toggleSeen", "spam", "move", "find", "print"];
}

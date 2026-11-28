import type { FolderSummaryDto, MessageFilterDto, TagItemDto, UnifiedViewKind } from "../ipc/types";

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

export function fnv1a(text: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash >>> 0;
}

export function parseHexColor(hex?: string | null): string | null {
  if (!hex) return null;
  const trimmed = hex.trim();
  if (/^#[0-9a-fA-F]{6}$/.test(trimmed)) {
    return trimmed.toLowerCase();
  }
  return null;
}

export function accountAccentHex(email: string, customColor?: string | null): string {
  const parsed = parseHexColor(customColor);
  if (parsed) return parsed;
  return AVATAR_COLORS[fnv1a(email.trim().toLowerCase()) % AVATAR_COLORS.length];
}

export function senderDisplayName(rawSender: string): string {
  const raw = rawSender.trim();
  const idx = raw.indexOf("<");
  if (idx > 0) {
    return raw.slice(0, idx).trim().replace(/^"|"$/g, "") || raw;
  }
  return raw || "(未知发件人)";
}

export function senderEmailAddress(rawSender: string): string {
  const m = rawSender.match(/<([^>]+)>/);
  const candidate = (m ? m[1] : rawSender).trim();
  return candidate.includes("@") ? candidate.toLowerCase() : "";
}

function isCjkChar(ch: string): boolean {
  const cp = ch.codePointAt(0) ?? 0;
  return (
    (cp >= 0x4e00 && cp <= 0x9fff) ||
    (cp >= 0x3400 && cp <= 0x4dbf) ||
    (cp >= 0xf900 && cp <= 0xfaff) ||
    (cp >= 0x30000 && cp <= 0x3134f)
  );
}

function stripBracketedText(text: string): string {
  let out = "";
  let depth = 0;
  for (const ch of text) {
    if (ch === "(" || ch === "<" || ch === "[") {
      depth += 1;
    } else if (ch === ")" || ch === ">" || ch === "]") {
      if (depth > 0) depth -= 1;
    } else if (depth === 0) {
      out += ch;
    }
  }
  return /[a-zA-Z0-9\p{L}\p{N}]/u.test(out) ? out : text;
}

export function senderInitials(name: string): string {
  const trimmed = name.trim();
  const firstAlnum = trimmed.match(/[\p{L}\p{N}]/u)?.[0];
  if (firstAlnum && isCjkChar(firstAlnum)) {
    return firstAlnum;
  }
  const outside = stripBracketedText(trimmed.toUpperCase());
  const letters = outside
    .split(/\s+/)
    .map((w) => w.match(/[\p{L}\p{N}]/u)?.[0] ?? "")
    .filter(Boolean);
  if (letters.length === 0) return "?";
  const first = letters[0];
  if (/^[A-Z0-9]$/.test(first)) {
    for (let i = letters.length - 1; i >= 1; i--) {
      if (/^[A-Z0-9]$/.test(letters[i])) {
        return first + letters[i];
      }
    }
    return first;
  }
  return letters.length > 1 ? first + letters[letters.length - 1] : first;
}

export function avatarColorForKey(key: string): string {
  return AVATAR_COLORS[fnv1a(key.trim().toLowerCase()) % AVATAR_COLORS.length];
}

export interface TagColorSpec {
  pillBg: string;
  pillText: string;
  rowBgLight?: string;
  rowBgDark?: string;
}

const KNOWN_TAG_COLORS: Readonly<Record<string, TagColorSpec>> = {
  work: {
    pillBg: "#3584e4",
    pillText: "#ffffff",
    rowBgLight: "#D2E1F5",
    rowBgDark: "#213046",
  },
  工作: {
    pillBg: "#3584e4",
    pillText: "#ffffff",
    rowBgLight: "#D2E1F5",
    rowBgDark: "#213046",
  },
  personal: {
    pillBg: "#26a269",
    pillText: "#ffffff",
    rowBgLight: "#D5F0E5",
    rowBgDark: "#243F36",
  },
  个人: {
    pillBg: "#26a269",
    pillText: "#ffffff",
    rowBgLight: "#D5F0E5",
    rowBgDark: "#243F36",
  },
  "to do": {
    pillBg: "#e66100",
    pillText: "#ffffff",
  },
  待办: {
    pillBg: "#e66100",
    pillText: "#ffffff",
  },
  important: {
    pillBg: "#e01b24",
    pillText: "#ffffff",
  },
  重要: {
    pillBg: "#e01b24",
    pillText: "#ffffff",
  },
  project: {
    pillBg: "#9141ac",
    pillText: "#ffffff",
  },
  项目: {
    pillBg: "#9141ac",
    pillText: "#ffffff",
  },
};

export function resolveMessageRowTint(
  tags: readonly string[],
  selected: boolean,
  isDark: boolean
): string | null {
  if (selected) return null;
  for (const tag of tags) {
    const spec = KNOWN_TAG_COLORS[tag.trim().toLowerCase()];
    if (!spec) continue;
    const tint = isDark ? spec.rowBgDark : spec.rowBgLight;
    if (tint) return tint;
  }
  return null;
}

export function resolveTagPillStyle(
  tag: string,
  selected: boolean,
  tagsMeta?: readonly TagItemDto[]
): { backgroundColor: string; color: string } {
  if (selected) {
    return {
      backgroundColor: "rgba(255, 255, 255, 0.22)",
      color: "#ffffff",
    };
  }
  const key = tag.trim().toLowerCase();
  const known = KNOWN_TAG_COLORS[key];
  if (known) {
    return {
      backgroundColor: known.pillBg,
      color: known.pillText,
    };
  }
  const meta = tagsMeta?.find((t) => t.name.trim().toLowerCase() === key);
  const hex = parseHexColor(meta?.color) ?? "#3584e4";
  return {
    backgroundColor: hex,
    color: "#ffffff",
  };
}

export interface SidebarFilterRule {
  id: string;
  label: string;
  query: string;
  filter: MessageFilterDto;
}

export const SIDEBAR_FILTER_RULES: readonly SidebarFilterRule[] = [
  { id: "unread", label: "未读邮件", query: "is:unread", filter: { unread: true } },
  { id: "attachment", label: "含附件", query: "has:attachment", filter: { hasAttachment: true } },
  { id: "starred", label: "已加星标", query: "is:starred", filter: { starred: true } },
  { id: "today", label: "今天", query: "", filter: { today: true } },
];

export type FolderRoleKind = "inbox" | "drafts" | "sent" | "archive" | "junk" | "trash";

const ROLE_ORDER: readonly FolderRoleKind[] = [
  "inbox",
  "sent",
  "drafts",
  "archive",
  "junk",
  "trash",
];

export function classifyFolderRole(path: string, name: string): FolderRoleKind | null {
  const p = path.trim().toUpperCase();
  const n = name.trim();
  const nu = n.toUpperCase();
  if (p === "INBOX" || nu === "INBOX" || n.includes("收件箱")) return "inbox";
  if (p.includes("DRAFT") || nu === "DRAFTS" || nu === "DRAFT" || n.includes("草稿")) return "drafts";
  if (p.includes("SENT") || nu === "SENT ITEMS" || nu === "SENT" || n.includes("已发送")) return "sent";
  if (p.includes("ARCHIVE") || nu === "ARCHIVE" || n.includes("归档") || n.includes("存档")) return "archive";
  if (
    p.includes("TRASH") ||
    p.includes("DELETED") ||
    nu === "DELETED ITEMS" ||
    nu === "TRASH" ||
    n.includes("已删除") ||
    n.includes("废件")
  ) {
    return "trash";
  }
  if (
    p.includes("JUNK") ||
    p.includes("SPAM") ||
    nu === "JUNK EMAIL" ||
    nu === "JUNK" ||
    nu === "SPAM" ||
    n.includes("垃圾")
  ) {
    return "junk";
  }
  return null;
}

export function findUnifiedSubrowFolder(
  kind: UnifiedViewKind,
  accountId: number,
  folders: readonly FolderSummaryDto[]
): FolderSummaryDto | null {
  const targetRole: FolderRoleKind | null =
    kind === "unifiedInbox"
      ? "inbox"
      : kind === "sent"
        ? "sent"
        : kind === "drafts"
          ? "drafts"
          : kind === "archive"
            ? "archive"
            : kind === "junk"
              ? "junk"
              : kind === "trash"
                ? "trash"
                : null;
  if (!targetRole) return null;
  return (
    folders.find(
      (f) => f.accountId === accountId && classifyFolderRole(f.path, f.name) === targetRole
    ) ?? null
  );
}

export interface FolderTreeNode {
  folder: FolderSummaryDto;
  role: FolderRoleKind | null;
  depth: number;
  hasChildren: boolean;
  expanded: boolean;
}

function detectDelimiter(paths: readonly string[]): "/" | "." | null {
  if (paths.some((p) => p.includes("/"))) return "/";
  if (paths.some((p) => p.includes("."))) return ".";
  return null;
}

function pathIsUnder(child: string, parent: string, delim: string): boolean {
  return (
    child.length > parent.length &&
    child[parent.length] === delim &&
    child.slice(0, parent.length).toLowerCase() === parent.toLowerCase()
  );
}

export function buildAccountFolderTree(
  accountFolders: readonly FolderSummaryDto[],
  collapsedPaths: ReadonlySet<string>
): FolderTreeNode[] {
  const roleFolders: Array<{ folder: FolderSummaryDto; role: FolderRoleKind }> = [];
  const customFolders: FolderSummaryDto[] = [];

  for (const f of accountFolders) {
    const role = classifyFolderRole(f.path, f.name);
    if (role) {
      roleFolders.push({ folder: f, role });
    } else {
      customFolders.push(f);
    }
  }

  roleFolders.sort(
    (a, b) => ROLE_ORDER.indexOf(a.role) - ROLE_ORDER.indexOf(b.role)
  );

  const nodes: FolderTreeNode[] = roleFolders.map(({ folder, role }) => ({
    folder,
    role,
    depth: 0,
    hasChildren: false,
    expanded: true,
  }));

  if (customFolders.length === 0) return nodes;

  const customPaths = customFolders.map((f) => f.path);
  const delim = detectDelimiter(customPaths);

  const leafName = (p: string): string => {
    if (!delim) return p;
    const idx = p.lastIndexOf(delim);
    return idx >= 0 ? p.slice(idx + 1) : p;
  };

  const sortedCustom = [...customFolders].sort((a, b) => {
    const la = leafName(a.path).toLowerCase();
    const lb = leafName(b.path).toLowerCase();
    return la.localeCompare(lb) || a.path.localeCompare(b.path);
  });

  const emitted: FolderSummaryDto[] = [];
  const remaining = [...sortedCustom];
  while (remaining.length > 0) {
    const idx = remaining.findIndex((candidate) => {
      if (!delim) return true;
      let nearestParent: string | null = null;
      for (const other of customPaths) {
        if (other !== candidate.path && pathIsUnder(candidate.path, other, delim)) {
          if (!nearestParent || other.length > nearestParent.length) {
            nearestParent = other;
          }
        }
      }
      return !nearestParent || emitted.some((e) => e.path === nearestParent);
    });
    if (idx >= 0) {
      emitted.push(remaining.splice(idx, 1)[0]);
    } else {
      emitted.push(...remaining.splice(0));
    }
  }

  for (const folder of emitted) {
    const isHidden =
      delim !== null &&
      customPaths.some(
        (anc) =>
          anc !== folder.path &&
          pathIsUnder(folder.path, anc, delim) &&
          collapsedPaths.has(anc)
      );
    if (isHidden) continue;

    const depth =
      delim === null
        ? 0
        : customPaths.filter(
            (anc) => anc !== folder.path && pathIsUnder(folder.path, anc, delim)
          ).length;
    const hasChildren =
      delim !== null &&
      customPaths.some((child) => child !== folder.path && pathIsUnder(child, folder.path, delim));

    nodes.push({
      folder,
      role: null,
      depth,
      hasChildren,
      expanded: !collapsedPaths.has(folder.path),
    });
  }

  return nodes;
}

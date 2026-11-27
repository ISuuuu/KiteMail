import { computed, ref } from "vue";
import type {
  CommandPaletteItem,
  EventTargetLike,
  KeyboardEventLike,
  ShortcutContext,
  ShortcutHelpGroup,
  ShortcutMatchResult,
} from "./types";

export const SINGLE_KEY_COMMAND_MAP: Readonly<Record<string, string>> = {
  c: "compose.new",
  r: "message.reply",
  a: "message.replyAll",
  f: "message.forward",
  e: "message.archive",
  s: "message.toggleStar",
  u: "message.toggleRead",
  j: "message.next",
  k: "message.prev",
  "?": "help.shortcuts",
  "#": "message.delete",
  "/": "search.focus",
};

export const SHORTCUT_HELP_GROUPS: readonly ShortcutHelpGroup[] = [
  {
    title: "全局与窗口导航",
    entries: [
      { keys: ["Ctrl", "K"], description: "打开 / 关闭命令面板" },
      { keys: ["Ctrl", "F"], description: "聚焦邮件搜索框" },
      { keys: ["Ctrl", "N"], description: "撰写新邮件" },
      { keys: ["F5"], description: "刷新当前邮箱视图" },
      { keys: ["Ctrl", "Alt", "1"], description: "右侧栏阅读布局" },
      { keys: ["Ctrl", "Alt", "2"], description: "下方栏阅读布局" },
      { keys: ["Ctrl", "Alt", "3"], description: "隐藏阅读窗格" },
      { keys: ["?"], description: "打开快捷键帮助", requiresSingleKey: true },
    ],
  },
  {
    title: "单键写信与邮件操作",
    entries: [
      { keys: ["c"], description: "写新邮件", requiresSingleKey: true },
      { keys: ["r"], description: "回复发件人", requiresSingleKey: true },
      { keys: ["a"], description: "回复全部", requiresSingleKey: true },
      { keys: ["f"], description: "转发邮件", requiresSingleKey: true },
      { keys: ["e"], description: "归档当前邮件", requiresSingleKey: true },
      { keys: ["s"], description: "切换星标状态", requiresSingleKey: true },
      { keys: ["u"], description: "切换已读 / 未读", requiresSingleKey: true },
      { keys: ["#"], description: "删除当前邮件", requiresSingleKey: true },
    ],
  },
  {
    title: "列表浏览与搜索",
    entries: [
      { keys: ["j"], description: "选中下一封邮件", requiresSingleKey: true },
      { keys: ["k"], description: "选中上一封邮件", requiresSingleKey: true },
      { keys: ["/"], description: "聚焦搜索框", requiresSingleKey: true },
      { keys: ["↓"], description: "选中下一封邮件" },
      { keys: ["↑"], description: "选中上一封邮件" },
      { keys: ["Delete"], description: "删除当前邮件" },
    ],
  },
];

// ---------- 纯函数工具（可在 Node 无 DOM 环境直接单测） ----------

function isSubsequence(needle: string, haystack: string): boolean {
  if (!needle) return true;
  let nIdx = 0;
  for (let hIdx = 0; hIdx < haystack.length && nIdx < needle.length; hIdx++) {
    if (haystack[hIdx] === needle[nIdx]) {
      nIdx += 1;
    }
  }
  return nIdx === needle.length;
}

function scoreField(token: string, field: string): number {
  if (!field) return 0;
  const lower = field.toLowerCase();
  if (lower === token) return 100;
  if (lower.startsWith(token)) return 80;
  if (lower.includes(token)) return 60;
  if (token.length >= 2 && isSubsequence(token, lower)) return 30;
  return 0;
}

/**
 * 模糊过滤命令面板条目：支持多词空格分词、标题/分类/关键词/快捷键/ID 匹配与权重排序。
 */
export function filterCommandItems(
  items: readonly CommandPaletteItem[],
  query: string
): CommandPaletteItem[] {
  const trimmed = query.trim().toLowerCase();
  if (!trimmed) return [...items];

  const tokens = trimmed.split(/\s+/).filter(Boolean);
  const scored: Array<{ item: CommandPaletteItem; score: number; idx: number }> = [];

  for (let idx = 0; idx < items.length; idx++) {
    const item = items[idx];
    const fields = [
      { text: item.title, weight: 3 },
      { text: item.category, weight: 1.5 },
      { text: item.shortcut ?? "", weight: 1.5 },
      { text: item.id, weight: 1 },
      ...(item.keywords ?? []).map((kw) => ({ text: kw, weight: 2 })),
    ];

    let totalScore = 0;
    let allTokensMatched = true;

    for (const token of tokens) {
      let bestTokenScore = 0;
      for (const f of fields) {
        const s = scoreField(token, f.text) * f.weight;
        if (s > bestTokenScore) bestTokenScore = s;
      }
      if (bestTokenScore <= 0) {
        allTokensMatched = false;
        break;
      }
      totalScore += bestTokenScore;
    }

    if (allTokensMatched) {
      scored.push({ item, score: totalScore, idx });
    }
  }

  scored.sort((a, b) => b.score - a.score || a.idx - b.idx);
  return scored.map((entry) => entry.item);
}

export function isEditableTarget(
  target?: EventTargetLike | EventTarget | null
): boolean {
  if (!target) return false;
  const el = target as EventTargetLike;
  if (el.isContentEditable) return true;
  const attr = typeof el.getAttribute === "function" ? el.getAttribute("contenteditable") : null;
  if (attr !== undefined && attr !== null && attr !== "false") {
    return true;
  }
  const tag = typeof el.tagName === "string" ? el.tagName.toUpperCase() : "";
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT";
}

export function isImeComposing(eventLike: KeyboardEventLike): boolean {
  return Boolean(
    eventLike.isComposing ||
      eventLike.key === "Process" ||
      eventLike.keyCode === 229
  );
}

/**
 * 匹配 Ctrl+K 命令面板开关与单键快捷键。
 * 严格尊重 IME 组合态、输入框/可编辑节点聚焦、模态弹窗打开状态及 singleKeyShortcuts 开关。
 */
export function matchSingleKeyShortcut(
  eventLike: KeyboardEventLike,
  context: ShortcutContext
): ShortcutMatchResult {
  const noMatch: ShortcutMatchResult = {
    matched: false,
    commandId: null,
    isPaletteToggle: false,
    preventDefault: false,
  };

  if (isImeComposing(eventLike)) {
    return noMatch;
  }

  const key = eventLike.key ?? "";
  const lower = key.toLowerCase();

  // Ctrl+K / Cmd+K 命令面板切换（非 IME 组合态下始终响应）
  if (
    (eventLike.ctrlKey || eventLike.metaKey) &&
    !eventLike.altKey &&
    !eventLike.shiftKey &&
    lower === "k"
  ) {
    return {
      matched: true,
      commandId: "command-palette.toggle",
      isPaletteToggle: true,
      preventDefault: true,
    };
  }

  // 单键快捷键守卫：开关关闭、弹窗打开、输入聚焦或按下修饰键均放行原生输入
  if (!context.singleKeyShortcuts) return noMatch;
  if (context.hasOpenModal) return noMatch;
  if (context.isInputFocused || isEditableTarget(eventLike.target)) {
    return noMatch;
  }
  if (eventLike.ctrlKey || eventLike.metaKey || eventLike.altKey) {
    return noMatch;
  }

  // `?` 与 `#` 通常伴随 Shift 键按下
  if (key === "?" || (eventLike.shiftKey && key === "/")) {
    return {
      matched: true,
      commandId: SINGLE_KEY_COMMAND_MAP["?"],
      isPaletteToggle: false,
      preventDefault: true,
    };
  }
  if (key === "#" || (eventLike.shiftKey && key === "3")) {
    return {
      matched: true,
      commandId: SINGLE_KEY_COMMAND_MAP["#"],
      isPaletteToggle: false,
      preventDefault: true,
    };
  }
  if (key === "/" && !eventLike.shiftKey) {
    return {
      matched: true,
      commandId: SINGLE_KEY_COMMAND_MAP["/"],
      isPaletteToggle: false,
      preventDefault: true,
    };
  }

  if (eventLike.shiftKey) return noMatch;

  const commandId = SINGLE_KEY_COMMAND_MAP[lower];
  if (!commandId) return noMatch;

  return {
    matched: true,
    commandId,
    isPaletteToggle: false,
    preventDefault: true,
  };
}

export function useCommandPalette(getItems: () => readonly CommandPaletteItem[]) {
  const query = ref("");
  const activeIndex = ref(0);

  const filteredItems = computed(() => filterCommandItems(getItems(), query.value));

  function reset(): void {
    query.value = "";
    activeIndex.value = findFirstEnabledIndex(filteredItems.value);
  }

  function findFirstEnabledIndex(list: readonly CommandPaletteItem[]): number {
    const idx = list.findIndex((item) => !item.disabled);
    return idx >= 0 ? idx : 0;
  }

  function setQuery(next: string): void {
    query.value = next;
    activeIndex.value = findFirstEnabledIndex(filteredItems.value);
  }

  function moveSelection(delta: number): void {
    const list = filteredItems.value;
    if (list.length === 0) {
      activeIndex.value = 0;
      return;
    }
    const len = list.length;
    let cursor = activeIndex.value;
    for (let step = 0; step < len; step++) {
      cursor = (cursor + delta + len) % len;
      if (!list[cursor].disabled) {
        activeIndex.value = cursor;
        return;
      }
    }
  }

  function getActiveItem(): CommandPaletteItem | null {
    const list = filteredItems.value;
    const item = list[activeIndex.value];
    if (!item || item.disabled) return null;
    return item;
  }

  return {
    query,
    activeIndex,
    filteredItems,
    reset,
    setQuery,
    moveSelection,
    getActiveItem,
  };
}

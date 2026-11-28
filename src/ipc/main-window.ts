/**
 * KiteMail 主窗口 IPC 客户端层。
 *
 * 参数包装约定（与 src-tauri/src/commands.rs 完全一致）：
 *   main_bootstrap()                   无参
 *   list_messages({ input })           包在 input 字段
 *   open_message({ input })            包在 input 字段
 *   set_reader_layout({ input })       包在 input 字段
 *   update_main_preferences({ patch }) 包在 patch 字段
 *   refresh_mail({ input })            包在 input 字段
 *   unlock_app({ input })              包在 input 字段
 *   lock_app()                         无参
 *   set_message_seen({ input })        包在 input 字段
 *   set_message_flagged({ input })     包在 input 字段
 *   allow_remote_images({ input })     包在 input 字段
 *   load_older_messages({ input })     包在 input 字段
 *   archive_message({ input })         包在 input 字段
 *   delete_message({ input })          包在 input 字段
 *   mark_message_spam({ input })       包在 input 字段
 *   move_message({ input })            包在 input 字段
 *   undo_message_move({ input })       包在 input 字段
 *   quit_app()                         无参
 *
 * 在非 Tauri 环境（无 window.__TAURI_INTERNALS__）下使用内置 Mock 传输层，
 * 以便独立浏览器预览和单元测试。
 */

import type {
  MainBootstrapDto,
  ListMessagesInput,
  ListMessagesOutput,
  ListThreadSummariesInput,
  ListThreadSummariesOutput,
  OpenMessageInput,
  OpenMessageOutput,
  SetReaderLayoutInput,
  SetReaderLayoutOutput,
  RefreshMailInput,
  RefreshMailOutput,
  UnlockAppInput,
  UnlockAppOutput,
  LockAppOutput,
  SetMessageSeenInput,
  SetMessageSeenOutput,
  SetMessageFlaggedInput,
  SetMessageFlaggedOutput,
  AllowRemoteImagesInput,
  AllowRemoteImagesOutput,
  LoadOlderMessagesInput,
  LoadOlderMessagesOutput,
  MessageActionInput,
  MoveMessageInput,
  MessageMoveOutput,
  UndoMessageMoveInput,
  UndoMessageMoveOutput,
  MainPreferencesPatch,
  MainPreferencesDto,
  MailChangedEvent,
  MailTemplateDto,
  SyncStatusEvent,
  SyncStatusDto,
  ReaderStatusEvent,
  LockStateEvent,
  MessageSummaryDto,
  FolderSummaryDto,
  TagItemDto,
  ViewDto,
  MessageSort,
  IpcError,
} from "./types";

// ---------- 传输层检测 ----------

export function isTauri(): boolean {
  return (
    typeof window !== "undefined" &&
    !!(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
  );
}

/** 从 Tauri IpcError 对象或通用异常中提取可读错误信息 */
export function extractErrorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  if (err && typeof err === "object") {
    const obj = err as Record<string, unknown>;
    if (typeof obj.message === "string" && obj.message.length > 0) {
      return typeof obj.code === "string" ? `[${obj.code}] ${obj.message}` : obj.message;
    }
    try {
      return JSON.stringify(err);
    } catch {
      return "未知错误";
    }
  }
  return String(err ?? "未知错误");
}

/** 判断异常是否为后端返回的 LOCKED 结构化错误 */
export function isLockedError(err: unknown): boolean {
  return Boolean(
    err &&
      typeof err === "object" &&
      (err as Partial<IpcError>).code === "LOCKED"
  );
}

/** 标准化 main://sync-status 事件载荷（兼容嵌套 syncStatus 与扁平字段） */
export function normalizeSyncStatusEvent(e: SyncStatusEvent): {
  syncStatus: SyncStatusDto;
  stateVersion?: number;
} {
  if ("syncStatus" in e && e.syncStatus) {
    return {
      syncStatus: {
        syncing: Boolean(e.syncStatus.syncing),
        message: e.syncStatus.message ?? null,
        lastSyncAt: e.syncStatus.lastSyncAt ?? null,
      },
      stateVersion: e.stateVersion,
    };
  }
  const flat = e as SyncStatusDto & { stateVersion?: number };
  return {
    syncStatus: {
      syncing: Boolean(flat.syncing),
      message: flat.message ?? null,
      lastSyncAt: flat.lastSyncAt ?? null,
    },
    stateVersion: flat.stateVersion,
  };
}

let globalReaderLayoutRevision = 0;

/** 生成严格递增的阅读器布局修订号（供组件与锁屏隐藏共享） */
export function nextReaderLayoutRevision(): number {
  globalReaderLayoutRevision += 1;
  return globalReaderLayoutRevision;
}

// ---------- Tauri invoke / listen ----------

async function tauriInvoke<T>(cmd: string, args?: unknown): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args as Record<string, unknown>);
}

export type UnlistenFn = () => void;

async function tauriListen<T>(
  event: string,
  handler: (payload: T) => void
): Promise<UnlistenFn> {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<T>(event, (ev) => handler(ev.payload));
}

// ---------- Mock 数据（非 Tauri 预览与单元测试用，支持 10,000 封演示收件箱） ----------

const MOCK_NOW = Math.floor(Date.now() / 1000);
const MOCK_INBOX_TOTAL = 10_000;

const PINNED_SAMPLES: ReadonlyArray<{
  sender: string;
  subject: string;
  preview: string;
  dateDisplay: string;
  unread: boolean;
  flagged: boolean;
  hasAttachment: boolean;
  tags: string[];
}> = [
  {
    sender: "林晓明 <xiaoming@example.com>",
    subject: "KiteMail Windows 主窗口 Tauri + Vue 基线确认",
    preview: "请确认 MSVC 工具链、中文 IME 输入以及 100%/150% DPI 缩放表现。",
    dateDisplay: "14:32",
    unread: true,
    flagged: true,
    hasAttachment: false,
    tags: ["重要"],
  },
  {
    sender: "Build Bot <ci@kitemail.example>",
    subject: "[CI] x86_64-pc-windows-msvc 夜间构建完成",
    preview: "工作区 Cargo.toml 与前端 vite build 全部通过。",
    dateDisplay: "11:18",
    unread: true,
    flagged: false,
    hasAttachment: true,
    tags: ["工作"],
  },
  {
    sender: "产品设计组 <design@kitemail.example>",
    subject: "三栏布局与阅读区字号反馈（含附件清单）",
    preview: "附件包含三栏间距规范和暗色/亮色主题色板对比，请查收。",
    dateDisplay: "昨天",
    unread: false,
    flagged: true,
    hasAttachment: true,
    tags: ["项目"],
  },
  {
    sender: "陈思远 <siyuan@example.org>",
    subject: "Re: 周五邮件客户端性能压测记录",
    preview: "一万封邮件分页加载耗时稳定在毫秒级，虚拟列表滚动无明显掉帧。",
    dateDisplay: "昨天",
    unread: false,
    flagged: false,
    hasAttachment: false,
    tags: [],
  },
];

function buildMockInboxMessage(index: number): MessageSummaryDto {
  if (index < PINNED_SAMPLES.length) {
    const s = PINNED_SAMPLES[index];
    return {
      id: index + 1,
      accountId: 1,
      folderId: 1,
      folderPath: "INBOX",
      sender: s.sender,
      recipients: "demo@kitemail.example",
      subject: s.subject,
      preview: s.preview,
      dateTs: MOCK_NOW - index * 3600 * (index < 2 ? 1 : 24),
      dateDisplay: s.dateDisplay,
      unread: s.unread,
      flagged: s.flagged,
      hasAttachment: s.hasAttachment,
      threadCount: index === 3 ? 3 : 1,
      importance: s.flagged ? 1 : 0,
      tags: [...s.tags],
      answered: index === 3,
      forwarded: false,
    };
  }

  const dayOffset = Math.floor(index / 25);
  const dateTs = MOCK_NOW - dayOffset * 86400 - (index % 25) * 900;
  return {
    id: index + 1,
    accountId: 1,
    folderId: 1,
    folderPath: "INBOX",
    sender: `发件人 ${(index % 120) + 1} <sender${(index % 120) + 1}@example.com>`,
    recipients: "demo@kitemail.example",
    subject: `演示邮件 #${index + 1}：三栏虚拟滚动与分页测试`,
    preview: `这是第 ${index + 1} 封演示邮件的摘要预览，用于验证 10,000 封邮件虚拟滚动与分批加载。`,
    dateTs,
    dateDisplay:
      dayOffset === 0
        ? "今天"
        : dayOffset === 1
          ? "昨天"
          : dayOffset <= 6
            ? `${dayOffset} 天前`
            : "更早",
    unread: index < 128 && index % 2 === 0,
    flagged: index % 29 === 0,
    hasAttachment: index % 7 === 0,
    threadCount: index % 11 === 0 ? 3 : 1,
    importance: index % 17 === 0 ? 1 : 0,
    tags: index % 19 === 0 ? ["待办"] : [],
    answered: index % 9 === 0,
    forwarded: false,
  };
}

let cachedMockAll: MessageSummaryDto[] | null = null;
function getMockMessages(): MessageSummaryDto[] {
  if (cachedMockAll) return cachedMockAll;

  const list: MessageSummaryDto[] = new Array(MOCK_INBOX_TOTAL + 10);
  for (let i = 0; i < MOCK_INBOX_TOTAL; i++) {
    list[i] = buildMockInboxMessage(i);
  }
  const extraFolders: Array<{ folderId: number; folderPath: string; label: string; count: number }> = [
    { folderId: 2, folderPath: "Sent", label: "已发送演示邮件", count: 4 },
    { folderId: 3, folderPath: "Drafts", label: "草稿演示邮件", count: 2 },
    { folderId: 4, folderPath: "Archive", label: "归档演示邮件", count: 2 },
    { folderId: 5, folderPath: "Junk", label: "垃圾演示邮件", count: 1 },
    { folderId: 6, folderPath: "Trash", label: "已删除演示邮件", count: 1 },
  ];
  let cursor = MOCK_INBOX_TOTAL;
  for (const f of extraFolders) {
    for (let j = 0; j < f.count; j++) {
      list[cursor] = {
        id: cursor + 1,
        accountId: 1,
        folderId: f.folderId,
        folderPath: f.folderPath,
        sender: f.folderId === 2 ? "demo@kitemail.example" : `测试联系人 ${j + 1} <contact${j + 1}@example.com>`,
        recipients: "partner@example.com",
        subject: `${f.label} #${j + 1}`,
        preview: `${f.folderPath} 文件夹演示邮件内容预览。`,
        dateTs: MOCK_NOW - (j + 1) * 7200,
        dateDisplay: "今天",
        unread: false,
        flagged: false,
        hasAttachment: false,
        threadCount: 1,
        importance: 0,
        tags: [],
        answered: false,
        forwarded: false,
      };
      cursor++;
    }
  }
  cachedMockAll = list;
  return list;
}

let mockStateVersion = 1;
let mockLocked = false;
let mockHasPasscode = true;
let mockCacheReady = true;
let mockPasscode = "1234";
let mockFailedAttempts = 0;
let mockUnlockCooldownUntilMs = 0;
let mockNextMessageId = MOCK_INBOX_TOTAL + 11;
const mockRecordedReaderLayouts: SetReaderLayoutInput[] = [];
const mockAllowedRemoteMessages = new Set<string>();
const mockAllowedRemoteSenders = new Set<string>();
const mockFolderHasMoreRemote = new Map<number, boolean>([[1, true]]);

const DEFAULT_MOCK_PREFS: MainPreferencesDto = {
  themeId: "system",
  darkMode: null,
  readerPosition: "right",
  sidebarWidth: 255,
  listWidth: 350,
  listBelowHeight: 360,
  sidebarCollapsed: false,
  compactList: false,
  showAvatars: true,
  listGroupByDate: true,
  readerZoomPercent: 100,
};

let mockPrefs: MainPreferencesDto = { ...DEFAULT_MOCK_PREFS };

const BASE_MOCK_FOLDERS: ReadonlyArray<Omit<FolderSummaryDto, "unreadCount" | "totalCount" | "hasMoreRemote">> = [
  { id: 1, accountId: 1, name: "收件箱", path: "INBOX" },
  { id: 2, accountId: 1, name: "已发送", path: "Sent" },
  { id: 3, accountId: 1, name: "草稿箱", path: "Drafts" },
  { id: 4, accountId: 1, name: "归档", path: "Archive" },
  { id: 5, accountId: 1, name: "垃圾邮件", path: "Junk" },
  { id: 6, accountId: 1, name: "已删除", path: "Trash" },
];

function computeMockFolders(): FolderSummaryDto[] {
  const all = getMockMessages();
  return BASE_MOCK_FOLDERS.map((base) => {
    let unreadCount = 0;
    let totalCount = 0;
    for (const m of all) {
      if (m.accountId === base.accountId && m.folderId === base.id) {
        totalCount += 1;
        if (m.unread) unreadCount += 1;
      }
    }
    return {
      ...base,
      unreadCount,
      totalCount,
      hasMoreRemote: mockFolderHasMoreRemote.get(base.id) ?? false,
    };
  });
}

function extractMockSenderEmail(sender: string): string {
  const match = sender.match(/<([^>]+)>/);
  const raw = match ? match[1] : sender;
  return raw.trim().toLowerCase();
}

function hasMockRemoteImages(msg: MessageSummaryDto): boolean {
  return msg.id === 2 || (msg.id > 4 && msg.id % 5 === 2);
}

function isMockRemoteImagesBlocked(msg: MessageSummaryDto | undefined): boolean {
  if (!msg || !hasMockRemoteImages(msg)) return false;
  const key = `${msg.accountId}:${msg.id}`;
  if (mockAllowedRemoteMessages.has(key)) return false;
  const senderEmail = extractMockSenderEmail(msg.sender);
  if (senderEmail && mockAllowedRemoteSenders.has(senderEmail)) return false;
  return true;
}

function makeMockRfcMessageId(accountId: number, messageId: number): string {
  return `<mock-${accountId}-${messageId}@kitemail.example>`;
}

function findMockFolderByPathOrName(
  targetFolderPath: string
): (typeof BASE_MOCK_FOLDERS)[number] | undefined {
  const normalized = targetFolderPath.trim().toLowerCase();
  return BASE_MOCK_FOLDERS.find(
    (f) => f.path.toLowerCase() === normalized || f.name.toLowerCase() === normalized
  );
}

function resolveMockFolderIdForView(view: ViewDto): number | null {
  if (view.kind === "accountFolder") return view.folderId;
  switch (view.kind) {
    case "unifiedInbox":
      return 1;
    case "sent":
      return 2;
    case "drafts":
      return 3;
    case "archive":
      return 4;
    case "junk":
      return 5;
    case "trash":
      return 6;
    default:
      return null;
  }
}

function makeLockedIpcError(requestId?: string | null): IpcError {
  return {
    code: "LOCKED",
    message: "应用已锁定，请先解锁后再访问邮件内容",
    retryable: false,
    ...(requestId ? { requestId } : {}),
  };
}

function mockCooldownSecs(failedAttempts: number): number {
  if (failedAttempts < 5) return 0;
  if (failedAttempts === 5) return 5;
  if (failedAttempts === 6) return 10;
  if (failedAttempts === 7) return 30;
  return 60;
}

const MOCK_TAGS: readonly TagItemDto[] = [
  { id: 1, name: "重要", color: "#e01b24", shortcut: 1 },
  { id: 2, name: "工作", color: "#3584e4", shortcut: 2 },
  { id: 3, name: "个人", color: "#26a269", shortcut: 3 },
  { id: 4, name: "待办", color: "#e66100", shortcut: 4 },
  { id: 5, name: "项目", color: "#9141ac", shortcut: 5 },
];
let mockTemplates: MailTemplateDto[] = [];

const MOCK_BOOTSTRAP: MainBootstrapDto = {
  mode: "demo",
  locked: false,
  hasPasscode: true,
  cacheReady: true,
  stateVersion: 1,
  accounts: [{ id: 1, displayName: "测试工作邮箱", email: "demo@kitemail.example" }],
  folders: [],
  currentView: { kind: "unifiedInbox" },
  preferences: { ...DEFAULT_MOCK_PREFS },
  syncStatus: { syncing: false, message: null, lastSyncAt: null },
  tags: [...MOCK_TAGS],
  templates: [],
};

// Mock 事件总线
type EventHandler<T> = (payload: T) => void;
interface MockBus {
  "main://mail-changed": EventHandler<MailChangedEvent>[];
  "main://sync-status": EventHandler<SyncStatusEvent>[];
  "main://reader-status": EventHandler<ReaderStatusEvent>[];
  "main://lock-state": EventHandler<LockStateEvent>[];
}
const mockBus: MockBus = {
  "main://mail-changed": [],
  "main://sync-status": [],
  "main://reader-status": [],
  "main://lock-state": [],
};

function mockListen<K extends keyof MockBus>(
  event: K,
  handler: MockBus[K][number]
): UnlistenFn {
  (mockBus[event] as EventHandler<unknown>[]).push(handler as EventHandler<unknown>);
  return () => {
    const arr = mockBus[event] as EventHandler<unknown>[];
    const idx = arr.indexOf(handler as EventHandler<unknown>);
    if (idx >= 0) arr.splice(idx, 1);
  };
}

function emitMock<K extends keyof MockBus>(event: K, payload: Parameters<MockBus[K][number]>[0]): void {
  for (const fn of [...(mockBus[event] as EventHandler<unknown>[])]) {
    fn(payload);
  }
}

function matchesMockView(msg: MessageSummaryDto, view: ViewDto): boolean {
  if (view.kind === "accountFolder") {
    return msg.accountId === view.accountId && msg.folderId === view.folderId;
  }
  switch (view.kind) {
    case "unifiedInbox":
      return msg.folderPath === "INBOX";
    case "unread":
      return msg.unread && msg.folderPath !== "Junk" && msg.folderPath !== "Trash";
    case "starred":
      return msg.flagged;
    case "sent":
      return msg.folderPath === "Sent";
    case "drafts":
      return msg.folderPath === "Drafts";
    case "archive":
      return msg.folderPath === "Archive";
    case "junk":
      return msg.folderPath === "Junk";
    case "trash":
      return msg.folderPath === "Trash";
    case "filters":
      return msg.unread || msg.flagged || msg.hasAttachment;
    case "tags":
      return msg.tags.length > 0;
    case "snoozed":
      return false;
  }
}

function matchesMockSearchQuery(msg: MessageSummaryDto, rawQuery: string): boolean {
  const q = rawQuery.trim().toLowerCase();
  if (!q) return true;
  const tokens: string[] = [];
  let cur = "";
  let inQuotes = false;
  for (const ch of q) {
    if (ch === '"') {
      inQuotes = !inQuotes;
      continue;
    }
    if (!inQuotes && /\s/.test(ch)) {
      if (cur) {
        tokens.push(cur);
        cur = "";
      }
    } else {
      cur += ch;
    }
  }
  if (cur) tokens.push(cur);

  for (const tok of tokens) {
    if (tok === "is:unread") {
      if (!msg.unread) return false;
      continue;
    }
    if (tok === "is:read") {
      if (msg.unread) return false;
      continue;
    }
    if (tok === "is:starred") {
      if (!msg.flagged) return false;
      continue;
    }
    if (tok === "has:attachment" || tok === "has:attachments") {
      if (!msg.hasAttachment) return false;
      continue;
    }
    if (tok.startsWith("tag:")) {
      const wantTag = tok.slice(4).trim();
      if (wantTag && !msg.tags.some((t) => t.toLowerCase() === wantTag)) return false;
      continue;
    }
    if (
      !msg.subject.toLowerCase().includes(tok) &&
      !msg.sender.toLowerCase().includes(tok) &&
      !msg.preview.toLowerCase().includes(tok)
    ) {
      return false;
    }
  }
  return true;
}

function compareMessages(a: MessageSummaryDto, b: MessageSummaryDto, sort: MessageSort): number {
  switch (sort) {
    case "dateNewest":
      return b.dateTs - a.dateTs || a.id - b.id;
    case "dateOldest":
      return a.dateTs - b.dateTs || a.id - b.id;
    case "senderAsc":
      return a.sender.localeCompare(b.sender, "zh-CN") || b.dateTs - a.dateTs;
    case "senderDesc":
      return b.sender.localeCompare(a.sender, "zh-CN") || b.dateTs - a.dateTs;
    case "subjectAsc":
      return a.subject.localeCompare(b.subject, "zh-CN") || b.dateTs - a.dateTs;
    case "subjectDesc":
      return b.subject.localeCompare(a.subject, "zh-CN") || b.dateTs - a.dateTs;
    case "unreadFirst":
      return Number(b.unread) - Number(a.unread) || b.dateTs - a.dateTs;
    case "starredFirst":
      return Number(b.flagged) - Number(a.flagged) || b.dateTs - a.dateTs;
    case "attachmentFirst":
      return Number(b.hasAttachment) - Number(a.hasAttachment) || b.dateTs - a.dateTs;
    case "importanceFirst":
      return b.importance - a.importance || b.dateTs - a.dateTs;
  }
}

// ---------- 公共 API ----------

export async function mainBootstrap(): Promise<MainBootstrapDto> {
  if (isTauri()) return tauriInvoke<MainBootstrapDto>("main_bootstrap");
  if (mockLocked) {
    return Promise.resolve({
      ...MOCK_BOOTSTRAP,
      locked: true,
      hasPasscode: mockHasPasscode,
      cacheReady: mockCacheReady,
      stateVersion: mockStateVersion,
      accounts: [],
      folders: [],
      preferences: { ...mockPrefs },
    });
  }
  return Promise.resolve({
    ...MOCK_BOOTSTRAP,
    locked: false,
    hasPasscode: mockHasPasscode,
    cacheReady: mockCacheReady,
    folders: computeMockFolders(),
    stateVersion: mockStateVersion,
    preferences: { ...mockPrefs },
    tags: MOCK_TAGS.map((t) => ({ ...t })),
    templates: mockTemplates.map((t) => ({ ...t, attachmentPaths: [...t.attachmentPaths] })),
  });
}

export async function listMessages(input: ListMessagesInput): Promise<ListMessagesOutput> {
  if (isTauri()) return tauriInvoke<ListMessagesOutput>("list_messages", { input });
  if (mockLocked) return Promise.reject(makeLockedIpcError(input.requestId));

  const all = getMockMessages();
  const q = input.query.trim().toLowerCase();
  const todayStart = new Date();
  todayStart.setHours(0, 0, 0, 0);
  const todayTs = Math.floor(todayStart.getTime() / 1000);
  const f = input.filter;
  const hasQuickFilter = Boolean(
    f.unread ||
      f.starred ||
      f.hasAttachment ||
      f.today ||
      f.toMe ||
      f.fromContacts ||
      f.highImportance ||
      f.tagged ||
      f.dateFromTs != null ||
      f.dateToTs != null
  );

  const filtered = all.filter((m) => {
    if (!((q || hasQuickFilter) && input.scope === "allFolders")) {
      if (!matchesMockView(m, input.view)) return false;
    }
    if (f.accountId != null && m.accountId !== f.accountId) return false;
    if (q && !matchesMockSearchQuery(m, q)) return false;
    if (f.unread && !m.unread) return false;
    if (f.starred && !m.flagged) return false;
    if (f.hasAttachment && !m.hasAttachment) return false;
    if (f.today && m.dateTs < todayTs) return false;
    if (f.toMe && !m.recipients.trim()) return false;
    if (f.highImportance && m.importance <= 0) return false;
    if (f.tagged && m.tags.length === 0) return false;
    if (f.dateFromTs != null && m.dateTs < f.dateFromTs) return false;
    if (f.dateToTs != null && m.dateTs >= f.dateToTs) return false;
    return true;
  });

  if (input.sort !== "dateNewest") {
    filtered.sort((a, b) => compareMessages(a, b, input.sort));
  }

  const page = filtered.slice(input.offset, input.offset + input.limit);
  return {
    requestId: input.requestId,
    stateVersion: mockStateVersion,
    offset: input.offset,
    total: filtered.length,
    hasMore: input.offset + page.length < filtered.length,
    items: page.map((m) => ({ ...m, tags: [...m.tags] })),
  };
}

export async function listThreadSummaries(
  input: ListThreadSummariesInput
): Promise<ListThreadSummariesOutput> {
  if (isTauri()) return tauriInvoke<ListThreadSummariesOutput>("list_thread_summaries", { input });
  if (mockLocked) return Promise.reject(makeLockedIpcError());

  const msg = getMockMessages().find(
    (m) => m.id === input.messageId && m.accountId === input.accountId
  );
  if (!msg || msg.threadCount <= 1) {
    return {
      accountId: input.accountId,
      messageId: input.messageId,
      items: [],
    };
  }
  const childCount = Math.max(1, msg.threadCount - 1);
  const items: MessageSummaryDto[] = [];
  for (let i = 0; i < childCount; i++) {
    items.push({
      ...msg,
      id: msg.id * 100 + i + 1,
      sender: i === 0 ? msg.sender : `协作者 ${i + 1} <member${i + 1}@example.com>`,
      preview: i === 0 ? "第一条历史回复摘要。" : `会话第 ${i + 1} 条补充回复。`,
      dateTs: msg.dateTs - (i + 1) * 1800,
      dateDisplay: "昨天",
      unread: false,
      flagged: false,
      hasAttachment: false,
      threadCount: 1,
      tags: [],
    });
  }
  return {
    accountId: input.accountId,
    messageId: input.messageId,
    items,
  };
}

export async function openMessage(input: OpenMessageInput): Promise<OpenMessageOutput> {
  if (isTauri()) return tauriInvoke<OpenMessageOutput>("open_message", { input });
  if (mockLocked) return Promise.reject(makeLockedIpcError(input.requestId));

  const msg = getMockMessages().find(
    (m) => m.id === input.messageId && m.accountId === input.accountId
  );
  const sessionId = `mock-session-${input.accountId}-${input.messageId}-${Date.now()}`;
  const isThread = (msg?.threadCount ?? 1) > 1;
  const remoteImagesBlocked = isMockRemoteImagesBlocked(msg);
  return {
    requestId: input.requestId,
    readerSessionId: sessionId,
    accountId: input.accountId,
    messageId: input.messageId,
    subject: msg?.subject ?? "(未知主题)",
    sender: msg?.sender ?? "",
    recipients: msg?.recipients ?? "",
    cc: "",
    dateDisplay: msg?.dateDisplay ?? "",
    bodyStatus: "ready",
    plainText:
      `主题：${msg?.subject ?? ""}\n发件人：${msg?.sender ?? ""}\n\n` +
      `${msg?.preview ?? ""}\n\n` +
      `这是邮件 #${input.messageId} 的正文演示内容。在纯前端浏览器预览环境下以安全转义纯文本呈现；` +
      `在 Windows Tauri 桌面端将由共享原生 WebView2 阅读器渲染完整 HTML 与会话卡片。\n\n感谢使用 KiteMail！`,
    hasHtml: false,
    htmlBody: null,
    flagged: msg?.flagged ?? false,
    remoteImagesBlocked,
    rfcMessageId: makeMockRfcMessageId(input.accountId, input.messageId),
    folderPath: msg?.folderPath ?? "INBOX",
    attachments: msg?.hasAttachment
      ? [{ id: 1, filename: "KiteMail-三栏布局规范.pdf", mimeType: "application/pdf", sizeBytes: 184320 }]
      : [],
    threadMessages: isThread
      ? [
          {
            id: input.messageId * 10 + 1,
            accountId: input.accountId,
            sender: msg?.sender ?? "",
            recipients: msg?.recipients ?? "",
            cc: "",
            dateDisplay: "昨天 16:40",
            preview: "第一条历史回复摘要。",
            plainText: "已附上第一轮压测数据与内存占用截图，10,000 封演示邮件滚动稳定。",
            hasHtml: false,
            htmlBody: null,
            unread: false,
            flagged: false,
          },
          {
            id: input.messageId,
            accountId: input.accountId,
            sender: msg?.sender ?? "",
            recipients: msg?.recipients ?? "",
            cc: "",
            dateDisplay: msg?.dateDisplay ?? "今天",
            preview: msg?.preview ?? "",
            plainText: msg?.preview ?? "",
            hasHtml: false,
            htmlBody: null,
            unread: msg?.unread ?? false,
            flagged: msg?.flagged ?? false,
          },
        ]
      : [],
  };
}

export async function setReaderLayout(input: SetReaderLayoutInput): Promise<SetReaderLayoutOutput> {
  if (isTauri()) return tauriInvoke<SetReaderLayoutOutput>("set_reader_layout", { input });
  mockRecordedReaderLayouts.push({ ...input });
  return {
    applied: true,
    readerSessionId: input.readerSessionId,
    layoutRevision: input.layoutRevision,
  };
}

export async function updateMainPreferences(patch: MainPreferencesPatch): Promise<MainPreferencesDto> {
  if (isTauri()) return tauriInvoke<MainPreferencesDto>("update_main_preferences", { patch });
  Object.assign(mockPrefs, patch);
  if ("darkMode" in patch) {
    mockPrefs.darkMode = patch.darkMode === true ? true : patch.darkMode === false ? false : null;
  }
  return { ...mockPrefs };
}

export async function refreshMail(input: RefreshMailInput): Promise<RefreshMailOutput> {
  if (isTauri()) return tauriInvoke<RefreshMailOutput>("refresh_mail", { input });
  if (mockLocked) return Promise.reject(makeLockedIpcError());
  mockStateVersion += 1;
  emitMock("main://sync-status", {
    stateVersion: mockStateVersion,
    syncStatus: {
      syncing: true,
      message: "正在刷新演示邮件…",
      lastSyncAt: null,
    },
  });
  setTimeout(() => {
    if (mockLocked) return;
    const nowSec = Math.floor(Date.now() / 1000);
    emitMock("main://sync-status", {
      stateVersion: mockStateVersion,
      syncStatus: {
        syncing: false,
        message: null,
        lastSyncAt: nowSec,
      },
    });
    emitMock("main://mail-changed", {
      stateVersion: mockStateVersion,
      view: input.view,
      reason: "refresh",
    });
  }, 150);
  return { accepted: true, stateVersion: mockStateVersion };
}

export async function unlockApp(input: UnlockAppInput): Promise<UnlockAppOutput> {
  if (isTauri()) return tauriInvoke<UnlockAppOutput>("unlock_app", { input });

  const nowMs = Date.now();
  if (mockUnlockCooldownUntilMs > nowMs) {
    const retryAfterSecs = Math.max(1, Math.ceil((mockUnlockCooldownUntilMs - nowMs) / 1000));
    return {
      unlocked: false,
      locked: mockLocked,
      error: `尝试次数过多，请 ${retryAfterSecs} 秒后再试`,
      retryAfterSecs,
    };
  }

  if (input.passcode === mockPasscode) {
    mockFailedAttempts = 0;
    mockUnlockCooldownUntilMs = 0;
    mockLocked = false;
    emitMock("main://lock-state", {
      locked: false,
      hasPasscode: mockHasPasscode,
      lockedReason: null,
    });
    return {
      unlocked: true,
      locked: false,
      error: null,
      retryAfterSecs: null,
    };
  }

  mockFailedAttempts += 1;
  const cooldown = mockCooldownSecs(mockFailedAttempts);
  if (cooldown > 0) {
    mockUnlockCooldownUntilMs = nowMs + cooldown * 1000;
  }
  return {
    unlocked: false,
    locked: true,
    error: cooldown > 0 ? `锁屏密码错误，请 ${cooldown} 秒后再试` : "锁屏密码错误",
    retryAfterSecs: cooldown > 0 ? cooldown : null,
  };
}

export async function lockApp(): Promise<LockAppOutput> {
  if (isTauri()) return tauriInvoke<LockAppOutput>("lock_app");
  mockLocked = true;
  emitMock("main://lock-state", {
    locked: true,
    hasPasscode: mockHasPasscode,
    lockedReason: "manual",
  });
  return { locked: true };
}

export async function setMessageSeen(input: SetMessageSeenInput): Promise<SetMessageSeenOutput> {
  if (isTauri()) return tauriInvoke<SetMessageSeenOutput>("set_message_seen", { input });
  if (mockLocked) return Promise.reject(makeLockedIpcError());

  const msg = getMockMessages().find(
    (m) => m.id === input.messageId && m.accountId === input.accountId
  );
  if (!msg) {
    return Promise.reject({
      code: "MESSAGE_NOT_FOUND",
      message: `未找到邮件 #${input.messageId}`,
      retryable: false,
    } satisfies IpcError);
  }
  msg.unread = !input.seen;
  mockStateVersion += 1;
  emitMock("main://mail-changed", {
    stateVersion: mockStateVersion,
    reason: "seenChanged",
    accountId: msg.accountId,
    folderId: msg.folderId,
    messageId: msg.id,
  });
  return {
    accountId: msg.accountId,
    messageId: msg.id,
    unread: msg.unread,
    stateVersion: mockStateVersion,
  };
}

export async function setMessageFlagged(
  input: SetMessageFlaggedInput
): Promise<SetMessageFlaggedOutput> {
  if (isTauri()) return tauriInvoke<SetMessageFlaggedOutput>("set_message_flagged", { input });
  if (mockLocked) return Promise.reject(makeLockedIpcError());

  const msg = getMockMessages().find(
    (m) => m.id === input.messageId && m.accountId === input.accountId
  );
  if (!msg) {
    return Promise.reject({
      code: "MESSAGE_NOT_FOUND",
      message: `未找到邮件 #${input.messageId}`,
      retryable: false,
    } satisfies IpcError);
  }
  msg.flagged = input.flagged;
  mockStateVersion += 1;
  emitMock("main://mail-changed", {
    stateVersion: mockStateVersion,
    reason: "flaggedChanged",
    accountId: msg.accountId,
    folderId: msg.folderId,
    messageId: msg.id,
  });
  return {
    accountId: msg.accountId,
    messageId: msg.id,
    flagged: msg.flagged,
    stateVersion: mockStateVersion,
  };
}

function moveMockMessageToFolder(
  accountId: number,
  messageId: number,
  targetFolderPath: string
): MessageMoveOutput {
  const msg = getMockMessages().find(
    (m) => m.id === messageId && m.accountId === accountId
  );
  if (!msg) {
    throw {
      code: "MESSAGE_NOT_FOUND",
      message: `未找到邮件 #${messageId}`,
      retryable: false,
    } satisfies IpcError;
  }
  const targetFolder = findMockFolderByPathOrName(targetFolderPath);
  if (!targetFolder) {
    throw {
      code: "INVALID_INPUT",
      message: `目标文件夹不存在: ${targetFolderPath}`,
      retryable: false,
    } satisfies IpcError;
  }

  const originalFolderPath = msg.folderPath;
  msg.folderId = targetFolder.id;
  msg.folderPath = targetFolder.path;
  mockStateVersion += 1;

  emitMock("main://mail-changed", {
    stateVersion: mockStateVersion,
    reason: "messageMoved",
    accountId: msg.accountId,
    folderId: msg.folderId,
    messageId: msg.id,
  });

  return {
    accountId: msg.accountId,
    messageId: msg.id,
    rfcMessageId: makeMockRfcMessageId(msg.accountId, msg.id),
    originalFolderPath,
    targetFolderPath: targetFolder.path,
    stateVersion: mockStateVersion,
  };
}

export async function allowRemoteImages(
  input: AllowRemoteImagesInput
): Promise<AllowRemoteImagesOutput> {
  if (isTauri()) return tauriInvoke<AllowRemoteImagesOutput>("allow_remote_images", { input });
  if (mockLocked) return Promise.reject(makeLockedIpcError());

  const msg = getMockMessages().find(
    (m) => m.id === input.messageId && m.accountId === input.accountId
  );
  if (!msg) {
    return Promise.reject({
      code: "MESSAGE_NOT_FOUND",
      message: `未找到邮件 #${input.messageId}`,
      retryable: false,
    } satisfies IpcError);
  }

  mockAllowedRemoteMessages.add(`${input.accountId}:${input.messageId}`);
  let senderTrusted = false;
  if (input.trustSender) {
    const email = extractMockSenderEmail(msg.sender);
    if (email) {
      mockAllowedRemoteSenders.add(email);
      senderTrusted = true;
    }
  }
  mockStateVersion += 1;
  return {
    allowed: true,
    senderTrusted,
    stateVersion: mockStateVersion,
  };
}

export async function loadOlderMessages(
  input: LoadOlderMessagesInput
): Promise<LoadOlderMessagesOutput> {
  if (isTauri()) return tauriInvoke<LoadOlderMessagesOutput>("load_older_messages", { input });
  if (mockLocked) return Promise.reject(makeLockedIpcError());

  const folderId = resolveMockFolderIdForView(input.view);
  if (folderId === null || !(mockFolderHasMoreRemote.get(folderId) ?? false)) {
    return {
      accepted: false,
      hasMoreRemote: false,
      stateVersion: mockStateVersion,
    };
  }

  const all = getMockMessages();
  const folderMeta = BASE_MOCK_FOLDERS.find((f) => f.id === folderId) ?? BASE_MOCK_FOLDERS[0];
  let minTs = MOCK_NOW - 86400 * 30;
  for (const m of all) {
    if (m.folderId === folderId && m.dateTs < minTs) {
      minTs = m.dateTs;
    }
  }
  const batchCount = Math.max(1, Math.min(input.limit || 20, 50));
  for (let i = 0; i < batchCount; i++) {
    const id = mockNextMessageId++;
    all.push({
      id,
      accountId: folderMeta.accountId,
      folderId: folderMeta.id,
      folderPath: folderMeta.path,
      sender: `历史发件人 ${i + 1} <archive${i + 1}@example.com>`,
      recipients: "demo@kitemail.example",
      subject: `历史拉取邮件 #${id}（远端同步）`,
      preview: `通过 load_older_messages 从远端拉取的历史邮件 #${id}。`,
      dateTs: minTs - (i + 1) * 3600,
      dateDisplay: "更早",
      unread: false,
      flagged: false,
      hasAttachment: false,
      threadCount: 1,
      importance: 0,
      tags: [],
      answered: false,
      forwarded: false,
    });
  }

  mockFolderHasMoreRemote.set(folderId, false);
  mockStateVersion += 1;
  emitMock("main://mail-changed", {
    stateVersion: mockStateVersion,
    view: input.view,
    reason: "olderMessagesLoaded",
    accountId: folderMeta.accountId,
    folderId: folderMeta.id,
  });

  return {
    accepted: true,
    hasMoreRemote: false,
    stateVersion: mockStateVersion,
  };
}

export async function archiveMessage(input: MessageActionInput): Promise<MessageMoveOutput> {
  if (isTauri()) return tauriInvoke<MessageMoveOutput>("archive_message", { input });
  if (mockLocked) return Promise.reject(makeLockedIpcError());
  return moveMockMessageToFolder(input.accountId, input.messageId, "Archive");
}

export async function deleteMessage(input: MessageActionInput): Promise<MessageMoveOutput> {
  if (isTauri()) return tauriInvoke<MessageMoveOutput>("delete_message", { input });
  if (mockLocked) return Promise.reject(makeLockedIpcError());
  return moveMockMessageToFolder(input.accountId, input.messageId, "Trash");
}

export async function markMessageSpam(input: MessageActionInput): Promise<MessageMoveOutput> {
  if (isTauri()) return tauriInvoke<MessageMoveOutput>("mark_message_spam", { input });
  if (mockLocked) return Promise.reject(makeLockedIpcError());
  return moveMockMessageToFolder(input.accountId, input.messageId, "Junk");
}

export async function moveMessage(input: MoveMessageInput): Promise<MessageMoveOutput> {
  if (isTauri()) return tauriInvoke<MessageMoveOutput>("move_message", { input });
  if (mockLocked) return Promise.reject(makeLockedIpcError());
  return moveMockMessageToFolder(input.accountId, input.messageId, input.targetFolderPath);
}

export async function undoMessageMove(
  input: UndoMessageMoveInput
): Promise<UndoMessageMoveOutput> {
  if (isTauri()) return tauriInvoke<UndoMessageMoveOutput>("undo_message_move", { input });
  if (mockLocked) return Promise.reject(makeLockedIpcError());

  const msg = getMockMessages().find(
    (m) => m.id === input.messageId && m.accountId === input.accountId
  );
  if (!msg) {
    return Promise.reject({
      code: "MESSAGE_NOT_FOUND",
      message: `未找到邮件 #${input.messageId}`,
      retryable: false,
    } satisfies IpcError);
  }
  const origFolder = findMockFolderByPathOrName(input.originalFolderPath);
  if (!origFolder) {
    return Promise.reject({
      code: "INVALID_INPUT",
      message: `原文件夹不存在: ${input.originalFolderPath}`,
      retryable: false,
    } satisfies IpcError);
  }

  msg.folderId = origFolder.id;
  msg.folderPath = origFolder.path;
  mockStateVersion += 1;

  emitMock("main://mail-changed", {
    stateVersion: mockStateVersion,
    reason: "messageMoveUndone",
    accountId: msg.accountId,
    folderId: msg.folderId,
    messageId: msg.id,
  });

  return {
    accepted: true,
    stateVersion: mockStateVersion,
  };
}

export async function quitApp(): Promise<void> {
  if (isTauri()) return tauriInvoke<void>("quit_app");
}

// ---------- 测试辅助（仅作用于 Mock 传输层） ----------

export function resetMockSession(options?: {
  locked?: boolean;
  hasPasscode?: boolean;
  cacheReady?: boolean;
  passcode?: string;
  failedAttempts?: number;
  cooldownMs?: number;
  darkMode?: boolean | null;
  inboxHasMoreRemote?: boolean;
  templates?: MailTemplateDto[];
}): void {
  cachedMockAll = null;
  mockStateVersion = 1;
  mockLocked = options?.locked ?? false;
  mockHasPasscode = options?.hasPasscode ?? true;
  mockCacheReady = options?.cacheReady ?? true;
  mockPasscode = options?.passcode ?? "1234";
  mockFailedAttempts = options?.failedAttempts ?? 0;
  mockUnlockCooldownUntilMs = options?.cooldownMs ? Date.now() + options.cooldownMs : 0;
  mockNextMessageId = MOCK_INBOX_TOTAL + 11;
  mockTemplates = options?.templates
    ? options.templates.map((t) => ({ ...t, attachmentPaths: [...t.attachmentPaths] }))
    : [];
  mockPrefs = {
    ...DEFAULT_MOCK_PREFS,
    ...(options && "darkMode" in options ? { darkMode: options.darkMode ?? null } : {}),
  };
  mockRecordedReaderLayouts.length = 0;
  mockAllowedRemoteMessages.clear();
  mockAllowedRemoteSenders.clear();
  mockFolderHasMoreRemote.clear();
  mockFolderHasMoreRemote.set(1, options?.inboxHasMoreRemote ?? true);
}

export function getMockRecordedReaderLayouts(): readonly SetReaderLayoutInput[] {
  return mockRecordedReaderLayouts;
}

export function emitMockEvent<K extends keyof MockBus>(
  event: K,
  payload: Parameters<MockBus[K][number]>[0]
): void {
  emitMock(event, payload);
}

// ---------- 事件订阅 ----------

export async function onMailChanged(handler: (e: MailChangedEvent) => void): Promise<UnlistenFn> {
  if (isTauri()) return tauriListen<MailChangedEvent>("main://mail-changed", handler);
  return mockListen("main://mail-changed", handler);
}

export async function onSyncStatus(handler: (e: SyncStatusEvent) => void): Promise<UnlistenFn> {
  if (isTauri()) return tauriListen<SyncStatusEvent>("main://sync-status", handler);
  return mockListen("main://sync-status", handler);
}

export async function onReaderStatus(handler: (e: ReaderStatusEvent) => void): Promise<UnlistenFn> {
  if (isTauri()) return tauriListen<ReaderStatusEvent>("main://reader-status", handler);
  return mockListen("main://reader-status", handler);
}

export async function onLockState(handler: (e: LockStateEvent) => void): Promise<UnlistenFn> {
  if (isTauri()) return tauriListen<LockStateEvent>("main://lock-state", handler);
  return mockListen("main://lock-state", handler);
}

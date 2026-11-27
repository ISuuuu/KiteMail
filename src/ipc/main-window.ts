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
 *
 * 在非 Tauri 环境（无 window.__TAURI_INTERNALS__）下使用内置 Mock 传输层，
 * 以便独立浏览器预览和单元测试。
 */

import type {
  MainBootstrapDto,
  ListMessagesInput,
  ListMessagesOutput,
  OpenMessageInput,
  OpenMessageOutput,
  SetReaderLayoutInput,
  SetReaderLayoutOutput,
  RefreshMailInput,
  RefreshMailOutput,
  MainPreferencesPatch,
  MainPreferencesDto,
  MailChangedEvent,
  SyncStatusEvent,
  ReaderStatusEvent,
  LockStateEvent,
  MessageSummaryDto,
  ViewDto,
  MessageSort,
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
const mockPrefs: MainPreferencesDto = {
  themeId: "system",
  darkMode: false,
  readerPosition: "right",
  sidebarWidth: 240,
  listWidth: 360,
  listBelowHeight: 360,
  sidebarCollapsed: false,
  compactList: false,
  showAvatars: true,
  listGroupByDate: true,
  readerZoomPercent: 100,
};

const MOCK_BOOTSTRAP: MainBootstrapDto = {
  mode: "demo",
  locked: false,
  stateVersion: 1,
  accounts: [{ id: 1, displayName: "测试工作邮箱", email: "demo@kitemail.example" }],
  folders: [
    { id: 1, accountId: 1, name: "收件箱", path: "INBOX", unreadCount: 64, totalCount: MOCK_INBOX_TOTAL, hasMoreRemote: false },
    { id: 2, accountId: 1, name: "已发送", path: "Sent", unreadCount: 0, totalCount: 4, hasMoreRemote: false },
    { id: 3, accountId: 1, name: "草稿箱", path: "Drafts", unreadCount: 0, totalCount: 2, hasMoreRemote: false },
    { id: 4, accountId: 1, name: "归档", path: "Archive", unreadCount: 0, totalCount: 2, hasMoreRemote: false },
    { id: 5, accountId: 1, name: "垃圾邮件", path: "Junk", unreadCount: 0, totalCount: 1, hasMoreRemote: false },
    { id: 6, accountId: 1, name: "已删除", path: "Trash", unreadCount: 0, totalCount: 1, hasMoreRemote: false },
  ],
  currentView: { kind: "unifiedInbox" },
  preferences: { ...mockPrefs },
  syncStatus: { syncing: false, message: null, lastSyncAt: null },
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
    case "snoozed":
      return false;
  }
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
  return Promise.resolve({
    ...MOCK_BOOTSTRAP,
    stateVersion: mockStateVersion,
    preferences: { ...mockPrefs },
  });
}

export async function listMessages(input: ListMessagesInput): Promise<ListMessagesOutput> {
  if (isTauri()) return tauriInvoke<ListMessagesOutput>("list_messages", { input });

  const all = getMockMessages();
  const q = input.query.trim().toLowerCase();
  const todayStart = new Date();
  todayStart.setHours(0, 0, 0, 0);
  const todayTs = Math.floor(todayStart.getTime() / 1000);

  const filtered = all.filter((m) => {
    if (!(q && input.scope === "allFolders")) {
      if (!matchesMockView(m, input.view)) return false;
    }
    if (
      q &&
      !m.subject.toLowerCase().includes(q) &&
      !m.sender.toLowerCase().includes(q) &&
      !m.preview.toLowerCase().includes(q)
    ) {
      return false;
    }
    if (input.filter.unread && !m.unread) return false;
    if (input.filter.starred && !m.flagged) return false;
    if (input.filter.hasAttachment && !m.hasAttachment) return false;
    if (input.filter.today && m.dateTs < todayTs) return false;
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
    items: page,
  };
}

export async function openMessage(input: OpenMessageInput): Promise<OpenMessageOutput> {
  if (isTauri()) return tauriInvoke<OpenMessageOutput>("open_message", { input });

  const msg = getMockMessages().find(
    (m) => m.id === input.messageId && m.accountId === input.accountId
  );
  const sessionId = `mock-session-${input.accountId}-${input.messageId}-${Date.now()}`;
  const isThread = (msg?.threadCount ?? 1) > 1;
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
  return {
    applied: true,
    readerSessionId: input.readerSessionId,
    layoutRevision: input.layoutRevision,
  };
}

export async function updateMainPreferences(patch: MainPreferencesPatch): Promise<MainPreferencesDto> {
  if (isTauri()) return tauriInvoke<MainPreferencesDto>("update_main_preferences", { patch });
  Object.assign(mockPrefs, patch);
  return { ...mockPrefs };
}

export async function refreshMail(input: RefreshMailInput): Promise<RefreshMailOutput> {
  if (isTauri()) return tauriInvoke<RefreshMailOutput>("refresh_mail", { input });
  mockStateVersion += 1;
  emitMock("main://sync-status", {
    syncing: true,
    message: "正在刷新演示邮件…",
    lastSyncAt: null,
    stateVersion: mockStateVersion,
  });
  setTimeout(() => {
    const nowSec = Math.floor(Date.now() / 1000);
    emitMock("main://sync-status", {
      syncing: false,
      message: null,
      lastSyncAt: nowSec,
      stateVersion: mockStateVersion,
    });
    emitMock("main://mail-changed", {
      stateVersion: mockStateVersion,
      view: input.view,
      reason: "refresh",
    });
  }, 150);
  return { accepted: true, stateVersion: mockStateVersion };
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

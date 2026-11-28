import {
  ref,
  computed,
  watch,
  provide,
  inject,
  onMounted,
  onUnmounted,
  getCurrentInstance,
  type InjectionKey,
} from "vue";
import type {
  ViewDto,
  FolderSummaryDto,
  AccountSummaryDto,
  MessageSummaryDto,
  OpenMessageOutput,
  ListMessagesOutput,
  MessageSort,
  SearchScope,
  MessageFilterDto,
  ReaderStatusEvent,
  MailChangedEvent,
  MailTemplateDto,
  SyncStatusDto,
  TagItemDto,
  MessageRef,
  ReaderPosition,
  MessageMoveOutput,
  ThreadCardActionEvent,
  ReaderFindResultEvent,
} from "../ipc/types";
import {
  isTauri,
  mainBootstrap,
  listMessages,
  listThreadSummaries,
  openMessage,
  setReaderLayout,
  updateMainPreferences,
  refreshMail,
  unlockApp,
  lockApp,
  setMessageSeen,
  setMessageFlagged,
  allowRemoteImages,
  loadOlderMessages as loadOlderMessagesIpc,
  archiveMessage,
  deleteMessage,
  markMessageSpam,
  unmarkMessageSpam,
  moveMessage,
  undoMessageMove,
  listTags,
  setMessageTags,
  getReaderViewOptions,
  setReaderViewOptions,
  findInReader,
  closeMainWindow,
  quitApp,
  onMailChanged,
  onSyncStatus,
  onReaderStatus,
  onLockState,
  onThreadCardAction,
  onReaderFindResult,
  extractErrorMessage,
  isLockedError,
  normalizeSyncStatusEvent,
  nextReaderLayoutRevision,
} from "../ipc/main-window";
import { usePaneLayout } from "./usePaneLayout";
import { classifyFolderRole, type FolderRoleKind } from "./parityHelpers";

// ---------- 日期分组（对齐 crates/mail-core/src/lib.rs date_group_label） ----------
export function dateGroupLabel(dateTs: number, nowTs: number): string {
  if (dateTs <= 0) return "更早";
  const msgDate = new Date(dateTs * 1000);
  const nowDate = new Date(nowTs * 1000);
  const msgDay = new Date(msgDate.getFullYear(), msgDate.getMonth(), msgDate.getDate());
  const today = new Date(nowDate.getFullYear(), nowDate.getMonth(), nowDate.getDate());
  const deltaDays = Math.round((today.getTime() - msgDay.getTime()) / 86400000);
  if (deltaDays <= 0) return "今天";
  if (deltaDays === 1) return "昨天";
  if (deltaDays <= 6) return "最近 7 天";
  if (deltaDays <= 13) return "最近 14 天";
  return "更早";
}

export const PAGE_SIZE = 50;
export const READER_BODY_TIMEOUT_MS = 15_000;
export const UNDO_TOAST_DURATION_MS = 8_000;

export interface UndoToastState {
  id: number;
  label: string;
  accountId: number;
  messageId: number;
  rfcMessageId: string;
  currentFolderPath: string;
  originalFolderPath: string;
}

export interface MessageGroup {
  label: string;
  items: MessageSummaryDto[];
}

export type VirtualListItem =
  | {
      type: "header";
      key: string;
      label: string;
      count: number;
      collapsed: boolean;
      top: number;
      height: number;
    }
  | {
      type: "row";
      key: string;
      message: MessageSummaryDto;
      groupLabel: string;
      isThreadChild: boolean;
      isLastThreadChild: boolean;
      top: number;
      height: number;
    };

export const LIST_DATE_HEADER_HEIGHT = 28;

/** 计算虚拟列表布局（支持日期分组折叠、会话子行内联展开与精确行键索引） */
export function buildMessageListVirtualLayout(
  groupedMessages: readonly MessageGroup[],
  options: {
    rowHeight: number;
    headerHeight?: number;
    collapsedDayGroups?: ReadonlySet<string>;
    expandedThreadKeys?: ReadonlySet<string>;
    threadChildren?: Readonly<Record<string, readonly MessageSummaryDto[]>>;
  }
): {
  items: VirtualListItem[];
  totalHeight: number;
  rowIndexByKey: Map<string, number>;
} {
  const items: VirtualListItem[] = [];
  const rowIndexByKey = new Map<string, number>();
  const rh = options.rowHeight;
  const hh = options.headerHeight ?? LIST_DATE_HEADER_HEIGHT;
  const collapsed = options.collapsedDayGroups ?? new Set<string>();
  const expandedThreads = options.expandedThreadKeys ?? new Set<string>();
  const threadChildren = options.threadChildren ?? {};
  let top = 0;

  for (const group of groupedMessages) {
    const isGroupCollapsed = Boolean(group.label) && collapsed.has(group.label);
    if (group.label) {
      items.push({
        type: "header",
        key: `hdr:${group.label}`,
        label: group.label,
        count: group.items.length,
        collapsed: isGroupCollapsed,
        top,
        height: hh,
      });
      top += hh;
    }
    if (isGroupCollapsed) {
      continue;
    }
    for (const msg of group.items) {
      const key = `${msg.accountId}:${msg.id}`;
      rowIndexByKey.set(key, items.length);
      items.push({
        type: "row",
        key,
        message: msg,
        groupLabel: group.label,
        isThreadChild: false,
        isLastThreadChild: false,
        top,
        height: rh,
      });
      top += rh;

      if (msg.threadCount > 1 && expandedThreads.has(key)) {
        const children = threadChildren[key] ?? [];
        for (let i = 0; i < children.length; i++) {
          const child = children[i];
          const childKey = `${child.accountId}:${child.id}`;
          if (!rowIndexByKey.has(childKey)) {
            rowIndexByKey.set(childKey, items.length);
          }
          items.push({
            type: "row",
            key: `child:${key}:${childKey}`,
            message: child,
            groupLabel: group.label,
            isThreadChild: true,
            isLastThreadChild: i === children.length - 1,
            top,
            height: rh,
          });
          top += rh;
        }
      }
    }
  }

  return { items, totalHeight: top, rowIndexByKey };
}

/** 查找包含选中邮件但当前处于折叠状态的日期分组名 */
export function resolveSelectedGroupToUnfold(
  selectedId: MessageRef | null,
  groupedMessages: readonly MessageGroup[],
  collapsedDayGroups: ReadonlySet<string>
): string | null {
  if (!selectedId || collapsedDayGroups.size === 0) return null;
  for (const group of groupedMessages) {
    if (!group.label || !collapsedDayGroups.has(group.label)) continue;
    if (
      group.items.some(
        (m) => m.accountId === selectedId.accountId && m.id === selectedId.messageId
      )
    ) {
      return group.label;
    }
  }
  return null;
}

/** 将 YYYY-MM-DD 起止日期字符串转为闭开区间 Unix 秒时间戳 [from, to) */
export function dateInputToUnixRange(
  fromStr: string,
  toStr: string
): { dateFromTs: number | null; dateToTs: number | null } {
  const parseLocalDayStart = (raw: string): number | null => {
    const trimmed = raw.trim();
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(trimmed);
    if (!m) return null;
    const year = Number(m[1]);
    const month = Number(m[2]) - 1;
    const day = Number(m[3]);
    const dt = new Date(year, month, day, 0, 0, 0, 0);
    if (Number.isNaN(dt.getTime())) return null;
    return Math.floor(dt.getTime() / 1000);
  };
  const dateFromTs = parseLocalDayStart(fromStr);
  const endDayStart = parseLocalDayStart(toStr);
  const dateToTs = endDayStart !== null ? endDayStart + 86400 : null;
  return { dateFromTs, dateToTs };
}

export interface AnchorLayoutItem {
  type?: string;
  key: string;
  top: number;
  height: number;
}

/** 按 accountId + messageId 在列表中定位邮件 */
export function findMessageByRef(
  items: readonly MessageSummaryDto[],
  target: MessageRef | null
): MessageSummaryDto | null {
  if (!target) return null;
  return (
    items.find((m) => m.accountId === target.accountId && m.id === target.messageId) ?? null
  );
}

/** 根据前后虚拟列表布局与当前 scrollTop 计算增量刷新后的锚点 scrollTop */
export function computeAnchorScrollTop(
  prevItems: readonly AnchorLayoutItem[],
  nextItems: readonly AnchorLayoutItem[],
  currentScrollTop: number,
  selectedKey?: string | null
): number {
  if (currentScrollTop <= 0 || nextItems.length === 0) return 0;

  const nextByKey = new Map<string, AnchorLayoutItem>();
  for (const item of nextItems) {
    nextByKey.set(item.key, item);
  }

  for (const prev of prevItems) {
    if (prev.type === "header") continue;
    if (prev.top + prev.height <= currentScrollTop) continue;
    if (prev.top > currentScrollTop + 600) break;
    const matched = nextByKey.get(prev.key);
    if (matched) {
      return Math.max(0, Math.round(matched.top + (currentScrollTop - prev.top)));
    }
  }

  if (selectedKey) {
    const selectedNext = nextByKey.get(selectedKey);
    const selectedPrev = prevItems.find((it) => it.key === selectedKey);
    if (selectedNext && selectedPrev) {
      return Math.max(0, Math.round(selectedNext.top + (currentScrollTop - selectedPrev.top)));
    }
  }

  const maxTop = Math.max(
    0,
    nextItems[nextItems.length - 1].top + nextItems[nextItems.length - 1].height
  );
  return Math.min(currentScrollTop, maxTop);
}

/** 分批合并纯函数（供组件与单元测试直接复用） */
export function mergePagedMessages(
  existing: readonly MessageSummaryDto[],
  incoming: readonly MessageSummaryDto[],
  reset: boolean
): MessageSummaryDto[] {
  if (reset) return [...incoming];
  const seen = new Set(existing.map((m) => `${m.accountId}:${m.id}`));
  const appended: MessageSummaryDto[] = [];
  for (const item of incoming) {
    const key = `${item.accountId}:${item.id}`;
    if (!seen.has(key)) {
      seen.add(key);
      appended.push(item);
    }
  }
  return [...existing, ...appended];
}

/** 校验列表分页响应是否属于当前查询代次与期望偏移（防乱序/过期） */
export function shouldApplyListResponse(
  requestEpoch: number,
  activeEpoch: number,
  expectedRequestId: string,
  expectedOffset: number,
  response: Pick<ListMessagesOutput, "requestId" | "offset">
): boolean {
  return (
    requestEpoch === activeEpoch &&
    response.requestId === expectedRequestId &&
    response.offset === expectedOffset
  );
}

/** 校验正文打开响应是否属于当前选信代次、选中邮件与可见阅读窗格 */
export function shouldApplyOpenResponse(
  requestEpoch: number,
  activeEpoch: number,
  expectedRequestId: string,
  selected: MessageRef | null,
  response: Pick<OpenMessageOutput, "requestId" | "accountId" | "messageId">,
  readerPosition: ReaderPosition
): boolean {
  return (
    requestEpoch === activeEpoch &&
    response.requestId === expectedRequestId &&
    selected !== null &&
    selected.accountId === response.accountId &&
    selected.messageId === response.messageId &&
    readerPosition !== "off"
  );
}

export function useMainWindow() {
  // --- 状态 ---
  const loading = ref(true);
  const error = ref<string | null>(null);
  const mode = ref<"demo" | "live">("demo");
  const locked = ref(false);
  const hasPasscode = ref(false);
  const cacheReady = ref(true);
  const lockedReason = ref<string | null>(null);
  const unlockSubmitting = ref(false);
  const unlockError = ref<string | null>(null);
  const unlockRetryAfterSecs = ref(0);
  const stateVersion = ref(0);
  const accounts = ref<AccountSummaryDto[]>([]);
  const folders = ref<FolderSummaryDto[]>([]);
  const tags = ref<TagItemDto[]>([]);
  const templates = ref<MailTemplateDto[]>([]);
  const currentView = ref<ViewDto>({ kind: "unifiedInbox" });
  const syncStatus = ref<SyncStatusDto>({
    syncing: false,
    message: null,
    lastSyncAt: null,
  });

  // 布局
  const layout = usePaneLayout();

  // 列表
  const messages = ref<MessageSummaryDto[]>([]);
  const listTotal = ref(0);
  const listLoading = ref(false);
  const listError = ref<string | null>(null);
  const hasMore = ref(false);
  const scrollResetToken = ref(0);

  // 查询状态与展开/折叠状态
  const searchQuery = ref("");
  const searchScope = ref<SearchScope>("thisFolder");
  const sortMode = ref<MessageSort>("dateNewest");
  const filterSet = ref<MessageFilterDto>({});
  const expandedThreadKeys = ref<Set<string>>(new Set());
  const threadChildren = ref<Record<string, MessageSummaryDto[]>>({});
  const collapsedDayGroups = ref<Set<string>>(new Set());

  // 请求代次防乱序
  let queryEpoch = 0;
  let inFlightOffset = -1;
  let pendingMailChangedReload = false;
  let foldersSyncPromise: Promise<void> | null = null;
  let foldersSyncQueued = false;
  let openEpoch = 0;
  let activeOpenRequestId = "";
  let lastReaderSessionId = "";
  let readerBodyTimer: ReturnType<typeof setTimeout> | null = null;
  let unlockCountdownTimer: ReturnType<typeof setInterval> | null = null;
  let undoToastTimer: ReturnType<typeof setTimeout> | null = null;
  let undoToastSeq = 0;

  // 选中状态
  const selectedId = ref<MessageRef | null>(null);
  const openedMessage = ref<OpenMessageOutput | null>(null);
  const readerLoading = ref(false);
  const readerError = ref<string | null>(null);
  const readerLayoutApplied = ref<boolean | undefined>(undefined);
  const readerLayoutVisible = ref<boolean | undefined>(undefined);
  const loadingOlder = ref(false);
  const allowingRemoteImages = ref(false);
  const movingMessage = ref(false);
  const undoToast = ref<UndoToastState | null>(null);
  const undoPending = ref(false);
  const availableTags = ref<TagItemDto[]>([]);
  const perMessageDark = ref<boolean | null>(null);
  const readingView = ref(false);
  const readerFindOpen = ref(false);
  const readerFindQuery = ref("");
  const readerFindCurrent = ref(0);
  const readerFindTotal = ref(0);
  const readerFindStepToken = ref(0);
  const readerFindStepDelta = ref(1);
  let threadCardActionHandler: ((e: ThreadCardActionEvent) => void) | null = null;

  // 解订阅句柄
  const unlisteners: Array<() => void> = [];

  // --- 派生 ---
  const selectedMessage = computed(() => findMessageByRef(messages.value, selectedId.value));
  const selectedUnread = computed(() => selectedMessage.value?.unread ?? false);
  const selectedTags = computed<string[]>(() => selectedMessage.value?.tags ?? []);
  const openedAccountEmail = computed(() => {
    const msg = openedMessage.value;
    if (!msg) return "";
    return accounts.value.find((acc) => acc.id === msg.accountId)?.email ?? "";
  });

  const currentFolderHasMoreRemote = computed(() => {
    const view = currentView.value;
    if (view.kind === "accountFolder") {
      const folder = folders.value.find(
        (f) => f.accountId === view.accountId && f.id === view.folderId
      );
      return Boolean(folder?.hasMoreRemote);
    }
    const roleMap: Partial<Record<ViewDto["kind"], FolderRoleKind>> = {
      unifiedInbox: "inbox",
      sent: "sent",
      drafts: "drafts",
      archive: "archive",
      junk: "junk",
      trash: "trash",
    };
    const targetRole = roleMap[view.kind];
    if (!targetRole) return false;
    return folders.value.some(
      (f) => classifyFolderRole(f.path, f.name) === targetRole && f.hasMoreRemote
    );
  });

  const groupedMessages = computed<MessageGroup[]>(() => {
    if (!layout.prefs.value.listGroupByDate) {
      return [{ label: "", items: messages.value }];
    }
    const now = Date.now() / 1000;
    const groups: Map<string, MessageSummaryDto[]> = new Map();
    for (const msg of messages.value) {
      const label = dateGroupLabel(msg.dateTs, now);
      let bucket = groups.get(label);
      if (!bucket) {
        bucket = [];
        groups.set(label, bucket);
      }
      bucket.push(msg);
    }
    const order = ["今天", "昨天", "最近 7 天", "最近 14 天", "更早"];
    return order
      .filter((l) => groups.has(l))
      .map((l) => ({ label: l, items: groups.get(l)! }));
  });

  function clearReaderBodyTimer() {
    if (readerBodyTimer !== null) {
      clearTimeout(readerBodyTimer);
      readerBodyTimer = null;
    }
  }

  function clearUnlockCountdown() {
    if (unlockCountdownTimer !== null) {
      clearInterval(unlockCountdownTimer);
      unlockCountdownTimer = null;
    }
  }

  function clearUndoToastTimer() {
    if (undoToastTimer !== null) {
      clearTimeout(undoToastTimer);
      undoToastTimer = null;
    }
  }

  function dismissUndoToast() {
    clearUndoToastTimer();
    undoToast.value = null;
  }

  function startUnlockCooldown(seconds: number) {
    clearUnlockCountdown();
    const initial = Math.max(0, Math.ceil(seconds));
    unlockRetryAfterSecs.value = initial;
    if (initial <= 0) return;
    unlockCountdownTimer = setInterval(() => {
      if (unlockRetryAfterSecs.value <= 1) {
        unlockRetryAfterSecs.value = 0;
        clearUnlockCountdown();
      } else {
        unlockRetryAfterSecs.value -= 1;
      }
    }, 1000);
  }

  function hideNativeReaderOnLock() {
    const sessionId = openedMessage.value?.readerSessionId || lastReaderSessionId;
    lastReaderSessionId = "";
    if (!sessionId) return;
    void setReaderLayout({
      readerSessionId: sessionId,
      layoutRevision: nextReaderLayoutRevision(),
      contentRect: { x: 0, y: 0, width: 0, height: 0 },
      clipRect: { x: 0, y: 0, width: 0, height: 0 },
      visible: false,
      devicePixelRatio: typeof window !== "undefined" ? window.devicePixelRatio : 1,
    }).catch(() => {
      /* 忽略非 Tauri 或过期会话拒绝 */
    });
  }

  function clearSensitiveMailState() {
    clearReaderBodyTimer();
    clearUndoToastTimer();
    undoToast.value = null;
    undoPending.value = false;
    loadingOlder.value = false;
    allowingRemoteImages.value = false;
    movingMessage.value = false;
    queryEpoch += 1;
    inFlightOffset = -1;
    pendingMailChangedReload = false;
    foldersSyncQueued = false;
    openEpoch += 1;
    activeOpenRequestId = "";
    accounts.value = [];
    folders.value = [];
    tags.value = [];
    templates.value = [];
    messages.value = [];
    listTotal.value = 0;
    hasMore.value = false;
    listLoading.value = false;
    listError.value = null;
    selectedId.value = null;
    openedMessage.value = null;
    readerLoading.value = false;
    readerError.value = null;
    availableTags.value = [];
    perMessageDark.value = null;
    readingView.value = false;
    readerFindOpen.value = false;
    readerFindQuery.value = "";
    readerFindCurrent.value = 0;
    readerFindTotal.value = 0;
    searchQuery.value = "";
    filterSet.value = {};
    expandedThreadKeys.value = new Set();
    threadChildren.value = {};
    collapsedDayGroups.value = new Set();
    layout.clearOverlays();
  }

  function enterLockedState(reason?: string | null, nextHasPasscode?: boolean) {
    hideNativeReaderOnLock();
    locked.value = true;
    if (nextHasPasscode !== undefined) {
      hasPasscode.value = nextHasPasscode;
    }
    if (reason !== undefined) {
      lockedReason.value = reason ?? null;
    }
    clearSensitiveMailState();
  }

  function handleIpcError(err: unknown): boolean {
    if (isLockedError(err)) {
      enterLockedState("locked");
      return true;
    }
    return false;
  }

  // --- 加载列表 ---
  async function loadMessages(resetList = true, options?: { preserveAnchor?: boolean }) {
    if (locked.value) return;

    const preserveAnchor = Boolean(resetList && options?.preserveAnchor);

    if (resetList) {
      queryEpoch += 1;
      inFlightOffset = -1;
      pendingMailChangedReload = false;
      if (!preserveAnchor) {
        messages.value = [];
        listTotal.value = 0;
        hasMore.value = false;
        scrollResetToken.value += 1;
      }
    }

    const epoch = queryEpoch;
    const offset = resetList ? 0 : messages.value.length;
    const limit = preserveAnchor
      ? Math.min(500, Math.max(PAGE_SIZE, messages.value.length))
      : PAGE_SIZE;

    if (!resetList && inFlightOffset === offset) return;
    inFlightOffset = offset;
    const requestId = `list-${epoch}-${offset}`;
    listLoading.value = true;
    listError.value = null;

    try {
      const result = await listMessages({
        requestId,
        view: currentView.value,
        query: searchQuery.value,
        scope: searchScope.value,
        sort: sortMode.value,
        filter: filterSet.value,
        offset,
        limit,
      });

      // 严格按代次与偏移校验，丢弃过期或乱序响应
      if (!locked.value && shouldApplyListResponse(epoch, queryEpoch, requestId, offset, result)) {
        messages.value = mergePagedMessages(messages.value, result.items, resetList);
        listTotal.value = result.total;
        hasMore.value = result.hasMore;
        stateVersion.value = Math.max(stateVersion.value, result.stateVersion);
        if (result.total > 0 || result.items.length > 0) {
          cacheReady.value = true;
        }
        if (preserveAnchor && openedMessage.value) {
          const updatedSelected = findMessageByRef(result.items, {
            accountId: openedMessage.value.accountId,
            messageId: openedMessage.value.messageId,
          });
          if (updatedSelected && updatedSelected.flagged !== openedMessage.value.flagged) {
            openedMessage.value = {
              ...openedMessage.value,
              flagged: updatedSelected.flagged,
            };
          }
        }
      }
    } catch (e) {
      if (handleIpcError(e)) return;
      if (epoch === queryEpoch) {
        listError.value = extractErrorMessage(e);
      }
    } finally {
      if (epoch === queryEpoch) {
        listLoading.value = false;
        inFlightOffset = -1;
        if (pendingMailChangedReload && !locked.value) {
          pendingMailChangedReload = false;
          void loadMessages(true, { preserveAnchor: true });
        }
      }
    }
  }

  function loadMore() {
    if (!listLoading.value && hasMore.value && !locked.value) {
      loadMessages(false);
    }
  }

  async function loadOlderMessages(limit = PAGE_SIZE) {
    if (locked.value || loadingOlder.value || !currentFolderHasMoreRemote.value) return;
    loadingOlder.value = true;
    listError.value = null;
    try {
      const res = await loadOlderMessagesIpc({ view: currentView.value, limit });
      if (locked.value) return;
      stateVersion.value = Math.max(stateVersion.value, res.stateVersion);
      if (!res.hasMoreRemote) {
        const view = currentView.value;
        folders.value = folders.value.map((f) => {
          if (view.kind === "accountFolder") {
            return f.accountId === view.accountId && f.id === view.folderId
              ? { ...f, hasMoreRemote: false }
              : f;
          }
          if (view.kind === "unifiedInbox" && classifyFolderRole(f.path, f.name) === "inbox") {
            return { ...f, hasMoreRemote: false };
          }
          return f;
        });
      }
      await syncFoldersMetadata();
      await loadMessages(true, { preserveAnchor: true });
    } catch (e) {
      if (handleIpcError(e)) return;
      listError.value = extractErrorMessage(e);
    } finally {
      loadingOlder.value = false;
    }
  }

  // --- 视图切换 ---
  function selectView(view: ViewDto) {
    currentView.value = view;
    selectedId.value = null;
    openedMessage.value = null;
    readerError.value = null;
    readerLoading.value = false;
    openEpoch += 1;
    activeOpenRequestId = "";
    searchQuery.value = "";
    searchScope.value = "thisFolder";
    filterSet.value = {};
    expandedThreadKeys.value = new Set();
    loadMessages(true);
  }

  /** 侧栏规则文件夹 / 标签 / 账号子项快捷筛选（单次刷新，避免重复请求） */
  function applySidebarFilter(options: {
    view?: ViewDto;
    query?: string;
    scope?: SearchScope;
    filter?: MessageFilterDto;
  }) {
    if (options.view) {
      currentView.value = options.view;
    }
    selectedId.value = null;
    openedMessage.value = null;
    readerError.value = null;
    readerLoading.value = false;
    openEpoch += 1;
    activeOpenRequestId = "";
    searchQuery.value = options.query ?? "";
    searchScope.value = options.scope ?? "thisFolder";
    filterSet.value = options.filter ? { ...options.filter } : {};
    expandedThreadKeys.value = new Set();
    loadMessages(true);
  }

  // --- 日期分组折叠 & 会话子行展开 ---
  function toggleDayGroup(label: string) {
    if (!label) return;
    const next = new Set(collapsedDayGroups.value);
    if (next.has(label)) {
      next.delete(label);
    } else {
      next.add(label);
    }
    collapsedDayGroups.value = next;
  }

  function collapseAllDayGroups() {
    const next = new Set<string>();
    for (const g of groupedMessages.value) {
      if (g.label) next.add(g.label);
    }
    collapsedDayGroups.value = next;
  }

  function expandAllDayGroups() {
    if (collapsedDayGroups.value.size > 0) {
      collapsedDayGroups.value = new Set();
    }
  }

  async function toggleThreadExpand(msg: MessageSummaryDto) {
    if (locked.value || msg.threadCount <= 1) return;
    const key = `${msg.accountId}:${msg.id}`;
    const next = new Set(expandedThreadKeys.value);
    if (next.has(key)) {
      next.delete(key);
      expandedThreadKeys.value = next;
      return;
    }
    next.add(key);
    expandedThreadKeys.value = next;

    if (!threadChildren.value[key]) {
      try {
        const res = await listThreadSummaries({
          accountId: msg.accountId,
          messageId: msg.id,
          subject: msg.subject,
        });
        if (!locked.value) {
          threadChildren.value = {
            ...threadChildren.value,
            [key]: res.items,
          };
        }
      } catch (e) {
        if (handleIpcError(e)) return;
      }
    }
  }

  // --- 搜索/筛选/排序 ---
  function setSearch(q: string) {
    if (searchQuery.value === q) return;
    searchQuery.value = q;
    loadMessages(true);
  }

  function setFilter(f: MessageFilterDto) {
    filterSet.value = { ...f };
    loadMessages(true);
  }

  function setSort(s: MessageSort) {
    if (sortMode.value === s) return;
    sortMode.value = s;
    loadMessages(true);
  }

  function setScope(s: SearchScope) {
    if (searchScope.value === s) return;
    searchScope.value = s;
    if (searchQuery.value.trim().length > 0) {
      loadMessages(true);
    }
  }

  // --- 刷新 ---
  async function loadTagsCatalog() {
    if (locked.value) return;
    try {
      const list = await listTags();
      availableTags.value = list;
      if (list.length > 0 || tags.value.length === 0) {
        tags.value = list;
      }
    } catch (e) {
      if (handleIpcError(e)) return;
    }
  }

  async function refresh() {
    if (locked.value) return;
    try {
      await refreshMail({ view: currentView.value });
      await Promise.all([loadMessages(true, { preserveAnchor: true }), loadTagsCatalog()]);
    } catch (e) {
      if (handleIpcError(e)) return;
      listError.value = extractErrorMessage(e);
    }
  }

  function scheduleReaderBodyTimeout(epoch: number, sessionId: string) {
    clearReaderBodyTimer();
    readerBodyTimer = setTimeout(() => {
      readerBodyTimer = null;
      if (
        !locked.value &&
        epoch === openEpoch &&
        openedMessage.value?.readerSessionId === sessionId &&
        openedMessage.value.bodyStatus === "loading"
      ) {
        openedMessage.value = {
          ...openedMessage.value,
          bodyStatus: "unavailable",
        };
        readerLoading.value = false;
        readerError.value = "正文加载超时，请点击重试";
      }
    }, READER_BODY_TIMEOUT_MS);
  }

  function isOpenedMessageNative(): boolean {
    const msg = openedMessage.value;
    if (!msg || !isTauri() || msg.bodyStatus !== "ready") return false;
    return msg.hasHtml || msg.threadMessages.length > 1;
  }

  async function syncReaderViewOptions(accountId: number, messageId: number) {
    try {
      const res = await getReaderViewOptions({ accountId, messageId });
      if (
        !locked.value &&
        openedMessage.value?.accountId === accountId &&
        openedMessage.value?.messageId === messageId
      ) {
        perMessageDark.value = res.perMessageDark;
        readingView.value = res.readingView;
      }
    } catch (e) {
      handleIpcError(e);
    }
  }

  // --- 打开邮件正文 ---
  async function fetchOpenedMessage(target: MessageRef) {
    if (locked.value || layout.prefs.value.readerPosition === "off") {
      return;
    }

    clearReaderBodyTimer();
    const epoch = ++openEpoch;
    const requestId = `open-${epoch}-${target.accountId}-${target.messageId}`;
    activeOpenRequestId = requestId;
    openedMessage.value = null;
    perMessageDark.value = null;
    readerFindCurrent.value = 0;
    readerFindTotal.value = 0;
    readerLoading.value = true;
    readerError.value = null;
    readerLayoutApplied.value = undefined;
    readerLayoutVisible.value = undefined;

    try {
      const result = await openMessage({
        requestId,
        accountId: target.accountId,
        messageId: target.messageId,
      });

      if (
        !locked.value &&
        shouldApplyOpenResponse(
          epoch,
          openEpoch,
          activeOpenRequestId,
          selectedId.value,
          result,
          layout.prefs.value.readerPosition
        )
      ) {
        lastReaderSessionId = result.readerSessionId;
        openedMessage.value = result;
        readerLoading.value = result.bodyStatus === "loading";
        void syncReaderViewOptions(result.accountId, result.messageId);
        if (result.bodyStatus === "loading") {
          scheduleReaderBodyTimeout(epoch, result.readerSessionId);
        } else if (result.bodyStatus === "unavailable") {
          readerError.value = "正文暂不可用，请点击重试";
        } else if (readerFindOpen.value && readerFindQuery.value.trim()) {
          void runReaderFind(readerFindQuery.value);
        }
      }
    } catch (e) {
      if (handleIpcError(e)) return;
      if (epoch === openEpoch) {
        readerError.value = extractErrorMessage(e);
        readerLoading.value = false;
      }
    }
  }

  // --- 选邮件（点击行与键盘共用） ---
  function selectMessage(target: MessageRef | null, forceReload = false) {
    if (!target) {
      clearReaderBodyTimer();
      openEpoch += 1;
      activeOpenRequestId = "";
      selectedId.value = null;
      openedMessage.value = null;
      perMessageDark.value = null;
      readerFindCurrent.value = 0;
      readerFindTotal.value = 0;
      readerLoading.value = false;
      readerError.value = null;
      readerLayoutApplied.value = undefined;
      readerLayoutVisible.value = undefined;
      return;
    }

    const isSame =
      selectedId.value?.messageId === target.messageId &&
      selectedId.value?.accountId === target.accountId;

    if (isSame && !forceReload) return;
    selectedId.value = { accountId: target.accountId, messageId: target.messageId };

    if (layout.prefs.value.readerPosition === "off") {
      clearReaderBodyTimer();
      openedMessage.value = null;
      readerLoading.value = false;
      readerError.value = null;
      readerLayoutApplied.value = undefined;
      readerLayoutVisible.value = undefined;
      return;
    }

    void fetchOpenedMessage(target);
  }

  function retryOpen() {
    if (selectedId.value) {
      selectMessage(selectedId.value, true);
    }
  }

  // --- 已读 / 星标切换 ---
  function applyFolderUnreadDelta(accountId: number, folderId: number, delta: number) {
    if (delta === 0) return;
    folders.value = folders.value.map((f) => {
      if (f.accountId === accountId && f.id === folderId) {
        return { ...f, unreadCount: Math.max(0, f.unreadCount + delta) };
      }
      return f;
    });
  }

  async function toggleMessageSeen(target: MessageRef, explicitSeen?: boolean) {
    if (locked.value) return;
    const current = findMessageByRef(messages.value, target);
    const prevUnread = current?.unread;
    const folderIdForDelta = current?.folderId;
    const seen = explicitSeen ?? Boolean(prevUnread ?? true);
    try {
      const res = await setMessageSeen({
        accountId: target.accountId,
        messageId: target.messageId,
        seen,
      });
      if (locked.value) return;
      stateVersion.value = Math.max(stateVersion.value, res.stateVersion);
      messages.value = messages.value.map((m) =>
        m.accountId === res.accountId && m.id === res.messageId
          ? { ...m, unread: res.unread }
          : m
      );
      if (
        prevUnread !== undefined &&
        prevUnread !== res.unread &&
        folderIdForDelta !== undefined
      ) {
        applyFolderUnreadDelta(res.accountId, folderIdForDelta, res.unread ? 1 : -1);
      }
    } catch (e) {
      if (handleIpcError(e)) return;
      listError.value = extractErrorMessage(e);
    }
  }

  async function toggleMessageFlagged(target: MessageRef, explicitFlagged?: boolean) {
    if (locked.value) return;
    const current = findMessageByRef(messages.value, target);
    const currentFlagged =
      current?.flagged ??
      (openedMessage.value?.accountId === target.accountId &&
      openedMessage.value?.messageId === target.messageId
        ? openedMessage.value.flagged
        : false);
    const flagged = explicitFlagged ?? !currentFlagged;
    try {
      const res = await setMessageFlagged({
        accountId: target.accountId,
        messageId: target.messageId,
        flagged,
      });
      if (locked.value) return;
      stateVersion.value = Math.max(stateVersion.value, res.stateVersion);
      messages.value = messages.value.map((m) =>
        m.accountId === res.accountId && m.id === res.messageId
          ? { ...m, flagged: res.flagged }
          : m
      );
      if (
        openedMessage.value &&
        openedMessage.value.accountId === res.accountId &&
        openedMessage.value.messageId === res.messageId
      ) {
        openedMessage.value = {
          ...openedMessage.value,
          flagged: res.flagged,
        };
      }
    } catch (e) {
      if (handleIpcError(e)) return;
      listError.value = extractErrorMessage(e);
    }
  }

  // --- 远程图片放行 ---
  async function allowOpenedRemoteImages(trustSender: boolean) {
    const current = openedMessage.value;
    if (locked.value || !current || allowingRemoteImages.value) return;
    allowingRemoteImages.value = true;
    try {
      const res = await allowRemoteImages({
        accountId: current.accountId,
        messageId: current.messageId,
        trustSender,
      });
      if (locked.value) return;
      stateVersion.value = Math.max(stateVersion.value, res.stateVersion);

      const stillSame =
        openedMessage.value?.accountId === current.accountId &&
        openedMessage.value?.messageId === current.messageId;
      if (!stillSame || !openedMessage.value) return;

      openedMessage.value = {
        ...openedMessage.value,
        remoteImagesBlocked: false,
      };

      if (!isTauri()) {
        const refreshed = await openMessage({
          requestId: `open-remote-${++openEpoch}-${current.accountId}-${current.messageId}`,
          accountId: current.accountId,
          messageId: current.messageId,
        });
        if (
          !locked.value &&
          openedMessage.value?.accountId === current.accountId &&
          openedMessage.value?.messageId === current.messageId
        ) {
          openedMessage.value = {
            ...refreshed,
            remoteImagesBlocked: false,
          };
        }
      }
    } catch (e) {
      if (handleIpcError(e)) return;
      readerError.value = extractErrorMessage(e);
    } finally {
      allowingRemoteImages.value = false;
    }
  }

  // --- 归档 / 删除 / 标垃圾 / 移动 / 撤销 ---
  function computeAdjacentTarget(target: MessageRef): MessageRef | null {
    const idx = messages.value.findIndex(
      (m) => m.accountId === target.accountId && m.id === target.messageId
    );
    if (idx < 0) return null;
    if (idx + 1 < messages.value.length) {
      const next = messages.value[idx + 1];
      return { accountId: next.accountId, messageId: next.id };
    }
    if (idx > 0) {
      const prev = messages.value[idx - 1];
      return { accountId: prev.accountId, messageId: prev.id };
    }
    return null;
  }

  function showUndoToast(label: string, res: MessageMoveOutput) {
    clearUndoToastTimer();
    undoToastSeq += 1;
    undoToast.value = {
      id: undoToastSeq,
      label,
      accountId: res.accountId,
      messageId: res.messageId,
      rfcMessageId: res.rfcMessageId,
      currentFolderPath: res.targetFolderPath,
      originalFolderPath: res.originalFolderPath,
    };
    undoToastTimer = setTimeout(() => {
      undoToastTimer = null;
      undoToast.value = null;
    }, UNDO_TOAST_DURATION_MS);
  }

  function resolveFolderDisplayName(accountId: number, folderPath: string): string {
    const matched = folders.value.find(
      (f) => f.accountId === accountId && f.path.toLowerCase() === folderPath.toLowerCase()
    );
    return matched?.name ?? folderPath;
  }

  async function performMoveAction(
    kind: "archive" | "delete" | "spam" | "unspam" | "move",
    target: MessageRef,
    targetFolderPath?: string
  ): Promise<MessageMoveOutput | null> {
    if (locked.value || movingMessage.value) return null;
    movingMessage.value = true;
    const nextTarget = computeAdjacentTarget(target);
    const wasSelected =
      selectedId.value?.accountId === target.accountId &&
      selectedId.value?.messageId === target.messageId;

    try {
      let res: MessageMoveOutput;
      if (kind === "archive") {
        res = await archiveMessage(target);
      } else if (kind === "delete") {
        res = await deleteMessage(target);
      } else if (kind === "spam") {
        res = await markMessageSpam(target);
      } else if (kind === "unspam") {
        res = await unmarkMessageSpam(target);
      } else {
        if (!targetFolderPath) return null;
        res = await moveMessage({ ...target, targetFolderPath });
      }
      if (locked.value) return null;

      stateVersion.value = Math.max(stateVersion.value, res.stateVersion);
      const leavesCurrentView =
        res.originalFolderPath !== res.targetFolderPath && currentView.value.kind !== "starred";

      if (leavesCurrentView) {
        messages.value = messages.value.filter(
          (m) => !(m.accountId === res.accountId && m.id === res.messageId)
        );
        listTotal.value = Math.max(0, listTotal.value - 1);
      }

      if (wasSelected && leavesCurrentView) {
        selectMessage(nextTarget);
      }

      const label =
        kind === "archive"
          ? "已归档邮件"
          : kind === "delete"
            ? "已移至已删除"
            : kind === "spam"
              ? "已标记为垃圾邮件"
              : kind === "unspam"
                ? "已移出垃圾邮件"
                : `已移动到 ${resolveFolderDisplayName(res.accountId, res.targetFolderPath)}`;
      showUndoToast(label, res);

      await syncFoldersMetadata();
      await loadMessages(true, { preserveAnchor: true });
      return res;
    } catch (e) {
      if (handleIpcError(e)) return null;
      listError.value = extractErrorMessage(e);
      return null;
    } finally {
      movingMessage.value = false;
    }
  }

  function archiveMessageByRef(target: MessageRef) {
    return performMoveAction("archive", target);
  }

  function deleteMessageByRef(target: MessageRef) {
    return performMoveAction("delete", target);
  }

  function markMessageSpamByRef(target: MessageRef) {
    return performMoveAction("spam", target);
  }

  function moveMessageByRef(target: MessageRef, targetFolderPath: string) {
    return performMoveAction("move", target, targetFolderPath);
  }

  function archiveSelectedMessage() {
    if (!selectedId.value) return Promise.resolve(null);
    return performMoveAction("archive", selectedId.value);
  }

  function deleteSelectedMessage() {
    if (!selectedId.value) return Promise.resolve(null);
    return performMoveAction("delete", selectedId.value);
  }

  function markSelectedSpam() {
    if (!selectedId.value) return Promise.resolve(null);
    return performMoveAction("spam", selectedId.value);
  }

  function unmarkSelectedSpam() {
    if (!selectedId.value) return Promise.resolve(null);
    return performMoveAction("unspam", selectedId.value);
  }

  function moveSelectedMessage(targetFolderPath: string) {
    if (!selectedId.value) return Promise.resolve(null);
    return performMoveAction("move", selectedId.value, targetFolderPath);
  }

  // --- 标签 / 单封邮件亮暗 / 阅读视图 / 正文查找 ---
  async function setOpenedMessageTags(tags: string[]) {
    const target = selectedId.value ?? (openedMessage.value
      ? { accountId: openedMessage.value.accountId, messageId: openedMessage.value.messageId }
      : null);
    if (locked.value || !target) return null;
    try {
      const res = await setMessageTags({
        accountId: target.accountId,
        messageId: target.messageId,
        tags,
      });
      if (locked.value) return null;
      stateVersion.value = Math.max(stateVersion.value, res.stateVersion);
      messages.value = messages.value.map((m) =>
        m.accountId === res.accountId && m.id === res.messageId
          ? { ...m, tags: [...res.tags] }
          : m
      );
      return res;
    } catch (e) {
      if (handleIpcError(e)) return null;
      listError.value = extractErrorMessage(e);
      return null;
    }
  }

  async function togglePerMessageDark() {
    const msg = openedMessage.value;
    if (locked.value || !msg) return null;
    const effective = perMessageDark.value ?? layout.effectiveDark.value;
    const nextDark = perMessageDark.value === null ? !effective : null;
    try {
      const res = await setReaderViewOptions({
        accountId: msg.accountId,
        messageId: msg.messageId,
        perMessageDark: nextDark,
      });
      if (!locked.value) {
        perMessageDark.value = res.perMessageDark;
        readingView.value = res.readingView;
      }
      return res;
    } catch (e) {
      if (handleIpcError(e)) return null;
      readerError.value = extractErrorMessage(e);
      return null;
    }
  }

  async function toggleReadingView() {
    const msg = openedMessage.value;
    if (locked.value || !msg) return null;
    const nextReading = !readingView.value;
    try {
      const res = await setReaderViewOptions({
        accountId: msg.accountId,
        messageId: msg.messageId,
        readingView: nextReading,
      });
      if (!locked.value) {
        perMessageDark.value = res.perMessageDark;
        readingView.value = res.readingView;
      }
      return res;
    } catch (e) {
      if (handleIpcError(e)) return null;
      readerError.value = extractErrorMessage(e);
      return null;
    }
  }

  function openReaderFind() {
    readerFindOpen.value = true;
  }

  function closeReaderFind() {
    readerFindOpen.value = false;
    readerFindQuery.value = "";
    readerFindCurrent.value = 0;
    readerFindTotal.value = 0;
    const msg = openedMessage.value;
    if (msg && isOpenedMessageNative()) {
      void findInReader({
        readerSessionId: msg.readerSessionId,
        query: "",
      }).catch(() => {});
    }
  }

  function toggleReaderFind() {
    if (readerFindOpen.value) {
      closeReaderFind();
    } else {
      openReaderFind();
    }
  }

  async function runReaderFind(query: string, step?: number) {
    readerFindQuery.value = query;
    const msg = openedMessage.value;
    if (!msg || locked.value) {
      readerFindCurrent.value = 0;
      readerFindTotal.value = 0;
      return;
    }
    if (isOpenedMessageNative()) {
      try {
        await findInReader({
          readerSessionId: msg.readerSessionId,
          query,
          ...(step !== undefined ? { step } : {}),
        });
      } catch (e) {
        if (handleIpcError(e)) return;
      }
      return;
    }
    if (step !== undefined) {
      readerFindStepDelta.value = step;
      readerFindStepToken.value += 1;
    }
  }

  function updateFallbackFindResult(current: number, total: number) {
    readerFindCurrent.value = current;
    readerFindTotal.value = total;
  }

  function registerThreadCardActionHandler(
    handler: ((e: ThreadCardActionEvent) => void) | null
  ) {
    threadCardActionHandler = handler;
  }

  async function closeWindow() {
    try {
      await closeMainWindow();
    } catch (e) {
      error.value = extractErrorMessage(e);
    }
  }

  async function undoLastMove(): Promise<boolean> {
    const toast = undoToast.value;
    if (!toast || locked.value || undoPending.value) return false;
    clearUndoToastTimer();
    undoPending.value = true;
    try {
      const res = await undoMessageMove({
        accountId: toast.accountId,
        messageId: toast.messageId,
        rfcMessageId: toast.rfcMessageId,
        currentFolderPath: toast.currentFolderPath,
        originalFolderPath: toast.originalFolderPath,
      });
      if (locked.value) return false;
      stateVersion.value = Math.max(stateVersion.value, res.stateVersion);
      undoToast.value = null;
      await syncFoldersMetadata();
      await loadMessages(true, { preserveAnchor: true });
      return res.accepted;
    } catch (e) {
      if (handleIpcError(e)) return false;
      listError.value = extractErrorMessage(e);
      return false;
    } finally {
      undoPending.value = false;
    }
  }

  // --- 锁屏 / 解锁 / 退出 ---
  async function lock() {
    try {
      const res = await lockApp();
      if (res.locked) {
        enterLockedState("manual");
      }
    } catch (e) {
      error.value = extractErrorMessage(e);
    }
  }

  async function unlock(passcode: string): Promise<boolean> {
    if (unlockSubmitting.value || unlockRetryAfterSecs.value > 0) return false;
    if (!passcode) {
      unlockError.value = "请输入锁屏密码";
      return false;
    }
    unlockSubmitting.value = true;
    unlockError.value = null;
    try {
      const res = await unlockApp({ passcode });
      if (res.unlocked && !res.locked) {
        clearUnlockCountdown();
        unlockRetryAfterSecs.value = 0;
        unlockError.value = null;
        locked.value = false;
        lockedReason.value = null;
        await init();
        return true;
      }
      unlockError.value = res.error ?? "锁屏密码错误";
      if (res.retryAfterSecs && res.retryAfterSecs > 0) {
        startUnlockCooldown(res.retryAfterSecs);
      }
      return false;
    } catch (e) {
      unlockError.value = extractErrorMessage(e);
      return false;
    } finally {
      unlockSubmitting.value = false;
    }
  }

  async function quit() {
    try {
      await quitApp();
    } catch (e) {
      error.value = extractErrorMessage(e);
    }
  }

  // 当从 "off" 切回 "right" 或 "below" 时，自动打开当前选中邮件
  watch(
    () => layout.prefs.value.readerPosition,
    (nextPos, prevPos) => {
      if (prevPos === "off" && nextPos !== "off" && selectedId.value) {
        void fetchOpenedMessage(selectedId.value);
      } else if (nextPos === "off") {
        openEpoch += 1;
        activeOpenRequestId = "";
        openedMessage.value = null;
        readerLoading.value = false;
        readerError.value = null;
      }
    }
  );

  // 键盘上下选邮件（自动展开所在折叠日期分组）
  function selectMessageAndUnfold(target: MessageRef) {
    const groupToUnfold = resolveSelectedGroupToUnfold(
      target,
      groupedMessages.value,
      collapsedDayGroups.value
    );
    if (groupToUnfold) {
      const next = new Set(collapsedDayGroups.value);
      next.delete(groupToUnfold);
      collapsedDayGroups.value = next;
    }
    selectMessage(target);
  }

  function selectPrev() {
    const flat = messages.value;
    if (!flat.length) return;
    if (!selectedId.value) {
      selectMessageAndUnfold({ accountId: flat[0].accountId, messageId: flat[0].id });
      return;
    }
    const idx = flat.findIndex(
      (m) => m.id === selectedId.value!.messageId && m.accountId === selectedId.value!.accountId
    );
    if (idx > 0) {
      selectMessageAndUnfold({ accountId: flat[idx - 1].accountId, messageId: flat[idx - 1].id });
    }
  }

  function selectNext() {
    const flat = messages.value;
    if (!flat.length) return;
    if (!selectedId.value) {
      selectMessageAndUnfold({ accountId: flat[0].accountId, messageId: flat[0].id });
      return;
    }
    const idx = flat.findIndex(
      (m) => m.id === selectedId.value!.messageId && m.accountId === selectedId.value!.accountId
    );
    if (idx >= 0 && idx < flat.length - 1) {
      selectMessageAndUnfold({ accountId: flat[idx + 1].accountId, messageId: flat[idx + 1].id });
    } else if (idx === flat.length - 1) {
      loadMore();
    }
  }

  // --- 事件处理 ---
  async function syncFoldersMetadata() {
    if (locked.value) return;
    if (foldersSyncPromise) {
      foldersSyncQueued = true;
      return foldersSyncPromise;
    }
    foldersSyncPromise = (async () => {
      do {
        foldersSyncQueued = false;
        if (locked.value) break;
        try {
          const bootstrap = await mainBootstrap();
          if (bootstrap.locked) {
            enterLockedState("locked", bootstrap.hasPasscode ?? hasPasscode.value);
            break;
          }
          accounts.value = bootstrap.accounts;
          folders.value = bootstrap.folders;
          tags.value = bootstrap.tags ?? [];
          templates.value = bootstrap.templates ?? [];
          hasPasscode.value = Boolean(bootstrap.hasPasscode);
          cacheReady.value = bootstrap.cacheReady ?? true;
        } catch (e) {
          handleIpcError(e);
        }
      } while (foldersSyncQueued && !locked.value);
    })().finally(() => {
      foldersSyncPromise = null;
    });
    return foldersSyncPromise;
  }

  function handleMailChanged(e: MailChangedEvent) {
    if (locked.value) return;
    if (e.stateVersion > stateVersion.value) {
      stateVersion.value = e.stateVersion;
      cacheReady.value = true;
      if (listLoading.value) {
        // 已有列表查询在途时不丢弃当前响应，完成时合并触发一次尾部刷新。
        pendingMailChangedReload = true;
      } else {
        void loadMessages(true, { preserveAnchor: true });
      }
      if (e.reason !== "seenChanged" && e.reason !== "flaggedChanged") {
        void syncFoldersMetadata();
      }
    }
  }

  function handleReaderStatus(e: ReaderStatusEvent) {
    if (locked.value) return;
    if (openedMessage.value && openedMessage.value.readerSessionId === e.readerSessionId) {
      openedMessage.value = {
        ...openedMessage.value,
        bodyStatus: e.bodyStatus,
        plainText: e.plainText ?? openedMessage.value.plainText,
        hasHtml: e.hasHtml ?? openedMessage.value.hasHtml,
        htmlBody: e.htmlBody ?? openedMessage.value.htmlBody,
      };
      if (e.layoutApplied !== undefined) {
        readerLayoutApplied.value = e.layoutApplied;
      }
      if (e.layoutVisible !== undefined) {
        readerLayoutVisible.value = e.layoutVisible;
      }
      if (e.bodyStatus !== "loading") {
        clearReaderBodyTimer();
        readerLoading.value = false;
      }
      if (e.bodyStatus === "unavailable") {
        readerError.value = e.detail ?? e.message ?? "正文加载失败，请点击重试";
      } else if (e.bodyStatus === "ready") {
        readerError.value = null;
        if (readerFindOpen.value && readerFindQuery.value.trim()) {
          void runReaderFind(readerFindQuery.value);
        }
      }
    }
  }

  function handleReaderFindResult(e: ReaderFindResultEvent) {
    if (locked.value) return;
    if (openedMessage.value && openedMessage.value.readerSessionId === e.readerSessionId) {
      readerFindCurrent.value = e.current;
      readerFindTotal.value = e.total;
    }
  }

  function handleThreadCardAction(e: ThreadCardActionEvent) {
    if (locked.value) return;
    threadCardActionHandler?.(e);
  }

  // --- 初始化 ---
  async function init() {
    loading.value = true;
    error.value = null;
    try {
      // 先订阅事件，再取快照（防止启动期间遗漏事件）
      if (unlisteners.length === 0) {
        unlisteners.push(await onMailChanged(handleMailChanged));
        unlisteners.push(
          await onSyncStatus((rawEvent) => {
            const normalized = normalizeSyncStatusEvent(rawEvent);
            syncStatus.value = normalized.syncStatus;
            if (normalized.stateVersion !== undefined) {
              stateVersion.value = Math.max(stateVersion.value, normalized.stateVersion);
            }
            if (!normalized.syncStatus.syncing && !locked.value) {
              cacheReady.value = true;
            }
          })
        );
        unlisteners.push(await onReaderStatus(handleReaderStatus));
        unlisteners.push(await onReaderFindResult(handleReaderFindResult));
        unlisteners.push(await onThreadCardAction(handleThreadCardAction));
        unlisteners.push(
          await onLockState((e) => {
            const wasLocked = locked.value;
            if (e.locked) {
              enterLockedState(e.lockedReason ?? e.reason ?? null, e.hasPasscode);
            } else {
              hasPasscode.value = e.hasPasscode;
              lockedReason.value = null;
              locked.value = false;
              clearUnlockCountdown();
              unlockRetryAfterSecs.value = 0;
              unlockError.value = null;
              if (wasLocked && !unlockSubmitting.value) {
                void init();
              }
            }
          })
        );
      }

      const bootstrapStartedAt = typeof performance !== "undefined" ? performance.now() : 0;
      const bootstrap = await mainBootstrap();
      const bootstrapEndedAt = typeof performance !== "undefined" ? performance.now() : 0;
      console.info(
        `[startup] mainBootstrap done locked=${bootstrap.locked} cacheReady=${Boolean(bootstrap.cacheReady)} folders=${bootstrap.folders.length} elapsed_ms=${(bootstrapEndedAt - bootstrapStartedAt).toFixed(1)}`
      );
      mode.value = bootstrap.mode;
      hasPasscode.value = Boolean(bootstrap.hasPasscode);
      cacheReady.value = bootstrap.cacheReady ?? true;
      stateVersion.value = bootstrap.stateVersion;
      syncStatus.value = bootstrap.syncStatus;
      layout.applyPrefs(bootstrap.preferences);

      if (bootstrap.locked) {
        enterLockedState(lockedReason.value ?? "startup", bootstrap.hasPasscode ?? true);
      } else {
        locked.value = false;
        lockedReason.value = null;
        accounts.value = bootstrap.accounts;
        folders.value = bootstrap.folders;
        tags.value = bootstrap.tags ?? [];
        templates.value = bootstrap.templates ?? [];
        currentView.value = bootstrap.currentView;
        // 壳层信息已就绪，先展示主界面与骨架，列表和标签异步并行加载。
        loading.value = false;
        await Promise.all([loadMessages(true), loadTagsCatalog()]);
      }
    } catch (e) {
      if (!handleIpcError(e)) {
        error.value = extractErrorMessage(e);
      }
    } finally {
      loading.value = false;
    }
  }

  function dispose() {
    clearReaderBodyTimer();
    clearUnlockCountdown();
    clearUndoToastTimer();
    for (const unlisten of unlisteners) unlisten();
    unlisteners.length = 0;
  }

  if (getCurrentInstance()) {
    onMounted(init);
    onUnmounted(dispose);
  }

  return {
    loading,
    error,
    mode,
    locked,
    hasPasscode,
    cacheReady,
    lockedReason,
    unlockSubmitting,
    unlockError,
    unlockRetryAfterSecs,
    stateVersion,
    accounts,
    folders,
    tags,
    templates,
    currentView,
    syncStatus,
    layout,
    messages,
    groupedMessages,
    listTotal,
    listLoading,
    listError,
    hasMore,
    currentFolderHasMoreRemote,
    loadingOlder,
    scrollResetToken,
    searchQuery,
    searchScope,
    sortMode,
    filterSet,
    expandedThreadKeys,
    threadChildren,
    collapsedDayGroups,
    selectedId,
    selectedMessage,
    selectedUnread,
    selectedTags,
    openedMessage,
    openedAccountEmail,
    readerLoading,
    readerError,
    readerLayoutApplied,
    readerLayoutVisible,
    allowingRemoteImages,
    movingMessage,
    undoToast,
    undoPending,
    availableTags,
    perMessageDark,
    readingView,
    readerFindOpen,
    readerFindQuery,
    readerFindCurrent,
    readerFindTotal,
    readerFindStepToken,
    readerFindStepDelta,
    init,
    dispose,
    selectView,
    applySidebarFilter,
    toggleDayGroup,
    collapseAllDayGroups,
    expandAllDayGroups,
    toggleThreadExpand,
    setSearch,
    setFilter,
    setSort,
    setScope,
    refresh,
    selectMessage,
    retryOpen,
    toggleMessageSeen,
    toggleMessageFlagged,
    allowOpenedRemoteImages,
    loadOlderMessages,
    archiveMessageByRef,
    deleteMessageByRef,
    markMessageSpamByRef,
    moveMessageByRef,
    archiveSelectedMessage,
    deleteSelectedMessage,
    markSelectedSpam,
    unmarkSelectedSpam,
    moveSelectedMessage,
    setOpenedMessageTags,
    togglePerMessageDark,
    toggleReadingView,
    openReaderFind,
    closeReaderFind,
    toggleReaderFind,
    runReaderFind,
    updateFallbackFindResult,
    registerThreadCardActionHandler,
    closeWindow,
    undoLastMove,
    dismissUndoToast,
    lock,
    unlock,
    quit,
    selectPrev,
    selectNext,
    loadMore,
    updateMainPreferences,
  };
}

export type UseMainWindowReturn = ReturnType<typeof useMainWindow>;

const MAIN_WINDOW_KEY: InjectionKey<UseMainWindowReturn> = Symbol("kitemail-main-window");

export function provideMainWindow(): UseMainWindowReturn {
  const state = useMainWindow();
  provide(MAIN_WINDOW_KEY, state);
  return state;
}

export function injectMainWindow(): UseMainWindowReturn {
  return inject(MAIN_WINDOW_KEY) ?? useMainWindow();
}

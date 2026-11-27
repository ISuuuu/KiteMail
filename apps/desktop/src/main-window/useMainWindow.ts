import {
  ref,
  computed,
  watch,
  provide,
  inject,
  onMounted,
  onUnmounted,
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
  SyncStatusDto,
  MessageRef,
  ReaderPosition,
} from "../ipc/types";
import {
  mainBootstrap,
  listMessages,
  openMessage,
  updateMainPreferences,
  refreshMail,
  onMailChanged,
  onSyncStatus,
  onReaderStatus,
  onLockState,
  extractErrorMessage,
} from "../ipc/main-window";
import { usePaneLayout } from "./usePaneLayout";

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

export interface MessageGroup {
  label: string;
  items: MessageSummaryDto[];
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
  const stateVersion = ref(0);
  const accounts = ref<AccountSummaryDto[]>([]);
  const folders = ref<FolderSummaryDto[]>([]);
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

  // 查询状态
  const searchQuery = ref("");
  const searchScope = ref<SearchScope>("thisFolder");
  const sortMode = ref<MessageSort>("dateNewest");
  const filterSet = ref<MessageFilterDto>({});

  // 请求代次防乱序
  let queryEpoch = 0;
  let inFlightOffset = -1;
  let openEpoch = 0;
  let activeOpenRequestId = "";

  // 选中状态
  const selectedId = ref<MessageRef | null>(null);
  const openedMessage = ref<OpenMessageOutput | null>(null);
  const readerLoading = ref(false);
  const readerError = ref<string | null>(null);

  // 解订阅句柄
  const unlisteners: Array<() => void> = [];

  // --- 派生 ---
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

  // --- 加载列表 ---
  async function loadMessages(resetList = true) {
    if (locked.value) return;

    if (resetList) {
      queryEpoch += 1;
      inFlightOffset = -1;
      messages.value = [];
      listTotal.value = 0;
      hasMore.value = false;
      scrollResetToken.value += 1;
    }

    const epoch = queryEpoch;
    const offset = resetList ? 0 : messages.value.length;

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
        limit: PAGE_SIZE,
      });

      // 严格按代次与偏移校验，丢弃过期或乱序响应
      if (shouldApplyListResponse(epoch, queryEpoch, requestId, offset, result)) {
        messages.value = mergePagedMessages(messages.value, result.items, resetList);
        listTotal.value = result.total;
        hasMore.value = result.hasMore;
        stateVersion.value = Math.max(stateVersion.value, result.stateVersion);
      }
    } catch (e) {
      if (epoch === queryEpoch) {
        listError.value = extractErrorMessage(e);
      }
    } finally {
      if (epoch === queryEpoch) {
        listLoading.value = false;
        inFlightOffset = -1;
      }
    }
  }

  function loadMore() {
    if (!listLoading.value && hasMore.value && !locked.value) {
      loadMessages(false);
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
    filterSet.value = {};
    loadMessages(true);
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
  async function refresh() {
    if (locked.value) return;
    try {
      await refreshMail({ view: currentView.value });
      await loadMessages(true);
    } catch (e) {
      listError.value = extractErrorMessage(e);
    }
  }

  // --- 打开邮件正文 ---
  async function fetchOpenedMessage(target: MessageRef) {
    if (locked.value || layout.prefs.value.readerPosition === "off") {
      return;
    }

    const epoch = ++openEpoch;
    const requestId = `open-${epoch}-${target.accountId}-${target.messageId}`;
    activeOpenRequestId = requestId;
    openedMessage.value = null;
    readerLoading.value = true;
    readerError.value = null;

    try {
      const result = await openMessage({
        requestId,
        accountId: target.accountId,
        messageId: target.messageId,
      });

      if (
        shouldApplyOpenResponse(
          epoch,
          openEpoch,
          activeOpenRequestId,
          selectedId.value,
          result,
          layout.prefs.value.readerPosition
        )
      ) {
        openedMessage.value = result;
        readerLoading.value = result.bodyStatus === "loading";
        if (result.bodyStatus === "unavailable") {
          readerError.value = "正文暂不可用";
        }
      }
    } catch (e) {
      if (epoch === openEpoch) {
        readerError.value = extractErrorMessage(e);
        readerLoading.value = false;
      }
    }
  }

  // --- 选邮件（点击行与键盘共用） ---
  function selectMessage(target: MessageRef | null, forceReload = false) {
    if (!target) {
      openEpoch += 1;
      activeOpenRequestId = "";
      selectedId.value = null;
      openedMessage.value = null;
      readerLoading.value = false;
      readerError.value = null;
      return;
    }

    const isSame =
      selectedId.value?.messageId === target.messageId &&
      selectedId.value?.accountId === target.accountId;

    if (isSame && !forceReload) return;
    selectedId.value = { accountId: target.accountId, messageId: target.messageId };

    if (layout.prefs.value.readerPosition === "off") {
      openedMessage.value = null;
      readerLoading.value = false;
      readerError.value = null;
      return;
    }

    void fetchOpenedMessage(target);
  }

  function retryOpen() {
    if (selectedId.value) {
      selectMessage(selectedId.value, true);
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

  // 键盘上下选邮件
  function selectPrev() {
    const flat = messages.value;
    if (!flat.length) return;
    if (!selectedId.value) {
      selectMessage({ accountId: flat[0].accountId, messageId: flat[0].id });
      return;
    }
    const idx = flat.findIndex(
      (m) => m.id === selectedId.value!.messageId && m.accountId === selectedId.value!.accountId
    );
    if (idx > 0) {
      selectMessage({ accountId: flat[idx - 1].accountId, messageId: flat[idx - 1].id });
    }
  }

  function selectNext() {
    const flat = messages.value;
    if (!flat.length) return;
    if (!selectedId.value) {
      selectMessage({ accountId: flat[0].accountId, messageId: flat[0].id });
      return;
    }
    const idx = flat.findIndex(
      (m) => m.id === selectedId.value!.messageId && m.accountId === selectedId.value!.accountId
    );
    if (idx >= 0 && idx < flat.length - 1) {
      selectMessage({ accountId: flat[idx + 1].accountId, messageId: flat[idx + 1].id });
    } else if (idx === flat.length - 1) {
      loadMore();
    }
  }

  // --- 事件处理 ---
  function handleMailChanged(e: MailChangedEvent) {
    if (locked.value) return;
    if (e.stateVersion > stateVersion.value) {
      stateVersion.value = e.stateVersion;
      void loadMessages(true);
    }
  }

  function handleReaderStatus(e: ReaderStatusEvent) {
    if (openedMessage.value && openedMessage.value.readerSessionId === e.readerSessionId) {
      openedMessage.value = {
        ...openedMessage.value,
        bodyStatus: e.bodyStatus,
        plainText: e.plainText ?? openedMessage.value.plainText,
        hasHtml: e.hasHtml ?? openedMessage.value.hasHtml,
        htmlBody: e.htmlBody ?? openedMessage.value.htmlBody,
      };
      if (e.bodyStatus !== "loading") readerLoading.value = false;
      if (e.bodyStatus === "unavailable") {
        readerError.value = e.message ?? "正文加载失败";
      } else if (e.bodyStatus === "ready") {
        readerError.value = null;
      }
    }
  }

  function clearLoadedMailState() {
    queryEpoch += 1;
    openEpoch += 1;
    activeOpenRequestId = "";
    messages.value = [];
    listTotal.value = 0;
    hasMore.value = false;
    selectedId.value = null;
    openedMessage.value = null;
    readerLoading.value = false;
    readerError.value = null;
    layout.clearOverlays();
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
          await onSyncStatus((e) => {
            syncStatus.value = {
              syncing: e.syncing,
              message: e.message,
              lastSyncAt: e.lastSyncAt,
            };
          })
        );
        unlisteners.push(await onReaderStatus(handleReaderStatus));
        unlisteners.push(
          await onLockState((e) => {
            const wasLocked = locked.value;
            locked.value = e.locked;
            if (e.locked) {
              clearLoadedMailState();
            } else if (wasLocked && !e.locked) {
              void loadMessages(true);
            }
          })
        );
      }

      const bootstrap = await mainBootstrap();
      mode.value = bootstrap.mode;
      locked.value = bootstrap.locked;
      stateVersion.value = bootstrap.stateVersion;
      accounts.value = bootstrap.accounts;
      folders.value = bootstrap.folders;
      currentView.value = bootstrap.currentView;
      syncStatus.value = bootstrap.syncStatus;
      layout.applyPrefs(bootstrap.preferences);

      if (!bootstrap.locked) {
        await loadMessages(true);
      }
    } catch (e) {
      error.value = extractErrorMessage(e);
    } finally {
      loading.value = false;
    }
  }

  function dispose() {
    for (const unlisten of unlisteners) unlisten();
    unlisteners.length = 0;
  }

  onMounted(init);
  onUnmounted(dispose);

  return {
    loading,
    error,
    mode,
    locked,
    stateVersion,
    accounts,
    folders,
    currentView,
    syncStatus,
    layout,
    messages,
    groupedMessages,
    listTotal,
    listLoading,
    listError,
    hasMore,
    scrollResetToken,
    searchQuery,
    searchScope,
    sortMode,
    filterSet,
    selectedId,
    openedMessage,
    readerLoading,
    readerError,
    init,
    selectView,
    setSearch,
    setFilter,
    setSort,
    setScope,
    refresh,
    selectMessage,
    retryOpen,
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

<template>
  <section class="km-list-pane" aria-label="邮件列表">
    <!-- 顶部栏 48px：左（侧栏切换 + 搜索图标） / 中（拖拽区） / 右（总数 + 排序筛选图标） -->
    <header class="km-list-header" data-tauri-drag-region>
      <div class="km-header-left">
        <button
          type="button"
          class="km-icon-btn"
          :title="isRail ? '展开侧栏' : '折叠侧栏'"
          :aria-label="isRail ? '展开侧栏' : '折叠侧栏'"
          @click="emit('toggleSidebar')"
        >
          <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
            <rect
              x="2"
              y="2.5"
              width="12"
              height="11"
              rx="1.8"
              fill="none"
              stroke="currentColor"
              stroke-width="1.35"
            />
            <path fill="none" stroke="currentColor" stroke-width="1.35" d="M6 2.5v11" />
          </svg>
        </button>

        <button
          type="button"
          class="km-icon-btn"
          :class="{ active: searchBarVisible }"
          title="搜索邮件 (Ctrl+F)"
          aria-label="搜索邮件"
          :aria-expanded="searchBarVisible"
          @click="toggleSearchBar"
        >
          <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
            <circle cx="7" cy="7" r="4.5" fill="none" stroke="currentColor" stroke-width="1.45" />
            <path
              fill="none"
              stroke="currentColor"
              stroke-width="1.45"
              stroke-linecap="round"
              d="m10.4 10.4 3.2 3.2"
            />
          </svg>
        </button>
      </div>

      <div class="km-header-spacer" data-tauri-drag-region>
        <span v-if="syncStatus?.syncing" class="km-sync-pill" role="status" :title="syncStatus.message || '正在同步邮件…'">
          <span class="km-sync-spinner" aria-hidden="true" />
        </span>
      </div>

      <div class="km-header-right">
        <span class="km-total-count" :title="`共 ${total.toLocaleString()} 封邮件`">
          {{ total.toLocaleString() }}
        </span>

        <!-- 排序与筛选菜单 -->
        <div ref="filterWrapRef" class="km-popover-wrap">
          <button
            type="button"
            class="km-icon-btn km-filter-btn"
            :class="{ active: hasActiveFilter }"
            :title="filterTooltip"
            aria-label="排序与筛选"
            aria-haspopup="menu"
            :aria-expanded="filterMenuOpen"
            @click="toggleFilterMenu"
          >
            <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
              <path
                fill="none"
                stroke="currentColor"
                stroke-width="1.5"
                stroke-linecap="round"
                d="M2.5 4.2h11M4.5 8h7M6.5 11.8h3"
              />
            </svg>
            <span v-if="hasActiveFilter" class="km-filter-active-dot" aria-hidden="true" />
          </button>

          <div v-if="filterMenuOpen" class="km-popover-menu" role="menu" aria-label="排序与筛选">
            <div class="km-menu-title">快速筛选</div>
            <label class="km-menu-check">
              <input
                type="checkbox"
                :checked="Boolean(filterSet.unread)"
                @change="toggleFilterFlag('unread')"
              />
              <span>仅未读</span>
            </label>
            <label class="km-menu-check">
              <input
                type="checkbox"
                :checked="Boolean(filterSet.starred)"
                @change="toggleFilterFlag('starred')"
              />
              <span>仅星标</span>
            </label>
            <label class="km-menu-check">
              <input
                type="checkbox"
                :checked="Boolean(filterSet.hasAttachment)"
                @change="toggleFilterFlag('hasAttachment')"
              />
              <span>含附件</span>
            </label>
            <label class="km-menu-check">
              <input
                type="checkbox"
                :checked="Boolean(filterSet.toMe)"
                @change="toggleFilterFlag('toMe')"
              />
              <span>直接发给我</span>
            </label>
            <label class="km-menu-check">
              <input
                type="checkbox"
                :checked="Boolean(filterSet.highImportance)"
                @change="toggleFilterFlag('highImportance')"
              />
              <span>高优先级</span>
            </label>
            <label class="km-menu-check">
              <input
                type="checkbox"
                :checked="Boolean(filterSet.tagged)"
                @change="toggleFilterFlag('tagged')"
              />
              <span>含标签</span>
            </label>
            <label class="km-menu-check">
              <input
                type="checkbox"
                :checked="Boolean(filterSet.today)"
                @change="toggleFilterFlag('today')"
              />
              <span>今天</span>
            </label>

            <div class="km-menu-divider" />
            <div class="km-menu-title">日期范围</div>
            <div class="km-date-presets">
              <button type="button" class="km-date-preset-btn" @click="applyDatePresetDays(7)">7 天</button>
              <button type="button" class="km-date-preset-btn" @click="applyDatePresetDays(30)">30 天</button>
              <button type="button" class="km-date-preset-btn" @click="applyDatePresetDays(90)">90 天</button>
            </div>
            <div class="km-date-inputs">
              <label class="km-date-field">
                <span>起</span>
                <input
                  v-model="dateFromDraft"
                  type="date"
                  class="km-date-input"
                  aria-label="开始日期"
                  @change="applyCustomDateRange"
                />
              </label>
              <label class="km-date-field">
                <span>止</span>
                <input
                  v-model="dateToDraft"
                  type="date"
                  class="km-date-input"
                  aria-label="结束日期"
                  @change="applyCustomDateRange"
                />
              </label>
            </div>
            <button
              v-if="hasDateRangeFilter"
              type="button"
              class="km-menu-subaction"
              @click="clearDateRange"
            >
              清除日期范围
            </button>

            <button
              v-if="hasActiveFilter"
              type="button"
              class="km-menu-reset"
              @click="clearAllFilters"
            >
              重置全部筛选 ({{ activeFilterCount }})
            </button>

            <div class="km-menu-divider" />
            <div class="km-menu-title">排序方式</div>
            <button
              v-for="opt in SORT_OPTIONS"
              :key="opt.value"
              type="button"
              class="km-menu-item"
              :class="{ active: sortMode === opt.value }"
              @click="selectSort(opt.value)"
            >
              <span>{{ opt.label }}</span>
              <svg v-if="sortMode === opt.value" viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
                <path
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.8"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  d="m3.5 8.5 3 3 6-6.5"
                />
              </svg>
            </button>
          </div>
        </div>

        <button
          v-if="showWindowClose"
          type="button"
          class="km-icon-btn km-window-close-btn"
          title="关闭主窗口"
          aria-label="关闭窗口"
          @click="emit('closeWindow')"
        >
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
            <path
              fill="currentColor"
              d="m3.7 2.8 4.3 4.3 4.3-4.3.9.9L8.9 8l4.3 4.3-.9.9L8 8.9l-4.3 4.3-.9-.9L7.1 8 2.8 3.7l.9-.9Z"
            />
          </svg>
        </button>
      </div>
    </header>

    <!-- 可展开第二行搜索框（仅点击搜索图标 / Ctrl+F / 有查询词时展示） -->
    <div v-if="searchBarVisible" class="km-search-row">
      <div class="km-search-box">
        <span class="km-search-icon" aria-hidden="true">
          <svg viewBox="0 0 16 16" width="13" height="13">
            <circle cx="7" cy="7" r="4.5" fill="none" stroke="currentColor" stroke-width="1.5" />
            <path
              fill="none"
              stroke="currentColor"
              stroke-width="1.5"
              stroke-linecap="round"
              d="m10.4 10.4 3.2 3.2"
            />
          </svg>
        </span>
        <input
          ref="searchInputRef"
          v-model="draftQuery"
          type="search"
          class="km-search-input"
          placeholder="搜索邮件… (Ctrl+F)"
          aria-label="搜索邮件"
          @compositionstart="isComposing = true"
          @compositionend="onCompositionEnd"
          @input="onSearchInput"
          @keydown.enter="commitSearchImmediate"
          @keydown.esc.stop="clearSearch"
        />
        <button
          type="button"
          class="km-clear-btn"
          :title="draftQuery ? '清空搜索' : '收起搜索'"
          :aria-label="draftQuery ? '清空搜索' : '收起搜索'"
          @click="clearSearch"
        >
          <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
            <path
              fill="none"
              stroke="currentColor"
              stroke-width="1.6"
              stroke-linecap="round"
              d="m4 4 8 8m0-8-8 8"
            />
          </svg>
        </button>
      </div>

      <button
        type="button"
        class="km-scope-btn"
        :class="{ active: searchScope === 'allFolders' }"
        :title="searchScope === 'allFolders' ? '当前：跨文件夹搜索' : '当前：仅搜本文件夹'"
        @click="toggleScope"
      >
        {{ searchScope === "allFolders" ? "全部" : "本夹" }}
      </button>
    </div>

    <!-- 启用筛选状态反馈条（仅在开启了快捷筛选或日期筛选时显示，常态隐藏） -->
    <div v-if="hasActiveFilter" class="km-active-filter-bar">
      <span class="km-filter-summary">
        已启用 {{ activeFilterCount }} 项筛选
        <span v-if="hasDateRangeFilter">（含日期范围）</span>
      </span>
      <button type="button" class="km-link-btn" @click="clearAllFilters">清除</button>
    </div>

    <!-- 错误提示条 -->
    <div v-if="error" class="km-list-error" role="alert">
      <span>{{ error }}</span>
      <button type="button" class="km-link-btn" @click="emit('refresh')">重试</button>
    </div>

    <!-- 虚拟滚动容器 -->
    <div
      ref="viewportRef"
      class="km-list-viewport"
      role="listbox"
      aria-label="邮件列表"
      @scroll="onScroll"
    >
      <div v-if="cacheReady === false && messages.length === 0" class="km-list-empty" role="status">
        <span class="km-initial-spinner" aria-hidden="true" />
        <p class="km-empty-title">正在初始化离线缓存并同步邮件…</p>
        <p v-if="syncStatus?.message" class="km-empty-sub">{{ syncStatus.message }}</p>
      </div>

      <div v-else-if="loading && messages.length === 0" class="km-list-empty">
        正在加载邮件列表…
      </div>

      <div v-else-if="!loading && messages.length === 0" class="km-list-empty">
        <p>没有符合条件的邮件</p>
        <button
          v-if="searchQuery || hasActiveFilter"
          type="button"
          class="km-empty-reset"
          @click="resetQueryAndFilters"
        >
          清空搜索与筛选
        </button>
      </div>

      <div
        v-else
        class="km-virtual-spacer"
        :style="{ height: `${totalHeight}px` }"
      >
        <div
          v-for="item in visibleItems"
          :key="item.key"
          class="km-virtual-slot"
          :style="{
            height: `${item.height}px`,
            transform: `translateY(${item.top}px)`,
          }"
        >
          <button
            v-if="item.type === 'header'"
            type="button"
            class="km-date-group-header"
            :aria-expanded="!item.collapsed"
            :title="item.collapsed ? `展开 ${item.label}` : `折叠 ${item.label}`"
            @click="emit('toggleDayGroup', item.label)"
          >
            <span class="km-date-group-left">
              <span class="km-date-group-label">{{ item.label }}</span>
              <svg
                class="km-date-chevron"
                :class="{ collapsed: item.collapsed }"
                viewBox="0 0 12 12"
                width="10"
                height="10"
                aria-hidden="true"
              >
                <path
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.5"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  d="m3 4.5 3 3 3-3"
                />
              </svg>
            </span>
            <span v-if="item.collapsed" class="km-date-group-count">{{ item.count }}</span>
          </button>

          <MessageRow
            v-else
            :message="item.message"
            :selected="isMessageSelected(item.message)"
            :wide="listWide"
            :compact="compactList"
            :showAvatar="showAvatars"
            :effectiveDark="effectiveDark"
            :tagsMeta="tagsMeta"
            :threadExpanded="isThreadExpanded(item.message)"
            :isThreadChild="item.isThreadChild"
            :isLastThreadChild="item.isLastThreadChild"
            @select="emit('selectMessage', $event)"
            @toggleSeen="(ref, seen) => emit('toggleSeen', ref, seen)"
            @toggleFlagged="(ref, flagged) => emit('toggleFlagged', ref, flagged)"
            @toggleThread="emit('toggleThread', $event)"
          />
        </div>
      </div>

      <div v-if="loading && messages.length > 0" class="km-loading-more">
        正在加载更多邮件…
      </div>

      <div v-if="hasMoreRemote && !loading" class="km-load-older-bar">
        <button
          type="button"
          class="km-load-older-btn"
          :disabled="loadingOlder"
          @click="emit('loadOlder')"
        >
          {{ loadingOlder ? "正在拉取更早邮件…" : "加载更早邮件" }}
        </button>
      </div>
    </div>
  </section>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type {
  MessageFilterDto,
  MessageRef,
  MessageSort,
  MessageSummaryDto,
  ReaderPosition,
  SearchScope,
  SyncStatusDto,
  TagItemDto,
} from "../ipc/types";
import {
  buildMessageListVirtualLayout,
  computeAnchorScrollTop,
  dateInputToUnixRange,
  type MessageGroup,
  type VirtualListItem,
} from "./useMainWindow";
import MessageRow from "./MessageRow.vue";

const SORT_OPTIONS: ReadonlyArray<{ value: MessageSort; label: string }> = [
  { value: "dateNewest", label: "日期：最新优先" },
  { value: "dateOldest", label: "日期：最早优先" },
  { value: "unreadFirst", label: "未读优先" },
  { value: "starredFirst", label: "星标优先" },
  { value: "attachmentFirst", label: "含附件优先" },
  { value: "importanceFirst", label: "重要优先" },
  { value: "senderAsc", label: "发件人 A → Z" },
  { value: "senderDesc", label: "发件人 Z → A" },
  { value: "subjectAsc", label: "主题 A → Z" },
  { value: "subjectDesc", label: "主题 Z → A" },
];

const props = withDefaults(
  defineProps<{
    messages: MessageSummaryDto[];
    groupedMessages: MessageGroup[];
    total: number;
    loading: boolean;
    error: string | null;
    hasMore: boolean;
    hasMoreRemote?: boolean;
    loadingOlder?: boolean;
    searchQuery: string;
    searchScope: SearchScope;
    sortMode: MessageSort;
    filterSet: MessageFilterDto;
    selectedId: MessageRef | null;
    listWide: boolean;
    readerPosition: ReaderPosition;
    compactList: boolean;
    showAvatars: boolean;
    isRail?: boolean;
    effectiveDark?: boolean;
    tagsMeta?: TagItemDto[];
    collapsedDayGroups?: ReadonlySet<string>;
    expandedThreadKeys?: ReadonlySet<string>;
    threadChildren?: Readonly<Record<string, readonly MessageSummaryDto[]>>;
    scrollResetToken?: number;
    syncStatus?: SyncStatusDto;
    cacheReady?: boolean;
    showWindowClose?: boolean;
  }>(),
  {
    isRail: false,
    effectiveDark: false,
    showWindowClose: false,
    tagsMeta: () => [],
    collapsedDayGroups: () => new Set<string>(),
    expandedThreadKeys: () => new Set<string>(),
    threadChildren: () => ({}),
  }
);

const emit = defineEmits<{
  selectMessage: [ref: MessageRef];
  toggleSeen: [ref: MessageRef, seen: boolean];
  toggleFlagged: [ref: MessageRef, flagged: boolean];
  toggleThread: [message: MessageSummaryDto];
  toggleDayGroup: [label: string];
  toggleSidebar: [];
  setSearch: [query: string];
  setScope: [scope: SearchScope];
  setSort: [sort: MessageSort];
  setFilter: [filter: MessageFilterDto];
  refresh: [];
  loadMore: [];
  loadOlder: [];
  closeWindow: [];
  overlayChange: [id: string, open: boolean];
}>();

const searchInputRef = ref<HTMLInputElement | null>(null);
const filterWrapRef = ref<HTMLElement | null>(null);
const viewportRef = ref<HTMLElement | null>(null);

// 可展开搜索栏状态：常态收起，点击搜索图标或 Ctrl+F 展开；有查询时保持展开
const searchOpen = ref(Boolean(props.searchQuery));
const draftQuery = ref(props.searchQuery);
const isComposing = ref(false);
let searchTimer: ReturnType<typeof setTimeout> | null = null;

const searchBarVisible = computed(
  () => searchOpen.value || draftQuery.value.length > 0 || props.searchQuery.length > 0
);

watch(
  () => props.searchQuery,
  (val) => {
    if (draftQuery.value !== val) {
      draftQuery.value = val;
    }
    if (!val && !searchOpen.value) {
      searchOpen.value = false;
    }
  }
);

function toggleSearchBar() {
  if (searchBarVisible.value && !draftQuery.value && !props.searchQuery) {
    searchOpen.value = false;
    return;
  }
  searchOpen.value = !searchOpen.value;
  if (searchOpen.value) {
    nextTick(() => {
      searchInputRef.value?.focus();
      searchInputRef.value?.select();
    });
  } else if (!draftQuery.value && !props.searchQuery) {
    searchOpen.value = false;
  }
}

function onSearchInput() {
  if (isComposing.value) return;
  scheduleSearchCommit();
}

function onCompositionEnd() {
  isComposing.value = false;
  scheduleSearchCommit();
}

function scheduleSearchCommit() {
  if (searchTimer) clearTimeout(searchTimer);
  searchTimer = setTimeout(() => {
    searchTimer = null;
    emit("setSearch", draftQuery.value);
  }, 180);
}

function commitSearchImmediate() {
  if (isComposing.value) return;
  if (searchTimer) {
    clearTimeout(searchTimer);
    searchTimer = null;
  }
  emit("setSearch", draftQuery.value);
}

function clearSearch() {
  if (searchTimer) {
    clearTimeout(searchTimer);
    searchTimer = null;
  }
  draftQuery.value = "";
  searchOpen.value = false;
  emit("setSearch", "");
}

function toggleScope() {
  emit("setScope", props.searchScope === "thisFolder" ? "allFolders" : "thisFolder");
}

// 筛选/排序弹出层
const filterMenuOpen = ref(false);
const dateFromDraft = ref("");
const dateToDraft = ref("");

function formatUnixToDateInput(ts?: number | null): string {
  if (ts == null || !Number.isFinite(ts)) return "";
  const d = new Date(ts * 1000);
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

watch(
  () => [props.filterSet.dateFromTs, props.filterSet.dateToTs] as const,
  ([fromTs, toTs]) => {
    dateFromDraft.value = formatUnixToDateInput(fromTs);
    dateToDraft.value = toTs != null ? formatUnixToDateInput(toTs - 86400) : "";
  },
  { immediate: true }
);

function setFilterMenu(open: boolean) {
  if (filterMenuOpen.value === open) return;
  filterMenuOpen.value = open;
  emit("overlayChange", "list-filter-menu", open);
}

function toggleFilterMenu() {
  setFilterMenu(!filterMenuOpen.value);
}

const hasDateRangeFilter = computed(
  () => props.filterSet.dateFromTs != null || props.filterSet.dateToTs != null
);

const activeFilterCount = computed(() => {
  const f = props.filterSet;
  return (
    Number(Boolean(f.unread)) +
    Number(Boolean(f.starred)) +
    Number(Boolean(f.hasAttachment)) +
    Number(Boolean(f.toMe)) +
    Number(Boolean(f.fromContacts)) +
    Number(Boolean(f.highImportance)) +
    Number(Boolean(f.tagged)) +
    Number(Boolean(f.today)) +
    Number(hasDateRangeFilter.value)
  );
});

const hasActiveFilter = computed(() => activeFilterCount.value > 0);

const filterTooltip = computed(() => {
  const sortLabel = SORT_OPTIONS.find((o) => o.value === props.sortMode)?.label ?? "排序";
  if (!hasActiveFilter.value) {
    return `排序与筛选（${sortLabel}）`;
  }
  return `排序：${sortLabel}｜已启用 ${activeFilterCount.value} 项筛选`;
});

function toggleFilterFlag(
  key: "unread" | "starred" | "hasAttachment" | "today" | "toMe" | "fromContacts" | "highImportance" | "tagged"
) {
  const next: MessageFilterDto = {
    ...props.filterSet,
    [key]: !props.filterSet[key],
  };
  emit("setFilter", next);
}

function applyDatePresetDays(days: number) {
  const now = new Date();
  now.setHours(0, 0, 0, 0);
  const start = Math.floor(now.getTime() / 1000) - (days - 1) * 86400;
  emit("setFilter", {
    ...props.filterSet,
    dateFromTs: start,
    dateToTs: null,
  });
}

function applyCustomDateRange() {
  const { dateFromTs, dateToTs } = dateInputToUnixRange(dateFromDraft.value, dateToDraft.value);
  emit("setFilter", {
    ...props.filterSet,
    dateFromTs,
    dateToTs,
  });
}

function clearDateRange() {
  dateFromDraft.value = "";
  dateToDraft.value = "";
  const next = { ...props.filterSet };
  delete next.dateFromTs;
  delete next.dateToTs;
  emit("setFilter", next);
}

function clearAllFilters() {
  dateFromDraft.value = "";
  dateToDraft.value = "";
  emit("setFilter", props.filterSet.accountId != null ? { accountId: props.filterSet.accountId } : {});
  setFilterMenu(false);
}

function resetQueryAndFilters() {
  clearSearch();
  clearAllFilters();
}

function selectSort(sort: MessageSort) {
  emit("setSort", sort);
  setFilterMenu(false);
}

// ---------- 虚拟列表布局计算 ----------
const rowHeight = computed(() => {
  if (props.listWide) return 38;
  return props.compactList ? 68 : 86;
});

const virtualLayout = computed(() =>
  buildMessageListVirtualLayout(props.groupedMessages, {
    rowHeight: rowHeight.value,
    collapsedDayGroups: props.collapsedDayGroups,
    expandedThreadKeys: props.expandedThreadKeys,
    threadChildren: props.threadChildren,
  })
);

const totalHeight = computed(() => virtualLayout.value.totalHeight);

const scrollTop = ref(0);
const viewportHeight = ref(600);
const OVERSCAN_PX = 360;

function findStartIndex(items: VirtualListItem[], targetTop: number): number {
  let lo = 0;
  let hi = items.length - 1;
  let ans = items.length;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (items[mid].top + items[mid].height >= targetTop) {
      ans = mid;
      hi = mid - 1;
    } else {
      lo = mid + 1;
    }
  }
  return ans;
}

const visibleItems = computed(() => {
  const items = virtualLayout.value.items;
  if (items.length === 0) return [];
  const minY = Math.max(0, scrollTop.value - OVERSCAN_PX);
  const maxY = scrollTop.value + viewportHeight.value + OVERSCAN_PX;
  const start = findStartIndex(items, minY);
  let end = start;
  while (end < items.length && items[end].top <= maxY) {
    end++;
  }
  return items.slice(start, end);
});

let scrollRaf: number | null = null;
function onScroll() {
  if (!viewportRef.value) return;
  if (scrollRaf !== null) return;
  scrollRaf = requestAnimationFrame(() => {
    scrollRaf = null;
    const el = viewportRef.value;
    if (!el) return;
    scrollTop.value = el.scrollTop;
    viewportHeight.value = el.clientHeight;
    if (!props.loading && el.scrollTop + el.clientHeight >= el.scrollHeight - 400) {
      if (props.hasMore) {
        emit("loadMore");
      } else if (props.hasMoreRemote && !props.loadingOlder) {
        emit("loadOlder");
      }
    }
  });
}

let lastHandledScrollResetToken = props.scrollResetToken ?? 0;

// 切文件夹/新搜索时回到首屏
watch(
  () => props.scrollResetToken,
  (token) => {
    lastHandledScrollResetToken = token ?? 0;
    nextTick(() => {
      if (viewportRef.value) {
        viewportRef.value.scrollTop = 0;
        scrollTop.value = 0;
      }
    });
  }
);

// 增量 mail-changed / 刷新时尽量保持当前滚动锚点
watch(
  virtualLayout,
  (nextLayout, prevLayout) => {
    const token = props.scrollResetToken ?? 0;
    if (token !== lastHandledScrollResetToken) return;
    if (!viewportRef.value || !prevLayout || prevLayout.items.length === 0 || nextLayout.items.length === 0) {
      return;
    }
    const currentTop = viewportRef.value.scrollTop;
    if (currentTop <= 0) return;
    const selectedKey = props.selectedId
      ? `${props.selectedId.accountId}:${props.selectedId.messageId}`
      : null;
    const anchoredTop = computeAnchorScrollTop(
      prevLayout.items,
      nextLayout.items,
      currentTop,
      selectedKey
    );
    nextTick(() => {
      if (viewportRef.value && (props.scrollResetToken ?? 0) === lastHandledScrollResetToken) {
        viewportRef.value.scrollTop = anchoredTop;
        scrollTop.value = anchoredTop;
      }
    });
  },
  { flush: "pre" }
);

function scrollSelectedIntoView(sel: MessageRef | null) {
  if (!sel || !viewportRef.value) return;
  const key = `${sel.accountId}:${sel.messageId}`;
  const idx = virtualLayout.value.rowIndexByKey.get(key);
  if (idx === undefined) return;
  const item = virtualLayout.value.items[idx];
  const el = viewportRef.value;
  const viewTop = el.scrollTop;
  const viewBottom = viewTop + el.clientHeight;
  if (item.top < viewTop) {
    el.scrollTop = item.top;
    scrollTop.value = item.top;
  } else if (item.top + item.height > viewBottom) {
    const nextTop = Math.max(0, item.top + item.height - el.clientHeight);
    el.scrollTop = nextTop;
    scrollTop.value = nextTop;
  }
}

// 键盘上下选信及折叠展开后确保选中行在视口内可见
watch(
  () => props.selectedId,
  (sel) => {
    nextTick(() => scrollSelectedIntoView(sel));
  }
);

watch(
  () => props.collapsedDayGroups,
  () => {
    if (props.selectedId) {
      nextTick(() => scrollSelectedIntoView(props.selectedId));
    }
  }
);

function isMessageSelected(msg: MessageSummaryDto): boolean {
  return (
    props.selectedId?.accountId === msg.accountId &&
    props.selectedId?.messageId === msg.id
  );
}

function isThreadExpanded(msg: MessageSummaryDto): boolean {
  return props.expandedThreadKeys.has(`${msg.accountId}:${msg.id}`);
}

// 全局 Ctrl+F 展开并聚焦搜索框 & 点击外部关闭菜单
function onFocusSearchEvent() {
  searchOpen.value = true;
  nextTick(() => {
    searchInputRef.value?.focus();
    searchInputRef.value?.select();
  });
}

function onDocumentPointerDown(e: PointerEvent) {
  if (!filterMenuOpen.value) return;
  if (filterWrapRef.value && !filterWrapRef.value.contains(e.target as Node)) {
    setFilterMenu(false);
  }
}

let ro: ResizeObserver | null = null;
onMounted(() => {
  window.addEventListener("km-focus-search", onFocusSearchEvent);
  document.addEventListener("pointerdown", onDocumentPointerDown);
  if (viewportRef.value) {
    viewportHeight.value = viewportRef.value.clientHeight || 600;
    ro = new ResizeObserver((entries) => {
      viewportHeight.value = entries[0].contentRect.height || 600;
    });
    ro.observe(viewportRef.value);
  }
});

onUnmounted(() => {
  window.removeEventListener("km-focus-search", onFocusSearchEvent);
  document.removeEventListener("pointerdown", onDocumentPointerDown);
  ro?.disconnect();
  if (scrollRaf !== null) cancelAnimationFrame(scrollRaf);
  if (searchTimer) clearTimeout(searchTimer);
  setFilterMenu(false);
});
</script>

<style scoped>
.km-list-pane {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--km-surface, #fafafb);
  color: var(--km-text);
  overflow: hidden;
}

:global(:root[data-theme="system"][data-color-mode="light"]) .km-list-pane {
  background: #fafafb;
}

:global(:root[data-theme="system"][data-color-mode="dark"]) .km-list-pane {
  background: #222226;
}

.km-list-header {
  height: 48px;
  min-height: 48px;
  padding: 0 10px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.km-header-left,
.km-header-right {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
}

.km-header-spacer {
  flex: 1 1 auto;
  height: 100%;
  min-width: 0;
  display: flex;
  align-items: center;
  justify-content: center;
}

.km-icon-btn {
  position: relative;
  width: 30px;
  height: 30px;
  border: none;
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-text-muted);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}

.km-icon-btn:hover {
  background: var(--km-hover-bg);
  color: var(--km-text);
}

.km-icon-btn.active {
  background: var(--km-accent-soft);
  color: var(--km-accent);
}

.km-window-close-btn:hover {
  background: var(--km-danger-soft);
  color: var(--km-danger);
}

.km-filter-active-dot {
  position: absolute;
  top: 5px;
  right: 5px;
  width: 6px;
  height: 6px;
  border-radius: 999px;
  background: var(--km-accent);
}

.km-total-count {
  font-size: 12px;
  font-weight: 500;
  color: var(--km-text-muted);
  padding: 0 4px;
  user-select: none;
}

.km-sync-pill {
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

/* 展开的第二行搜索框 */
.km-search-row {
  padding: 4px 10px 8px;
  display: flex;
  align-items: center;
  gap: 6px;
  border-bottom: 1px solid var(--km-border);
}

.km-search-box {
  flex: 1 1 auto;
  min-width: 0;
  height: 30px;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 8px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-md);
  background: var(--km-canvas);
}

.km-search-box:focus-within {
  border-color: var(--km-accent);
}

.km-search-icon {
  display: inline-flex;
  align-items: center;
  color: var(--km-text-muted);
  flex-shrink: 0;
}

.km-search-input {
  flex: 1 1 auto;
  min-width: 0;
  border: none;
  background: transparent;
  outline: none;
  font-size: 13px;
}

.km-clear-btn {
  border: none;
  background: transparent;
  color: var(--km-text-muted);
  cursor: pointer;
  padding: 2px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.km-clear-btn:hover {
  color: var(--km-text);
}

.km-scope-btn {
  height: 30px;
  padding: 0 8px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-text-muted);
  font-size: 12px;
  cursor: pointer;
  white-space: nowrap;
  flex-shrink: 0;
}

.km-scope-btn:hover {
  background: var(--km-hover-bg);
  color: var(--km-text);
}

.km-scope-btn.active {
  background: var(--km-accent-soft);
  border-color: var(--km-accent);
  color: var(--km-accent);
  font-weight: 600;
}

.km-popover-wrap {
  position: relative;
}

.km-popover-menu {
  position: absolute;
  top: 34px;
  right: 0;
  width: 224px;
  max-height: 440px;
  overflow-y: auto;
  padding: 6px;
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  box-shadow: var(--km-shadow-popover);
  z-index: 40;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.km-menu-title {
  padding: 4px 8px;
  font-size: 11px;
  font-weight: 600;
  color: var(--km-text-muted);
}

.km-menu-check {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 5px 8px;
  border-radius: var(--km-radius-sm);
  cursor: pointer;
  font-size: 12px;
}

.km-menu-check:hover,
.km-menu-item:hover {
  background: var(--km-hover-bg);
}

.km-menu-item {
  border: none;
  background: transparent;
  padding: 5px 8px;
  border-radius: var(--km-radius-sm);
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 12px;
  cursor: pointer;
  text-align: left;
}

.km-menu-item.active {
  color: var(--km-accent);
  font-weight: 600;
}

.km-date-presets {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 4px;
  padding: 2px 6px 4px;
}

.km-date-preset-btn {
  border: 1px solid var(--km-border);
  background: transparent;
  color: var(--km-text);
  border-radius: var(--km-radius-sm);
  padding: 3px 0;
  font-size: 11px;
  cursor: pointer;
}

.km-date-preset-btn:hover {
  background: var(--km-hover-bg);
  border-color: var(--km-accent);
}

.km-date-inputs {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 2px 6px 4px;
}

.km-date-field {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-date-input {
  flex: 1 1 auto;
  min-width: 0;
  height: 24px;
  padding: 0 6px;
  border: 1px solid var(--km-border);
  border-radius: 4px;
  background: var(--km-canvas);
  font-size: 11px;
}

.km-menu-subaction {
  border: none;
  background: transparent;
  color: var(--km-accent);
  text-align: left;
  padding: 3px 8px;
  font-size: 11px;
  cursor: pointer;
}

.km-menu-reset {
  margin-top: 2px;
  border: none;
  background: var(--km-accent-soft);
  color: var(--km-accent);
  padding: 5px 8px;
  border-radius: var(--km-radius-sm);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
}

.km-menu-divider {
  height: 1px;
  background: var(--km-border);
  margin: 4px 0;
}

.km-active-filter-bar {
  height: 26px;
  padding: 0 12px;
  border-bottom: 1px solid var(--km-border);
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 11px;
  color: var(--km-accent);
  background: var(--km-accent-soft);
}

.km-sync-spinner,
.km-initial-spinner {
  width: 12px;
  height: 12px;
  border-radius: 999px;
  border: 2px solid color-mix(in srgb, var(--km-accent) 25%, transparent);
  border-top-color: var(--km-accent);
  animation: km-list-spin 0.8s linear infinite;
  flex-shrink: 0;
}

.km-initial-spinner {
  width: 22px;
  height: 22px;
}

@keyframes km-list-spin {
  to {
    transform: rotate(360deg);
  }
}

.km-empty-title {
  margin: 0;
  font-weight: 600;
  color: var(--km-text);
}

.km-empty-sub {
  margin: 0;
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-filter-summary {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.km-link-btn {
  border: none;
  background: transparent;
  color: var(--km-accent);
  cursor: pointer;
  padding: 0;
  font-size: 11px;
  font-weight: 600;
}

.km-list-error {
  padding: 8px 12px;
  background: var(--km-danger-soft);
  color: var(--km-danger);
  font-size: 12px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.km-list-viewport {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
  position: relative;
}

.km-virtual-spacer {
  position: relative;
  width: 100%;
}

.km-virtual-slot {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  will-change: transform;
}

.km-date-group-header {
  width: 100%;
  height: 28px;
  padding: 0 14px;
  border: none;
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 12px;
  font-weight: 600;
  color: var(--km-text-muted);
  background: transparent;
  cursor: pointer;
  text-align: left;
}

.km-date-group-header:hover {
  color: var(--km-text);
}

.km-date-group-left {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.km-date-chevron {
  opacity: 0.65;
  transition: transform 120ms ease;
}

.km-date-chevron.collapsed {
  transform: rotate(-90deg);
}

.km-date-group-count {
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-list-empty {
  height: 100%;
  min-height: 160px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  color: var(--km-text-muted);
}

.km-empty-reset {
  padding: 5px 12px;
  border-radius: var(--km-radius-sm);
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  cursor: pointer;
}

.km-loading-more {
  padding: 10px;
  text-align: center;
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-load-older-bar {
  padding: 10px 12px 14px;
  display: flex;
  justify-content: center;
}

.km-load-older-btn {
  padding: 6px 14px;
  border-radius: var(--km-radius-sm);
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  color: var(--km-text);
  font-size: 12px;
  cursor: pointer;
}

.km-load-older-btn:hover:not(:disabled) {
  border-color: var(--km-accent);
  color: var(--km-accent);
}

.km-load-older-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
</style>

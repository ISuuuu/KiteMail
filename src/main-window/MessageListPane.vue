<template>
  <section class="km-list-pane" aria-label="邮件列表">
    <!-- 顶部栏 48px -->
    <header class="km-list-header">
      <div class="km-search-box">
        <span class="km-search-icon" aria-hidden="true">🔍</span>
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
          v-if="draftQuery"
          type="button"
          class="km-clear-btn"
          title="清空搜索"
          aria-label="清空搜索"
          @click="clearSearch"
        >
          ✕
        </button>
      </div>

      <!-- 搜索范围 -->
      <button
        type="button"
        class="km-toolbar-btn"
        :class="{ active: searchScope === 'allFolders' }"
        :title="searchScope === 'allFolders' ? '当前：跨文件夹搜索' : '当前：仅搜本文件夹'"
        @click="toggleScope"
      >
        {{ searchScope === "allFolders" ? "全部" : "本夹" }}
      </button>

      <!-- 筛选弹出菜单 -->
      <div ref="filterWrapRef" class="km-popover-wrap">
        <button
          type="button"
          class="km-toolbar-btn"
          :class="{ active: hasActiveFilter }"
          aria-haspopup="menu"
          :aria-expanded="filterMenuOpen"
          @click="toggleFilterMenu"
        >
          筛选{{ activeFilterCount > 0 ? `(${activeFilterCount})` : "" }}
        </button>

        <div v-if="filterMenuOpen" class="km-popover-menu" role="menu">
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
              :checked="Boolean(filterSet.today)"
              @change="toggleFilterFlag('today')"
            />
            <span>今天</span>
          </label>
          <button
            v-if="hasActiveFilter"
            type="button"
            class="km-menu-reset"
            @click="clearAllFilters"
          >
            重置筛选
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
            <span v-if="sortMode === opt.value">✓</span>
          </button>
        </div>
      </div>

      <button
        type="button"
        class="km-toolbar-btn"
        title="刷新当前列表 (F5)"
        aria-label="刷新"
        @click="emit('refresh')"
      >
        ↻
      </button>
    </header>

    <!-- 列表摘要栏 -->
    <div class="km-list-subbar">
      <span class="km-total-text">共 {{ total.toLocaleString() }} 封</span>
      <span v-if="syncStatus?.syncing" class="km-sync-text" role="status">
        <span class="km-sync-spinner" aria-hidden="true" />
        <span>{{ syncStatus.message || "正在同步邮件…" }}</span>
      </span>
      <span v-else-if="hasActiveFilter" class="km-filter-summary">
        已启用筛选
        <button type="button" class="km-link-btn" @click="clearAllFilters">清除</button>
      </span>
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
          <div v-if="item.type === 'header'" class="km-date-group-header">
            {{ item.label }}
          </div>
          <MessageRow
            v-else
            :message="item.message"
            :selected="isMessageSelected(item.message)"
            :wide="listWide"
            :compact="compactList"
            :showAvatar="showAvatars"
            @select="emit('selectMessage', $event)"
            @toggleSeen="(ref, seen) => emit('toggleSeen', ref, seen)"
            @toggleFlagged="(ref, flagged) => emit('toggleFlagged', ref, flagged)"
          />
        </div>
      </div>

      <div v-if="loading && messages.length > 0" class="km-loading-more">
        正在加载更多邮件…
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
} from "../ipc/types";
import { computeAnchorScrollTop, type MessageGroup } from "./useMainWindow";
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

const props = defineProps<{
  messages: MessageSummaryDto[];
  groupedMessages: MessageGroup[];
  total: number;
  loading: boolean;
  error: string | null;
  hasMore: boolean;
  searchQuery: string;
  searchScope: SearchScope;
  sortMode: MessageSort;
  filterSet: MessageFilterDto;
  selectedId: MessageRef | null;
  listWide: boolean;
  readerPosition: ReaderPosition;
  compactList: boolean;
  showAvatars: boolean;
  scrollResetToken?: number;
  syncStatus?: SyncStatusDto;
  cacheReady?: boolean;
}>();

const emit = defineEmits<{
  selectMessage: [ref: MessageRef];
  toggleSeen: [ref: MessageRef, seen: boolean];
  toggleFlagged: [ref: MessageRef, flagged: boolean];
  setSearch: [query: string];
  setScope: [scope: SearchScope];
  setSort: [sort: MessageSort];
  setFilter: [filter: MessageFilterDto];
  refresh: [];
  loadMore: [];
  overlayChange: [id: string, open: boolean];
}>();

const searchInputRef = ref<HTMLInputElement | null>(null);
const filterWrapRef = ref<HTMLElement | null>(null);
const viewportRef = ref<HTMLElement | null>(null);

// 搜索与 IME 组合状态
const draftQuery = ref(props.searchQuery);
const isComposing = ref(false);
let searchTimer: ReturnType<typeof setTimeout> | null = null;

watch(
  () => props.searchQuery,
  (val) => {
    if (draftQuery.value !== val) {
      draftQuery.value = val;
    }
  }
);

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
  emit("setSearch", "");
}

function toggleScope() {
  emit("setScope", props.searchScope === "thisFolder" ? "allFolders" : "thisFolder");
}

// 筛选/排序弹出层
const filterMenuOpen = ref(false);

function setFilterMenu(open: boolean) {
  if (filterMenuOpen.value === open) return;
  filterMenuOpen.value = open;
  emit("overlayChange", "list-filter-menu", open);
}

function toggleFilterMenu() {
  setFilterMenu(!filterMenuOpen.value);
}

const activeFilterCount = computed(() => {
  const f = props.filterSet;
  return Number(Boolean(f.unread)) + Number(Boolean(f.starred)) + Number(Boolean(f.hasAttachment)) + Number(Boolean(f.today));
});

const hasActiveFilter = computed(() => activeFilterCount.value > 0);

function toggleFilterFlag(key: keyof MessageFilterDto) {
  const next: MessageFilterDto = {
    ...props.filterSet,
    [key]: !props.filterSet[key],
  };
  emit("setFilter", next);
}

function clearAllFilters() {
  emit("setFilter", {});
  setFilterMenu(false);
}

function resetQueryAndFilters() {
  clearSearch();
  emit("setFilter", {});
}

function selectSort(sort: MessageSort) {
  emit("setSort", sort);
  setFilterMenu(false);
}

// ---------- 虚拟列表布局计算 ----------
type VirtualItem =
  | { type: "header"; key: string; label: string; top: number; height: number }
  | { type: "row"; key: string; message: MessageSummaryDto; top: number; height: number };

const rowHeight = computed(() => {
  if (props.listWide) return 38;
  return props.compactList ? 66 : 90;
});

const virtualLayout = computed(() => {
  const items: VirtualItem[] = [];
  const rowIndexByKey = new Map<string, number>();
  let top = 0;
  const rh = rowHeight.value;

  for (const group of props.groupedMessages) {
    if (group.label) {
      items.push({
        type: "header",
        key: `hdr:${group.label}`,
        label: group.label,
        top,
        height: 28,
      });
      top += 28;
    }
    for (const msg of group.items) {
      const key = `${msg.accountId}:${msg.id}`;
      rowIndexByKey.set(key, items.length);
      items.push({
        type: "row",
        key,
        message: msg,
        top,
        height: rh,
      });
      top += rh;
    }
  }

  return { items, totalHeight: top, rowIndexByKey };
});

const totalHeight = computed(() => virtualLayout.value.totalHeight);

const scrollTop = ref(0);
const viewportHeight = ref(600);
const OVERSCAN_PX = 360;

function findStartIndex(items: VirtualItem[], targetTop: number): number {
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
    if (props.hasMore && !props.loading && el.scrollTop + el.clientHeight >= el.scrollHeight - 400) {
      emit("loadMore");
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

// 键盘上下选信时确保选中行在视口内可见
watch(
  () => props.selectedId,
  (sel) => {
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
);

function isMessageSelected(msg: MessageSummaryDto): boolean {
  return (
    props.selectedId?.accountId === msg.accountId &&
    props.selectedId?.messageId === msg.id
  );
}

// 全局 Ctrl+F 聚焦 & 点击外部关闭菜单
function onFocusSearchEvent() {
  searchInputRef.value?.focus();
  searchInputRef.value?.select();
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
  background: var(--km-surface);
  color: var(--km-text);
  overflow: hidden;
}

.km-list-header {
  height: 48px;
  min-height: 48px;
  padding: 0 10px;
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
  font-size: 12px;
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
  padding: 0 2px;
  font-size: 11px;
}

.km-toolbar-btn {
  height: 30px;
  padding: 0 8px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface-raised);
  color: var(--km-text);
  font-size: 12px;
  cursor: pointer;
  white-space: nowrap;
  flex-shrink: 0;
}

.km-toolbar-btn:hover {
  background: var(--km-hover-bg);
}

.km-toolbar-btn.active {
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
  width: 200px;
  max-height: 360px;
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

.km-menu-reset {
  margin-top: 2px;
  border: none;
  background: var(--km-accent-soft);
  color: var(--km-accent);
  padding: 4px 8px;
  border-radius: var(--km-radius-sm);
  font-size: 12px;
  cursor: pointer;
}

.km-menu-divider {
  height: 1px;
  background: var(--km-border);
  margin: 4px 0;
}

.km-list-subbar {
  height: 28px;
  min-height: 28px;
  padding: 0 12px;
  border-bottom: 1px solid var(--km-border);
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-sync-text {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  color: var(--km-accent);
  font-weight: 500;
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
  height: 28px;
  padding: 0 14px;
  display: flex;
  align-items: center;
  font-size: 11px;
  font-weight: 700;
  color: var(--km-text-muted);
  background: var(--km-surface);
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
</style>

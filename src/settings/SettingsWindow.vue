<template>
  <div ref="rootRef" class="km-set-root">
    <!-- 左栏：标题栏 + 分组导航（宽窗常显；窄窗仅导航态显示） -->
    <aside
      v-show="showSidebar"
      class="km-set-sidebar"
      :style="narrow ? undefined : { width: `${sidebarWidth}px` }"
    >
      <header class="km-set-titlebar">
        <div class="km-set-drag" data-tauri-drag-region>
          <span class="km-set-title">偏好设置</span>
        </div>
        <!-- 搜索开关（GPUI render_sidebar：标题右侧搜索图标） -->
        <button
          type="button"
          class="km-set-search-toggle"
          :title="searchOpen ? '收起搜索 (Esc)' : '搜索设置 (Ctrl+F)'"
          :aria-label="searchOpen ? '收起设置搜索' : '搜索设置'"
          @click="toggleSearch"
        >
          <SettingsIcon :name="searchOpen ? 'close' : 'search'" :size="14" />
        </button>
        <WindowControls v-if="narrowOnlySidebar" />
      </header>

      <!-- 搜索输入框（展开时自动聚焦，Esc 收起） -->
      <div v-if="searchOpen" class="km-set-search-box">
        <input
          ref="searchInputRef"
          v-model="searchQuery"
          type="text"
          class="km-set-search-input"
          placeholder="搜索设置…"
          aria-label="搜索设置"
          @keydown.escape.stop.prevent="closeSearch"
        />
      </div>

      <!-- 有查询时：两行结果替换导航；空结果：原提示样式（GPUI render_search_results） -->
      <div v-if="searchOpen && searchQuery.trim()" class="km-set-search-results" role="listbox">
        <template v-if="searchResults.length > 0">
          <button
            v-for="result in searchResults"
            :key="result.id"
            type="button"
            class="km-set-search-item"
            role="option"
            @click="activateSearchHit(result)"
          >
            <span class="km-set-search-item-title">{{ result.title }}</span>
            <span class="km-set-search-item-tab">{{ searchTabLabel(result.tabId) }}</span>
          </button>
        </template>
        <div v-else class="km-set-search-empty">
          <SettingsIcon name="search" :size="16" />
          <span>未找到匹配的设置项</span>
        </div>
      </div>

      <nav v-else class="km-set-nav" aria-label="设置分类">
        <div v-for="group in SETTINGS_TAB_GROUPS" :key="group.id" class="km-set-nav-group">
          <div class="km-set-nav-group-label">{{ group.label }}</div>
          <button
            v-for="tab in tabsOfGroup(group.id)"
            :key="tab.id"
            type="button"
            class="km-set-nav-item"
            :class="{ active: activeTabId === tab.id }"
            :title="tab.label"
            @click="selectTab(tab.id)"
          >
            <SettingsIcon :name="tab.icon" :size="14" class="km-set-nav-icon" />
            <span class="km-set-nav-text">{{ tab.label }}</span>
          </button>
        </div>
      </nav>
    </aside>

    <!-- 右栏：内容顶栏 + 独立滚动内容 -->
    <section v-show="showContent" class="km-set-content-col">
      <header class="km-set-content-header">
        <button
          v-if="narrow && narrowShowContent"
          type="button"
          class="km-set-back-btn"
          @click="onNarrowBack"
        >
          ← 返回
        </button>
        <div class="km-set-drag km-set-header-drag" data-tauri-drag-region>
          <span class="km-set-header-title">{{ headerTitle }}</span>
        </div>
        <!-- 账号编辑态单顶栏（WP-07）：移除 / 保存上提到壳层（GPUI mod.rs#L2465） -->
        <div v-if="accountChrome.editorActive" class="km-set-editor-actions">
          <button
            v-if="accountChrome.canRemove"
            type="button"
            class="km-set-editor-btn km-set-editor-danger"
            :disabled="accountChrome.busy"
            @click="accountsPageRef?.toggleRemoveConfirm()"
          >
            移除
          </button>
          <button
            type="button"
            class="km-set-editor-btn km-set-editor-primary"
            :disabled="accountChrome.busy"
            @click="accountsPageRef?.saveFromChrome()"
          >
            {{ accountChrome.busy ? "正在保存…" : "保存" }}
          </button>
        </div>
        <WindowControls />
      </header>

      <div
        v-if="activeTab.id !== 'mailAccounts'"
        class="km-set-subtitle"
      >
        {{ activeTab.subtitle }}
      </div>

      <div class="km-set-content-scroll km-pref-panels">
        <div v-if="state.loading.value" class="km-set-state">
          正在加载偏好设置…
        </div>
        <div v-else-if="state.loadError.value" class="km-set-state km-set-state-error" role="alert">
          <p>{{ state.loadError.value }}</p>
          <button type="button" class="km-set-retry-btn" @click="state.init()">
            重试
          </button>
        </div>
        <template v-else>
          <GeneralPage v-if="activeTab.id === 'general'" :state="state" />
          <AccountsPage
            v-else-if="activeTab.id === 'mailAccounts'"
            ref="accountsPageRef"
            hostChrome
            :highlightToken="accountHighlightToken"
            @chrome="onAccountChrome"
          />
          <TagPage v-else-if="activeTab.id === 'tags'" />
          <FilterPage v-else-if="activeTab.id === 'filters'" />
          <SmartFolderPage v-else-if="activeTab.id === 'smartFolders'" />
          <SendersPage
            v-else-if="activeTab.id === 'senders'"
            :state="state"
            @goto-privacy="selectTab('privacy')"
          />
          <PgpKeysPage v-else-if="activeTab.id === 'openPgp'" />
          <CloudStoragePage v-else-if="activeTab.id === 'cloudStorage'" />
          <TranslationPage v-else-if="activeTab.id === 'translation'" />
          <ContactsSourcesPage v-else-if="activeTab.id === 'contactsSources'" />
          <AppearancePage v-else-if="activeTab.id === 'appearance'" :state="state" />
          <FocusPage v-else-if="activeTab.id === 'focusMode'" :state="state" />
          <SidebarPage v-else-if="activeTab.id === 'sidebar'" :state="state" />
          <MessageListPage v-else-if="activeTab.id === 'messageList'" :state="state" />
          <ConversationsPage v-else-if="activeTab.id === 'conversations'" :state="state" />
          <ReadingPage v-else-if="activeTab.id === 'reading'" :state="state" />
          <ComposingPage v-else-if="activeTab.id === 'composing'" :state="state" />
          <PrivacyPage v-else-if="activeTab.id === 'privacy'" :state="state" />
          <McpPage v-else-if="activeTab.id === 'mcp'" :state="state" />
          <DateTimePage v-else-if="activeTab.id === 'dateTime'" :state="state" />
          <SystemPage v-else-if="activeTab.id === 'system'" :state="state" />
          <BackupPage v-else-if="activeTab.id === 'backup'" :state="state" />
          <PlaceholderPage v-else :label="activeTab.label" />
        </template>

        <div
          v-if="state.banner.value && !state.loading.value && !state.loadError.value"
          class="km-banner"
          :class="`is-${state.banner.value.kind}`"
          role="status"
        >
          {{ state.banner.value.text }}
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { KNOWN_THEMES, resolveDarkMode } from "../main-window/usePaneLayout";
import type { FullPreferencesDto } from "../preferences/types";
import { usePreferences } from "../preferences/usePreferences";
import {
  filterSearchEntries,
  HIGHLIGHT_CLASS,
  HIGHLIGHT_MS,
  type PrefSearchEntry,
} from "./settingsSearch";
import AccountsPage from "./AccountsPage.vue";
import AppearancePage from "./AppearancePage.vue";
import BackupPage from "./BackupPage.vue";
import CloudStoragePage from "./CloudStoragePage.vue";
import ComposingPage from "./ComposingPage.vue";
import ContactsSourcesPage from "./ContactsSourcesPage.vue";
import ConversationsPage from "./ConversationsPage.vue";
import DateTimePage from "./DateTimePage.vue";
import FilterPage from "./FilterPage.vue";
import FocusPage from "./FocusPage.vue";
import GeneralPage from "./GeneralPage.vue";
import McpPage from "./McpPage.vue";
import MessageListPage from "./MessageListPage.vue";
import PgpKeysPage from "./PgpKeysPage.vue";
import PrivacyPage from "./PrivacyPage.vue";
import ReadingPage from "./ReadingPage.vue";
import SidebarPage from "./SidebarPage.vue";
import SystemPage from "./SystemPage.vue";
import PlaceholderPage from "./PlaceholderPage.vue";
import SendersPage from "./SendersPage.vue";
import SmartFolderPage from "./SmartFolderPage.vue";
import TagPage from "./TagPage.vue";
import TranslationPage from "./TranslationPage.vue";
import SettingsIcon from "./SettingsIcon.vue";
import WindowControls from "./WindowControls.vue";
import {
  DEFAULT_SETTINGS_TAB,
  SETTINGS_TAB_GROUPS,
  computeSidebarWidth,
  findSettingsTab,
  isNarrowViewport,
  tabsOfGroup,
  type SettingsTabId,
} from "./nav";

/**
 * 独立设置窗壳层（WP-01）：
 * - 双栏 46px 顶栏（左「偏好设置」/ 右当前页标题），标题区可拖动，窗体按钮同排；
 * - 左栏 27% 钳制 180–250px，账号/设置两组 22 页签，行高 34px 圆角 8px；
 * - 左右两栏独立滚动；窄窗（≤640px）切换全宽导航 / 全宽内容。
 */
const state = usePreferences();

const rootRef = ref<HTMLElement | null>(null);
const windowWidth = ref(920);
const narrowShowContent = ref(false);
// 设置窗自身维护 22 页签路由（usePreferences.activeTab 保持旧弹窗 6 页签语义不变）。
const activeTabId = ref<SettingsTabId>(DEFAULT_SETTINGS_TAB);

let resizeObserver: ResizeObserver | null = null;

const narrow = computed(() => isNarrowViewport(windowWidth.value));
const sidebarWidth = computed(() => computeSidebarWidth(windowWidth.value));
const narrowOnlySidebar = computed(() => narrow.value && !narrowShowContent.value);
const showSidebar = computed(() => !narrow.value || !narrowShowContent.value);
const showContent = computed(() => !narrow.value || narrowShowContent.value);
const activeTab = computed(() => findSettingsTab(activeTabId.value));

// ---- 账号编辑态单顶栏（WP-07）：AccountsPage 上抛状态，壳层承载 返回/移除/保存 ----
const accountsPageRef = ref<InstanceType<typeof AccountsPage> | null>(null);
const accountChrome = reactive({
  editorActive: false,
  title: "编辑账号",
  canRemove: false,
  busy: false,
});

function onAccountChrome(
  chrome: { editorActive: boolean; title: string; canRemove: boolean; busy: boolean }
) {
  Object.assign(accountChrome, chrome);
}

const headerTitle = computed(() =>
  activeTab.value.id === "mailAccounts" && accountChrome.editorActive
    ? accountChrome.title
    : activeTab.value.label
);

/** 窄窗「返回」：账号编辑态先回账号列表，列表态再回导航（GPUI 语义）。 */
function onNarrowBack() {
  if (activeTab.value.id === "mailAccounts" && accountChrome.editorActive) {
    accountsPageRef.value?.backToAccountList();
    return;
  }
  narrowShowContent.value = false;
}

function selectTab(id: SettingsTabId) {
  activeTabId.value = id;
  // 窄窗：点击页签进入内容（对齐 GPUI select_tab → narrow_show_content = true）。
  // 用实时 innerWidth 兜底（ResizeObserver 在个别环境下有延迟）。
  if (narrow.value || isNarrowViewport(window.innerWidth)) {
    narrowShowContent.value = true;
    windowWidth.value = Math.min(windowWidth.value, window.innerWidth);
  }
}

// ---- 设置搜索（WP-09，对齐 GPUI mod.rs render_sidebar / search.rs） ----
const searchOpen = ref(false);
const searchQuery = ref("");
const searchInputRef = ref<HTMLInputElement | null>(null);
const searchResults = computed(() => filterSearchEntries(searchQuery.value));
/** 账号卡高亮 token（递增触发 AccountsPage 卡片级高亮） */
const accountHighlightToken = ref(0);
/** 命中行高亮的清理定时器（重复命中时先清旧的） */
let highlightTimer: ReturnType<typeof setTimeout> | null = null;

function searchTabLabel(tabId: SettingsTabId): string {
  return findSettingsTab(tabId).label;
}

function toggleSearch() {
  if (searchOpen.value) {
    closeSearch();
  } else {
    searchOpen.value = true;
    nextTick(() => searchInputRef.value?.focus());
  }
}

function closeSearch() {
  // 对齐 GPUI close_search：清空查询与输入。
  searchOpen.value = false;
  searchQuery.value = "";
}

/** 命中激活：切页 → 滚动至设置行 → 1.6s 行高亮（窄窗收起搜索面板）。 */
function activateSearchHit(entry: PrefSearchEntry) {
  selectTab(entry.tabId);
  if (narrow.value || isNarrowViewport(window.innerWidth)) {
    // 窄窗：进入内容并收起搜索面板（GPUI apply_search_hit_immediate）。
    searchOpen.value = false;
  }
  if (highlightTimer) {
    clearTimeout(highlightTimer);
    document.querySelectorAll(`.${HIGHLIGHT_CLASS}`).forEach((el) => el.classList.remove(HIGHLIGHT_CLASS));
    highlightTimer = null;
  }
  void nextTick(() => {
    // 等页面渲染完成后再定位（切页 + 异步数据均有缓冲）。
    window.setTimeout(() => {
      // 邮箱账号页：账号列表卡片级高亮（AccountsPage token 触发，含滚动）
      if (entry.tabId === "mailAccounts") {
        accountHighlightToken.value += 1;
        return;
      }
      if (entry.anchorId) {
        const el = document.getElementById(entry.anchorId);
        if (el) {
          el.scrollIntoView({ block: "center" });
          el.classList.add(HIGHLIGHT_CLASS);
          highlightTimer = setTimeout(() => {
            el.classList.remove(HIGHLIGHT_CLASS);
            highlightTimer = null;
          }, HIGHLIGHT_MS);
          return;
        }
      }
      // 无锚点条目（并行开发中的三页）：仅切页不滚动
    }, 60);
  });
}

function onSettingsKeydown(e: KeyboardEvent) {
  if (e.isComposing || e.key === "Process" || e.keyCode === 229) return;
  // Ctrl+F：打开并聚焦设置搜索
  if (e.ctrlKey && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "f") {
    e.preventDefault();
    searchOpen.value = true;
    nextTick(() => searchInputRef.value?.focus());
  }
}

// 窗口拉宽脱离窄窗时复位分支状态，回到常规双栏。
watch(narrow, (isNarrow, prev) => {
  if (prev && !isNarrow) {
    narrowShowContent.value = false;
  }
});

// ---- 主题同步（独立 WebView 需自行应用亮暗配色基准） ----
function syncDocumentTheme(prefs: FullPreferencesDto) {
  const id = KNOWN_THEMES.includes(prefs.themeId as (typeof KNOWN_THEMES)[number])
    ? prefs.themeId
    : "system";
  const dark = resolveDarkMode(prefs.darkMode);
  const root = document.documentElement;
  root.setAttribute("data-theme", id);
  root.setAttribute("data-color-mode", dark ? "dark" : "light");
}

watch(
  () => state.preferences.value,
  (prefs) => {
    if (prefs) syncDocumentTheme(prefs);
  },
  { immediate: true }
);

let schemeQuery: MediaQueryList | null = null;
function onSystemSchemeChange() {
  if (state.preferences.value.darkMode === null) {
    syncDocumentTheme(state.preferences.value);
  }
}

onMounted(() => {
  // 加载偏好数据；页签路由由设置窗本地 activeTabId 管理，不写入旧弹窗的 6 页签状态。
  void state.init();
  window.addEventListener("keydown", onSettingsKeydown);
  if (rootRef.value) {
    const rect = rootRef.value.getBoundingClientRect();
    if (rect.width > 0) {
      windowWidth.value = rect.width;
    }
    resizeObserver = new ResizeObserver((entries) => {
      const cr = entries[0]?.contentRect;
      if (cr && cr.width > 0) {
        windowWidth.value = cr.width;
      }
    });
    resizeObserver.observe(rootRef.value);
  }
  if (typeof window.matchMedia === "function") {
    schemeQuery = window.matchMedia("(prefers-color-scheme: dark)");
    schemeQuery.addEventListener("change", onSystemSchemeChange);
  }
});

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onSettingsKeydown);
  if (highlightTimer) {
    clearTimeout(highlightTimer);
    highlightTimer = null;
  }
  resizeObserver?.disconnect();
  resizeObserver = null;
  schemeQuery?.removeEventListener("change", onSystemSchemeChange);
  schemeQuery = null;
});
</script>

<style scoped>
.km-set-root {
  width: 100%;
  height: 100%;
  display: flex;
  overflow: hidden;
  background: var(--km-canvas);
  color: var(--km-text);
}

/* ---- 左栏 ---- */
.km-set-sidebar {
  flex-shrink: 0;
  min-width: 180px;
  max-width: 250px;
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--km-sidebar);
  border-right: 1px solid var(--km-border);
}

.km-set-titlebar,
.km-set-content-header {
  height: 46px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 0 12px;
}

.km-set-titlebar {
  justify-content: space-between;
}

/* 搜索开关按钮 */
.km-set-search-toggle {
  width: 28px;
  height: 28px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-text-muted);
  cursor: pointer;
  padding: 0;
  flex-shrink: 0;
}

.km-set-search-toggle:hover {
  background: var(--km-hover-bg);
  color: var(--km-text);
}

/* 搜索输入框行（GPUI：px_2p5 py_2 border_b） */
.km-set-search-box {
  flex-shrink: 0;
  padding: 8px 10px;
  border-bottom: 1px solid var(--km-border);
}

.km-set-search-input {
  width: 100%;
  height: 30px;
  padding: 0 10px;
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  color: var(--km-text);
  font-size: 12px;
  user-select: text;
}

.km-set-search-input:focus {
  outline: 2px solid var(--km-accent);
  outline-offset: -2px;
}

/* 搜索结果列表（替换导航；两行：设置名 + 所属页） */
.km-set-search-results {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 8px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.km-set-search-item {
  width: 100%;
  padding: 7px 10px;
  border: none;
  border-radius: var(--km-radius-sm);
  background: transparent;
  text-align: left;
  display: flex;
  flex-direction: column;
  gap: 2px;
  cursor: pointer;
}

.km-set-search-item:hover {
  background: var(--km-surface);
}

.km-set-search-item-title {
  font-size: 13px;
  color: var(--km-text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.km-set-search-item-tab {
  font-size: 12px;
  color: var(--km-text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* 空结果提示（GPUI 居中图标 + 文案） */
.km-set-search-empty {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 16px;
  color: var(--km-text-muted);
  font-size: 12px;
}

.km-set-drag {
  flex: 1;
  min-width: 0;
  height: 100%;
  display: flex;
  align-items: center;
  overflow: hidden;
}

.km-set-title,
.km-set-header-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--km-text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.km-set-nav {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 10px 8px 12px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.km-set-nav-group {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.km-set-nav-group-label {
  padding: 0 10px 4px;
  font-size: 12px;
  font-weight: 600;
  color: var(--km-text-muted);
}

.km-set-nav-item {
  height: 34px;
  padding: 0 10px;
  gap: 8px;
  border: none;
  border-radius: var(--km-radius-md);
  background: transparent;
  color: var(--km-text);
  display: flex;
  align-items: center;
  cursor: pointer;
  font-size: 12px;
  font-weight: 500;
  text-align: left;
  width: 100%;
}

.km-set-nav-item:hover:not(.active) {
  background: var(--km-surface);
}

.km-set-nav-item.active {
  background: var(--km-accent-soft);
  color: var(--km-accent);
  font-weight: 600;
}

.km-set-nav-icon {
  flex-shrink: 0;
  color: var(--km-text-muted);
}

.km-set-nav-item.active .km-set-nav-icon {
  color: var(--km-accent);
}

.km-set-nav-text {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* ---- 右栏 ---- */
.km-set-content-col {
  flex: 1;
  min-width: 0;
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--km-canvas);
}

.km-set-content-header {
  justify-content: flex-start;
  gap: 8px;
  padding: 0 16px;
  border-bottom: 1px solid var(--km-border);
}

.km-set-header-drag {
  flex: 1;
}

.km-set-back-btn {
  flex-shrink: 0;
  height: 28px;
  padding: 0 10px;
  border: none;
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-text);
  font-size: 12px;
  cursor: pointer;
}

.km-set-back-btn:hover {
  background: var(--km-hover-bg);
}

/* 账号编辑态单顶栏动作按钮（对齐 GPUI danger/primary compact） */
.km-set-editor-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

.km-set-editor-btn {
  height: 28px;
  padding: 0 12px;
  border-radius: var(--km-radius-md);
  border: 1px solid transparent;
  background: transparent;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  white-space: nowrap;
}

.km-set-editor-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.km-set-editor-danger {
  border-color: color-mix(in srgb, var(--km-danger) 50%, var(--km-border));
  background: transparent;
  color: var(--km-danger);
}

.km-set-editor-danger:hover:not(:disabled) {
  background: var(--km-danger-soft);
}

.km-set-editor-primary {
  border-color: var(--km-accent);
  background: var(--km-accent);
  color: var(--km-accent-fg);
}

.km-set-editor-primary:hover:not(:disabled) {
  background: color-mix(in srgb, var(--km-accent) 88%, var(--km-accent-fg));
}

.km-set-subtitle {
  flex-shrink: 0;
  padding: 12px 24px 0;
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-set-content-scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 20px 24px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 16px;
}

.km-system-panels {
  display: flex;
  flex-direction: column;
  gap: 16px;
  width: 100%;
}

.km-set-state {
  padding: 28px;
  text-align: center;
  color: var(--km-text-muted);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
}

.km-set-state-error {
  color: var(--km-danger);
}

.km-set-state p {
  margin: 0;
  word-break: break-word;
}

.km-set-retry-btn {
  padding: 6px 16px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-md);
  background: var(--km-accent);
  color: var(--km-accent-fg);
  cursor: pointer;
  font-size: 12px;
}

/* 窄窗：全宽导航 / 全宽内容切换（≤640px） */
@media (max-width: 640px) {
  .km-set-sidebar {
    width: 100%;
    max-width: 100%;
    border-right: none;
  }
}
</style>

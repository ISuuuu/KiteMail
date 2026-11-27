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
        <WindowControls v-if="narrowOnlySidebar" @close="requestCloseWindow" />
      </header>

      <nav class="km-set-nav" aria-label="设置分类">
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
        <WindowControls @close="requestCloseWindow" />
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
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { KNOWN_THEMES, resolveDarkMode } from "../main-window/usePaneLayout";
import { isTauriRuntime } from "./windowEnv";
import type { FullPreferencesDto } from "../preferences/types";
import { usePreferences } from "../preferences/usePreferences";
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
  SETTINGS_TABS,
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
  if (narrow.value) {
    narrowShowContent.value = true;
  }
}

// ---- 入口定位与默认页签（WP-10，对照 GPUI app.rs#L11619 open_preferences） ----
// GPUI 载体为持久偏好 settings_open_accounts；该字段不在前端偏好 IPC 契约内，
// 这里以 URL 参数 ?tab=<页签 id> 作为最小等价载体（新建窗口时由入口方携带）。
function initialTabFromUrl(): SettingsTabId | null {
  if (typeof window === "undefined") return null;
  try {
    const tab = new URLSearchParams(window.location.search).get("tab");
    if (!tab) return null;
    const meta = SETTINGS_TABS.find((t) => t.id === tab);
    return meta ? meta.id : null;
  } catch {
    return null;
  }
}

// ---- Ctrl+W / Esc 逐层收合与关窗守卫（WP-10，对照 GPUI mod.rs#L2677 CloseOverlay） ----
let keyboardListenerAttached = false;

/** 原生关窗（浏览器预览无 Tauri 桥，安全跳过）。 */
function closeNativeWindow() {
  if (!isTauriRuntime()) return;
  void getCurrentWindow().close();
}

/**
 * 宿主关窗入口：账号编辑态脏数据时挂起未保存守卫（GPUI request_leave_editor 语义），
 * 返回 true 表示已被守卫拦截（壳层暂不关窗）。
 */
function requestCloseWindow(): boolean {
  if (accountsPageRef.value?.requestHostClose()) {
    return true;
  }
  closeNativeWindow();
  return false;
}

function onKeyDown(event: KeyboardEvent) {
  // Ctrl+W 关窗（守卫拦截时显示未保存横幅而非关窗）
  if (event.ctrlKey && !event.altKey && !event.shiftKey && event.key.toLowerCase() === "w") {
    event.preventDefault();
    event.stopPropagation();
    requestCloseWindow();
    return;
  }
  if (event.key !== "Escape") {
    return;
  }
  // ComboSelect 菜单打开时优先由其自身 Esc 关闭（GPUI CloseOverlay 优先级）
  if (document.querySelector(".km-combo-menu")) {
    return;
  }
  event.preventDefault();
  event.stopPropagation();
  // 逐层收合（GPUI mod.rs#L2690 CloseOverlay 顺序的等价实现）：
  // 1) 未保存守卫横幅 → 取消（留在编辑页）
  if (accountsPageRef.value?.pendingLeave) {
    accountsPageRef.value.cancelPendingLeave();
    return;
  }
  // 2) 账号编辑态 → 先回账号列表（脏数据由守卫挂起，见 useAccountLayers.requestBack）
  if (activeTab.value.id === "mailAccounts" && accountChrome.editorActive) {
    accountsPageRef.value?.backToAccountList();
    return;
  }
  // 3) 窄窗内容 → 回导航
  if (narrow.value && narrowShowContent.value) {
    narrowShowContent.value = false;
    return;
  }
  // 4) 最外层 → 守卫感知关窗
  requestCloseWindow();
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
  // 入口定位：URL ?tab= 参数（settings_open_accounts / 账号管理入口的最小等价载体），
  // 未携带时默认「常规」（GPUI open_preferences）。
  const initialTab = initialTabFromUrl();
  if (initialTab) {
    activeTabId.value = initialTab;
  }
  // 加载偏好数据；页签路由由设置窗本地 activeTabId 管理，不写入旧弹窗的 6 页签状态。
  void state.init();
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
  // Ctrl+W / Esc 逐层收合（GPUI mod.rs#L2677）：capture 确保先于页内输入处理
  if (!keyboardListenerAttached) {
    window.addEventListener("keydown", onKeyDown, true);
    keyboardListenerAttached = true;
  }
});

onBeforeUnmount(() => {
  resizeObserver?.disconnect();
  resizeObserver = null;
  schemeQuery?.removeEventListener("change", onSystemSchemeChange);
  schemeQuery = null;
  if (keyboardListenerAttached) {
    window.removeEventListener("keydown", onKeyDown, true);
    keyboardListenerAttached = false;
  }
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

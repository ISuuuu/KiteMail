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
        <WindowControls v-if="narrowOnlySidebar" />
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
          @click="narrowShowContent = false"
        >
          ← 返回
        </button>
        <div class="km-set-drag km-set-header-drag" data-tauri-drag-region>
          <span class="km-set-header-title">{{ activeTab.label }}</span>
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
          <AccountsPage v-else-if="activeTab.id === 'mailAccounts'" />
          <TagPage v-else-if="activeTab.id === 'tags'" />
          <FilterPage v-else-if="activeTab.id === 'filters'" />
          <SmartFolderPage v-else-if="activeTab.id === 'smartFolders'" />
          <SendersPage
            v-else-if="activeTab.id === 'senders'"
            :state="state"
            @goto-privacy="selectTab('privacy')"
          />
          <PgpKeysPage v-else-if="activeTab.id === 'openPgp'" />
          <AppearancePage v-else-if="activeTab.id === 'appearance'" :state="state" />
          <template v-else-if="activeTab.id === 'composing'">
            <ComposePanel :state="state" />
          </template>
          <template v-else-if="activeTab.id === 'privacy'">
            <PrivacyPanel :state="state" />
          </template>
          <template v-else-if="activeTab.id === 'system'">
            <div class="km-panels-wide km-system-panels">
              <StoragePanel :state="state" />
              <AboutPanel :state="state" />
            </div>
          </template>
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
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { KNOWN_THEMES, resolveDarkMode } from "../main-window/usePaneLayout";
import type { FullPreferencesDto } from "../preferences/types";
import {
  AboutPanel,
  ComposePanel,
  PrivacyPanel,
  StoragePanel,
} from "../preferences/panels";
import { usePreferences } from "../preferences/usePreferences";
import AccountsPage from "./AccountsPage.vue";
import AppearancePage from "./AppearancePage.vue";
import FilterPage from "./FilterPage.vue";
import GeneralPage from "./GeneralPage.vue";
import PgpKeysPage from "./PgpKeysPage.vue";
import PlaceholderPage from "./PlaceholderPage.vue";
import SendersPage from "./SendersPage.vue";
import SmartFolderPage from "./SmartFolderPage.vue";
import TagPage from "./TagPage.vue";
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

function selectTab(id: SettingsTabId) {
  activeTabId.value = id;
  // 窄窗：点击页签进入内容（对齐 GPUI select_tab → narrow_show_content = true）。
  if (narrow.value) {
    narrowShowContent.value = true;
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

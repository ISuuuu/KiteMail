<template>
  <div
    v-if="open"
    class="km-prefs-backdrop"
    role="dialog"
    aria-modal="true"
    aria-labelledby="km-prefs-modal-title"
    @click.self="closeModal"
  >
    <section class="km-prefs-modal">
      <header class="km-modal-header">
        <div class="km-modal-title-group">
          <h2 id="km-prefs-modal-title" class="km-modal-title">偏好设置</h2>
          <span class="km-modal-subtitle">外观、同步、隐私安全与本地存储</span>
        </div>

        <button
          type="button"
          class="km-close-btn"
          title="关闭 (Esc)"
          aria-label="关闭偏好设置窗口"
          @click="closeModal"
        >
          ✕
        </button>
      </header>

      <div class="km-modal-body">
        <!-- 左侧 6 分栏导航 -->
        <nav class="km-prefs-nav" aria-label="设置分类">
          <button
            v-for="tab in TABS"
            :key="tab.id"
            type="button"
            class="km-nav-item"
            :class="{ active: state.activeTab.value === tab.id }"
            @click="state.activeTab.value = tab.id"
          >
            <span class="km-nav-label">{{ tab.label }}</span>
            <span class="km-nav-desc">{{ tab.desc }}</span>
          </button>
        </nav>

        <!-- 右侧内容区（面板共享实现见 ./panels/） -->
        <main class="km-prefs-content km-pref-panels">
          <div v-if="state.loading.value" class="km-state-empty">
            正在加载偏好设置…
          </div>
          <div v-else-if="state.loadError.value" class="km-state-error" role="alert">
            <p>{{ state.loadError.value }}</p>
            <button
              type="button"
              class="km-btn km-btn-secondary"
              @click="state.init(state.activeTab.value)"
            >
              重试
            </button>
          </div>
          <template v-else>
            <component :is="activePanel" :state="state" @openAccounts="emit('openAccounts')" />

            <!-- 全局操作状态横幅 -->
            <div
              v-if="state.banner.value"
              class="km-banner"
              :class="`is-${state.banner.value.kind}`"
              role="status"
            >
              {{ state.banner.value.text }}
            </div>
          </template>
        </main>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, watch } from "vue";
import type { PasscodeStatus, PreferenceTabId } from "./types";
import { usePreferences } from "./usePreferences";
import { PREFERENCE_PANELS } from "./panels";

const props = withDefaults(
  defineProps<{
    open: boolean;
    initialTab?: PreferenceTabId;
  }>(),
  {
    initialTab: "appearance",
  }
);

const emit = defineEmits<{
  "update:open": [open: boolean];
  close: [];
  preferencesUpdated: [prefs: import("./types").FullPreferencesDto];
  passcodeChanged: [status: PasscodeStatus];
  openAccounts: [];
  overlayChange: [id: string, open: boolean];
}>();

const OVERLAY_ID = "preferences-modal";

const TABS: ReadonlyArray<{ id: PreferenceTabId; label: string; desc: string }> = [
  { id: "appearance", label: "外观与布局", desc: "主题、深浅色与窗格" },
  { id: "sync", label: "同步与通知", desc: "轮询、托盘与提示" },
  { id: "compose", label: "写信与阅读", desc: "撤销、签名与已读" },
  { id: "privacy", label: "隐私与安全锁", desc: "远图白名单与口令" },
  { id: "storage", label: "存储与缓存", desc: "目录统计与清理" },
  { id: "about", label: "关于与更新", desc: "版本形态与更新" },
];

const state = usePreferences({
  onLocked() {
    closeModal();
  },
  onPreferencesUpdated(prefs) {
    emit("preferencesUpdated", prefs);
  },
  onPasscodeChanged(status) {
    emit("passcodeChanged", status);
  },
});

const activePanel = computed(() => PREFERENCE_PANELS[state.activeTab.value]);

function closeModal() {
  if (!props.open) return;
  emit("overlayChange", OVERLAY_ID, false);
  emit("update:open", false);
  emit("close");
}

function handleKeydown(ev: KeyboardEvent) {
  if (!props.open) return;
  if (ev.key === "Escape") {
    ev.preventDefault();
    ev.stopPropagation();
    closeModal();
  }
}

function attachGlobalListeners() {
  if (typeof window !== "undefined") {
    window.addEventListener("keydown", handleKeydown);
  }
}

function detachGlobalListeners() {
  if (typeof window !== "undefined") {
    window.removeEventListener("keydown", handleKeydown);
  }
}

watch(
  () => props.open,
  (isOpen, prevOpen) => {
    if (isOpen && !prevOpen) {
      emit("overlayChange", OVERLAY_ID, true);
      attachGlobalListeners();
      void state.init(props.initialTab);
    } else if (!isOpen && prevOpen) {
      detachGlobalListeners();
      emit("overlayChange", OVERLAY_ID, false);
    }
  }
);

watch(
  () => props.initialTab,
  (tab) => {
    if (props.open && tab) {
      state.activeTab.value = tab;
    }
  }
);

onMounted(() => {
  if (props.open) {
    emit("overlayChange", OVERLAY_ID, true);
    attachGlobalListeners();
    void state.init(props.initialTab);
  }
});

onUnmounted(() => {
  detachGlobalListeners();
  if (props.open) {
    emit("overlayChange", OVERLAY_ID, false);
  }
  state.dispose();
});
</script>

<style scoped>
.km-prefs-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  background: rgba(0, 0, 0, 0.48);
  backdrop-filter: blur(4px);
}

.km-prefs-modal {
  width: min(920px, 100%);
  height: min(650px, calc(100vh - 32px));
  display: flex;
  flex-direction: column;
  border-radius: var(--km-radius-lg);
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  box-shadow: var(--km-shadow-popover);
  overflow: hidden;
}

.km-modal-header {
  height: 52px;
  padding: 0 18px;
  border-bottom: 1px solid var(--km-border);
  background: var(--km-surface);
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-shrink: 0;
}

.km-modal-title-group {
  display: flex;
  align-items: baseline;
  gap: 10px;
}

.km-modal-title {
  margin: 0;
  font-size: 15px;
  font-weight: 700;
  color: var(--km-text);
}

.km-modal-subtitle {
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-close-btn {
  width: 30px;
  height: 30px;
  border: none;
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-text-muted);
  font-size: 14px;
  cursor: pointer;
}

.km-close-btn:hover {
  background: var(--km-hover-bg);
  color: var(--km-text);
}

.km-modal-body {
  flex: 1;
  min-height: 0;
  display: flex;
}

.km-prefs-nav {
  width: 216px;
  border-right: 1px solid var(--km-border);
  background: var(--km-sidebar);
  padding: 10px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  overflow-y: auto;
}

.km-nav-item {
  border: 1px solid transparent;
  border-radius: var(--km-radius-md);
  background: transparent;
  padding: 9px 10px;
  text-align: left;
  display: flex;
  flex-direction: column;
  gap: 2px;
  cursor: pointer;
}

.km-nav-item:hover {
  background: var(--km-hover-bg);
}

.km-nav-item.active {
  background: var(--km-surface);
  border-color: var(--km-accent);
  box-shadow: var(--km-shadow-card);
}

.km-nav-label {
  font-size: 13px;
  font-weight: 600;
  color: var(--km-text);
}

.km-nav-desc {
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-prefs-content {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  padding: 18px 22px;
  display: flex;
  flex-direction: column;
  gap: 14px;
  background: var(--km-canvas);
}

.km-state-empty,
.km-state-error {
  padding: 28px;
  text-align: center;
  color: var(--km-text-muted);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
}

.km-state-error {
  color: var(--km-danger);
}

@media (max-width: 760px) {
  .km-modal-body {
    flex-direction: column;
  }

  .km-prefs-nav {
    width: 100%;
    flex-direction: row;
    overflow-x: auto;
    border-right: none;
    border-bottom: 1px solid var(--km-border);
  }
}
</style>

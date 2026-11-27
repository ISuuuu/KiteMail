<template>
  <section class="km-reader-pane" :class="{ below: isBelow }" aria-label="邮件阅读区">
    <!-- 顶部栏 48px：仅呈现已接通入口（布局切换、主题切换、重试） -->
    <header class="km-reader-header">
      <div class="km-layout-switch" role="group" aria-label="阅读窗格位置">
        <button
          type="button"
          class="km-seg-btn"
          :class="{ active: readerPosition === 'right' }"
          title="右侧阅读 (Ctrl+Alt+1)"
          @click="emit('setReaderPosition', 'right')"
        >
          右侧
        </button>
        <button
          type="button"
          class="km-seg-btn"
          :class="{ active: readerPosition === 'below' }"
          title="下方阅读 (Ctrl+Alt+2)"
          @click="emit('setReaderPosition', 'below')"
        >
          下方
        </button>
        <button
          type="button"
          class="km-seg-btn"
          title="关闭阅读窗格 (Ctrl+Alt+3)"
          @click="emit('setReaderPosition', 'off')"
        >
          隐藏
        </button>
      </div>

      <div class="km-reader-actions">
        <template v-if="message">
          <button
            type="button"
            class="km-action-btn"
            :title="selectedUnread ? '标为已读' : '标为未读'"
            @click="emit('toggleSeen', Boolean(selectedUnread))"
          >
            {{ selectedUnread ? "标为已读" : "标为未读" }}
          </button>
          <button
            type="button"
            class="km-action-btn"
            :class="{ flagged: message.flagged }"
            :title="message.flagged ? '取消星标' : '加上星标'"
            @click="emit('toggleFlagged', !message.flagged)"
          >
            {{ message.flagged ? "★ 已星标" : "☆ 星标" }}
          </button>
        </template>

        <button
          v-if="(error || message?.bodyStatus === 'unavailable') && message"
          type="button"
          class="km-action-btn km-action-retry"
          @click="emit('retry')"
        >
          重试正文
        </button>

        <!-- 亮暗快速切换 -->
        <button
          type="button"
          class="km-action-btn"
          :title="effectiveDark ? '切换为浅色模式' : '切换为深色模式'"
          @click="emit('toggleDarkMode')"
        >
          {{ effectiveDark ? "🌙 深色" : "☀ 浅色" }}
        </button>

        <!-- 7 套内置主题选择器 -->
        <div ref="themeWrapRef" class="km-popover-wrap">
          <button
            type="button"
            class="km-action-btn"
            aria-haspopup="menu"
            :aria-expanded="themeMenuOpen"
            @click="toggleThemeMenu"
          >
            <span class="km-swatch-dot" :style="{ backgroundColor: currentThemeSwatch }" />
            <span>{{ currentThemeLabel }}</span>
          </button>

          <div v-if="themeMenuOpen" class="km-theme-popover" role="menu">
            <div class="km-menu-title">界面主题</div>
            <button
              v-for="theme in BUILTIN_THEMES"
              :key="theme.id"
              type="button"
              class="km-theme-item"
              :class="{ active: themeId === theme.id }"
              @click="pickTheme(theme.id)"
            >
              <span class="km-theme-left">
                <span class="km-swatch-dot" :style="{ backgroundColor: theme.swatch }" />
                <span>{{ theme.label }}</span>
              </span>
              <span v-if="themeId === theme.id">✓</span>
            </button>
            <div class="km-menu-divider" />
            <button
              type="button"
              class="km-theme-item"
              :class="{ active: themeId === 'system' && followSystem }"
              @click="pickFollowSystem"
            >
              <span>跟随系统深浅色</span>
              <span v-if="themeId === 'system' && followSystem">✓</span>
            </button>
          </div>
        </div>
      </div>
    </header>

    <!-- 空状态 -->
    <div v-if="!message && !loading && !error" class="km-reader-empty">
      <div class="km-empty-icon" aria-hidden="true">✉</div>
      <p>选择一封邮件查看正文</p>
    </div>

    <!-- 仅加载中骨架 -->
    <div v-else-if="loading && !message" class="km-reader-skeleton" aria-busy="true" aria-label="正在加载邮件">
      <div class="km-sk-line km-sk-title" />
      <div class="km-sk-line km-sk-meta" />
      <div class="km-sk-line km-sk-meta short" />
      <div class="km-sk-card">
        <div class="km-sk-line" />
        <div class="km-sk-line" />
        <div class="km-sk-line short" />
      </div>
    </div>

    <!-- 仅错误 -->
    <div v-else-if="error && !message" class="km-reader-empty" role="alert">
      <p class="km-error-msg">{{ error }}</p>
      <button type="button" class="km-retry-btn" @click="emit('retry')">重试</button>
    </div>

    <!-- 邮件详情 -->
    <div v-else-if="message" class="km-reader-body-wrap">
      <div class="km-mail-meta">
        <div class="km-subject-row">
          <h1 class="km-subject-title">{{ message.subject || "(无主题)" }}</h1>
          <button
            type="button"
            class="km-star-badge-btn"
            :class="{ flagged: message.flagged }"
            :title="message.flagged ? '取消星标' : '加上星标'"
            :aria-label="message.flagged ? '取消星标' : '加上星标'"
            @click="emit('toggleFlagged', !message.flagged)"
          >
            {{ message.flagged ? "★" : "☆" }}
          </button>
        </div>

        <div class="km-meta-grid">
          <div class="km-meta-line">
            <span class="km-meta-label">发件人：</span>
            <span class="km-meta-value">{{ message.sender || "(未知)" }}</span>
          </div>
          <div class="km-meta-line">
            <span class="km-meta-label">收件人：</span>
            <span class="km-meta-value">{{ message.recipients || "(无)" }}</span>
          </div>
          <div v-if="message.cc" class="km-meta-line">
            <span class="km-meta-label">抄送：</span>
            <span class="km-meta-value">{{ message.cc }}</span>
          </div>
          <div class="km-meta-line km-meta-time">
            <span class="km-meta-label">时间：</span>
            <span class="km-meta-value">{{ message.dateDisplay }}</span>
            <span v-if="message.threadMessages.length > 1" class="km-thread-badge">
              会话 {{ message.threadMessages.length }} 封
            </span>
          </div>
        </div>

        <div v-if="message.attachments.length > 0" class="km-attachments-strip">
          <div
            v-for="(att, idx) in message.attachments"
            :key="att.id ?? `${att.filename}-${idx}`"
            class="km-attachment-pill"
          >
            <span aria-hidden="true">📎</span>
            <span class="km-att-name">{{ att.filename }}</span>
            <span class="km-att-size">{{ formatBytes(att.sizeBytes) }}</span>
          </div>
        </div>
      </div>

      <ReaderViewport
        :readerSessionId="message.readerSessionId"
        :bodyStatus="message.bodyStatus"
        :hasHtml="message.hasHtml"
        :plainText="message.plainText"
        :htmlBody="message.htmlBody"
        :threadMessages="message.threadMessages"
        :overlayOccluded="Boolean(overlayOccluded)"
        :errorMessage="error"
        :zoomPercent="zoomPercent"
        @retry="emit('retry')"
      />
    </div>
  </section>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { BUILTIN_THEMES, type OpenMessageOutput, type ReaderPosition } from "../ipc/types";
import ReaderViewport from "./ReaderViewport.vue";

const props = defineProps<{
  message: OpenMessageOutput | null;
  loading: boolean;
  error: string | null;
  selectedUnread?: boolean;
  readerPosition: ReaderPosition;
  themeId: string;
  effectiveDark?: boolean;
  followSystem?: boolean;
  overlayOccluded?: boolean;
  zoomPercent?: number;
  isBelow?: boolean;
}>();

const emit = defineEmits<{
  retry: [];
  toggleSeen: [seen: boolean];
  toggleFlagged: [flagged: boolean];
  setReaderPosition: [pos: ReaderPosition];
  setTheme: [themeId: string, darkMode?: boolean];
  toggleDarkMode: [];
  toggleFollowSystem: [];
  overlayChange: [id: string, open: boolean];
}>();

const themeWrapRef = ref<HTMLElement | null>(null);
const themeMenuOpen = ref(false);

function setThemeMenu(open: boolean) {
  if (themeMenuOpen.value === open) return;
  themeMenuOpen.value = open;
  emit("overlayChange", "reader-theme-menu", open);
}

function toggleThemeMenu() {
  setThemeMenu(!themeMenuOpen.value);
}

function pickTheme(id: string) {
  emit("setTheme", id);
  setThemeMenu(false);
}

function pickFollowSystem() {
  emit("toggleFollowSystem");
  setThemeMenu(false);
}

const currentThemeMeta = computed(
  () => BUILTIN_THEMES.find((t) => t.id === props.themeId) ?? BUILTIN_THEMES[0]
);
const currentThemeLabel = computed(() => currentThemeMeta.value.label);
const currentThemeSwatch = computed(() => currentThemeMeta.value.swatch);

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function onDocumentPointerDown(e: PointerEvent) {
  if (!themeMenuOpen.value) return;
  if (themeWrapRef.value && !themeWrapRef.value.contains(e.target as Node)) {
    setThemeMenu(false);
  }
}

onMounted(() => {
  document.addEventListener("pointerdown", onDocumentPointerDown);
});

onUnmounted(() => {
  document.removeEventListener("pointerdown", onDocumentPointerDown);
  setThemeMenu(false);
});
</script>

<style scoped>
.km-reader-pane {
  flex: 1 1 auto;
  min-width: 0;
  min-height: 0;
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--km-canvas);
  color: var(--km-text);
  overflow: hidden;
}

.km-reader-pane.below {
  min-height: 180px;
}

.km-reader-header {
  height: 48px;
  min-height: 48px;
  padding: 0 16px;
  border-bottom: 1px solid var(--km-border);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.km-layout-switch {
  display: inline-flex;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  overflow: hidden;
  background: var(--km-surface);
}

.km-seg-btn {
  height: 28px;
  padding: 0 10px;
  border: none;
  background: transparent;
  font-size: 12px;
  cursor: pointer;
  color: var(--km-text-muted);
}

.km-seg-btn + .km-seg-btn {
  border-left: 1px solid var(--km-border);
}

.km-seg-btn:hover {
  background: var(--km-hover-bg);
  color: var(--km-text);
}

.km-seg-btn.active {
  background: var(--km-accent-soft);
  color: var(--km-accent);
  font-weight: 600;
}

.km-reader-actions {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}

.km-action-btn {
  height: 28px;
  padding: 0 10px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface-raised);
  font-size: 12px;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  cursor: pointer;
}

.km-action-btn:hover {
  background: var(--km-hover-bg);
}

.km-action-btn.flagged {
  color: var(--km-star-gold);
  border-color: color-mix(in srgb, var(--km-star-gold) 45%, var(--km-border));
}

.km-action-retry {
  color: var(--km-danger);
  border-color: color-mix(in srgb, var(--km-danger) 45%, var(--km-border));
}

.km-reader-skeleton {
  flex: 1 1 auto;
  padding: 20px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.km-sk-card {
  margin-top: 10px;
  padding: 18px;
  border-radius: var(--km-radius-lg);
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.km-sk-line {
  height: 12px;
  border-radius: 6px;
  background: color-mix(in srgb, var(--km-text-muted) 16%, transparent);
  animation: km-sk-pulse 1.3s ease-in-out infinite;
}

.km-sk-title {
  width: 62%;
  height: 20px;
}

.km-sk-meta {
  width: 44%;
}

.km-sk-line.short {
  width: 30%;
}

@keyframes km-sk-pulse {
  0%,
  100% {
    opacity: 0.45;
  }
  50% {
    opacity: 0.9;
  }
}

.km-popover-wrap {
  position: relative;
}

.km-theme-popover {
  position: absolute;
  top: 32px;
  right: 0;
  width: 190px;
  padding: 6px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-md);
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

.km-theme-item {
  border: none;
  background: transparent;
  border-radius: var(--km-radius-sm);
  padding: 6px 8px;
  font-size: 12px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  cursor: pointer;
}

.km-theme-item:hover {
  background: var(--km-hover-bg);
}

.km-theme-item.active {
  color: var(--km-accent);
  font-weight: 600;
}

.km-theme-left {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}

.km-swatch-dot {
  width: 10px;
  height: 10px;
  border-radius: 999px;
  display: inline-block;
  flex-shrink: 0;
}

.km-menu-divider {
  height: 1px;
  background: var(--km-border);
  margin: 4px 0;
}

.km-reader-empty {
  flex: 1 1 auto;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--km-text-muted);
}

.km-empty-icon {
  font-size: 36px;
  opacity: 0.4;
}

.km-error-msg {
  color: var(--km-danger);
  margin: 0;
}

.km-retry-btn {
  padding: 5px 14px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface-raised);
  cursor: pointer;
}

.km-reader-body-wrap {
  flex: 1 1 auto;
  min-height: 0;
  padding: 14px 20px 18px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  overflow: hidden;
}

.km-mail-meta {
  display: flex;
  flex-direction: column;
  gap: 8px;
  flex-shrink: 0;
}

.km-subject-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.km-subject-title {
  margin: 0;
  font-size: 19px;
  font-weight: 700;
  line-height: 1.3;
  user-select: text;
}

.km-star-badge-btn {
  border: none;
  background: transparent;
  padding: 2px 4px;
  color: var(--km-text-muted);
  font-size: 16px;
  flex-shrink: 0;
  cursor: pointer;
  border-radius: var(--km-radius-sm);
}

.km-star-badge-btn.flagged {
  color: var(--km-star-gold);
}

.km-star-badge-btn:hover {
  background: var(--km-hover-bg);
}

.km-meta-grid {
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-size: 12px;
}

.km-meta-line {
  display: flex;
  align-items: baseline;
  gap: 6px;
  min-width: 0;
}

.km-meta-label {
  color: var(--km-text-muted);
  flex-shrink: 0;
}

.km-meta-value {
  color: var(--km-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  user-select: text;
}

.km-thread-badge {
  margin-left: 8px;
  padding: 1px 8px;
  border-radius: 999px;
  background: var(--km-accent-soft);
  color: var(--km-accent);
  font-size: 11px;
  font-weight: 600;
}

.km-attachments-strip {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.km-attachment-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px;
  border-radius: 999px;
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  font-size: 12px;
}

.km-att-name {
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-att-size {
  color: var(--km-text-muted);
  font-size: 11px;
}
</style>

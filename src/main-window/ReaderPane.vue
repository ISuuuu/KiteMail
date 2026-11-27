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
            class="km-action-btn km-action-primary"
            title="回复发件人"
            @click="emit('reply')"
          >
            回复
          </button>
          <button
            type="button"
            class="km-action-btn"
            title="回复所有收件人"
            @click="emit('replyAll')"
          >
            全部回复
          </button>
          <button
            type="button"
            class="km-action-btn"
            title="转发此邮件"
            @click="emit('forward')"
          >
            转发
          </button>
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
          <button
            type="button"
            class="km-action-btn"
            title="归档邮件"
            :disabled="movingMessage"
            @click="emit('archive')"
          >
            归档
          </button>
          <button
            type="button"
            class="km-action-btn"
            title="标记为垃圾邮件"
            :disabled="movingMessage"
            @click="emit('markSpam')"
          >
            垃圾邮件
          </button>
          <button
            type="button"
            class="km-action-btn km-action-danger"
            title="删除邮件 (Delete)"
            :disabled="movingMessage"
            @click="emit('delete')"
          >
            删除
          </button>

          <button
            type="button"
            class="km-action-btn"
            title="独立窗口阅读此邮件"
            @click="emit('openStandalone')"
          >
            独立阅读
          </button>
          <button
            type="button"
            class="km-action-btn"
            title="打印预览 / 打印"
            @click="emit('print')"
          >
            打印
          </button>
          <button
            v-if="message.attachments.length > 0"
            type="button"
            class="km-action-btn"
            title="展开邮件附件抽屉"
            @click="emit('openAttachments')"
          >
            📎 附件 ({{ message.attachments.length }})
          </button>

          <!-- 移动到文件夹弹出菜单 -->
          <div ref="moveWrapRef" class="km-popover-wrap">
            <button
              type="button"
              class="km-action-btn"
              aria-haspopup="menu"
              :aria-expanded="moveMenuOpen"
              :disabled="movingMessage || accountFolders.length === 0"
              @click="toggleMoveMenu"
            >
              <span>移动到</span>
              <span aria-hidden="true">▾</span>
            </button>

            <div v-if="moveMenuOpen" class="km-theme-popover km-move-popover" role="menu">
              <div class="km-menu-title">移动到文件夹</div>
              <button
                v-for="folder in accountFolders"
                :key="`${folder.accountId}:${folder.id}`"
                type="button"
                class="km-theme-item"
                :class="{ active: isCurrentFolder(folder.path) }"
                :disabled="isCurrentFolder(folder.path)"
                @click="pickMoveFolder(folder.path)"
              >
                <span>{{ folder.name }}</span>
                <span v-if="isCurrentFolder(folder.path)" class="km-current-tag">当前</span>
              </button>
            </div>
          </div>
        </template>

        <button
          v-if="(error || message?.bodyStatus === 'unavailable') && message"
          type="button"
          class="km-action-btn km-action-retry"
          @click="emit('retry')"
        >
          重试正文
        </button>

        <!-- 三态亮暗快速循环切换 -->
        <button
          type="button"
          class="km-action-btn"
          :title="colorModeBtnTitle"
          @click="emit('toggleDarkMode')"
        >
          {{ colorModeBtnLabel }}
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
            <div class="km-menu-title">亮暗模式</div>
            <button
              type="button"
              class="km-theme-item"
              :class="{ active: darkMode === null || darkMode === undefined }"
              @click="pickDarkMode(null)"
            >
              <span>跟随系统</span>
              <span v-if="darkMode === null || darkMode === undefined">✓</span>
            </button>
            <button
              type="button"
              class="km-theme-item"
              :class="{ active: darkMode === false }"
              @click="pickDarkMode(false)"
            >
              <span>浅色</span>
              <span v-if="darkMode === false">✓</span>
            </button>
            <button
              type="button"
              class="km-theme-item"
              :class="{ active: darkMode === true }"
              @click="pickDarkMode(true)"
            >
              <span>深色</span>
              <span v-if="darkMode === true">✓</span>
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
          <button
            v-for="(att, idx) in message.attachments"
            :key="att.id ?? `${att.filename}-${idx}`"
            type="button"
            class="km-attachment-pill"
            title="打开附件抽屉保存或预览"
            @click="emit('openAttachments')"
          >
            <span aria-hidden="true">📎</span>
            <span class="km-att-name">{{ att.filename }}</span>
            <span class="km-att-size">{{ formatBytes(att.sizeBytes) }}</span>
          </button>
        </div>
      </div>

      <!-- 远程图片隐私拦截提示条 -->
      <div
        v-if="message.remoteImagesBlocked === true"
        class="km-remote-img-banner"
        role="region"
        aria-label="远程图片隐私保护"
      >
        <span class="km-remote-img-text">已拦截远程图片以保护隐私</span>
        <div class="km-remote-img-actions">
          <button
            type="button"
            class="km-remote-img-btn"
            :disabled="allowingRemoteImages"
            @click="emit('allowRemoteImages', false)"
          >
            仅本次显示
          </button>
          <button
            type="button"
            class="km-remote-img-btn primary"
            :disabled="allowingRemoteImages"
            @click="emit('allowRemoteImages', true)"
          >
            始终信任此发件人
          </button>
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
import {
  BUILTIN_THEMES,
  type FolderSummaryDto,
  type OpenMessageOutput,
  type ReaderPosition,
} from "../ipc/types";
import ReaderViewport from "./ReaderViewport.vue";

const props = defineProps<{
  message: OpenMessageOutput | null;
  loading: boolean;
  error: string | null;
  selectedUnread?: boolean;
  readerPosition: ReaderPosition;
  themeId: string;
  darkMode?: boolean | null;
  effectiveDark?: boolean;
  followSystem?: boolean;
  overlayOccluded?: boolean;
  zoomPercent?: number;
  isBelow?: boolean;
  folders?: FolderSummaryDto[];
  allowingRemoteImages?: boolean;
  movingMessage?: boolean;
}>();

const emit = defineEmits<{
  retry: [];
  reply: [];
  replyAll: [];
  forward: [];
  toggleSeen: [seen: boolean];
  toggleFlagged: [flagged: boolean];
  setReaderPosition: [pos: ReaderPosition];
  setTheme: [themeId: string, darkMode?: boolean | null];
  setDarkMode: [mode: boolean | null];
  toggleDarkMode: [];
  toggleFollowSystem: [];
  allowRemoteImages: [trustSender: boolean];
  archive: [];
  markSpam: [];
  delete: [];
  moveToFolder: [targetFolderPath: string];
  openStandalone: [];
  print: [];
  openAttachments: [];
  overlayChange: [id: string, open: boolean];
}>();

const themeWrapRef = ref<HTMLElement | null>(null);
const themeMenuOpen = ref(false);
const moveWrapRef = ref<HTMLElement | null>(null);
const moveMenuOpen = ref(false);

function setThemeMenu(open: boolean) {
  if (themeMenuOpen.value === open) return;
  if (open) setMoveMenu(false);
  themeMenuOpen.value = open;
  emit("overlayChange", "reader-theme-menu", open);
}

function toggleThemeMenu() {
  setThemeMenu(!themeMenuOpen.value);
}

function setMoveMenu(open: boolean) {
  if (moveMenuOpen.value === open) return;
  if (open) setThemeMenu(false);
  moveMenuOpen.value = open;
  emit("overlayChange", "reader-move-menu", open);
}

function toggleMoveMenu() {
  setMoveMenu(!moveMenuOpen.value);
}

function pickTheme(id: string) {
  emit("setTheme", id);
  setThemeMenu(false);
}

function pickDarkMode(mode: boolean | null) {
  emit("setDarkMode", mode);
  setThemeMenu(false);
}

function pickMoveFolder(targetFolderPath: string) {
  emit("moveToFolder", targetFolderPath);
  setMoveMenu(false);
}

const accountFolders = computed(() => {
  if (!props.message || !props.folders) return [];
  return props.folders.filter((f) => f.accountId === props.message!.accountId);
});

function isCurrentFolder(folderPath: string): boolean {
  const current = props.message?.folderPath;
  if (!current) return false;
  return current.toLowerCase() === folderPath.toLowerCase();
}

const colorModeBtnLabel = computed(() => {
  if (props.darkMode === null || props.darkMode === undefined) {
    return props.effectiveDark ? "🌓 跟随系统·暗" : "🌓 跟随系统·亮";
  }
  return props.darkMode ? "🌙 深色" : "☀ 浅色";
});

const colorModeBtnTitle = computed(() => {
  if (props.darkMode === null || props.darkMode === undefined) {
    return "当前：跟随系统深浅色（点击切换为浅色）";
  }
  return props.darkMode
    ? "当前：深色模式（点击切换为跟随系统）"
    : "当前：浅色模式（点击切换为深色）";
});

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
  if (themeMenuOpen.value && themeWrapRef.value && !themeWrapRef.value.contains(e.target as Node)) {
    setThemeMenu(false);
  }
  if (moveMenuOpen.value && moveWrapRef.value && !moveWrapRef.value.contains(e.target as Node)) {
    setMoveMenu(false);
  }
}

function onDocumentKeyDown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    setThemeMenu(false);
    setMoveMenu(false);
  }
}

onMounted(() => {
  document.addEventListener("pointerdown", onDocumentPointerDown);
  document.addEventListener("keydown", onDocumentKeyDown);
});

onUnmounted(() => {
  document.removeEventListener("pointerdown", onDocumentPointerDown);
  document.removeEventListener("keydown", onDocumentKeyDown);
  setThemeMenu(false);
  setMoveMenu(false);
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
  min-height: 48px;
  padding: 6px 14px;
  border-bottom: 1px solid var(--km-border);
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 8px;
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
  flex-wrap: wrap;
  gap: 6px;
}

.km-action-btn {
  height: 28px;
  padding: 0 9px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface-raised);
  font-size: 12px;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  cursor: pointer;
  white-space: nowrap;
}

.km-action-btn:hover:not(:disabled) {
  background: var(--km-hover-bg);
}

.km-action-primary {
  background: var(--km-accent-soft);
  color: var(--km-accent);
  border-color: color-mix(in srgb, var(--km-accent) 40%, var(--km-border));
  font-weight: 600;
}

.km-action-primary:hover:not(:disabled) {
  background: var(--km-accent-soft-hover);
}

.km-action-btn:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.km-action-btn.flagged {
  color: var(--km-star-gold);
  border-color: color-mix(in srgb, var(--km-star-gold) 45%, var(--km-border));
}

.km-action-danger:hover:not(:disabled),
.km-action-retry {
  color: var(--km-danger);
  border-color: color-mix(in srgb, var(--km-danger) 45%, var(--km-border));
}

.km-move-popover {
  width: 176px;
}

.km-current-tag {
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-theme-item:disabled {
  opacity: 0.55;
  cursor: default;
}

.km-remote-img-banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 8px;
  padding: 8px 12px;
  border: 1px solid color-mix(in srgb, var(--km-accent) 35%, var(--km-border));
  border-radius: var(--km-radius-md);
  background: var(--km-accent-soft);
  font-size: 12px;
  flex-shrink: 0;
}

.km-remote-img-text {
  color: var(--km-text);
  font-weight: 500;
}

.km-remote-img-actions {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.km-remote-img-btn {
  height: 26px;
  padding: 0 10px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface-raised);
  color: var(--km-text);
  font-size: 12px;
  cursor: pointer;
}

.km-remote-img-btn.primary {
  background: var(--km-accent);
  border-color: var(--km-accent);
  color: var(--km-accent-fg);
}

.km-remote-img-btn:hover:not(:disabled) {
  filter: brightness(0.96);
}

.km-remote-img-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
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
  color: var(--km-text);
  font-size: 12px;
  cursor: pointer;
}

.km-attachment-pill:hover {
  border-color: var(--km-accent);
  background: var(--km-accent-soft);
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

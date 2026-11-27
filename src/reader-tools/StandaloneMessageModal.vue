<template>
  <Teleport to="body">
    <div
      v-if="open"
      class="km-standalone-backdrop"
      role="dialog"
      aria-modal="true"
      :aria-label="messageData?.subject ?? '独立读信浮窗'"
      @pointerdown.self="closeModal"
    >
      <section class="km-standalone-modal" @keydown="onKeyDown">
        <!-- 顶部工具栏 -->
        <header class="km-standalone-header">
          <div class="km-standalone-title-wrap">
            <span v-if="windowLabel" class="km-win-badge">{{ windowLabel }}</span>
            <h2 class="km-standalone-title" :title="headerTitle">
              {{ headerTitle }}
            </h2>
          </div>

          <div class="km-standalone-toolbar">
            <div class="km-mode-switch" role="group" aria-label="阅读与打印视图切换">
              <button
                type="button"
                class="km-seg-btn"
                :class="{ active: mode === 'read' }"
                @click="switchMode('read')"
              >
                阅读视图
              </button>
              <button
                type="button"
                class="km-seg-btn"
                :class="{ active: mode === 'print' }"
                @click="switchMode('print')"
              >
                打印视图
              </button>
            </div>

            <div class="km-zoom-group" aria-label="阅读缩放">
              <button
                type="button"
                class="km-btn km-btn-icon"
                title="缩小"
                aria-label="缩小字体"
                :disabled="zoomPercent <= 80"
                @click="adjustZoom(-10)"
              >
                A-
              </button>
              <button
                type="button"
                class="km-btn km-zoom-reset"
                title="重置缩放"
                @click="zoomPercent = 100"
              >
                {{ zoomPercent }}%
              </button>
              <button
                type="button"
                class="km-btn km-btn-icon"
                title="放大"
                aria-label="放大字体"
                :disabled="zoomPercent >= 160"
                @click="adjustZoom(10)"
              >
                A+
              </button>
            </div>

            <button
              type="button"
              class="km-btn"
              :disabled="!messageData"
              @click="emitAction('reply')"
            >
              回复
            </button>
            <button
              type="button"
              class="km-btn"
              :disabled="!messageData"
              @click="emitAction('replyAll')"
            >
              全部回复
            </button>
            <button
              type="button"
              class="km-btn"
              :disabled="!messageData"
              @click="emitAction('forward')"
            >
              转发
            </button>
            <button
              type="button"
              class="km-btn km-btn-primary"
              :disabled="loading || !target"
              @click="handlePrint"
            >
              打印
            </button>
            <button
              type="button"
              class="km-icon-btn"
              title="关闭 (Esc)"
              aria-label="关闭独立读信浮窗"
              @click="closeModal"
            >
              ✕
            </button>
          </div>
        </header>

        <!-- 打印选项条 -->
        <div v-if="mode === 'print'" class="km-print-bar">
          <label class="km-checkbox-label">
            <input
              v-model="includeHeader"
              type="checkbox"
              @change="reloadPrintHtml"
            />
            包含邮件头信息（发件人 / 收件人 / 日期）
          </label>
          <span class="km-print-hint">已启用安全沙箱预览</span>
        </div>

        <!-- 错误提示 -->
        <div v-if="error" class="km-standalone-banner km-banner-error" role="alert">
          <span>{{ error }}</span>
          <button type="button" class="km-btn" @click="reloadCurrentTarget">重试</button>
        </div>

        <!-- 内容区 -->
        <div class="km-standalone-body">
          <div v-if="loading && !messageData && !printData" class="km-center-state">
            正在加载邮件内容…
          </div>

          <!-- 打印视图 -->
          <div v-else-if="mode === 'print'" class="km-print-stage">
            <div v-if="printLoading" class="km-center-state">正在生成打印预览…</div>
            <iframe
              v-else-if="printData"
              ref="printFrameRef"
              class="km-print-iframe"
              title="邮件打印预览"
              sandbox="allow-same-origin allow-modals"
              :srcdoc="printData.htmlDocument"
              :style="{ zoom: `${zoomPercent}%` }"
            />
            <div v-else class="km-center-state">暂无可打印的邮件内容</div>
          </div>

          <!-- 阅读视图 -->
          <div v-else-if="messageData" class="km-read-stage">
            <header class="km-mail-header">
              <h1 class="km-mail-subject">{{ messageData.subject || "(无主题)" }}</h1>
              <div class="km-mail-meta-grid">
                <div><span class="km-meta-label">发件人</span><span>{{ messageData.sender }}</span></div>
                <div v-if="messageData.recipients">
                  <span class="km-meta-label">收件人</span><span>{{ messageData.recipients }}</span>
                </div>
                <div v-if="messageData.cc">
                  <span class="km-meta-label">抄送</span><span>{{ messageData.cc }}</span>
                </div>
                <div v-if="messageData.dateDisplay">
                  <span class="km-meta-label">时间</span><span>{{ messageData.dateDisplay }}</span>
                </div>
              </div>

              <div v-if="messageData.attachments.length > 0" class="km-mail-attach-strip">
                <span class="km-meta-label">附件 ({{ messageData.attachments.length }})</span>
                <div class="km-attach-chips">
                  <span
                    v-for="(att, idx) in messageData.attachments"
                    :key="`${att.id ?? att.filename}-${idx}`"
                    class="km-attach-chip"
                  >
                    📎 {{ att.filename }} ({{ formatAttachmentSize(att.sizeBytes) }})
                  </span>
                </div>
              </div>
            </header>

            <div class="km-mail-content" :style="{ fontSize: `${Math.round((14 * zoomPercent) / 100)}px` }">
              <iframe
                v-if="messageData.hasHtml && messageData.htmlBody"
                class="km-read-iframe"
                title="邮件 HTML 正文"
                sandbox=""
                :srcdoc="messageData.htmlBody"
              />
              <pre v-else class="km-read-plain">{{ messageData.plainText ?? "" }}</pre>
            </div>
          </div>

          <div v-else class="km-center-state">请选择一封邮件打开</div>
        </div>
      </section>
    </div>
  </Teleport>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { extractErrorMessage } from "../ipc/main-window";
import { getMessagePrintHtml, openMessageWindow } from "./ipc";
import { formatAttachmentSize } from "./useAttachments";
import type {
  GetMessagePrintHtmlOutput,
  OpenMessageOutput,
  StandaloneMessageTarget,
  StandaloneReaderMode,
} from "./types";

const OVERLAY_ID = "standalone-reader";

const props = withDefaults(
  defineProps<{
    open: boolean;
    target: StandaloneMessageTarget | null;
    initialMode?: StandaloneReaderMode;
  }>(),
  {
    initialMode: "read",
  }
);

const emit = defineEmits<{
  "update:open": [open: boolean];
  close: [];
  reply: [msg: OpenMessageOutput];
  replyAll: [msg: OpenMessageOutput];
  forward: [msg: OpenMessageOutput];
  overlayChange: [id: string, open: boolean];
}>();

const mode = ref<StandaloneReaderMode>(props.initialMode);
const includeHeader = ref(true);
const zoomPercent = ref(100);

const loading = ref(false);
const printLoading = ref(false);
const error = ref<string | null>(null);
const windowLabel = ref("");
const messageData = ref<OpenMessageOutput | null>(null);
const printData = ref<GetMessagePrintHtmlOutput | null>(null);
const printFrameRef = ref<HTMLIFrameElement | null>(null);

let loadGen = 0;
let overlayReportedOpen = false;

function reportOverlay(next: boolean) {
  if (overlayReportedOpen === next) return;
  overlayReportedOpen = next;
  emit("overlayChange", OVERLAY_ID, next);
}

const headerTitle = computed(
  () => messageData.value?.subject || printData.value?.subject || "独立读信浮窗"
);

function adjustZoom(delta: number) {
  zoomPercent.value = Math.min(160, Math.max(80, zoomPercent.value + delta));
}

async function loadMessageForTarget(target: StandaloneMessageTarget) {
  const gen = ++loadGen;
  loading.value = true;
  error.value = null;

  try {
    const winRes = await openMessageWindow({
      accountId: target.accountId,
      messageId: target.messageId,
    });
    if (gen !== loadGen) return;
    windowLabel.value = winRes.windowLabel;
    messageData.value = winRes.message;

    if (mode.value === "print") {
      await fetchPrintDocument(target, gen);
    }
  } catch (err) {
    if (gen === loadGen) {
      error.value = extractErrorMessage(err);
    }
  } finally {
    if (gen === loadGen) {
      loading.value = false;
    }
  }
}

async function fetchPrintDocument(
  target: StandaloneMessageTarget,
  expectedGen = loadGen
): Promise<boolean> {
  printLoading.value = true;
  error.value = null;
  try {
    const doc = await getMessagePrintHtml({
      accountId: target.accountId,
      messageId: target.messageId,
      includeHeader: includeHeader.value,
    });
    if (expectedGen !== loadGen) return false;
    printData.value = doc;
    return true;
  } catch (err) {
    if (expectedGen === loadGen) {
      error.value = extractErrorMessage(err);
    }
    return false;
  } finally {
    if (expectedGen === loadGen) {
      printLoading.value = false;
    }
  }
}

async function reloadPrintHtml() {
  if (!props.target) return;
  await fetchPrintDocument(props.target);
}

async function reloadCurrentTarget() {
  if (!props.target) return;
  await loadMessageForTarget(props.target);
}

async function switchMode(nextMode: StandaloneReaderMode) {
  mode.value = nextMode;
  if (nextMode === "print" && props.target && !printData.value) {
    await fetchPrintDocument(props.target);
  }
}

async function handlePrint() {
  if (!props.target) return;
  if (mode.value !== "print") {
    mode.value = "print";
  }
  if (!printData.value) {
    const ok = await fetchPrintDocument(props.target);
    if (!ok) return;
  }
  await nextTick();
  const frameWin = printFrameRef.value?.contentWindow;
  if (frameWin && typeof frameWin.print === "function") {
    try {
      frameWin.focus();
      frameWin.print();
      return;
    } catch {
      // 若被沙箱或浏览器策略阻止，则降级到当前窗口打印
    }
  }
  if (typeof window !== "undefined" && typeof window.print === "function") {
    window.print();
  }
}

function emitAction(kind: "reply" | "replyAll" | "forward") {
  if (!messageData.value) return;
  if (kind === "reply") emit("reply", messageData.value);
  else if (kind === "replyAll") emit("replyAll", messageData.value);
  else emit("forward", messageData.value);
}

function closeModal() {
  loadGen += 1;
  reportOverlay(false);
  emit("update:open", false);
  emit("close");
}

function onKeyDown(e: KeyboardEvent) {
  if (!props.open) return;
  if (e.isComposing || e.key === "Process" || e.keyCode === 229) return;
  if (e.key === "Escape") {
    e.preventDefault();
    e.stopPropagation();
    closeModal();
  }
}

function attachGlobalListeners() {
  if (typeof window !== "undefined") {
    window.addEventListener("keydown", onKeyDown);
  }
}

function detachGlobalListeners() {
  if (typeof window !== "undefined") {
    window.removeEventListener("keydown", onKeyDown);
  }
}

watch(
  () => [props.open, props.target?.accountId, props.target?.messageId] as const,
  ([isOpen, accountId, messageId], prev) => {
    const prevOpen = prev?.[0] ?? false;
    if (isOpen && !prevOpen) {
      mode.value = props.initialMode ?? "read";
      printData.value = null;
      messageData.value = null;
      reportOverlay(true);
      attachGlobalListeners();
    } else if (!isOpen && prevOpen) {
      detachGlobalListeners();
      reportOverlay(false);
      return;
    }

    if (isOpen && accountId !== undefined && messageId !== undefined) {
      printData.value = null;
      void loadMessageForTarget({ accountId, messageId });
    }
  }
);

watch(
  () => props.initialMode,
  (nextMode) => {
    if (nextMode) {
      void switchMode(nextMode);
    }
  }
);

onMounted(() => {
  if (props.open) {
    mode.value = props.initialMode ?? "read";
    reportOverlay(true);
    attachGlobalListeners();
    if (props.target) {
      void loadMessageForTarget(props.target);
    }
  }
});

onUnmounted(() => {
  detachGlobalListeners();
  reportOverlay(false);
});
</script>

<style scoped>
.km-standalone-backdrop {
  position: fixed;
  inset: 0;
  z-index: 86;
  background: rgba(0, 0, 0, 0.44);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
}

.km-standalone-modal {
  width: min(920px, 100%);
  height: min(740px, 92vh);
  display: flex;
  flex-direction: column;
  background: var(--km-surface);
  color: var(--km-text);
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-lg);
  box-shadow: var(--km-shadow-popover);
  overflow: hidden;
  user-select: text;
}

.km-standalone-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 10px;
  padding: 10px 14px;
  background: var(--km-sidebar);
  border-bottom: 1px solid var(--km-border);
  flex-shrink: 0;
}

.km-standalone-title-wrap {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  flex: 1 1 200px;
}

.km-win-badge {
  flex-shrink: 0;
  padding: 2px 7px;
  border-radius: 999px;
  background: var(--km-accent-soft);
  color: var(--km-accent);
  font-family: var(--km-font-mono);
  font-size: 11px;
}

.km-standalone-title {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-standalone-toolbar {
  display: inline-flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
}

.km-mode-switch,
.km-zoom-group {
  display: inline-flex;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  overflow: hidden;
  background: var(--km-surface);
}

.km-seg-btn {
  height: 26px;
  padding: 0 10px;
  border: none;
  background: transparent;
  color: var(--km-text-muted);
  font-size: 12px;
  cursor: pointer;
}

.km-seg-btn + .km-seg-btn {
  border-left: 1px solid var(--km-border);
}

.km-seg-btn.active {
  background: var(--km-accent-soft);
  color: var(--km-accent);
  font-weight: 600;
}

.km-zoom-group .km-btn {
  border: none;
  border-radius: 0;
  height: 26px;
}

.km-zoom-group .km-btn + .km-btn {
  border-left: 1px solid var(--km-border);
}

.km-zoom-reset {
  min-width: 48px;
}

.km-print-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 7px 14px;
  background: var(--km-canvas);
  border-bottom: 1px solid var(--km-border);
  font-size: 12px;
  flex-shrink: 0;
}

.km-checkbox-label {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  cursor: pointer;
}

.km-print-hint {
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-standalone-banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 8px 14px;
  font-size: 12px;
  border-bottom: 1px solid var(--km-border);
  flex-shrink: 0;
}

.km-banner-error {
  background: var(--km-danger-soft);
  color: var(--km-danger);
}

.km-standalone-body {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.km-center-state {
  flex: 1 1 auto;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--km-text-muted);
  font-size: 13px;
}

.km-print-stage {
  flex: 1 1 auto;
  min-height: 0;
  padding: 14px;
  background: var(--km-canvas);
  display: flex;
  flex-direction: column;
}

.km-print-iframe {
  flex: 1 1 auto;
  width: 100%;
  min-height: 0;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: #ffffff;
}

.km-read-stage {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  padding: 18px 22px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.km-mail-header {
  padding-bottom: 14px;
  border-bottom: 1px solid var(--km-border);
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.km-mail-subject {
  margin: 0;
  font-size: 18px;
  font-weight: 700;
}

.km-mail-meta-grid {
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 12.5px;
}

.km-mail-meta-grid > div {
  display: flex;
  gap: 8px;
}

.km-meta-label {
  width: 56px;
  flex-shrink: 0;
  color: var(--km-text-muted);
  font-weight: 600;
}

.km-mail-attach-strip {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  font-size: 12px;
}

.km-attach-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.km-attach-chip {
  padding: 2px 8px;
  border-radius: 999px;
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  font-size: 11.5px;
}

.km-mail-content {
  flex: 1 1 auto;
  min-height: 180px;
  display: flex;
  flex-direction: column;
}

.km-read-iframe {
  flex: 1 1 auto;
  width: 100%;
  min-height: 320px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: #ffffff;
}

.km-read-plain {
  margin: 0;
  font-family: var(--km-font-sans);
  line-height: 1.7;
  white-space: pre-wrap;
  word-break: break-word;
}

.km-btn {
  height: 26px;
  padding: 0 10px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface-raised);
  color: var(--km-text);
  font-size: 12px;
  cursor: pointer;
  white-space: nowrap;
}

.km-btn-icon {
  padding: 0 8px;
}

.km-btn:hover:not(:disabled) {
  background: var(--km-hover-bg);
}

.km-btn:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.km-btn-primary {
  background: var(--km-accent);
  border-color: var(--km-accent);
  color: var(--km-accent-fg);
  font-weight: 600;
}

.km-btn-primary:hover:not(:disabled) {
  background: var(--km-accent);
  opacity: 0.92;
}

.km-icon-btn {
  width: 28px;
  height: 28px;
  border: none;
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-text-muted);
  cursor: pointer;
  font-size: 13px;
}

.km-icon-btn:hover {
  background: var(--km-hover-bg);
  color: var(--km-text);
}
</style>

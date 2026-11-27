<template>
  <Teleport to="body">
    <div
      v-if="open"
      class="km-compose-backdrop"
      role="dialog"
      aria-modal="true"
      :aria-label="headerTitle"
      @pointerdown.self="onBackdropPointerDown"
    >
      <section class="km-compose-modal" @keydown="onModalKeyDown">
        <!-- 顶部栏 -->
        <header class="km-compose-header">
          <div class="km-compose-title-wrap">
            <h2 class="km-compose-title">{{ headerTitle }}</h2>
            <span class="km-draft-status" :class="{ saving: state.savingDraft.value }">
              {{ draftStatusText }}
            </span>
          </div>

          <div class="km-compose-header-actions">
            <!-- 草稿列表切换 -->
            <div v-if="state.drafts.value.length > 0" ref="draftWrapRef" class="km-popover-wrap">
              <button
                type="button"
                class="km-hdr-btn"
                aria-haspopup="menu"
                :aria-expanded="draftMenuOpen"
                @click="toggleDraftMenu"
              >
                草稿 ({{ state.drafts.value.length }}) ▾
              </button>
              <div v-if="draftMenuOpen" class="km-compose-popover km-draft-popover" role="menu">
                <div class="km-popover-title">本地草稿箱</div>
                <div
                  v-for="d in state.drafts.value"
                  :key="d.id"
                  class="km-draft-item"
                  :class="{ active: d.id === state.form.value.localDraftId }"
                >
                  <button
                    type="button"
                    class="km-draft-pick"
                    @click="pickDraft(d)"
                  >
                    <span class="km-draft-subj">{{ d.subject || "(无主题)" }}</span>
                    <span class="km-draft-meta">
                      {{ d.toAddr || "无收件人" }} · {{ formatTime(d.updatedAt) }}
                    </span>
                  </button>
                  <button
                    type="button"
                    class="km-draft-del"
                    title="删除此草稿"
                    aria-label="删除此草稿"
                    @click.stop="state.deleteDraftById(d.id)"
                  >
                    ✕
                  </button>
                </div>
              </div>
            </div>

            <button
              type="button"
              class="km-close-btn"
              title="保存草稿并关闭 (Esc)"
              aria-label="关闭写邮件窗口"
              @click="handleClose"
            >
              ✕
            </button>
          </div>
        </header>

        <!-- 收发件与主题表单区 -->
        <div class="km-compose-fields">
          <div class="km-field-row">
            <label class="km-field-label" for="km-compose-from">发件人</label>
            <select
              v-if="accounts.length > 1"
              id="km-compose-from"
              class="km-field-select"
              :value="state.form.value.accountId"
              @change="onAccountChange"
            >
              <option v-for="acc in accounts" :key="acc.id" :value="acc.id">
                {{ acc.displayName ? `${acc.displayName} <${acc.email}>` : acc.email }}
              </option>
            </select>
            <div v-else class="km-from-readonly">
              {{ singleAccountLabel }}
            </div>
          </div>

          <div class="km-field-row">
            <label class="km-field-label" for="km-compose-to">收件人</label>
            <input
              id="km-compose-to"
              ref="toInputRef"
              type="text"
              class="km-field-input"
              placeholder="输入收件人邮箱，多个地址用逗号分隔"
              :value="state.form.value.toAddr"
              @input="onTextInput('toAddr', $event)"
            />
            <button
              type="button"
              class="km-cc-toggle"
              @click="state.updateField('showCcBcc', !state.form.value.showCcBcc)"
            >
              {{ state.form.value.showCcBcc ? "隐藏抄送/密送" : "抄送/密送" }}
            </button>
          </div>

          <template v-if="state.form.value.showCcBcc">
            <div class="km-field-row">
              <label class="km-field-label" for="km-compose-cc">抄送</label>
              <input
                id="km-compose-cc"
                type="text"
                class="km-field-input"
                placeholder="抄送邮箱（可选）"
                :value="state.form.value.ccAddr"
                @input="onTextInput('ccAddr', $event)"
              />
            </div>
            <div class="km-field-row">
              <label class="km-field-label" for="km-compose-bcc">密送</label>
              <input
                id="km-compose-bcc"
                type="text"
                class="km-field-input"
                placeholder="密送邮箱（可选）"
                :value="state.form.value.bccAddr"
                @input="onTextInput('bccAddr', $event)"
              />
            </div>
          </template>

          <div class="km-field-row">
            <label class="km-field-label" for="km-compose-subject">主题</label>
            <input
              id="km-compose-subject"
              type="text"
              class="km-field-input km-subject-input"
              placeholder="输入邮件主题"
              :value="state.form.value.subject"
              @input="onTextInput('subject', $event)"
            />
          </div>
        </div>

        <!-- 格式切换与快捷键提示栏 -->
        <div class="km-format-bar">
          <div class="km-format-switch" role="group" aria-label="正文格式">
            <button
              v-for="fmt in BODY_FORMATS"
              :key="fmt.value"
              type="button"
              class="km-seg-btn"
              :class="{ active: state.form.value.bodyFormat === fmt.value }"
              @click="onFormatChange(fmt.value)"
            >
              {{ fmt.label }}
            </button>
          </div>

          <div class="km-format-right">
            <button
              v-if="state.form.value.bodyFormat !== 'plain'"
              type="button"
              class="km-hdr-btn"
              :class="{ active: previewOpen }"
              @click="previewOpen = !previewOpen"
            >
              {{ previewOpen ? "返回编辑" : "预览效果" }}
            </button>
            <span class="km-shortcut-hint">Ctrl+Enter 发送</span>
          </div>
        </div>

        <!-- 正文编辑或预览区 -->
        <div class="km-editor-area">
          <textarea
            v-if="!previewOpen || state.form.value.bodyFormat === 'plain'"
            ref="bodyInputRef"
            class="km-body-textarea"
            :class="{ mono: state.form.value.bodyFormat !== 'plain' }"
            :placeholder="bodyPlaceholder"
            :value="state.form.value.bodyText"
            @input="onTextInput('bodyText', $event)"
          />
          <div v-else class="km-preview-pane">
            <div
              v-if="state.form.value.bodyFormat === 'markdown'"
              class="km-md-preview"
              v-html="markdownPreviewHtml"
            />
            <iframe
              v-else
              class="km-html-preview"
              title="HTML 邮件预览"
              sandbox=""
              :srcdoc="state.form.value.bodyText"
            />
          </div>
        </div>

        <!-- 附件栏 -->
        <div class="km-attach-bar">
          <input
            ref="fileInputRef"
            type="file"
            multiple
            class="km-hidden-file"
            @change="onFilePicked"
          />
          <button type="button" class="km-attach-btn" @click="triggerFilePicker">
            📎 添加附件
          </button>

          <div v-if="showPathInput" class="km-path-input-wrap">
            <input
              v-model="manualPath"
              type="text"
              class="km-path-input"
              placeholder="输入本地文件路径后回车"
              @keydown.enter.prevent="commitManualPath"
            />
            <button type="button" class="km-hdr-btn" @click="commitManualPath">添加</button>
          </div>
          <button
            v-else
            type="button"
            class="km-link-btn"
            @click="showPathInput = true"
          >
            + 输入路径
          </button>

          <div class="km-attach-chips">
            <span
              v-for="(p, idx) in state.form.value.attachmentPaths"
              :key="`${p}:${idx}`"
              class="km-attach-chip"
              :title="p"
            >
              <span class="km-attach-name">{{ basenameOf(p) }}</span>
              <button
                type="button"
                class="km-attach-rm"
                aria-label="移除附件"
                @click="state.removeAttachmentAt(idx)"
              >
                ✕
              </button>
            </span>
          </div>
        </div>

        <!-- 忘带附件提醒条 -->
        <div
          v-if="state.missingAttachmentCheck.value.shouldWarn"
          class="km-alert-banner km-warn-banner"
          role="alert"
        >
          <span class="km-alert-text">
            检测到正文或主题提及附件（{{ state.missingAttachmentCheck.value.matchedKeywords.join("、") }}），但尚未添加任何附件。
          </span>
          <div class="km-alert-actions">
            <button type="button" class="km-hdr-btn" @click="triggerFilePicker">
              添加附件
            </button>
            <button
              type="button"
              class="km-hdr-btn km-warn-confirm-btn"
              @click="onIgnoreAndSendIfPending"
            >
              {{ pendingConfirmSend ? "忽略并继续发送" : "忽略提醒" }}
            </button>
          </div>
        </div>

        <!-- 状态提示或错误条 -->
        <div v-if="state.error.value" class="km-alert-banner km-error-banner" role="alert">
          <span>{{ state.error.value }}</span>
        </div>
        <div
          v-else-if="state.statusNotice.value"
          class="km-alert-banner km-notice-banner"
          role="status"
        >
          <span>{{ state.statusNotice.value }}</span>
        </div>

        <!-- 底部操作栏 -->
        <footer class="km-compose-footer">
          <div class="km-footer-left">
            <button
              type="button"
              class="km-send-btn"
              :disabled="state.sending.value"
              @click="handleSend()"
            >
              {{ sendButtonLabel }}
            </button>

            <!-- 定时发送选择 -->
            <div ref="scheduleWrapRef" class="km-popover-wrap">
              <button
                type="button"
                class="km-hdr-btn"
                aria-haspopup="menu"
                :aria-expanded="scheduleMenuOpen"
                @click="toggleScheduleMenu"
              >
                {{ state.form.value.scheduledAt ? "已定时 ▾" : "定时发送 ▾" }}
              </button>

              <div v-if="scheduleMenuOpen" class="km-compose-popover km-schedule-popover" role="menu">
                <div class="km-popover-title">选择发送时间</div>
                <button
                  type="button"
                  class="km-schedule-item"
                  @click="pickScheduleOffset(3600)"
                >
                  1 小时后
                </button>
                <button
                  type="button"
                  class="km-schedule-item"
                  @click="pickScheduleOffset(3 * 3600)"
                >
                  3 小时后
                </button>
                <button
                  type="button"
                  class="km-schedule-item"
                  @click="pickTomorrowMorning()"
                >
                  明天上午 09:00
                </button>
                <div class="km-schedule-custom">
                  <input
                    v-model="customScheduleLocal"
                    type="datetime-local"
                    class="km-schedule-input"
                    aria-label="自定义定时发送时间"
                  />
                  <button
                    type="button"
                    class="km-hdr-btn"
                    :disabled="!customScheduleLocal"
                    @click="applyCustomSchedule"
                  >
                    应用
                  </button>
                </div>
                <button
                  v-if="state.form.value.scheduledAt"
                  type="button"
                  class="km-schedule-item km-schedule-clear"
                  @click="clearSchedule"
                >
                  取消定时（改为立即发送）
                </button>
              </div>
            </div>

            <span v-if="state.form.value.scheduledAt" class="km-scheduled-chip">
              将于 {{ formatDateTime(state.form.value.scheduledAt) }} 发送
              <button
                type="button"
                class="km-attach-rm"
                aria-label="取消定时发送"
                @click="clearSchedule"
              >
                ✕
              </button>
            </span>
          </div>

          <div class="km-footer-right">
            <button
              type="button"
              class="km-hdr-btn"
              :disabled="state.savingDraft.value"
              @click="handleSaveDraft"
            >
              存草稿
            </button>
            <button
              type="button"
              class="km-hdr-btn km-danger-btn"
              @click="handleDiscard"
            >
              放弃草稿
            </button>
          </div>
        </footer>
      </section>
    </div>
  </Teleport>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import {
  renderSimpleMarkdownToHtml,
  useCompose,
  type UseComposeReturn,
} from "./useCompose";
import type {
  BodyFormat,
  ComposeAccountOption,
  ComposeFormState,
  ComposeSeed,
  LocalDraftDto,
  SendMailOutput,
} from "./types";

const props = defineProps<{
  open: boolean;
  accounts: ComposeAccountOption[];
  seed?: ComposeSeed | null;
  composeState?: UseComposeReturn;
}>();

const emit = defineEmits<{
  "update:open": [open: boolean];
  overlayChange: [open: boolean];
  sent: [result: SendMailOutput];
  draftSaved: [draft: LocalDraftDto];
  closed: [];
}>();

const BODY_FORMATS: ReadonlyArray<{ value: BodyFormat; label: string }> = [
  { value: "plain", label: "纯文本" },
  { value: "markdown", label: "Markdown" },
  { value: "html", label: "HTML" },
];

const ownsState = !props.composeState;
const state = props.composeState ?? useCompose({ accounts: props.accounts });

const toInputRef = ref<HTMLInputElement | null>(null);
const bodyInputRef = ref<HTMLTextAreaElement | null>(null);
const fileInputRef = ref<HTMLInputElement | null>(null);
const draftWrapRef = ref<HTMLElement | null>(null);
const scheduleWrapRef = ref<HTMLElement | null>(null);

const draftMenuOpen = ref(false);
const scheduleMenuOpen = ref(false);
const previewOpen = ref(false);
const showPathInput = ref(false);
const manualPath = ref("");
const customScheduleLocal = ref("");
const pendingConfirmSend = ref(false);

const headerTitle = computed(() => {
  if (state.form.value.localDraftId && state.form.value.mode === "new") {
    return "编辑草稿";
  }
  switch (state.form.value.mode) {
    case "reply":
      return "回复邮件";
    case "replyAll":
      return "全部回复";
    case "forward":
      return "转发邮件";
    default:
      return "写邮件";
  }
});

const singleAccountLabel = computed(() => {
  const active =
    props.accounts.find((a) => a.id === state.form.value.accountId) ?? props.accounts[0];
  if (!active) return state.form.value.fromAddr || "默认账号";
  return active.displayName ? `${active.displayName} <${active.email}>` : active.email;
});

const draftStatusText = computed(() => {
  if (state.savingDraft.value) return "正在保存草稿…";
  if (state.dirty.value) return "有未保存更改";
  if (state.lastSavedAt.value) {
    return `草稿已保存 ${formatTime(state.lastSavedAt.value)}`;
  }
  return "新邮件";
});

const bodyPlaceholder = computed(() => {
  if (state.form.value.bodyFormat === "markdown") {
    return "使用 Markdown 编写邮件正文（支持标题、加粗、列表、代码块、引用）…";
  }
  if (state.form.value.bodyFormat === "html") {
    return "输入或粘贴 HTML 邮件正文…";
  }
  return "输入邮件正文…";
});

const markdownPreviewHtml = computed(() =>
  renderSimpleMarkdownToHtml(state.form.value.bodyText)
);

const sendButtonLabel = computed(() => {
  if (state.sending.value) return "发送中…";
  return state.form.value.scheduledAt ? "定时发送" : "发送";
});

function formatTime(epochSecs: number): string {
  const d = new Date(epochSecs * 1000);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  const ss = String(d.getSeconds()).padStart(2, "0");
  return `${hh}:${mm}:${ss}`;
}

function formatDateTime(epochSecs: number): string {
  const d = new Date(epochSecs * 1000);
  const mon = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `${mon}-${day} ${hh}:${mm}`;
}

function basenameOf(filePath: string): string {
  const normalized = filePath.replace(/\\/g, "/");
  const idx = normalized.lastIndexOf("/");
  return idx >= 0 ? normalized.slice(idx + 1) : normalized;
}

function onAccountChange(e: Event) {
  const value = (e.target as HTMLSelectElement).value;
  state.updateField("accountId", value);
}

function onTextInput<
  K extends "toAddr" | "ccAddr" | "bccAddr" | "subject" | "bodyText"
>(key: K, e: Event) {
  pendingConfirmSend.value = false;
  state.updateField(key, (e.target as HTMLInputElement | HTMLTextAreaElement).value as ComposeFormState[K]);
}

function onFormatChange(fmt: BodyFormat) {
  state.updateField("bodyFormat", fmt);
  if (fmt === "plain") {
    previewOpen.value = false;
  }
}

function triggerFilePicker() {
  fileInputRef.value?.click();
}

function onFilePicked(e: Event) {
  const input = e.target as HTMLInputElement;
  const files = input.files;
  if (!files || files.length === 0) return;
  const paths: string[] = [];
  for (let i = 0; i < files.length; i++) {
    const file = files[i] as File & { path?: string };
    paths.push(file.path && file.path.trim().length > 0 ? file.path : file.name);
  }
  state.addAttachmentPaths(paths);
  pendingConfirmSend.value = false;
  input.value = "";
}

function commitManualPath() {
  const trimmed = manualPath.value.trim();
  if (trimmed) {
    state.addAttachmentPaths([trimmed]);
    manualPath.value = "";
    pendingConfirmSend.value = false;
  }
  showPathInput.value = false;
}

function toggleDraftMenu() {
  scheduleMenuOpen.value = false;
  draftMenuOpen.value = !draftMenuOpen.value;
}

function pickDraft(draft: LocalDraftDto) {
  draftMenuOpen.value = false;
  previewOpen.value = false;
  pendingConfirmSend.value = false;
  state.openDraft(draft);
}

function toggleScheduleMenu() {
  draftMenuOpen.value = false;
  scheduleMenuOpen.value = !scheduleMenuOpen.value;
}

function pickScheduleOffset(offsetSecs: number) {
  const target = Math.floor(Date.now() / 1000) + offsetSecs;
  state.updateField("scheduledAt", target);
  scheduleMenuOpen.value = false;
}

function pickTomorrowMorning() {
  const now = new Date();
  const tomorrow = new Date(
    now.getFullYear(),
    now.getMonth(),
    now.getDate() + 1,
    9,
    0,
    0,
    0
  );
  state.updateField("scheduledAt", Math.floor(tomorrow.getTime() / 1000));
  scheduleMenuOpen.value = false;
}

function applyCustomSchedule() {
  if (!customScheduleLocal.value) return;
  const ms = Date.parse(customScheduleLocal.value);
  if (Number.isNaN(ms)) return;
  state.updateField("scheduledAt", Math.floor(ms / 1000));
  scheduleMenuOpen.value = false;
}

function clearSchedule() {
  state.updateField("scheduledAt", null);
  scheduleMenuOpen.value = false;
}

async function handleSaveDraft() {
  const draft = await state.saveDraftNow(false);
  if (draft) {
    emit("draftSaved", draft);
  }
}

async function handleDiscard() {
  previewOpen.value = false;
  pendingConfirmSend.value = false;
  await state.discardDraft();
  emit("update:open", false);
  emit("closed");
}

async function handleClose() {
  draftMenuOpen.value = false;
  scheduleMenuOpen.value = false;
  await state.closeWithAutoSave();
  emit("update:open", false);
  emit("closed");
}

async function handleSend(ignoreAttachmentWarning = false) {
  const res = await state.send({ ignoreAttachmentWarning });
  if (res.sent) {
    pendingConfirmSend.value = false;
    previewOpen.value = false;
    emit("sent", res.output);
    emit("update:open", false);
    return;
  }
  if (res.needsAttachmentConfirm) {
    pendingConfirmSend.value = true;
  }
}

async function onIgnoreAndSendIfPending() {
  const shouldSendNow = pendingConfirmSend.value;
  state.ignoreAttachmentWarning();
  pendingConfirmSend.value = false;
  if (shouldSendNow) {
    await handleSend(true);
  }
}

function onBackdropPointerDown() {
  void handleClose();
}

function onModalKeyDown(e: KeyboardEvent) {
  if (e.isComposing || e.key === "Process" || e.keyCode === 229) return;

  if (e.key === "Escape") {
    e.preventDefault();
    e.stopPropagation();
    if (draftMenuOpen.value) {
      draftMenuOpen.value = false;
      return;
    }
    if (scheduleMenuOpen.value) {
      scheduleMenuOpen.value = false;
      return;
    }
    void handleClose();
    return;
  }

  if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key === "Enter") {
    e.preventDefault();
    void handleSend();
  }
}

function onDocumentPointerDown(e: PointerEvent) {
  const target = e.target as Node;
  if (draftMenuOpen.value && draftWrapRef.value && !draftWrapRef.value.contains(target)) {
    draftMenuOpen.value = false;
  }
  if (
    scheduleMenuOpen.value &&
    scheduleWrapRef.value &&
    !scheduleWrapRef.value.contains(target)
  ) {
    scheduleMenuOpen.value = false;
  }
}

function focusInitialInput() {
  void nextTick(() => {
    if (state.form.value.mode === "reply" || state.form.value.mode === "replyAll") {
      bodyInputRef.value?.focus();
    } else {
      toInputRef.value?.focus();
    }
  });
}

watch(
  () => props.accounts,
  (next) => {
    state.setAccounts(next);
  },
  { deep: true, immediate: true }
);

watch(
  () => props.seed,
  (nextSeed) => {
    if (props.open && nextSeed) {
      previewOpen.value = false;
      pendingConfirmSend.value = false;
      state.openCompose(nextSeed);
      focusInitialInput();
    }
  }
);

watch(
  () => props.open,
  (isOpen, prevOpen) => {
    if (isOpen === prevOpen) return;
    emit("overlayChange", isOpen);
    if (isOpen) {
      if (props.seed && !state.open.value) {
        state.openCompose(props.seed);
      } else if (!state.open.value) {
        state.openCompose({ mode: "new" });
      }
      previewOpen.value = false;
      pendingConfirmSend.value = false;
      focusInitialInput();
    } else {
      draftMenuOpen.value = false;
      scheduleMenuOpen.value = false;
      if (state.open.value) {
        state.open.value = false;
      }
    }
  }
);

onMounted(() => {
  document.addEventListener("pointerdown", onDocumentPointerDown);
  if (props.open) {
    emit("overlayChange", true);
    if (!state.open.value) {
      state.openCompose(props.seed ?? { mode: "new" });
    }
    focusInitialInput();
  }
});

onUnmounted(() => {
  document.removeEventListener("pointerdown", onDocumentPointerDown);
  if (props.open) {
    emit("overlayChange", false);
  }
  if (ownsState) {
    state.dispose();
  }
});
</script>

<style scoped>
.km-compose-backdrop {
  position: fixed;
  inset: 0;
  z-index: 80;
  background: rgba(0, 0, 0, 0.36);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
}

.km-compose-modal {
  width: min(760px, 100%);
  height: min(680px, 100%);
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

.km-compose-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 14px;
  border-bottom: 1px solid var(--km-border);
  background: var(--km-sidebar);
  flex-shrink: 0;
}

.km-compose-title-wrap {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}

.km-compose-title {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
}

.km-draft-status {
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-draft-status.saving {
  color: var(--km-accent);
}

.km-compose-header-actions {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}

.km-hdr-btn {
  height: 28px;
  padding: 0 10px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface-raised);
  color: var(--km-text);
  font-size: 12px;
  cursor: pointer;
  white-space: nowrap;
}

.km-hdr-btn:hover:not(:disabled) {
  background: var(--km-hover-bg);
}

.km-hdr-btn.active {
  background: var(--km-accent-soft);
  border-color: var(--km-accent);
  color: var(--km-accent);
}

.km-hdr-btn:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.km-danger-btn:hover:not(:disabled) {
  color: var(--km-danger);
  border-color: color-mix(in srgb, var(--km-danger) 45%, var(--km-border));
}

.km-close-btn {
  width: 28px;
  height: 28px;
  border: none;
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-text-muted);
  cursor: pointer;
  font-size: 13px;
}

.km-close-btn:hover {
  background: var(--km-hover-bg);
  color: var(--km-text);
}

.km-compose-fields {
  display: flex;
  flex-direction: column;
  border-bottom: 1px solid var(--km-border);
  flex-shrink: 0;
}

.km-field-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 36px;
  padding: 4px 14px;
  border-bottom: 1px solid color-mix(in srgb, var(--km-border) 55%, transparent);
}

.km-field-row:last-child {
  border-bottom: none;
}

.km-field-label {
  width: 48px;
  flex-shrink: 0;
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-field-input,
.km-field-select {
  flex: 1 1 auto;
  min-width: 0;
  border: none;
  background: transparent;
  color: var(--km-text);
  font-size: 13px;
  padding: 4px 0;
  outline: none;
}

.km-field-select option {
  background: var(--km-surface);
  color: var(--km-text);
}

.km-subject-input {
  font-weight: 600;
}

.km-from-readonly {
  font-size: 13px;
  color: var(--km-text);
}

.km-cc-toggle {
  border: none;
  background: transparent;
  color: var(--km-accent);
  font-size: 12px;
  cursor: pointer;
  padding: 2px 6px;
  border-radius: var(--km-radius-sm);
}

.km-cc-toggle:hover {
  background: var(--km-accent-soft);
}

.km-format-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 14px;
  border-bottom: 1px solid var(--km-border);
  background: var(--km-canvas);
  flex-shrink: 0;
}

.km-format-switch {
  display: inline-flex;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  overflow: hidden;
  background: var(--km-surface);
}

.km-seg-btn {
  height: 24px;
  padding: 0 10px;
  border: none;
  background: transparent;
  font-size: 12px;
  color: var(--km-text-muted);
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

.km-format-right {
  display: inline-flex;
  align-items: center;
  gap: 10px;
}

.km-shortcut-hint {
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-editor-area {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--km-surface);
}

.km-body-textarea {
  flex: 1 1 auto;
  width: 100%;
  min-height: 0;
  border: none;
  resize: none;
  padding: 12px 14px;
  background: transparent;
  color: var(--km-text);
  line-height: 1.55;
  outline: none;
}

.km-body-textarea.mono {
  font-family: var(--km-font-mono);
  font-size: 12.5px;
}

.km-preview-pane {
  flex: 1 1 auto;
  min-height: 0;
  overflow: auto;
  padding: 12px 14px;
}

.km-md-preview {
  line-height: 1.6;
}

.km-md-preview :deep(pre) {
  padding: 8px 10px;
  border-radius: var(--km-radius-sm);
  background: var(--km-canvas);
  border: 1px solid var(--km-border);
  overflow: auto;
  font-family: var(--km-font-mono);
}

.km-md-preview :deep(blockquote) {
  margin: 6px 0;
  padding-left: 10px;
  border-left: 3px solid var(--km-accent);
  color: var(--km-text-muted);
}

.km-html-preview {
  width: 100%;
  height: 100%;
  min-height: 220px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: #ffffff;
}

.km-attach-bar {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  padding: 8px 14px;
  border-top: 1px solid var(--km-border);
  background: var(--km-canvas);
  flex-shrink: 0;
}

.km-hidden-file {
  display: none;
}

.km-attach-btn {
  height: 26px;
  padding: 0 9px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface-raised);
  font-size: 12px;
  cursor: pointer;
}

.km-attach-btn:hover {
  background: var(--km-hover-bg);
}

.km-link-btn {
  border: none;
  background: transparent;
  color: var(--km-accent);
  font-size: 12px;
  cursor: pointer;
  padding: 2px 6px;
}

.km-path-input-wrap {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.km-path-input {
  width: 210px;
  height: 26px;
  padding: 0 8px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface);
  font-size: 12px;
}

.km-attach-chips {
  display: inline-flex;
  flex-wrap: wrap;
  gap: 6px;
}

.km-attach-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 2px 8px;
  border-radius: 999px;
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  font-size: 12px;
  max-width: 220px;
}

.km-attach-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-attach-rm {
  border: none;
  background: transparent;
  color: var(--km-text-muted);
  cursor: pointer;
  padding: 0 2px;
  font-size: 11px;
}

.km-attach-rm:hover {
  color: var(--km-danger);
}

.km-alert-banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 8px;
  padding: 8px 14px;
  font-size: 12px;
  border-top: 1px solid var(--km-border);
  flex-shrink: 0;
}

.km-warn-banner {
  background: color-mix(in srgb, var(--km-star-gold) 16%, var(--km-surface));
  color: var(--km-text);
}

.km-warn-confirm-btn {
  border-color: var(--km-accent);
  color: var(--km-accent);
}

.km-error-banner {
  background: var(--km-danger-soft);
  color: var(--km-danger);
}

.km-notice-banner {
  background: var(--km-accent-soft);
  color: var(--km-accent);
}

.km-alert-actions {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.km-compose-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 8px;
  padding: 10px 14px;
  border-top: 1px solid var(--km-border);
  background: var(--km-sidebar);
  flex-shrink: 0;
}

.km-footer-left,
.km-footer-right {
  display: inline-flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
}

.km-send-btn {
  height: 30px;
  padding: 0 16px;
  border: none;
  border-radius: var(--km-radius-sm);
  background: var(--km-accent);
  color: var(--km-accent-fg);
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
}

.km-send-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.km-scheduled-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 3px 8px;
  border-radius: var(--km-radius-sm);
  background: var(--km-accent-soft);
  color: var(--km-accent);
  font-size: 12px;
}

.km-popover-wrap {
  position: relative;
}

.km-compose-popover {
  position: absolute;
  z-index: 90;
  min-width: 240px;
  background: var(--km-surface-raised);
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-md);
  box-shadow: var(--km-shadow-popover);
  padding: 6px;
}

.km-draft-popover {
  top: calc(100% + 4px);
  right: 0;
  width: 290px;
  max-height: 280px;
  overflow: auto;
}

.km-schedule-popover {
  bottom: calc(100% + 6px);
  left: 0;
  width: 260px;
}

.km-popover-title {
  padding: 4px 8px;
  font-size: 11px;
  font-weight: 600;
  color: var(--km-text-muted);
}

.km-draft-item {
  display: flex;
  align-items: center;
  gap: 4px;
  border-radius: var(--km-radius-sm);
}

.km-draft-item.active {
  background: var(--km-accent-soft);
}

.km-draft-pick {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  padding: 6px 8px;
  border: none;
  background: transparent;
  text-align: left;
  cursor: pointer;
}

.km-draft-pick:hover {
  background: var(--km-hover-bg);
}

.km-draft-subj {
  font-size: 12px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  width: 100%;
}

.km-draft-meta {
  font-size: 11px;
  color: var(--km-text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  width: 100%;
}

.km-draft-del {
  border: none;
  background: transparent;
  color: var(--km-text-muted);
  padding: 4px 8px;
  cursor: pointer;
}

.km-draft-del:hover {
  color: var(--km-danger);
}

.km-schedule-item {
  width: 100%;
  text-align: left;
  padding: 6px 8px;
  border: none;
  border-radius: var(--km-radius-sm);
  background: transparent;
  font-size: 12px;
  cursor: pointer;
}

.km-schedule-item:hover {
  background: var(--km-hover-bg);
}

.km-schedule-clear {
  color: var(--km-danger);
  border-top: 1px solid var(--km-border);
  margin-top: 4px;
  border-radius: 0 0 var(--km-radius-sm) var(--km-radius-sm);
}

.km-schedule-custom {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 6px 4px 2px;
}

.km-schedule-input {
  flex: 1 1 auto;
  min-width: 0;
  height: 26px;
  padding: 0 6px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface);
  font-size: 11px;
}
</style>

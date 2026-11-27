<template>
  <Teleport to="body">
    <div
      v-if="open"
      class="km-drawer-backdrop"
      role="dialog"
      aria-modal="true"
      aria-label="邮件附件列表"
      @pointerdown.self="closeDrawer"
    >
      <aside class="km-attachment-drawer" @keydown="onKeyDown">
        <header class="km-drawer-header">
          <div class="km-drawer-title-wrap">
            <h2 class="km-drawer-title">
              邮件附件 ({{ attachments.length }})
            </h2>
            <p v-if="subject" class="km-drawer-subtitle" :title="subject">
              {{ subject }}
            </p>
            <span class="km-drawer-meta">
              总大小 {{ formatAttachmentSize(totalBytes) }}
            </span>
          </div>

          <div class="km-drawer-header-actions">
            <button
              v-if="savableIds.length > 1"
              type="button"
              class="km-btn km-btn-primary"
              :disabled="state.savingAll.value"
              @click="handleSaveAll"
            >
              {{ state.savingAll.value ? "保存中…" : "全部保存" }}
            </button>
            <button
              type="button"
              class="km-icon-btn"
              title="关闭 (Esc)"
              aria-label="关闭附件抽屉"
              @click="closeDrawer"
            >
              ✕
            </button>
          </div>
        </header>

        <div
          v-if="state.error.value || state.previewError.value"
          class="km-drawer-banner km-banner-error"
          role="alert"
        >
          <span>{{ state.error.value ?? state.previewError.value }}</span>
          <button
            type="button"
            class="km-banner-close"
            aria-label="关闭错误提示"
            @click="state.clearStatus()"
          >
            ✕
          </button>
        </div>
        <div
          v-else-if="state.statusMessage.value"
          class="km-drawer-banner km-banner-success"
          role="status"
        >
          <span>{{ state.statusMessage.value }}</span>
          <button
            type="button"
            class="km-banner-close"
            aria-label="关闭提示"
            @click="state.clearStatus()"
          >
            ✕
          </button>
        </div>

        <div class="km-drawer-body">
          <div v-if="attachments.length === 0" class="km-empty-state">
            <span class="km-empty-icon">📎</span>
            <p class="km-empty-text">当前邮件没有附件</p>
          </div>

          <ul v-else class="km-attach-list">
            <li
              v-for="(att, idx) in attachments"
              :key="`${att.id ?? att.filename}-${idx}`"
              class="km-attach-card"
            >
              <div class="km-attach-thumb" :class="{ 'is-image': isImageAttachment(att) }">
                <img
                  v-if="isImageAttachment(att) && cachedDataUrl(att)"
                  :src="cachedDataUrl(att)!"
                  :alt="att.filename"
                  class="km-attach-thumb-img"
                />
                <span v-else class="km-attach-badge">
                  {{ badgeTextFor(att) }}
                </span>
              </div>

              <div class="km-attach-info">
                <div class="km-attach-name-row">
                  <span class="km-attach-filename" :title="att.filename">
                    {{ att.filename }}
                  </span>
                  <span v-if="isImageAttachment(att)" class="km-tag-pill">图片</span>
                </div>
                <div class="km-attach-sub">
                  <span>{{ formatAttachmentSize(att.sizeBytes) }}</span>
                  <span>·</span>
                  <span class="km-attach-mime" :title="att.mimeType">{{ att.mimeType }}</span>
                </div>
              </div>

              <div class="km-attach-actions">
                <button
                  type="button"
                  class="km-btn"
                  :disabled="!hasNumericId(att) || isPreviewLoading(att)"
                  :title="hasNumericId(att) ? '预览附件' : '暂无缓存 ID，无法预览'"
                  @click="handlePreview(att)"
                >
                  {{ isPreviewLoading(att) ? "加载…" : isImageAttachment(att) ? "大图预览" : "预览" }}
                </button>
                <button
                  type="button"
                  class="km-btn"
                  :disabled="!hasNumericId(att) || isSaving(att)"
                  :title="hasNumericId(att) ? '保存附件到本地' : '暂无缓存 ID，无法保存'"
                  @click="handleSaveOne(att)"
                >
                  {{ isSaving(att) ? "保存中…" : "保存附件" }}
                </button>
              </div>
            </li>
          </ul>
        </div>
      </aside>

      <!-- 抽屉内嵌大图/文本灯箱预览 -->
      <div
        v-if="state.activePreview.value"
        class="km-lightbox-overlay"
        role="dialog"
        aria-modal="true"
        :aria-label="state.activePreview.value.filename"
        @pointerdown.self="state.closePreview()"
      >
        <div class="km-lightbox-panel">
          <header class="km-lightbox-header">
            <div class="km-lightbox-meta">
              <strong class="km-lightbox-title">{{ state.activePreview.value.filename }}</strong>
              <span class="km-lightbox-sub">
                {{ state.activePreview.value.mimeType }} ·
                {{ formatAttachmentSize(state.activePreview.value.sizeBytes) }}
              </span>
            </div>
            <div class="km-lightbox-actions">
              <button
                type="button"
                class="km-btn km-btn-primary"
                :disabled="state.savingId.value === state.activePreview.value.attachmentId"
                @click="state.saveSingle(state.activePreview.value.attachmentId)"
              >
                保存附件
              </button>
              <button
                type="button"
                class="km-icon-btn"
                aria-label="关闭预览"
                @click="state.closePreview()"
              >
                ✕
              </button>
            </div>
          </header>

          <div class="km-lightbox-stage">
            <img
              v-if="isPreviewImage(state.activePreview.value.mimeType, state.activePreview.value.dataUrl)"
              :src="state.activePreview.value.dataUrl"
              :alt="state.activePreview.value.filename"
              class="km-lightbox-img"
            />
            <pre v-else class="km-lightbox-text">{{
              decodeTextPreviewDataUrl(state.activePreview.value.dataUrl) ?? "该文件类型暂不支持内嵌文本预览，请点击右上角保存后查看。"
            }}</pre>
          </div>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, watch } from "vue";
import {
  decodeTextPreviewDataUrl,
  formatAttachmentSize,
  isImageAttachment,
  resolveNumericAttachmentId,
  useAttachments,
} from "./useAttachments";
import type { AttachmentDto } from "./types";

const OVERLAY_ID = "attachment-drawer";

const props = defineProps<{
  open: boolean;
  attachments: AttachmentDto[];
  subject?: string;
}>();

const emit = defineEmits<{
  "update:open": [open: boolean];
  close: [];
  overlayChange: [id: string, open: boolean];
}>();

const state = useAttachments();

let overlayReportedOpen = false;
function reportOverlay(next: boolean) {
  if (overlayReportedOpen === next) return;
  overlayReportedOpen = next;
  emit("overlayChange", OVERLAY_ID, next);
}

const totalBytes = computed(() =>
  props.attachments.reduce((sum, item) => sum + (Number.isFinite(item.sizeBytes) ? item.sizeBytes : 0), 0)
);

const savableIds = computed(() => {
  const ids: number[] = [];
  for (const att of props.attachments) {
    const id = resolveNumericAttachmentId(att.id);
    if (id !== null) ids.push(id);
  }
  return ids;
});

function hasNumericId(att: AttachmentDto): boolean {
  return resolveNumericAttachmentId(att.id) !== null;
}

function cachedDataUrl(att: AttachmentDto): string | null {
  const id = resolveNumericAttachmentId(att.id);
  if (id === null) return null;
  return state.previewCache.value[id]?.dataUrl ?? null;
}

function isPreviewLoading(att: AttachmentDto): boolean {
  const id = resolveNumericAttachmentId(att.id);
  return id !== null && state.previewLoadingId.value === id;
}

function isSaving(att: AttachmentDto): boolean {
  const id = resolveNumericAttachmentId(att.id);
  return id !== null && state.savingId.value === id;
}

function badgeTextFor(att: AttachmentDto): string {
  const dotIdx = att.filename.lastIndexOf(".");
  if (dotIdx >= 0 && dotIdx < att.filename.length - 1) {
    return att.filename.slice(dotIdx + 1, dotIdx + 5).toUpperCase();
  }
  return isImageAttachment(att) ? "IMG" : "FILE";
}

function isPreviewImage(mimeType: string, dataUrl: string): boolean {
  return mimeType.toLowerCase().startsWith("image/") || dataUrl.startsWith("data:image/");
}

async function handlePreview(att: AttachmentDto) {
  const id = resolveNumericAttachmentId(att.id);
  if (id === null) return;
  await state.openPreview(id);
}

async function handleSaveOne(att: AttachmentDto) {
  const id = resolveNumericAttachmentId(att.id);
  if (id === null) return;
  await state.saveSingle(id);
}

async function handleSaveAll() {
  if (savableIds.value.length === 0) return;
  await state.saveMany(savableIds.value);
}

function preloadDrawerImageThumbnails() {
  for (const att of props.attachments) {
    if (!isImageAttachment(att)) continue;
    const id = resolveNumericAttachmentId(att.id);
    if (id !== null && !state.previewCache.value[id]) {
      void state.fetchPreview(id);
    }
  }
}

function closeDrawer() {
  state.closePreview();
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
    if (state.activePreview.value) {
      state.closePreview();
      return;
    }
    closeDrawer();
  }
}

function attachGlobalKeyListener() {
  if (typeof window !== "undefined") {
    window.addEventListener("keydown", onKeyDown);
  }
}

function detachGlobalKeyListener() {
  if (typeof window !== "undefined") {
    window.removeEventListener("keydown", onKeyDown);
  }
}

watch(
  () => props.open,
  (isOpen, prevOpen) => {
    if (isOpen === prevOpen) return;
    reportOverlay(isOpen);
    if (isOpen) {
      attachGlobalKeyListener();
      preloadDrawerImageThumbnails();
    } else {
      detachGlobalKeyListener();
      state.closePreview();
      state.clearStatus();
    }
  }
);

watch(
  () => props.attachments,
  () => {
    if (props.open) {
      preloadDrawerImageThumbnails();
    }
  }
);

onMounted(() => {
  if (props.open) {
    reportOverlay(true);
    attachGlobalKeyListener();
    preloadDrawerImageThumbnails();
  }
});

onUnmounted(() => {
  detachGlobalKeyListener();
  reportOverlay(false);
});
</script>

<style scoped>
.km-drawer-backdrop {
  position: fixed;
  inset: 0;
  z-index: 85;
  background: rgba(0, 0, 0, 0.36);
  display: flex;
  justify-content: flex-end;
}

.km-attachment-drawer {
  width: min(460px, 100vw);
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--km-surface);
  color: var(--km-text);
  border-left: 1px solid var(--km-border);
  box-shadow: var(--km-shadow-popover);
  user-select: text;
}

.km-drawer-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 16px;
  border-bottom: 1px solid var(--km-border);
  background: var(--km-sidebar);
  flex-shrink: 0;
}

.km-drawer-title-wrap {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.km-drawer-title {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
}

.km-drawer-subtitle {
  margin: 0;
  font-size: 12px;
  color: var(--km-text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-drawer-meta {
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-drawer-header-actions {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

.km-drawer-banner {
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

.km-banner-success {
  background: var(--km-accent-soft);
  color: var(--km-accent);
}

.km-banner-close {
  border: none;
  background: transparent;
  color: inherit;
  cursor: pointer;
  padding: 2px 4px;
}

.km-drawer-body {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  padding: 12px 14px;
}

.km-empty-state {
  height: 100%;
  min-height: 180px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--km-text-muted);
}

.km-empty-icon {
  font-size: 28px;
}

.km-empty-text {
  margin: 0;
  font-size: 13px;
}

.km-attach-list {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.km-attach-card {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-md);
  background: var(--km-surface-raised);
  box-shadow: var(--km-shadow-card);
}

.km-attach-thumb {
  width: 48px;
  height: 48px;
  flex-shrink: 0;
  border-radius: var(--km-radius-sm);
  border: 1px solid var(--km-border);
  background: var(--km-canvas);
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
}

.km-attach-thumb.is-image {
  background: var(--km-accent-soft);
}

.km-attach-thumb-img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.km-attach-badge {
  font-family: var(--km-font-mono);
  font-size: 11px;
  font-weight: 700;
  color: var(--km-accent);
}

.km-attach-info {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.km-attach-name-row {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.km-attach-filename {
  font-size: 13px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-tag-pill {
  flex-shrink: 0;
  padding: 1px 6px;
  border-radius: 999px;
  background: var(--km-accent-soft);
  color: var(--km-accent);
  font-size: 10.5px;
  font-weight: 600;
}

.km-attach-sub {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 11.5px;
  color: var(--km-text-muted);
  min-width: 0;
}

.km-attach-mime {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-attach-actions {
  display: flex;
  flex-direction: column;
  gap: 5px;
  flex-shrink: 0;
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
  opacity: 0.92;
  background: var(--km-accent);
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

.km-lightbox-overlay {
  position: fixed;
  inset: 0;
  z-index: 95;
  background: rgba(10, 12, 16, 0.76);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
}

.km-lightbox-panel {
  width: min(860px, 100%);
  max-height: 90vh;
  display: flex;
  flex-direction: column;
  background: var(--km-surface);
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-lg);
  box-shadow: var(--km-shadow-popover);
  overflow: hidden;
}

.km-lightbox-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 14px;
  background: var(--km-sidebar);
  border-bottom: 1px solid var(--km-border);
}

.km-lightbox-meta {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.km-lightbox-title {
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-lightbox-sub {
  font-size: 11.5px;
  color: var(--km-text-muted);
}

.km-lightbox-actions {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

.km-lightbox-stage {
  flex: 1 1 auto;
  min-height: 240px;
  overflow: auto;
  padding: 18px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--km-canvas);
}

.km-lightbox-img {
  max-width: 100%;
  max-height: 70vh;
  border-radius: var(--km-radius-sm);
  object-fit: contain;
}

.km-lightbox-text {
  width: 100%;
  margin: 0;
  padding: 14px;
  border-radius: var(--km-radius-sm);
  border: 1px solid var(--km-border);
  background: var(--km-surface);
  font-family: var(--km-font-mono);
  font-size: 12.5px;
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-word;
}
</style>

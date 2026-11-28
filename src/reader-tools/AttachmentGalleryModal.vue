<template>
  <Teleport to="body" :disabled="embedded">
    <component
      :is="embedded ? 'section' : 'div'"
      v-if="open"
      :class="embedded ? 'km-gallery-embedded' : 'km-gallery-backdrop'"
      :role="embedded ? undefined : 'dialog'"
      :aria-modal="embedded ? undefined : 'true'"
      aria-label="全局附件图库"
      @pointerdown.self="!embedded && closeModal()"
    >
      <component
        :is="embedded ? 'div' : 'section'"
        :class="embedded ? 'km-gallery-embedded-frame' : 'km-gallery-modal'"
      >
        <!-- 顶部标题栏 -->
        <header
          class="km-gallery-header"
          :data-tauri-drag-region="embedded ? '' : undefined"
        >
          <div
            class="km-gallery-title-wrap"
            :data-tauri-drag-region="embedded ? '' : undefined"
          >
            <h2
              class="km-gallery-title"
              :data-tauri-drag-region="embedded ? '' : undefined"
            >
              全局附件图库
            </h2>
            <span
              class="km-gallery-count"
              :data-tauri-drag-region="embedded ? '' : undefined"
            >
              共 {{ state.total.value }} 项缓存附件
            </span>
          </div>

          <div class="km-gallery-header-actions">
            <button
              type="button"
              class="km-btn"
              :disabled="state.loading.value"
              @click="reloadGallery"
            >
              {{ state.loading.value ? "刷新中…" : "刷新" }}
            </button>
            <button
              v-if="embedded"
              type="button"
              class="km-btn"
              title="返回邮件视图"
              @click="closeModal"
            >
              返回邮件
            </button>
            <button
              v-else
              type="button"
              class="km-icon-btn"
              title="关闭 (Esc)"
              aria-label="关闭全局附件图库"
              @click="closeModal"
            >
              ✕
            </button>
            <button
              v-if="embedded"
              type="button"
              class="km-window-close-btn"
              title="关闭主窗口"
              aria-label="关闭窗口"
              @click="emit('closeWindow')"
            >
              <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
                <path
                  fill="currentColor"
                  d="m3.7 2.8 4.3 4.3 4.3-4.3.9.9L8.9 8l4.3 4.3-.9.9L8 8.9l-4.3 4.3-.9-.9L7.1 8 2.8 3.7l.9-.9Z"
                />
              </svg>
            </button>
          </div>
        </header>

        <!-- 搜索与类型切换栏 -->
        <div class="km-gallery-toolbar">
          <div class="km-search-box">
            <input
              ref="searchInputRef"
              v-model="searchDraft"
              type="search"
              class="km-search-input"
              placeholder="搜索附件名、发件人、主题或类型…"
              aria-label="搜索附件"
              @input="onSearchInput"
              @keydown.enter.prevent="applySearchNow"
            />
            <button
              v-if="searchDraft"
              type="button"
              class="km-search-clear"
              aria-label="清空搜索"
              @click="clearSearch"
            >
              ✕
            </button>
          </div>

          <div class="km-filter-tabs" role="tablist" aria-label="附件类型筛选">
            <button
              v-for="tab in FILTER_TABS"
              :key="tab.value"
              type="button"
              role="tab"
              class="km-tab-btn"
              :class="{ active: state.filter.value === tab.value }"
              :aria-selected="state.filter.value === tab.value"
              @click="state.setFilter(tab.value)"
            >
              {{ tab.label }} ({{ state.counts.value[tab.value] }})
            </button>
          </div>
        </div>

        <!-- 状态与错误提示条 -->
        <div
          v-if="state.error.value || state.previewError.value"
          class="km-gallery-banner km-banner-error"
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
          class="km-gallery-banner km-banner-success"
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

        <!-- 内容区 -->
        <div class="km-gallery-body">
          <div v-if="state.loading.value && state.items.value.length === 0" class="km-center-state">
            正在加载附件索引…
          </div>

          <div v-else-if="state.filteredItems.value.length === 0" class="km-center-state">
            <p class="km-empty-title">没有匹配的附件</p>
            <p class="km-empty-hint">
              {{ state.hasMore.value ? "当前页暂无匹配项，可点击下方「加载更多」或更换筛选条件。" : "请尝试调整搜索关键词或切换分类。" }}
            </p>
          </div>

          <div v-else class="km-gallery-grid">
            <article
              v-for="item in state.filteredItems.value"
              :key="item.id"
              class="km-gallery-card"
            >
              <button
                type="button"
                class="km-card-preview-box"
                :class="{ 'is-image': item.isImage }"
                :aria-label="`预览 ${item.filename}`"
                @click="openLightboxFor(item)"
              >
                <img
                  v-if="cardThumbnailUrl(item)"
                  :src="cardThumbnailUrl(item)!"
                  :alt="item.filename"
                  class="km-card-thumb"
                />
                <div v-else class="km-card-placeholder">
                  <span class="km-card-ext">{{ extensionLabel(item.filename) }}</span>
                  <span class="km-card-kind">{{ item.isImage ? "点击查看大图" : "文档预览" }}</span>
                </div>
                <span
                  class="km-cache-badge"
                  :class="item.downloaded ? 'is-cached' : 'is-remote'"
                >
                  {{ item.downloaded ? "已缓存" : "未下载" }}
                </span>
              </button>

              <div class="km-card-body">
                <h3 class="km-card-filename" :title="item.filename">
                  {{ item.filename }}
                </h3>
                <div class="km-card-meta">
                  <span>{{ formatAttachmentSize(item.sizeBytes) }}</span>
                  <span>·</span>
                  <span>{{ item.dateDisplay }}</span>
                </div>
                <div class="km-card-subject" :title="`${item.sender} · ${item.subject}`">
                  {{ item.subject }}
                </div>
                <div class="km-card-sender" :title="item.sender">
                  {{ item.sender }}
                </div>
              </div>

              <footer class="km-card-footer">
                <button
                  type="button"
                  class="km-btn km-btn-sm"
                  :disabled="state.savingId.value === item.id"
                  @click="state.saveSingle(item.id)"
                >
                  {{ state.savingId.value === item.id ? "保存中…" : "保存" }}
                </button>
                <button
                  type="button"
                  class="km-btn km-btn-sm"
                  @click="emitOpenMessage(item.accountId, item.messageId)"
                >
                  查看原邮件
                </button>
              </footer>
            </article>
          </div>
        </div>

        <!-- 分页底栏 -->
        <footer class="km-gallery-footer">
          <span class="km-footer-info">
            已加载 {{ state.items.value.length }} / {{ state.total.value }} 项
          </span>
          <button
            v-if="state.hasMore.value"
            type="button"
            class="km-btn km-btn-primary"
            :disabled="state.loadingMore.value"
            @click="loadMoreItems"
          >
            {{ state.loadingMore.value ? "加载中…" : "加载更多" }}
          </button>
        </footer>
      </component>

      <!-- 内置灯箱大图预览：embedded 模式下保留 Teleport 到 body 覆盖全窗预览体验 -->
      <Teleport to="body" :disabled="!embedded">
        <div
          v-if="state.activePreview.value"
          class="km-lightbox-overlay"
          role="dialog"
          aria-modal="true"
          :aria-label="state.activePreview.value.filename"
          @pointerdown.self="closeLightbox"
        >
          <div class="km-lightbox-panel">
            <header class="km-lightbox-header">
              <div class="km-lightbox-meta">
                <strong class="km-lightbox-title">{{ state.activePreview.value.filename }}</strong>
                <span class="km-lightbox-sub">
                  {{ state.activePreview.value.mimeType }} ·
                  {{ formatAttachmentSize(state.activePreview.value.sizeBytes) }}
                  <template v-if="activeItem">
                    · {{ activeItem.sender }} · {{ activeItem.subject }}
                  </template>
                </span>
              </div>

              <div class="km-lightbox-actions">
                <button
                  v-if="isLightboxImage"
                  type="button"
                  class="km-btn"
                  @click="zoomPercent = Math.max(50, zoomPercent - 25)"
                >
                  -
                </button>
                <span v-if="isLightboxImage" class="km-zoom-label">{{ zoomPercent }}%</span>
                <button
                  v-if="isLightboxImage"
                  type="button"
                  class="km-btn"
                  @click="zoomPercent = Math.min(250, zoomPercent + 25)"
                >
                  +
                </button>
                <button
                  v-if="activeItem"
                  type="button"
                  class="km-btn"
                  @click="emitOpenMessage(activeItem.accountId, activeItem.messageId)"
                >
                  查看原邮件
                </button>
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
                  aria-label="关闭灯箱"
                  @click="closeLightbox"
                >
                  ✕
                </button>
              </div>
            </header>

            <div class="km-lightbox-stage">
              <button
                v-if="previewableIds.length > 1"
                type="button"
                class="km-nav-btn km-nav-prev"
                aria-label="上一个附件"
                @click="stepPreview(-1)"
              >
                ‹
              </button>

              <img
                v-if="isLightboxImage"
                :src="state.activePreview.value.dataUrl"
                :alt="state.activePreview.value.filename"
                class="km-lightbox-img"
                :style="{ transform: `scale(${zoomPercent / 100})` }"
              />
              <iframe
                v-else-if="isLightboxPdf"
                :src="state.activePreview.value.dataUrl"
                :title="state.activePreview.value.filename"
                class="km-lightbox-pdf"
              />
              <pre v-else class="km-lightbox-text">{{
                decodeTextPreviewDataUrl(state.activePreview.value.dataUrl) ?? "该文件类型暂不支持内嵌预览，请点击上方保存后查看。"
              }}</pre>

              <button
                v-if="previewableIds.length > 1"
                type="button"
                class="km-nav-btn km-nav-next"
                aria-label="下一个附件"
                @click="stepPreview(1)"
              >
                ›
              </button>
            </div>
          </div>
        </div>
      </Teleport>
    </component>
  </Teleport>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import {
  decodeTextPreviewDataUrl,
  formatAttachmentSize,
  isPdfAttachment,
  resolveDrawerThumbUrl,
  useAttachments,
} from "./useAttachments";
import type { AttachmentFilterKind, CachedAttachmentDto } from "./types";

const OVERLAY_ID = "attachment-gallery";

const FILTER_TABS: ReadonlyArray<{ value: AttachmentFilterKind; label: string }> = [
  { value: "all", label: "全部" },
  { value: "images", label: "图片" },
  { value: "documents", label: "文档" },
];

const props = withDefaults(
  defineProps<{
    open: boolean;
    embedded?: boolean;
  }>(),
  {
    embedded: false,
  }
);

const emit = defineEmits<{
  "update:open": [open: boolean];
  close: [];
  openMessage: [accountId: number, messageId: number];
  overlayChange: [id: string, open: boolean];
  closeWindow: [];
}>();

const state = useAttachments({ pageSize: 12 });
const searchInputRef = ref<HTMLInputElement | null>(null);
const searchDraft = ref("");
const zoomPercent = ref(100);
let searchTimer: ReturnType<typeof setTimeout> | null = null;

let overlayReportedOpen = false;
function reportOverlay(next: boolean) {
  if (props.embedded) {
    if (overlayReportedOpen) {
      overlayReportedOpen = false;
      emit("overlayChange", OVERLAY_ID, false);
    }
    return;
  }
  if (overlayReportedOpen === next) return;
  overlayReportedOpen = next;
  emit("overlayChange", OVERLAY_ID, next);
}

const activeItem = computed<CachedAttachmentDto | null>(() => {
  const id = state.activePreview.value?.attachmentId;
  if (id === undefined) return null;
  return state.items.value.find((item) => item.id === id) ?? null;
});

const isLightboxImage = computed(() => {
  const pv = state.activePreview.value;
  if (!pv) return false;
  return pv.mimeType.toLowerCase().startsWith("image/") || pv.dataUrl.startsWith("data:image/");
});

const isLightboxPdf = computed(() => {
  const pv = state.activePreview.value;
  if (!pv) return false;
  return (
    isPdfAttachment({ mimeType: pv.mimeType, filename: pv.filename }) ||
    pv.dataUrl.startsWith("data:application/pdf")
  );
});

function cardThumbnailUrl(item: CachedAttachmentDto): string | null {
  return resolveDrawerThumbUrl(item, state.thumbUrls.value);
}

const previewableIds = computed(() => state.filteredItems.value.map((item) => item.id));

function extensionLabel(filename: string): string {
  const dotIdx = filename.lastIndexOf(".");
  if (dotIdx >= 0 && dotIdx < filename.length - 1) {
    return filename.slice(dotIdx + 1, dotIdx + 5).toUpperCase();
  }
  return "FILE";
}

function prefetchVisibleImageThumbnails() {
  // 仅调度当前页可见项，缩略图并发与缓存上限由 composable 控制
  void state.prefetchThumbnails(
    state.filteredItems.value.map((item) => ({
      id: item.id,
      isImage: item.isImage,
      isPdf: isPdfAttachment(item),
    }))
  );
}

async function reloadGallery() {
  await state.loadGallery(true);
  prefetchVisibleImageThumbnails();
}

async function loadMoreItems() {
  await state.loadMore();
  prefetchVisibleImageThumbnails();
}

function onSearchInput() {
  if (searchTimer !== null) clearTimeout(searchTimer);
  searchTimer = setTimeout(() => {
    searchTimer = null;
    void applySearchNow();
  }, 180);
}

async function applySearchNow() {
  if (searchTimer !== null) {
    clearTimeout(searchTimer);
    searchTimer = null;
  }
  await state.setQuery(searchDraft.value);
  prefetchVisibleImageThumbnails();
}

async function clearSearch() {
  searchDraft.value = "";
  await applySearchNow();
}

async function openLightboxFor(item: CachedAttachmentDto) {
  zoomPercent.value = 100;
  await state.openPreview(item.id);
}

function closeLightbox() {
  zoomPercent.value = 100;
  state.closePreview();
}

async function stepPreview(delta: number) {
  const ids = previewableIds.value;
  const currentId = state.activePreview.value?.attachmentId;
  if (ids.length <= 1 || currentId === undefined) return;
  const idx = ids.indexOf(currentId);
  const nextIdx = idx < 0 ? 0 : (idx + delta + ids.length) % ids.length;
  zoomPercent.value = 100;
  await state.openPreview(ids[nextIdx]);
}

function emitOpenMessage(accountId: number, messageId: number) {
  emit("openMessage", accountId, messageId);
}

function closeModal() {
  if (searchTimer !== null) {
    clearTimeout(searchTimer);
    searchTimer = null;
  }
  closeLightbox();
  reportOverlay(false);
  emit("update:open", false);
  emit("close");
}

function onKeyDown(e: KeyboardEvent) {
  if (!props.open || e.defaultPrevented) return;
  if (e.isComposing || e.key === "Process" || e.keyCode === 229) return;
  if (e.key === "Escape") {
    if (state.activePreview.value) {
      e.preventDefault();
      e.stopPropagation();
      closeLightbox();
      return;
    }
    if (props.embedded) {
      return;
    }
    e.preventDefault();
    e.stopPropagation();
    closeModal();
    return;
  }
  if (state.activePreview.value) {
    if (e.key === "ArrowLeft") {
      e.preventDefault();
      void stepPreview(-1);
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      void stepPreview(1);
    }
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
  () => state.filter.value,
  () => {
    if (props.open) {
      prefetchVisibleImageThumbnails();
    }
  }
);

watch(
  () => props.open,
  (isOpen, prevOpen) => {
    if (isOpen === prevOpen) return;
    reportOverlay(isOpen);
    if (isOpen) {
      attachGlobalListeners();
      void reloadGallery();
      void nextTick(() => searchInputRef.value?.focus());
    } else {
      detachGlobalListeners();
      closeLightbox();
      state.invalidateThumbnails();
      state.clearStatus();
    }
  }
);

watch(
  () => props.embedded,
  () => {
    reportOverlay(props.open);
  }
);

onMounted(() => {
  if (props.open) {
    reportOverlay(true);
    attachGlobalListeners();
    void reloadGallery();
    void nextTick(() => searchInputRef.value?.focus());
  }
});

onUnmounted(() => {
  if (searchTimer !== null) {
    clearTimeout(searchTimer);
    searchTimer = null;
  }
  detachGlobalListeners();
  state.invalidateThumbnails();
  state.clearThumbnails();
  reportOverlay(false);
});
</script>

<style scoped>
.km-gallery-embedded {
  flex: 1 1 auto;
  min-width: 0;
  height: 100%;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--km-surface);
  color: var(--km-text);
  user-select: text;
}

.km-gallery-embedded-frame {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.km-gallery-backdrop {
  position: fixed;
  inset: 0;
  z-index: 84;
  background: rgba(0, 0, 0, 0.42);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
}

.km-gallery-modal {
  width: min(980px, 100%);
  height: min(720px, 92vh);
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

.km-gallery-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 16px;
  background: var(--km-sidebar);
  border-bottom: 1px solid var(--km-border);
  flex-shrink: 0;
}

.km-gallery-embedded .km-gallery-header {
  height: 48px;
  min-height: 48px;
  padding: 0 10px 0 16px;
  background: var(--km-surface);
}

.km-gallery-embedded .km-gallery-title-wrap {
  min-width: 0;
  white-space: nowrap;
}

.km-gallery-embedded .km-gallery-header-actions {
  flex-wrap: nowrap;
}

.km-gallery-title-wrap {
  display: flex;
  align-items: baseline;
  gap: 10px;
}

.km-gallery-title {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
}

.km-gallery-count {
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-gallery-header-actions {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}

.km-gallery-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 10px;
  padding: 10px 16px;
  background: var(--km-canvas);
  border-bottom: 1px solid var(--km-border);
  flex-shrink: 0;
}

.km-search-box {
  position: relative;
  flex: 1 1 280px;
  max-width: 440px;
  display: flex;
  align-items: center;
}

.km-search-input {
  width: 100%;
  height: 30px;
  padding: 0 28px 0 10px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface);
  color: var(--km-text);
  font-size: 12.5px;
}

.km-search-clear {
  position: absolute;
  right: 6px;
  border: none;
  background: transparent;
  color: var(--km-text-muted);
  cursor: pointer;
  font-size: 11px;
  padding: 2px 4px;
}

.km-filter-tabs {
  display: inline-flex;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  overflow: hidden;
  background: var(--km-surface);
}

.km-tab-btn {
  height: 28px;
  padding: 0 12px;
  border: none;
  background: transparent;
  color: var(--km-text-muted);
  font-size: 12px;
  cursor: pointer;
}

.km-tab-btn + .km-tab-btn {
  border-left: 1px solid var(--km-border);
}

.km-tab-btn.active {
  background: var(--km-accent-soft);
  color: var(--km-accent);
  font-weight: 600;
}

.km-gallery-banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 8px 16px;
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
}

.km-gallery-body {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  padding: 16px;
}

.km-center-state {
  min-height: 220px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  color: var(--km-text-muted);
  text-align: center;
}

.km-empty-title {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
  color: var(--km-text);
}

.km-empty-hint {
  margin: 0;
  font-size: 12px;
}

.km-gallery-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(210px, 1fr));
  gap: 12px;
}

.km-gallery-card {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-md);
  background: var(--km-surface-raised);
  box-shadow: var(--km-shadow-card);
  overflow: hidden;
}

.km-card-preview-box {
  position: relative;
  width: 100%;
  height: 126px;
  border: none;
  border-bottom: 1px solid var(--km-border);
  padding: 0;
  background: var(--km-canvas);
  cursor: pointer;
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
}

.km-card-preview-box.is-image {
  background: var(--km-accent-soft);
}

.km-card-thumb {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.km-card-placeholder {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
}

.km-card-ext {
  font-family: var(--km-font-mono);
  font-size: 15px;
  font-weight: 700;
  color: var(--km-accent);
}

.km-card-kind {
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-cache-badge {
  position: absolute;
  top: 8px;
  right: 8px;
  padding: 2px 6px;
  border-radius: 999px;
  font-size: 10.5px;
  font-weight: 600;
  background: rgba(0, 0, 0, 0.56);
  color: #ffffff;
}

.km-cache-badge.is-cached {
  background: color-mix(in srgb, var(--km-accent) 88%, #000000);
}

.km-card-body {
  flex: 1 1 auto;
  padding: 10px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.km-card-filename {
  margin: 0;
  font-size: 12.5px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-card-meta {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-card-subject,
.km-card-sender {
  font-size: 11.5px;
  color: var(--km-text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-card-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
  padding: 8px 10px;
  border-top: 1px solid color-mix(in srgb, var(--km-border) 60%, transparent);
  background: var(--km-canvas);
}

.km-gallery-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 16px;
  background: var(--km-sidebar);
  border-top: 1px solid var(--km-border);
  flex-shrink: 0;
}

.km-footer-info {
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-btn {
  height: 28px;
  padding: 0 11px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface-raised);
  color: var(--km-text);
  font-size: 12px;
  cursor: pointer;
  white-space: nowrap;
}

.km-btn-sm {
  height: 24px;
  padding: 0 8px;
  font-size: 11.5px;
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

.km-window-close-btn {
  width: 32px;
  height: 32px;
  border: none;
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-text-muted);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  padding: 0;
  flex-shrink: 0;
}

.km-window-close-btn:hover {
  background: var(--km-danger-soft);
  color: var(--km-danger);
}

.km-lightbox-overlay {
  position: fixed;
  inset: 0;
  z-index: 95;
  background: rgba(8, 10, 14, 0.82);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
}

.km-lightbox-panel {
  width: min(960px, 100%);
  height: min(680px, 92vh);
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
  flex-shrink: 0;
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
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-lightbox-actions {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

.km-zoom-label {
  min-width: 40px;
  text-align: center;
  font-size: 11.5px;
  color: var(--km-text-muted);
}

.km-lightbox-stage {
  position: relative;
  flex: 1 1 auto;
  min-height: 0;
  overflow: auto;
  padding: 20px;
  background: var(--km-canvas);
  display: flex;
  align-items: center;
  justify-content: center;
}

.km-lightbox-img {
  max-width: 100%;
  max-height: 100%;
  object-fit: contain;
  border-radius: var(--km-radius-sm);
  transition: transform 0.14s ease;
}

.km-lightbox-pdf {
  width: 100%;
  height: 100%;
  min-height: 360px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: #ffffff;
}

.km-lightbox-text {
  width: 100%;
  max-height: 100%;
  overflow: auto;
  margin: 0;
  padding: 16px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface);
  font-family: var(--km-font-mono);
  font-size: 12.5px;
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-word;
}

.km-nav-btn {
  position: absolute;
  top: 50%;
  transform: translateY(-50%);
  z-index: 2;
  width: 34px;
  height: 44px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface-raised);
  color: var(--km-text);
  font-size: 20px;
  cursor: pointer;
}

.km-nav-prev {
  left: 12px;
}

.km-nav-next {
  right: 12px;
}
</style>

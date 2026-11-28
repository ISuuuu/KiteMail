<template>
  <section
    ref="drawerRootRef"
    class="km-inline-drawer"
    :class="{ expanded }"
    :style="drawerContainerStyle"
    aria-label="邮件附件抽屉"
    @keydown="onKeyDown"
  >
    <!-- 顶部居中拖拽/收合把手 -->
    <div
      class="km-drawer-handle"
      role="separator"
      aria-orientation="horizontal"
      aria-label="拖动调整附件抽屉高度，点击收合或展开"
      :aria-valuenow="drawerHeight"
      aria-valuemin="110"
      aria-valuemax="420"
      tabindex="0"
      @pointerdown="onHandlePointerDown"
      @pointermove="onHandlePointerMove"
      @pointerup="onHandlePointerUp"
      @pointercancel="onHandlePointerUp"
      @keydown.enter.prevent="toggleExpanded"
      @keydown.space.prevent="toggleExpanded"
    >
      <span class="km-drawer-handle-pill" />
    </div>

    <!-- 抽屉标题与工具栏 -->
    <header class="km-drawer-header">
      <button
        type="button"
        class="km-drawer-summary-btn"
        :aria-expanded="expanded"
        :title="expanded ? '收合附件抽屉' : '展开附件抽屉'"
        @click="toggleExpanded"
      >
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
          <path
            fill="currentColor"
            d="M11.6 3.4a2.8 2.8 0 0 1 0 3.96L6.45 12.5a1.85 1.85 0 0 1-2.62-2.62l4.8-4.8a.9.9 0 0 1 1.28 1.28l-4.8 4.8a.55.55 0 0 0 .78.78l5.15-5.14a1.5 1.5 0 1 0-2.12-2.12L3.8 9.8a2.45 2.45 0 1 0 3.46 3.46l4.02-4.02a.65.65 0 1 1 .92.92l-4.02 4.02A3.75 3.75 0 1 1 2.88 8.9l5.12-5.12a2.8 2.8 0 0 1 3.6-.38Z"
          />
        </svg>
        <span class="km-drawer-title">附件 ({{ aggregatedItems.length }})</span>
        <span class="km-drawer-meta">{{ formatAttachmentSize(totalBytes) }}</span>
        <svg
          class="km-drawer-chevron"
          :class="{ collapsed: !expanded }"
          viewBox="0 0 16 16"
          width="12"
          height="12"
          aria-hidden="true"
        >
          <path fill="currentColor" d="M4.2 5.8 8 9.6l3.8-3.8 1.06 1.06L8 11.72 3.14 6.86Z" />
        </svg>
      </button>

      <div class="km-drawer-toolbar">
        <label
          v-if="expanded && viewMode === 'grid' && showSizeSlider"
          class="km-thumb-slider-wrap"
          title="调整网格缩略图大小"
        >
          <span class="sr-only">缩略图大小</span>
          <input
            v-model.number="thumbSize"
            type="range"
            min="40"
            max="112"
            step="4"
            class="km-thumb-slider"
            aria-label="缩略图大小"
            @change="persistDrawerPrefs"
          />
        </label>

        <button
          v-if="expanded && viewMode === 'list'"
          type="button"
          class="km-drawer-icon-btn"
          :title="sortOrder === 'asc' ? '文件名正序（点击切换倒序）' : '文件名倒序（点击切换正序）'"
          :aria-label="sortOrder === 'asc' ? '切换文件名倒序' : '切换文件名正序'"
          @click="toggleSortOrder"
        >
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
            <path
              v-if="sortOrder === 'asc'"
              fill="currentColor"
              d="M4.5 2.5v9.2L2.4 9.6l-.9.9L5 14l3.5-3.5-.9-.9-2.1 2.1V2.5h-1Zm4.5 1h5v1.3H9V3.5Zm0 3.7h4v1.3H9V7.2Zm0 3.7h3v1.3H9v-1.3Z"
            />
            <path
              v-else
              fill="currentColor"
              d="M4.5 13.5V4.3L2.4 6.4l-.9-.9L5 2l3.5 3.5-.9.9-2.1-2.1v9.2h-1Zm4.5-10h5v1.3H9V3.5Zm0 3.7h4v1.3H9V7.2Zm0 3.7h3v1.3H9v-1.3Z"
            />
          </svg>
        </button>

        <button
          v-if="expanded"
          type="button"
          class="km-drawer-icon-btn"
          :title="viewMode === 'grid' ? '切换为文件列表' : '切换为缩略图网格'"
          :aria-label="viewMode === 'grid' ? '切换为文件列表' : '切换为缩略图网格'"
          @click="toggleViewMode"
        >
          <svg v-if="viewMode === 'grid'" viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
            <path
              fill="currentColor"
              d="M2.5 3.5h11v1.5h-11V3.5Zm0 3.75h11v1.5h-11v-1.5Zm0 3.75h11v1.5h-11V11Z"
            />
          </svg>
          <svg v-else viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
            <path
              fill="currentColor"
              d="M2.5 2.5h4.8v4.8H2.5V2.5Zm6.2 0h4.8v4.8H8.7V2.5ZM2.5 8.7h4.8v4.8H2.5V8.7Zm6.2 0h4.8v4.8H8.7V8.7Z"
            />
          </svg>
        </button>

        <button
          v-if="savableIds.length > 0"
          type="button"
          class="km-btn km-btn-save-all"
          :disabled="state.savingAll.value"
          @click="saveAllAttachmentsPicker"
        >
          {{ state.savingAll.value ? "保存中…" : "全部保存" }}
        </button>
      </div>
    </header>

    <!-- 状态提示条 -->
    <div
      v-if="expanded && (state.error.value || state.previewError.value)"
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
      v-else-if="expanded && state.statusMessage.value"
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

    <!-- 展开内容区（网格 / 列表） -->
    <div v-if="expanded" class="km-drawer-body">
      <!-- 网格模式 -->
      <div v-if="viewMode === 'grid'" class="km-attach-grid">
        <div
          v-for="item in displayedItems"
          :key="item.id"
          class="km-grid-card"
          :style="{ width: `${Math.max(thumbSize + 36, 92)}px` }"
          :title="`${item.filename} (${formatAttachmentSize(item.sizeBytes)}) · 双击打开`"
          tabindex="0"
          @dblclick="handlePrimaryOpen(item)"
          @keydown.enter.prevent="handlePrimaryOpen(item)"
        >
          <div
            class="km-grid-thumb"
            :class="{ 'is-image': item.isImage, 'is-pdf': item.isPdf }"
            :style="{ width: `${thumbSize}px`, height: `${thumbSize}px` }"
          >
            <img
              v-if="item.isImage && cachedDataUrl(item.id)"
              :src="cachedDataUrl(item.id)!"
              :alt="item.filename"
              class="km-attach-thumb-img"
            />
            <div v-else-if="item.isPdf" class="km-pdf-thumb">
              <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true">
                <path
                  fill="currentColor"
                  d="M6 2h8l5 5v13a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2Zm7 1.5V8h4.5L13 3.5Z"
                />
              </svg>
              <span>PDF</span>
            </div>
            <span v-else class="km-attach-badge">{{ badgeTextFor(item) }}</span>

            <!-- 悬停快捷操作 -->
            <div class="km-grid-hover-actions">
              <button
                type="button"
                class="km-hover-mini-btn"
                :title="item.isImage || item.isPdf ? '预览' : '用系统默认应用打开'"
                :aria-label="`打开 ${item.filename}`"
                @click.stop="handlePrimaryOpen(item)"
              >
                <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
                  <path
                    fill="currentColor"
                    d="M8 3.2c3.2 0 5.9 2.04 6.9 4.8-1 2.76-3.7 4.8-6.9 4.8S2.1 10.76 1.1 8c1-2.76 3.7-4.8 6.9-4.8Zm0 1.3a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7Zm0 1.6a1.9 1.9 0 1 1 0 3.8 1.9 1.9 0 0 1 0-3.8Z"
                  />
                </svg>
              </button>
              <button
                type="button"
                class="km-hover-mini-btn"
                title="保存此附件"
                :aria-label="`保存 ${item.filename}`"
                :disabled="state.savingId.value === item.id"
                @click.stop="saveAttachmentById(item.id)"
              >
                <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
                  <path
                    fill="currentColor"
                    d="M8.65 2v7.2l2.25-2.25.92.92L8 11.7 4.18 7.87l.92-.92L7.35 9.2V2h1.3ZM2.5 12.7h11V14h-11v-1.3Z"
                  />
                </svg>
              </button>
            </div>
          </div>

          <span class="km-grid-name" :title="item.filename">{{ item.filename }}</span>
          <span class="km-grid-size">{{ formatAttachmentSize(item.sizeBytes) }}</span>
        </div>
      </div>

      <!-- 列表模式 -->
      <ul v-else class="km-attach-list">
        <li
          v-for="item in displayedItems"
          :key="item.id"
          class="km-list-row"
          tabindex="0"
          @dblclick="handlePrimaryOpen(item)"
          @keydown.enter.prevent="handlePrimaryOpen(item)"
        >
          <span class="km-list-ext" :class="{ 'is-pdf': item.isPdf, 'is-image': item.isImage }">
            {{ badgeTextFor(item) }}
          </span>
          <span class="km-list-filename" :title="item.filename">{{ item.filename }}</span>
          <span class="km-list-size">{{ formatAttachmentSize(item.sizeBytes) }}</span>
          <div class="km-list-actions">
            <button
              type="button"
              class="km-btn-xs"
              @click.stop="handlePrimaryOpen(item)"
            >
              {{ item.isImage || item.isPdf ? "预览" : "打开" }}
            </button>
            <button
              type="button"
              class="km-btn-xs"
              :disabled="state.savingId.value === item.id"
              @click.stop="saveAttachmentById(item.id)"
            >
              {{ state.savingId.value === item.id ? "保存中…" : "下载" }}
            </button>
          </div>
        </li>
      </ul>
    </div>

    <!-- 仅灯箱预览浮层使用 Teleport 并上报遮挡 -->
    <Teleport to="body">
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
            <iframe
              v-else-if="isPreviewPdf(state.activePreview.value.mimeType, state.activePreview.value.filename, state.activePreview.value.dataUrl)"
              :src="state.activePreview.value.dataUrl"
              :title="state.activePreview.value.filename"
              class="km-lightbox-pdf"
            />
            <pre v-else class="km-lightbox-text">{{
              decodeTextPreviewDataUrl(state.activePreview.value.dataUrl) ?? "该文件类型暂不支持内嵌文本预览，请点击右上角保存后查看。"
            }}</pre>
          </div>
        </div>
      </div>
    </Teleport>
  </section>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import {
  aggregateThreadAttachments,
  decodeTextPreviewDataUrl,
  formatAttachmentSize,
  isPdfAttachment,
  sortDrawerAttachments,
  useAttachments,
} from "./useAttachments";
import type { AttachmentDto, DrawerAttachmentItem } from "./types";
import type { ThreadMessageDto } from "../ipc/types";

const LIGHTBOX_OVERLAY_ID = "attachment-lightbox";
const STORAGE_KEY = "kitemail.attachmentDrawer.v1";
const MIN_DRAWER_HEIGHT = 110;
const MAX_DRAWER_HEIGHT = 420;
const DEFAULT_DRAWER_HEIGHT = 160;
const DEFAULT_THUMB_SIZE = 56;

const props = withDefaults(
  defineProps<{
    open?: boolean;
    attachments: AttachmentDto[];
    threadMessages?: ThreadMessageDto[];
    subject?: string;
  }>(),
  {
    open: true,
    threadMessages: () => [],
  }
);

const emit = defineEmits<{
  "update:open": [open: boolean];
  close: [];
  overlayChange: [id: string, open: boolean];
}>();

const state = useAttachments();
const drawerRootRef = ref<HTMLElement | null>(null);
const drawerWidth = ref(673);

interface DrawerPrefs {
  expanded: boolean;
  height: number;
  viewMode: "grid" | "list";
  sortOrder: "asc" | "desc";
  thumbSize: number;
}

function loadDrawerPrefs(): DrawerPrefs {
  const defaults: DrawerPrefs = {
    expanded: true,
    height: DEFAULT_DRAWER_HEIGHT,
    viewMode: "grid",
    sortOrder: "asc",
    thumbSize: DEFAULT_THUMB_SIZE,
  };
  if (typeof localStorage === "undefined") return defaults;
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return defaults;
    const parsed = JSON.parse(raw) as Partial<DrawerPrefs>;
    return {
      expanded: typeof parsed.expanded === "boolean" ? parsed.expanded : defaults.expanded,
      height:
        typeof parsed.height === "number" && Number.isFinite(parsed.height)
          ? Math.min(MAX_DRAWER_HEIGHT, Math.max(MIN_DRAWER_HEIGHT, Math.round(parsed.height)))
          : defaults.height,
      viewMode: parsed.viewMode === "list" ? "list" : "grid",
      sortOrder: parsed.sortOrder === "desc" ? "desc" : "asc",
      thumbSize:
        typeof parsed.thumbSize === "number" && Number.isFinite(parsed.thumbSize)
          ? Math.min(112, Math.max(40, Math.round(parsed.thumbSize)))
          : defaults.thumbSize,
    };
  } catch {
    return defaults;
  }
}

const initialPrefs = loadDrawerPrefs();
const expanded = ref(initialPrefs.expanded);
const drawerHeight = ref(initialPrefs.height);
const viewMode = ref<"grid" | "list">(initialPrefs.viewMode);
const sortOrder = ref<"asc" | "desc">(initialPrefs.sortOrder);
const thumbSize = ref(initialPrefs.thumbSize);

function persistDrawerPrefs() {
  if (typeof localStorage === "undefined") return;
  try {
    const payload: DrawerPrefs = {
      expanded: expanded.value,
      height: drawerHeight.value,
      viewMode: viewMode.value,
      sortOrder: sortOrder.value,
      thumbSize: thumbSize.value,
    };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(payload));
  } catch {
    /* 忽略存储受限 */
  }
}

const aggregatedItems = computed<DrawerAttachmentItem[]>(() =>
  aggregateThreadAttachments(props.attachments, props.threadMessages)
);

const displayedItems = computed<DrawerAttachmentItem[]>(() => {
  if (viewMode.value === "list") {
    return sortDrawerAttachments(aggregatedItems.value, sortOrder.value);
  }
  return aggregatedItems.value;
});

const totalBytes = computed(() =>
  aggregatedItems.value.reduce(
    (sum, item) => sum + (Number.isFinite(item.sizeBytes) ? item.sizeBytes : 0),
    0
  )
);

const savableIds = computed(() => aggregatedItems.value.map((item) => item.id));
const showSizeSlider = computed(() => drawerWidth.value >= 420);

const drawerContainerStyle = computed(() => {
  if (!expanded.value) return {};
  return {
    height: `${drawerHeight.value}px`,
  };
});

let lightboxReported = false;
function reportLightboxOverlay(next: boolean) {
  if (lightboxReported === next) return;
  lightboxReported = next;
  emit("overlayChange", LIGHTBOX_OVERLAY_ID, next);
}

watch(
  () => Boolean(state.activePreview.value),
  (isOpen) => {
    reportLightboxOverlay(isOpen);
  }
);

function cachedDataUrl(id: number): string | null {
  return state.previewCache.value[id]?.dataUrl ?? null;
}

function badgeTextFor(att: DrawerAttachmentItem): string {
  const dotIdx = att.filename.lastIndexOf(".");
  if (dotIdx >= 0 && dotIdx < att.filename.length - 1) {
    return att.filename.slice(dotIdx + 1, dotIdx + 5).toUpperCase();
  }
  if (att.isPdf) return "PDF";
  return att.isImage ? "IMG" : "FILE";
}

function isPreviewImage(mimeType: string, dataUrl: string): boolean {
  return mimeType.toLowerCase().startsWith("image/") || dataUrl.startsWith("data:image/");
}

function isPreviewPdf(mimeType: string, filename: string, dataUrl: string): boolean {
  return (
    isPdfAttachment({ mimeType, filename }) || dataUrl.startsWith("data:application/pdf")
  );
}

async function openAttachmentById(id: number) {
  const item = aggregatedItems.value.find((it) => it.id === id);
  if (item && !item.isImage && !item.isPdf) {
    await state.openWithDefaultApp(id);
    return;
  }
  await state.openPreview(id);
}

async function handlePrimaryOpen(item: DrawerAttachmentItem) {
  if (item.isImage || item.isPdf) {
    await state.openPreview(item.id);
  } else {
    await state.openWithDefaultApp(item.id);
  }
}

async function saveAttachmentById(id: number) {
  await state.saveSingle(id);
}

async function saveAllAttachmentsPicker() {
  if (savableIds.value.length === 0) return;
  await state.saveManyWithPicker(savableIds.value);
}

function preloadDrawerImageThumbnails() {
  for (const item of aggregatedItems.value) {
    if (!item.isImage) continue;
    if (!state.previewCache.value[item.id]) {
      void state.fetchPreview(item.id);
    }
  }
}

function toggleExpanded() {
  expanded.value = !expanded.value;
  emit("update:open", expanded.value);
  if (!expanded.value) emit("close");
  persistDrawerPrefs();
}

function expandDrawer() {
  if (!expanded.value) {
    expanded.value = true;
    emit("update:open", true);
    persistDrawerPrefs();
  }
}

function toggleViewMode() {
  viewMode.value = viewMode.value === "grid" ? "list" : "grid";
  persistDrawerPrefs();
}

function toggleSortOrder() {
  sortOrder.value = sortOrder.value === "asc" ? "desc" : "asc";
  persistDrawerPrefs();
}

// --- 把手拖拽调整高度 / 点击切换收合 ---
let draggingHandle = false;
let dragMoved = false;
let dragStartY = 0;
let dragStartHeight = DEFAULT_DRAWER_HEIGHT;
let activePointerId: number | null = null;

function onHandlePointerDown(e: PointerEvent) {
  if (e.button !== 0) return;
  e.preventDefault();
  draggingHandle = true;
  dragMoved = false;
  activePointerId = e.pointerId;
  dragStartY = e.clientY;
  dragStartHeight = expanded.value ? drawerHeight.value : MIN_DRAWER_HEIGHT;
  (e.currentTarget as HTMLElement | null)?.setPointerCapture(e.pointerId);
}

function onHandlePointerMove(e: PointerEvent) {
  if (!draggingHandle || e.pointerId !== activePointerId) return;
  const deltaY = dragStartY - e.clientY;
  if (Math.abs(deltaY) >= 4) {
    dragMoved = true;
    if (!expanded.value) {
      expanded.value = true;
    }
    drawerHeight.value = Math.min(
      MAX_DRAWER_HEIGHT,
      Math.max(MIN_DRAWER_HEIGHT, Math.round(dragStartHeight + deltaY))
    );
  }
}

function onHandlePointerUp(e: PointerEvent) {
  if (!draggingHandle || e.pointerId !== activePointerId) return;
  const el = e.currentTarget as HTMLElement | null;
  if (el?.hasPointerCapture?.(e.pointerId)) {
    el.releasePointerCapture(e.pointerId);
  }
  draggingHandle = false;
  activePointerId = null;
  if (!dragMoved) {
    toggleExpanded();
  } else {
    persistDrawerPrefs();
  }
}

function onKeyDown(e: KeyboardEvent) {
  if (e.isComposing || e.key === "Process" || e.keyCode === 229) return;
  if (e.key === "Escape" && state.activePreview.value) {
    e.preventDefault();
    e.stopPropagation();
    state.closePreview();
  }
}

watch(aggregatedItems, () => {
  if (expanded.value) {
    preloadDrawerImageThumbnails();
  }
});

watch(
  () => props.open,
  (nextOpen) => {
    if (nextOpen) {
      preloadDrawerImageThumbnails();
    }
  }
);

let ro: ResizeObserver | null = null;

onMounted(() => {
  if (drawerRootRef.value && typeof ResizeObserver !== "undefined") {
    drawerWidth.value = drawerRootRef.value.getBoundingClientRect().width || 673;
    ro = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width;
      if (w && w > 0) drawerWidth.value = w;
    });
    ro.observe(drawerRootRef.value);
  }
  if (expanded.value) {
    preloadDrawerImageThumbnails();
  }
});

onUnmounted(() => {
  ro?.disconnect();
  reportLightboxOverlay(false);
});

defineExpose({
  openAttachmentById,
  saveAttachmentById,
  saveAllAttachmentsPicker,
  expandDrawer,
});
</script>

<style scoped>
.km-inline-drawer {
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  background: var(--km-drawer-bg, #f4f4f5);
  color: var(--km-text);
  border-top: 1px solid var(--km-border);
  overflow: hidden;
  user-select: text;
}

:global(:root[data-color-mode="dark"]) .km-inline-drawer {
  background: var(--km-drawer-bg, #29292d);
}

.km-drawer-handle {
  height: 10px;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: row-resize;
  flex-shrink: 0;
  touch-action: none;
}

.km-drawer-handle-pill {
  width: 38px;
  height: 4px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--km-text-muted) 42%, transparent);
  transition: background-color 120ms ease, width 120ms ease;
}

.km-drawer-handle:hover .km-drawer-handle-pill {
  width: 48px;
  background: var(--km-accent);
}

.km-drawer-header {
  height: 32px;
  padding: 0 12px 4px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  flex-shrink: 0;
}

.km-drawer-summary-btn {
  border: none;
  background: transparent;
  color: var(--km-text);
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 2px 6px;
  border-radius: var(--km-radius-sm);
  cursor: pointer;
  font-size: 12px;
  font-weight: 600;
}

.km-drawer-summary-btn:hover {
  background: var(--km-hover-bg);
}

.km-drawer-title {
  font-size: 12px;
  font-weight: 600;
}

.km-drawer-meta {
  font-size: 11px;
  font-weight: 400;
  color: var(--km-text-muted);
}

.km-drawer-chevron {
  color: var(--km-text-muted);
  transition: transform 140ms ease;
}

.km-drawer-chevron.collapsed {
  transform: rotate(180deg);
}

.km-drawer-toolbar {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

.km-thumb-slider-wrap {
  display: inline-flex;
  align-items: center;
}

.km-thumb-slider {
  width: 78px;
  accent-color: var(--km-accent);
  cursor: pointer;
}

.km-drawer-icon-btn {
  width: 26px;
  height: 26px;
  border: none;
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-text-muted);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}

.km-drawer-icon-btn:hover {
  background: var(--km-hover-bg);
  color: var(--km-text);
}

.km-btn-save-all {
  height: 24px;
  padding: 0 10px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface-raised);
  color: var(--km-text);
  font-size: 11.5px;
  font-weight: 500;
  cursor: pointer;
}

.km-btn-save-all:hover:not(:disabled) {
  border-color: var(--km-accent);
  color: var(--km-accent);
}

.km-drawer-banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 4px 12px;
  font-size: 11.5px;
  border-top: 1px solid var(--km-border);
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
  padding: 0 4px;
}

.km-drawer-body {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
  padding: 6px 12px 10px;
}

.km-attach-grid {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
}

.km-grid-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  padding: 6px;
  border-radius: var(--km-radius-md);
  cursor: default;
  transition: background-color 120ms ease;
}

.km-grid-card:hover,
.km-grid-card:focus-visible {
  background: var(--km-hover-bg);
  outline: none;
}

.km-grid-thumb {
  position: relative;
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  background: var(--km-card-bg, var(--km-surface-raised));
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
  flex-shrink: 0;
}

.km-grid-thumb.is-image {
  background: var(--km-accent-soft);
}

.km-attach-thumb-img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.km-pdf-thumb {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 2px;
  color: var(--km-danger);
  font-family: var(--km-font-mono);
  font-size: 10px;
  font-weight: 700;
}

.km-attach-badge {
  font-family: var(--km-font-mono);
  font-size: 11px;
  font-weight: 700;
  color: var(--km-accent);
}

.km-grid-hover-actions {
  position: absolute;
  inset: 0;
  background: rgba(18, 20, 26, 0.56);
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  opacity: 0;
  transition: opacity 120ms ease;
}

.km-grid-card:hover .km-grid-hover-actions,
.km-grid-card:focus-within .km-grid-hover-actions {
  opacity: 1;
}

.km-hover-mini-btn {
  width: 24px;
  height: 24px;
  border: none;
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.92);
  color: #1f2328;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}

.km-hover-mini-btn:hover {
  background: #ffffff;
  color: var(--km-accent);
}

.km-grid-name {
  max-width: 100%;
  font-size: 11.5px;
  font-weight: 500;
  text-align: center;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-grid-size {
  font-size: 10.5px;
  color: var(--km-text-muted);
}

.km-attach-list {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.km-list-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 5px 8px;
  border-radius: var(--km-radius-sm);
  border: 1px solid var(--km-border);
  background: var(--km-card-bg, var(--km-surface-raised));
  font-size: 12px;
}

.km-list-row:hover {
  background: var(--km-hover-bg);
}

.km-list-ext {
  min-width: 38px;
  text-align: center;
  padding: 1px 5px;
  border-radius: 4px;
  font-family: var(--km-font-mono);
  font-size: 10px;
  font-weight: 700;
  background: var(--km-accent-soft);
  color: var(--km-accent);
}

.km-list-ext.is-pdf {
  background: var(--km-danger-soft);
  color: var(--km-danger);
}

.km-list-filename {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 500;
}

.km-list-size {
  font-size: 11px;
  color: var(--km-text-muted);
  flex-shrink: 0;
}

.km-list-actions {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
}

.km-btn-xs {
  height: 22px;
  padding: 0 8px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface);
  color: var(--km-text);
  font-size: 11px;
  cursor: pointer;
}

.km-btn-xs:hover:not(:disabled) {
  border-color: var(--km-accent);
  color: var(--km-accent);
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
}

.km-btn-primary {
  background: var(--km-accent);
  border-color: var(--km-accent);
  color: var(--km-accent-fg);
  font-weight: 600;
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

.km-lightbox-pdf {
  width: 100%;
  height: 70vh;
  min-height: 360px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: #ffffff;
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

.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
  white-space: nowrap;
}
</style>

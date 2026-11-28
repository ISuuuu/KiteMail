/**
 * KiteMail 全局附件图库、图片灯箱预览缓存与单件/批量保存 composable。
 * 纯 TS + Vue 响应式状态机，可在无 DOM Node 环境下直接单元测试。
 */

import { computed, ref } from "vue";
import { extractErrorMessage, isLockedError } from "../ipc/main-window";
import type { AttachmentDto, ThreadMessageDto } from "../ipc/types";
import {
  fetchAttachmentPreview,
  fetchAttachmentThumbnail,
  listCachedAttachments,
  openAttachment,
  pickDirectory,
  saveAllAttachments,
  saveAttachment,
} from "./ipc";
import type {
  AttachmentFilterKind,
  CachedAttachmentDto,
  DrawerAttachmentItem,
  FetchAttachmentPreviewOutput,
  FetchAttachmentThumbnailOutput,
  OpenAttachmentOutput,
  PickDirectoryOutput,
  SaveAllAttachmentsOutput,
  SaveAttachmentOutput,
} from "./types";

const IMAGE_EXTENSIONS = new Set([
  "png",
  "jpg",
  "jpeg",
  "gif",
  "webp",
  "svg",
  "bmp",
  "ico",
  "avif",
]);

/** 判断附件是否为图片类型 */
export function isImageAttachment(att: {
  isImage?: boolean;
  mimeType?: string;
  filename?: string;
}): boolean {
  if (typeof att.isImage === "boolean") return att.isImage;
  const mime = (att.mimeType ?? "").toLowerCase();
  if (mime.startsWith("image/")) return true;
  const name = att.filename ?? "";
  const dotIdx = name.lastIndexOf(".");
  if (dotIdx >= 0) {
    const ext = name.slice(dotIdx + 1).toLowerCase();
    return IMAGE_EXTENSIONS.has(ext);
  }
  return false;
}

/** 判断附件是否为 PDF 文档 */
export function isPdfAttachment(att: {
  mimeType?: string;
  filename?: string;
}): boolean {
  const mime = (att.mimeType ?? "").trim().toLowerCase();
  if (mime === "application/pdf") return true;
  const name = (att.filename ?? "").trim().toLowerCase();
  return name.endsWith(".pdf");
}

/** 聚合并按 id 去重整组会话附件 */
export function aggregateThreadAttachments(
  attachments: readonly AttachmentDto[] = [],
  threadMessages: readonly ThreadMessageDto[] = []
): DrawerAttachmentItem[] {
  const seen = new Set<number>();
  const result: DrawerAttachmentItem[] = [];
  const pushItem = (att: AttachmentDto, messageId?: number) => {
    const numId = resolveNumericAttachmentId(att.id);
    if (numId === null || seen.has(numId)) return;
    seen.add(numId);
    result.push({
      id: numId,
      filename: att.filename,
      mimeType: att.mimeType,
      sizeBytes: att.sizeBytes,
      downloaded: Boolean(att.downloaded),
      isImage: isImageAttachment({ mimeType: att.mimeType, filename: att.filename }),
      isPdf: isPdfAttachment({ mimeType: att.mimeType, filename: att.filename }),
      ...(messageId !== undefined ? { messageId } : {}),
    });
  };
  for (const att of attachments) {
    pushItem(att);
  }
  for (const tm of threadMessages) {
    for (const att of tm.attachments ?? []) {
      pushItem(att, tm.id);
    }
  }
  return result;
}

/** 按文件名正序或倒序排序抽屉附件 */
export function sortDrawerAttachments<T extends { filename: string }>(
  list: readonly T[],
  order: "asc" | "desc"
): T[] {
  const factor = order === "asc" ? 1 : -1;
  return [...list].sort(
    (a, b) =>
      a.filename.localeCompare(b.filename, "zh-CN", {
        numeric: true,
        sensitivity: "base",
      }) * factor
  );
}

/** 格式化文件大小显示 */
export function formatAttachmentSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "0 B";
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

/** 提取合法数值附件 ID（过滤缺失或非数值占位 ID） */
export function resolveNumericAttachmentId(id: string | number | undefined): number | null {
  if (typeof id === "number") {
    return Number.isSafeInteger(id) && id > 0 ? id : null;
  }
  if (typeof id === "string" && id.trim().length > 0) {
    const parsed = Number(id.trim());
    return Number.isSafeInteger(parsed) && parsed > 0 ? parsed : null;
  }
  return null;
}

/** 解码 `data:text/...` 预览内容用于纯文本附件预览 */
export function decodeTextPreviewDataUrl(dataUrl: string): string | null {
  const match = dataUrl.match(/^data:text\/[^,;]+(?:;charset=[^,;]+)?,(.*)$/i);
  if (!match) return null;
  try {
    return decodeURIComponent(match[1]);
  } catch {
    return match[1];
  }
}

/** 从缩略图缓存取网格缩略图 URL（图片与 PDF 都由后端返回小图） */
export function resolveDrawerThumbUrl(
  item: Pick<DrawerAttachmentItem, "id">,
  thumbUrls: Readonly<Record<number, string | null | undefined>>
): string | null {
  return thumbUrls[item.id] ?? null;
}

export interface UseAttachmentsOptions {
  pageSize?: number;
  /** 缩略图缓存条目上限，超出按最早写入淘汰 */
  thumbCacheLimit?: number;
  /** 缩略图并发上限（先按 2 验证） */
  thumbConcurrency?: number;
  /** 缩略图获取实现，默认走轻量 IPC，便于测试注入 */
  fetchThumbnailFn?: (input: { attachmentId: number }) => Promise<FetchAttachmentThumbnailOutput>;
}

interface ThumbTask {
  start: () => void;
  abandon: () => void;
}

export function useAttachments(options: UseAttachmentsOptions = {}) {
  const pageSize = Math.max(1, options.pageSize ?? 12);
  const thumbCacheLimit = Math.max(1, options.thumbCacheLimit ?? 160);
  const thumbConcurrency = Math.max(1, options.thumbConcurrency ?? 2);
  const fetchThumbnail = options.fetchThumbnailFn ?? fetchAttachmentThumbnail;

  const items = ref<CachedAttachmentDto[]>([]);
  const total = ref(0);
  const hasMore = ref(false);
  const stateVersion = ref(0);

  const query = ref("");
  const filter = ref<AttachmentFilterKind>("all");

  const loading = ref(false);
  const loadingMore = ref(false);
  const error = ref<string | null>(null);
  const locked = ref(false);
  const statusMessage = ref<string | null>(null);

  /** 完整预览只保留当前打开项，关闭时释放 */
  const previewCache = ref<Record<number, FetchAttachmentPreviewOutput>>({});
  const previewLoadingId = ref<number | null>(null);

  /** 缩略图缓存：id -> 小图 data URL，条目数受上限约束 */
  const thumbUrls = ref<Record<number, string>>({});
  const thumbOrder: number[] = [];
  let thumbGeneration = 0;
  let thumbRunning = 0;
  const thumbInFlight = new Map<number, Promise<string | null>>();
  const thumbQueue: ThumbTask[] = [];
  const activePreview = ref<FetchAttachmentPreviewOutput | null>(null);
  const previewError = ref<string | null>(null);

  const savingId = ref<number | null>(null);
  const savingAll = ref(false);
  const lastSaveResult = ref<SaveAttachmentOutput | null>(null);
  const lastBatchSaveResult = ref<SaveAllAttachmentsOutput | null>(null);

  let loadGeneration = 0;

  const filteredItems = computed(() => {
    switch (filter.value) {
      case "images":
        return items.value.filter((item) => isImageAttachment(item));
      case "documents":
        return items.value.filter((item) => !isImageAttachment(item));
      default:
        return items.value;
    }
  });

  const counts = computed(() => {
    let images = 0;
    let documents = 0;
    for (const item of items.value) {
      if (isImageAttachment(item)) images += 1;
      else documents += 1;
    }
    return {
      all: items.value.length,
      images,
      documents,
    };
  });

  function handleIpcFailure(err: unknown, assignTarget: "main" | "preview" = "main") {
    if (isLockedError(err)) {
      locked.value = true;
      clearThumbnails();
      previewCache.value = {};
      activePreview.value = null;
    }
    const msg = extractErrorMessage(err);
    if (assignTarget === "preview") {
      previewError.value = msg;
    } else {
      error.value = msg;
    }
  }

  function cacheThumbUrl(id: number, url: string): void {
    const existing = thumbOrder.indexOf(id);
    if (existing >= 0) thumbOrder.splice(existing, 1);
    thumbOrder.push(id);
    const next = { ...thumbUrls.value, [id]: url };
    while (thumbOrder.length > thumbCacheLimit) {
      const evicted = thumbOrder.shift();
      if (evicted !== undefined) delete next[evicted];
    }
    thumbUrls.value = next;
  }

  function drainThumbQueue(): void {
    while (thumbRunning < thumbConcurrency && thumbQueue.length > 0) {
      const task = thumbQueue.shift();
      task?.start();
    }
  }

  /** 递增请求代次并丢弃在途/排队结果：切邮件、关抽屉、锁屏时调用 */
  function invalidateThumbnails(): void {
    thumbGeneration += 1;
    for (const task of thumbQueue.splice(0)) task.abandon();
    thumbInFlight.clear();
  }

  /** 清空缩略图缓存与在途请求 */
  function clearThumbnails(): void {
    invalidateThumbnails();
    thumbOrder.length = 0;
    thumbUrls.value = {};
  }

  /** 请求单个缩略图：命中缓存直接返回，同 id 的在途请求合并 */
  function requestThumbnail(attachmentId: number): Promise<string | null> {
    const cached = thumbUrls.value[attachmentId];
    if (cached) return Promise.resolve(cached);
    const pending = thumbInFlight.get(attachmentId);
    if (pending) return pending;

    const generation = thumbGeneration;
    let settle: (url: string | null) => void = () => {};
    const promise = new Promise<string | null>((resolve) => {
      settle = resolve;
    });
    const run = async (): Promise<void> => {
      thumbRunning += 1;
      try {
        const preview = await fetchThumbnail({ attachmentId });
        if (generation !== thumbGeneration) {
          settle(null);
          return;
        }
        const url = preview.thumbnailDataUrl ?? null;
        if (url) cacheThumbUrl(attachmentId, url);
        settle(url);
      } catch (err) {
        if (generation === thumbGeneration && isLockedError(err)) {
          handleIpcFailure(err, "preview");
        }
        settle(null);
      } finally {
        thumbRunning = Math.max(0, thumbRunning - 1);
        if (thumbInFlight.get(attachmentId) === promise) {
          thumbInFlight.delete(attachmentId);
        }
        drainThumbQueue();
      }
    };
    thumbInFlight.set(attachmentId, promise);
    thumbQueue.push({
      start: () => {
        void run();
      },
      abandon: () => {
        if (thumbInFlight.get(attachmentId) === promise) {
          thumbInFlight.delete(attachmentId);
        }
        settle(null);
      },
    });
    drainThumbQueue();
    return promise;
  }

  /** 按可见项调度缩略图：仅图片与 PDF，并发受限，缓存命中的跳过 */
  async function prefetchThumbnails(
    items: readonly Pick<DrawerAttachmentItem, "id" | "isImage" | "isPdf">[]
  ): Promise<void> {
    const tasks: Promise<string | null>[] = [];
    for (const item of items) {
      if (!item.isImage && !item.isPdf) continue;
      if (thumbUrls.value[item.id]) continue;
      tasks.push(requestThumbnail(item.id));
    }
    if (tasks.length > 0) {
      await Promise.allSettled(tasks);
    }
  }

  async function loadGallery(reset = true): Promise<boolean> {
    if (reset) {
      loading.value = true;
    } else {
      if (loading.value || loadingMore.value || !hasMore.value) return false;
      loadingMore.value = true;
    }
    error.value = null;
    const gen = ++loadGeneration;
    const offset = reset ? 0 : items.value.length;

    try {
      const res = await listCachedAttachments({
        query: query.value,
        offset,
        limit: pageSize,
      });
      if (gen !== loadGeneration) return false;

      locked.value = false;
      if (reset) {
        items.value = res.items;
      } else {
        const seen = new Set(items.value.map((i) => i.id));
        const next = [...items.value];
        for (const item of res.items) {
          if (!seen.has(item.id)) next.push(item);
        }
        items.value = next;
      }
      total.value = res.total;
      hasMore.value = res.hasMore;
      stateVersion.value = res.stateVersion;
      return true;
    } catch (err) {
      if (gen === loadGeneration) {
        handleIpcFailure(err, "main");
      }
      return false;
    } finally {
      if (gen === loadGeneration) {
        loading.value = false;
        loadingMore.value = false;
      }
    }
  }

  async function setQuery(nextQuery: string): Promise<boolean> {
    query.value = nextQuery;
    return loadGallery(true);
  }

  function setFilter(nextFilter: AttachmentFilterKind): void {
    filter.value = nextFilter;
  }

  async function loadMore(): Promise<boolean> {
    return loadGallery(false);
  }

  async function fetchPreview(
    attachmentId: number,
    opts?: { force?: boolean; silent?: boolean }
  ): Promise<FetchAttachmentPreviewOutput | null> {
    if (!opts?.force && previewCache.value[attachmentId]) {
      if (!opts?.silent) {
        previewError.value = null;
      }
      return previewCache.value[attachmentId];
    }

    previewLoadingId.value = attachmentId;
    if (!opts?.silent) {
      previewError.value = null;
    }
    try {
      const preview = await fetchAttachmentPreview({ attachmentId });
      locked.value = false;
      // 只保留当前打开项的完整预览，避免多封大附件驻留
      previewCache.value = { [attachmentId]: preview };
      return preview;
    } catch (err) {
      if (!opts?.silent) {
        handleIpcFailure(err, "preview");
      }
      return null;
    } finally {
      if (previewLoadingId.value === attachmentId) {
        previewLoadingId.value = null;
      }
    }
  }

  async function openPreview(attachmentId: number): Promise<FetchAttachmentPreviewOutput | null> {
    const preview = await fetchPreview(attachmentId);
    if (preview) {
      activePreview.value = preview;
    }
    return preview;
  }

  function closePreview(): void {
    activePreview.value = null;
    previewCache.value = {};
    previewError.value = null;
  }

  function markItemsDownloaded(ids: number[]): void {
    if (ids.length === 0) return;
    const idSet = new Set(ids);
    items.value = items.value.map((item) =>
      idSet.has(item.id) ? { ...item, downloaded: true } : item
    );
  }

  async function saveSingle(
    attachmentId: number,
    destPath?: string | null
  ): Promise<SaveAttachmentOutput | null> {
    savingId.value = attachmentId;
    error.value = null;
    statusMessage.value = null;
    try {
      const res = await saveAttachment({ attachmentId, destPath });
      locked.value = false;
      lastSaveResult.value = res;
      if (res.saved) {
        markItemsDownloaded([attachmentId]);
      }
      statusMessage.value = res.message;
      return res;
    } catch (err) {
      handleIpcFailure(err, "main");
      return null;
    } finally {
      if (savingId.value === attachmentId) {
        savingId.value = null;
      }
    }
  }

  async function saveMany(
    attachmentIds: number[],
    destDir?: string | null
  ): Promise<SaveAllAttachmentsOutput | null> {
    savingAll.value = true;
    error.value = null;
    statusMessage.value = null;
    try {
      const res = await saveAllAttachments({ attachmentIds, destDir });
      locked.value = false;
      lastBatchSaveResult.value = res;
      if (res.savedCount > 0) {
        markItemsDownloaded(attachmentIds);
      }
      statusMessage.value = res.message;
      return res;
    } catch (err) {
      handleIpcFailure(err, "main");
      return null;
    } finally {
      savingAll.value = false;
    }
  }

  async function openWithDefaultApp(attachmentId: number): Promise<OpenAttachmentOutput | null> {
    error.value = null;
    statusMessage.value = null;
    try {
      const res = await openAttachment({ attachmentId });
      locked.value = false;
      markItemsDownloaded([attachmentId]);
      statusMessage.value = res.message;
      return res;
    } catch (err) {
      handleIpcFailure(err, "main");
      return null;
    }
  }

  async function pickSaveDirectory(): Promise<PickDirectoryOutput | null> {
    error.value = null;
    try {
      const res = await pickDirectory();
      locked.value = false;
      return res;
    } catch (err) {
      handleIpcFailure(err, "main");
      return null;
    }
  }

  async function saveManyWithPicker(
    attachmentIds: number[]
  ): Promise<SaveAllAttachmentsOutput | null> {
    if (attachmentIds.length === 0) return null;
    const picked = await pickSaveDirectory();
    if (!picked || picked.cancelled || !picked.path) {
      return null;
    }
    return saveMany(attachmentIds, picked.path);
  }

  function clearStatus(): void {
    error.value = null;
    previewError.value = null;
    statusMessage.value = null;
  }

  function reset(): void {
    loadGeneration += 1;
    items.value = [];
    total.value = 0;
    hasMore.value = false;
    stateVersion.value = 0;
    query.value = "";
    filter.value = "all";
    loading.value = false;
    loadingMore.value = false;
    error.value = null;
    locked.value = false;
    statusMessage.value = null;
    clearThumbnails();
    previewCache.value = {};
    previewLoadingId.value = null;
    activePreview.value = null;
    previewError.value = null;
    savingId.value = null;
    savingAll.value = false;
    lastSaveResult.value = null;
    lastBatchSaveResult.value = null;
  }

  return {
    items,
    filteredItems,
    counts,
    total,
    hasMore,
    stateVersion,
    query,
    filter,
    loading,
    loadingMore,
    error,
    locked,
    statusMessage,
    previewCache,
    previewLoadingId,
    thumbUrls,
    activePreview,
    previewError,
    savingId,
    savingAll,
    lastSaveResult,
    lastBatchSaveResult,
    loadGallery,
    loadMore,
    setQuery,
    setFilter,
    fetchPreview,
    openPreview,
    closePreview,
    requestThumbnail,
    prefetchThumbnails,
    invalidateThumbnails,
    clearThumbnails,
    saveSingle,
    saveMany,
    openWithDefaultApp,
    pickSaveDirectory,
    saveManyWithPicker,
    clearStatus,
    reset,
  };
}

export type UseAttachmentsReturn = ReturnType<typeof useAttachments>;

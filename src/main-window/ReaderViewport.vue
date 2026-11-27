<template>
  <div ref="viewportRef" class="km-reader-viewport">
    <!-- 加载状态 -->
    <div v-if="bodyStatus === 'loading'" class="km-body-state">
      <span>正在加载邮件正文…</span>
    </div>

    <!-- 不可用 / 错误回退状态 -->
    <div v-else-if="bodyStatus === 'unavailable'" class="km-body-unavailable">
      <div class="km-unavailable-banner" role="alert">
        <span>{{ errorMessage || "正文阅读器暂不可用" }}</span>
        <button type="button" class="km-retry-btn" @click="emit('retry')">重试</button>
      </div>
      <pre v-if="plainText" class="km-plain-body">{{ plainText }}</pre>
    </div>

    <!-- 原生 WebView2 宿主占位（Tauri + HTML 正文且未被浮层遮挡） -->
    <div
      v-else-if="nativeHostActive"
      class="km-native-placeholder"
      :style="{ fontSize: `${zoomFactor * 13}px` }"
    >
      <span v-if="overlayOccluded" class="km-occluded-hint">
        菜单打开期间已暂隐原生阅读器控件
      </span>
      <pre v-else-if="plainText" class="km-plain-body km-plain-backdrop">{{ plainText }}</pre>
    </div>

    <!-- 浏览器预览或纯文本正文渲染（严格转义，不使用 v-html） -->
    <div v-else class="km-fallback-scroll" :style="{ fontSize: `${zoomFactor * 13}px` }">
      <template v-if="threadMessages && threadMessages.length > 1">
        <article
          v-for="item in threadMessages"
          :key="item.id"
          class="km-thread-card"
        >
          <header class="km-thread-head">
            <span class="km-thread-sender">{{ item.sender }}</span>
            <span class="km-thread-date">{{ item.dateDisplay }}</span>
          </header>
          <pre class="km-plain-body">{{ item.plainText || item.preview || "(无正文内容)" }}</pre>
        </article>
      </template>

      <iframe
        v-else-if="hasHtml && htmlBody && !tauriMode"
        class="km-sandbox-frame"
        sandbox=""
        referrerpolicy="no-referrer"
        title="邮件 HTML 预览"
        :srcdoc="htmlBody"
      />

      <pre v-else class="km-plain-body">{{ plainText || "(无正文内容)" }}</pre>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import type { BodyStatus, RectDto, ThreadMessageDto } from "../ipc/types";
import { isTauri, setReaderLayout } from "../ipc/main-window";

const props = defineProps<{
  readerSessionId: string;
  bodyStatus: BodyStatus;
  hasHtml: boolean;
  plainText: string | null;
  htmlBody: string | null;
  threadMessages?: ThreadMessageDto[];
  overlayOccluded: boolean;
  errorMessage?: string | null;
  zoomPercent?: number;
}>();

const emit = defineEmits<{
  retry: [];
}>();

const viewportRef = ref<HTMLElement | null>(null);
const tauriMode = isTauri();
const documentHidden = ref(typeof document !== "undefined" ? document.hidden : false);

const zoomFactor = computed(() => {
  const z = props.zoomPercent ?? 100;
  return Math.min(2.5, Math.max(0.5, z / 100));
});

const nativeHostActive = computed(
  () => tauriMode && props.hasHtml && props.bodyStatus === "ready"
);

// 几何与可见性同步（ResizeObserver + rAF 每帧最多一次 set_reader_layout）
let layoutRevision = 0;
let rafId: number | null = null;
let ro: ResizeObserver | null = null;
let lastSentSession = "";
let lastSentVisible = false;

function computeRects(): { contentRect: RectDto; clipRect: RectDto; valid: boolean } {
  const el = viewportRef.value;
  if (!el) {
    return {
      contentRect: { x: 0, y: 0, width: 0, height: 0 },
      clipRect: { x: 0, y: 0, width: 0, height: 0 },
      valid: false,
    };
  }
  const rect = el.getBoundingClientRect();
  const winW = window.innerWidth || 0;
  const winH = window.innerHeight || 0;
  const left = Math.max(0, rect.left);
  const top = Math.max(0, rect.top);
  const right = Math.min(winW, rect.right);
  const bottom = Math.min(winH, rect.bottom);
  const clipW = Math.max(0, right - left);
  const clipH = Math.max(0, bottom - top);

  return {
    contentRect: {
      x: Math.round(rect.left),
      y: Math.round(rect.top),
      width: Math.max(0, Math.round(rect.width)),
      height: Math.max(0, Math.round(rect.height)),
    },
    clipRect: {
      x: Math.round(left),
      y: Math.round(top),
      width: Math.round(clipW),
      height: Math.round(clipH),
    },
    valid: clipW > 0 && clipH > 0 && rect.width > 0 && rect.height > 0,
  };
}

function flushLayoutSync(forceHide = false, sessionOverride?: string) {
  const sessionId = sessionOverride ?? props.readerSessionId;
  if (!sessionId) return;

  const { contentRect, clipRect, valid } = computeRects();
  const shouldBeVisible =
    !forceHide &&
    !props.overlayOccluded &&
    !documentHidden.value &&
    nativeHostActive.value &&
    valid;

  layoutRevision += 1;
  lastSentSession = sessionId;
  lastSentVisible = shouldBeVisible;

  setReaderLayout({
    readerSessionId: sessionId,
    layoutRevision,
    contentRect,
    clipRect,
    visible: shouldBeVisible,
    devicePixelRatio: typeof window !== "undefined" ? window.devicePixelRatio : 1,
  }).catch(() => {
    /* 忽略非 Tauri 或过期会话拒绝 */
  });
}

function scheduleLayoutSync() {
  if (!props.readerSessionId) return;
  if (rafId !== null) return;
  rafId = requestAnimationFrame(() => {
    rafId = null;
    flushLayoutSync(false);
  });
}

function onVisibilityChange() {
  documentHidden.value = document.hidden;
  if (document.hidden) {
    flushLayoutSync(true);
  } else {
    scheduleLayoutSync();
  }
}

watch(
  () => [props.readerSessionId, props.bodyStatus, props.hasHtml, props.overlayOccluded] as const,
  ([nextSession, , , occluded], [prevSession]) => {
    if (prevSession && prevSession !== nextSession && lastSentVisible) {
      flushLayoutSync(true, prevSession);
    }
    if (occluded) {
      flushLayoutSync(true);
    } else {
      scheduleLayoutSync();
    }
  }
);

onMounted(() => {
  if (viewportRef.value) {
    ro = new ResizeObserver(() => scheduleLayoutSync());
    ro.observe(viewportRef.value);
  }
  window.addEventListener("resize", scheduleLayoutSync);
  document.addEventListener("visibilitychange", onVisibilityChange);
  scheduleLayoutSync();
});

onUnmounted(() => {
  if (rafId !== null) {
    cancelAnimationFrame(rafId);
    rafId = null;
  }
  ro?.disconnect();
  window.removeEventListener("resize", scheduleLayoutSync);
  document.removeEventListener("visibilitychange", onVisibilityChange);
  if (lastSentSession && lastSentVisible) {
    flushLayoutSync(true, lastSentSession);
  }
});
</script>

<style scoped>
.km-reader-viewport {
  flex: 1 1 auto;
  min-height: 120px;
  display: flex;
  flex-direction: column;
  background: var(--km-surface-raised);
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-lg);
  box-shadow: var(--km-shadow-card);
  overflow: hidden;
  position: relative;
}

.km-body-state,
.km-native-placeholder {
  flex: 1 1 auto;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--km-text-muted);
  padding: 20px;
}

.km-native-placeholder {
  align-items: flex-start;
  justify-content: flex-start;
  overflow: auto;
}

.km-occluded-hint {
  margin: auto;
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-plain-backdrop {
  opacity: 0.72;
}

.km-body-unavailable {
  flex: 1 1 auto;
  display: flex;
  flex-direction: column;
  overflow: auto;
  padding: 16px;
  gap: 12px;
}

.km-unavailable-banner {
  padding: 10px 14px;
  border-radius: var(--km-radius-md);
  background: var(--km-danger-soft);
  color: var(--km-danger);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.km-retry-btn {
  padding: 4px 12px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface-raised);
  color: var(--km-text);
  cursor: pointer;
}

.km-fallback-scroll {
  flex: 1 1 auto;
  overflow: auto;
  padding: 20px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.km-plain-body {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: var(--km-font-sans);
  line-height: 1.6;
  color: var(--km-text);
  user-select: text;
}

.km-sandbox-frame {
  width: 100%;
  flex: 1 1 auto;
  min-height: 240px;
  border: none;
  background: #ffffff;
}

.km-thread-card {
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-md);
  padding: 14px;
  background: var(--km-surface);
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.km-thread-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding-bottom: 6px;
  border-bottom: 1px solid var(--km-border);
  font-size: 12px;
}

.km-thread-sender {
  font-weight: 600;
}

.km-thread-date {
  color: var(--km-text-muted);
}
</style>

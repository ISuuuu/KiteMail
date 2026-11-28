<template>
  <div
    ref="viewportRef"
    class="km-reader-viewport"
    :class="viewportToneClass"
  >
    <!-- 加载骨架 -->
    <div v-if="bodyStatus === 'loading'" class="km-body-skeleton" aria-busy="true" aria-label="正在加载邮件正文">
      <div class="km-body-sk-head">
        <span class="km-body-spinner" aria-hidden="true" />
        <span>正在加载邮件正文…</span>
      </div>
      <div class="km-body-sk-line w-85" />
      <div class="km-body-sk-line w-92" />
      <div class="km-body-sk-line w-70" />
      <div class="km-body-sk-line w-60" />
    </div>

    <!-- 不可用 / 错误回退状态 -->
    <div v-else-if="bodyStatus === 'unavailable'" class="km-body-unavailable">
      <div class="km-unavailable-banner" role="alert">
        <span>{{ errorMessage || "正文阅读器暂不可用，请重试" }}</span>
        <button type="button" class="km-retry-btn" @click="emit('retry')">重试正文</button>
      </div>
      <pre v-if="plainText" class="km-plain-body">{{ plainText }}</pre>
    </div>

    <!-- 原生 WebView2 宿主占位（Tauri + HTML/多封会话正文且未被浮层遮挡） -->
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
    <div
      v-else
      ref="fallbackScrollRef"
      class="km-fallback-scroll"
      :class="{ 'km-reading-view': readingView }"
      :style="{ fontSize: `${zoomFactor * 13}px` }"
    >
      <div class="km-fallback-stack">
        <article
          v-for="card in renderedCards"
          :key="card.id"
          class="km-thread-card"
          :class="{ folded: isCardFolded(card.id) }"
        >
          <!-- 卡片头 -->
          <header
            class="km-thread-head"
            @click="toggleCardFold(card.id)"
          >
            <span
              class="km-card-avatar"
              :style="{ backgroundColor: card.avatarColor }"
              aria-hidden="true"
            >
              {{ card.avatarInitial }}
            </span>

            <div class="km-card-sender-col">
              <span class="km-thread-sender">{{ card.senderName || "(未知发件人)" }}</span>
              <span v-if="card.senderEmail && !isCardFolded(card.id)" class="km-thread-email">
                {{ card.senderEmail }}
              </span>
            </div>

            <span v-if="isCardFolded(card.id)" class="km-folded-snippet">
              {{ card.snippet }}
            </span>

            <span v-if="!isCardFolded(card.id)" class="km-card-spacer" />

            <button
              v-if="!isCardFolded(card.id) && card.recipientCount > 0"
              type="button"
              class="km-rcpt-pill"
              :class="{ open: isRecipientsExpanded(card.id) }"
              :aria-expanded="isRecipientsExpanded(card.id)"
              @click.stop="toggleRecipients(card.id)"
            >
              收件人 ({{ card.recipientCount }})
            </button>

            <span v-if="card.flagged" class="km-card-star" title="已加星标">★</span>
            <span class="km-thread-date">{{ card.dateDisplay }}</span>

            <div class="km-card-menu-wrap" @click.stop>
              <button
                type="button"
                class="km-card-more-btn"
                title="此封邮件操作"
                aria-label="此封邮件操作"
                aria-haspopup="menu"
                :aria-expanded="openCardMenuId === card.id"
                @click="toggleCardMenu(card.id)"
              >
                ⋯
              </button>
              <div
                v-if="openCardMenuId === card.id"
                class="km-card-menu"
                role="menu"
              >
                <button type="button" class="km-card-menu-item" @click="triggerCardAction('reply', card)">
                  回复此封
                </button>
                <button type="button" class="km-card-menu-item" @click="triggerCardAction('replyAll', card)">
                  全部回复
                </button>
                <button type="button" class="km-card-menu-item" @click="triggerCardAction('forward', card)">
                  转发此封
                </button>
                <button type="button" class="km-card-menu-item" @click="triggerCardAction('copyBody', card)">
                  复制正文
                </button>
              </div>
            </div>
          </header>

          <!-- 收件人详情 -->
          <div
            v-if="!isCardFolded(card.id) && isRecipientsExpanded(card.id)"
            class="km-rcpts-detail"
          >
            <div v-if="card.recipients">收件人：{{ card.recipients }}</div>
            <div v-if="card.cc">抄送：{{ card.cc }}</div>
          </div>

          <!-- 卡片正文与附件 -->
          <div v-if="!isCardFolded(card.id)" class="km-card-body">
            <iframe
              v-if="renderedCards.length === 1 && hasHtml && htmlBody && !tauriMode && !findQuery"
              class="km-sandbox-frame"
              sandbox=""
              referrerpolicy="no-referrer"
              title="邮件 HTML 预览"
              :srcdoc="htmlBody"
            />

            <pre v-else class="km-plain-body"><template v-for="(seg, idx) in card.segments" :key="idx"><mark
                  v-if="seg.hit"
                  :data-hit-index="seg.hitIndex"
                  class="km-find-hit"
                  :class="{ current: seg.hitIndex === activeFindIndex }"
                >{{ seg.text }}</mark><span v-else>{{ seg.text }}</span></template></pre>

            <!-- 单卡附件胶囊：主体打开、右侧下载箭头独立保存 -->
            <div v-if="card.attachments.length > 0" class="km-card-atts">
              <div
                v-for="(att, idx) in card.attachments"
                :key="att.id ?? `${att.filename}-${idx}`"
                class="km-card-att-chip"
              >
                <button
                  type="button"
                  class="km-card-att-open"
                  :title="`打开 ${att.filename}`"
                  @click="emitAttachmentCardAction('openAttachment', card.id, att.id)"
                >
                  <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
                    <path
                      fill="currentColor"
                      d="M11.6 3.4a2.8 2.8 0 0 1 0 3.96L6.45 12.5a1.85 1.85 0 0 1-2.62-2.62l4.8-4.8a.9.9 0 0 1 1.28 1.28l-4.8 4.8a.55.55 0 0 0 .78.78l5.15-5.14a1.5 1.5 0 1 0-2.12-2.12L3.8 9.8a2.45 2.45 0 1 0 3.46 3.46l4.02-4.02a.65.65 0 1 1 .92.92l-4.02 4.02A3.75 3.75 0 1 1 2.88 8.9l5.12-5.12a2.8 2.8 0 0 1 3.6-.38Z"
                    />
                  </svg>
                  <span class="km-card-att-name">{{ att.filename }}</span>
                  <span class="km-card-att-size">{{ formatAttachmentSize(att.sizeBytes) }}</span>
                </button>
                <button
                  type="button"
                  class="km-card-att-dl"
                  :title="`保存 ${att.filename}`"
                  :aria-label="`下载 ${att.filename}`"
                  @click="emitAttachmentCardAction('saveAttachment', card.id, att.id)"
                >
                  <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
                    <path
                      fill="currentColor"
                      d="M8.65 2.5v6.9l2.15-2.15.92.92L8 11.9 4.28 8.17l.92-.92L7.35 9.4V2.5h1.3ZM2.5 12.8h11V14h-11v-1.2Z"
                    />
                  </svg>
                </button>
              </div>
            </div>
          </div>
        </article>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type {
  AttachmentDto,
  BodyStatus,
  RectDto,
  ThreadCardActionEvent,
  ThreadMessageDto,
} from "../ipc/types";
import { isTauri, nextReaderLayoutRevision, setReaderLayout } from "../ipc/main-window";
import { formatAttachmentSize, resolveNumericAttachmentId } from "../reader-tools/useAttachments";
import {
  avatarToneColor,
  buildFallbackFindSegments,
  countRecipientsDisplay,
  formatAvatarInitial,
  nextFindIndex,
  splitSenderDisplay,
  type FindTextSegment,
} from "./readerHelpers";

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
  accountId?: number;
  messageId?: number;
  sender?: string;
  recipients?: string;
  cc?: string;
  dateDisplay?: string;
  attachments?: AttachmentDto[];
  flagged?: boolean;
  effectiveDark?: boolean;
  perMessageDark?: boolean | null;
  readingView?: boolean;
  findQuery?: string;
  findStepToken?: number;
  findStepDelta?: number;
}>();

const emit = defineEmits<{
  retry: [];
  cardAction: [event: ThreadCardActionEvent];
  findResult: [result: { current: number; total: number }];
}>();

const viewportRef = ref<HTMLElement | null>(null);
const fallbackScrollRef = ref<HTMLElement | null>(null);
const tauriMode = isTauri();
const documentHidden = ref(typeof document !== "undefined" ? document.hidden : false);

const foldedCardIds = ref<number[]>([]);
const expandedRecipientIds = ref<number[]>([]);
const openCardMenuId = ref<number | null>(null);
const activeFindIndex = ref(-1);

const zoomFactor = computed(() => {
  const z = props.zoomPercent ?? 100;
  return Math.min(2.5, Math.max(0.5, z / 100));
});

const nativeHostActive = computed(
  () =>
    tauriMode &&
    props.bodyStatus === "ready" &&
    (props.hasHtml || (props.threadMessages?.length ?? 0) > 1)
);

const viewportToneClass = computed(() => {
  if (props.perMessageDark === true) return "km-body-force-dark";
  if (props.perMessageDark === false) return "km-body-force-light";
  return "";
});

interface NormalizedCard {
  id: number;
  sender: string;
  senderName: string;
  senderEmail: string;
  avatarInitial: string;
  avatarColor: string;
  recipients: string;
  cc: string;
  recipientCount: number;
  dateDisplay: string;
  flagged: boolean;
  bodyText: string;
  snippet: string;
  attachments: AttachmentDto[];
  segments: FindTextSegment[];
}

const baseCards = computed<Omit<NormalizedCard, "segments">[]>(() => {
  const source: Array<{
    id: number;
    sender: string;
    recipients: string;
    cc: string;
    dateDisplay: string;
    flagged: boolean;
    bodyText: string;
    snippet: string;
    attachments: AttachmentDto[];
  }> =
    props.threadMessages && props.threadMessages.length > 0
      ? props.threadMessages.map((item) => ({
          id: item.id,
          sender: item.sender,
          recipients: item.recipients,
          cc: item.cc,
          dateDisplay: item.dateDisplay,
          flagged: item.flagged,
          bodyText: item.plainText || item.preview || "(无正文内容)",
          snippet: item.preview || item.plainText || "",
          attachments: item.attachments ?? [],
        }))
      : [
          {
            id: props.messageId ?? 0,
            sender: props.sender ?? "",
            recipients: props.recipients ?? "",
            cc: props.cc ?? "",
            dateDisplay: props.dateDisplay ?? "",
            flagged: Boolean(props.flagged),
            bodyText: props.plainText || "(无正文内容)",
            snippet: props.plainText || "",
            attachments: props.attachments ?? [],
          },
        ];

  return source.map((item) => {
    const parsed = splitSenderDisplay(item.sender);
    return {
      ...item,
      senderName: parsed.name,
      senderEmail: parsed.email,
      avatarInitial: formatAvatarInitial(parsed.name, parsed.email),
      avatarColor: avatarToneColor(parsed.email || parsed.name || String(item.id)),
      recipientCount: countRecipientsDisplay(item.recipients, item.cc),
    };
  });
});

const findComputation = computed(() => {
  const q = props.findQuery ?? "";
  let totalHits = 0;
  const cards: NormalizedCard[] = baseCards.value.map((card) => {
    if (foldedCardIds.value.includes(card.id)) {
      return {
        ...card,
        segments: [{ text: card.bodyText, hit: false, hitIndex: -1 }],
      };
    }
    const res = buildFallbackFindSegments(card.bodyText, q, totalHits);
    totalHits += res.matchCount;
    return {
      ...card,
      segments: res.segments,
    };
  });
  return { cards, totalHits };
});

const renderedCards = computed(() => findComputation.value.cards);
const totalFindHits = computed(() => findComputation.value.totalHits);

function isCardFolded(id: number): boolean {
  return foldedCardIds.value.includes(id);
}

function toggleCardFold(id: number) {
  if (renderedCards.value.length <= 1) return;
  if (foldedCardIds.value.includes(id)) {
    foldedCardIds.value = foldedCardIds.value.filter((x) => x !== id);
  } else {
    foldedCardIds.value = [...foldedCardIds.value, id];
  }
}

function isRecipientsExpanded(id: number): boolean {
  return expandedRecipientIds.value.includes(id);
}

function toggleRecipients(id: number) {
  if (expandedRecipientIds.value.includes(id)) {
    expandedRecipientIds.value = expandedRecipientIds.value.filter((x) => x !== id);
  } else {
    expandedRecipientIds.value = [...expandedRecipientIds.value, id];
  }
}

function toggleCardMenu(id: number) {
  openCardMenuId.value = openCardMenuId.value === id ? null : id;
}

function triggerCardAction(
  kind: "reply" | "replyAll" | "forward" | "copyBody",
  card: NormalizedCard
) {
  openCardMenuId.value = null;
  emit("cardAction", {
    kind,
    accountId: props.accountId ?? 0,
    messageId: props.messageId ?? card.id,
    targetId: card.id,
    text: kind === "copyBody" ? card.bodyText : null,
  });
}

function emitAttachmentCardAction(
  kind: "openAttachment" | "saveAttachment",
  cardMessageId: number,
  rawAttachmentId?: string | number
) {
  const numId = resolveNumericAttachmentId(rawAttachmentId);
  if (numId === null) return;
  emit("cardAction", {
    kind,
    accountId: props.accountId ?? 0,
    messageId: cardMessageId,
    targetId: numId,
    text: null,
  });
}

function scrollToActiveHit() {
  if (activeFindIndex.value < 0 || !fallbackScrollRef.value) return;
  void nextTick(() => {
    const el = fallbackScrollRef.value?.querySelector(
      `mark[data-hit-index="${activeFindIndex.value}"]`
    ) as HTMLElement | null;
    el?.scrollIntoView({ block: "center" });
  });
}

watch(
  [() => props.findQuery, totalFindHits],
  () => {
    if (nativeHostActive.value) return;
    const total = totalFindHits.value;
    activeFindIndex.value = total > 0 ? 0 : -1;
    emit("findResult", {
      current: total > 0 ? 1 : 0,
      total,
    });
    if (total > 0) scrollToActiveHit();
  },
  { immediate: true }
);

watch(
  () => props.findStepToken,
  () => {
    if (nativeHostActive.value) return;
    const total = totalFindHits.value;
    if (total <= 0) {
      activeFindIndex.value = -1;
      emit("findResult", { current: 0, total: 0 });
      return;
    }
    const next = nextFindIndex(activeFindIndex.value, total, props.findStepDelta ?? 1);
    activeFindIndex.value = next;
    emit("findResult", { current: next + 1, total });
    scrollToActiveHit();
  }
);

watch(
  () => [props.accountId, props.messageId] as const,
  () => {
    foldedCardIds.value = [];
    expandedRecipientIds.value = [];
    openCardMenuId.value = null;
  }
);

// 几何与可见性同步（ResizeObserver + rAF 每帧最多一次 set_reader_layout）
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

  const layoutRevision = nextReaderLayoutRevision();
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

function onDocumentPointerDown() {
  openCardMenuId.value = null;
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
  if (viewportRef.value && typeof ResizeObserver !== "undefined") {
    ro = new ResizeObserver(() => scheduleLayoutSync());
    ro.observe(viewportRef.value);
  }
  window.addEventListener("resize", scheduleLayoutSync);
  document.addEventListener("visibilitychange", onVisibilityChange);
  document.addEventListener("pointerdown", onDocumentPointerDown);
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
  document.removeEventListener("pointerdown", onDocumentPointerDown);
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
  background: var(--km-reader-canvas, #f1f1f1);
  color: var(--km-text);
  overflow: hidden;
  position: relative;
}

:global(:root[data-color-mode="dark"]) .km-reader-viewport {
  background: var(--km-reader-canvas, #131315);
}

.km-reader-viewport.km-body-force-light {
  --km-reader-canvas: #f1f1f1;
  --km-card-bg: #ffffff;
  --km-text: #2e2e32;
  --km-text-muted: #707074;
  --km-border: #d9d9d9;
  color: #2e2e32;
}

.km-reader-viewport.km-body-force-dark {
  --km-reader-canvas: #131315;
  --km-card-bg: #1d1d20;
  --km-text: #f3f3f6;
  --km-text-muted: #a1a1aa;
  --km-border: #333339;
  color: #f3f3f6;
}

.km-body-skeleton {
  flex: 1 1 auto;
  padding: 20px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.km-body-sk-head {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--km-text-muted);
  margin-bottom: 4px;
}

.km-body-spinner {
  width: 14px;
  height: 14px;
  border-radius: 999px;
  border: 2px solid color-mix(in srgb, var(--km-accent) 25%, transparent);
  border-top-color: var(--km-accent);
  animation: km-body-spin 0.8s linear infinite;
}

.km-body-sk-line {
  height: 12px;
  border-radius: 6px;
  background: color-mix(in srgb, var(--km-text-muted) 16%, transparent);
  animation: km-body-pulse 1.3s ease-in-out infinite;
}

.km-body-sk-line.w-85 {
  width: 85%;
}

.km-body-sk-line.w-92 {
  width: 92%;
}

.km-body-sk-line.w-70 {
  width: 70%;
}

.km-body-sk-line.w-60 {
  width: 60%;
}

@keyframes km-body-spin {
  to {
    transform: rotate(360deg);
  }
}

@keyframes km-body-pulse {
  0%,
  100% {
    opacity: 0.45;
  }
  50% {
    opacity: 0.9;
  }
}

.km-native-placeholder {
  flex: 1 1 auto;
  display: flex;
  align-items: flex-start;
  justify-content: flex-start;
  overflow: auto;
  color: var(--km-text-muted);
  padding: 16px 14px 20px 21px;
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
  padding: 16px 14px 20px 21px;
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
  padding: 15px 14px 20px 14px;
}

.km-fallback-stack {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.km-fallback-scroll.km-reading-view .km-fallback-stack {
  max-width: 720px;
  margin: 0 auto;
}

.km-fallback-scroll.km-reading-view .km-plain-body {
  line-height: 1.78;
}

.km-thread-card {
  border: 1px solid var(--km-border);
  border-radius: 12px;
  padding: 18px 20px;
  background: var(--km-card-bg, #ffffff);
  box-shadow: var(--km-shadow-card);
  display: flex;
  flex-direction: column;
  gap: 10px;
}

:global(:root[data-color-mode="dark"]) .km-reader-viewport:not(.km-body-force-light) .km-thread-card {
  background: var(--km-card-bg, #1d1d20);
}

.km-thread-card.folded {
  padding: 12px 18px;
}

.km-thread-head {
  display: flex;
  align-items: center;
  gap: 10px;
  cursor: pointer;
  min-width: 0;
}

.km-card-avatar {
  width: 34px;
  height: 34px;
  border-radius: 50%;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: #ffffff;
  font-weight: 700;
  font-size: 14px;
}

.km-thread-card.folded .km-card-avatar {
  width: 28px;
  height: 28px;
  font-size: 12px;
}

.km-card-sender-col {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.km-thread-sender {
  font-size: 13px;
  font-weight: 700;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.km-thread-email {
  font-size: 11.5px;
  color: var(--km-text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.km-folded-snippet {
  flex: 1 1 auto;
  min-width: 0;
  font-size: 12px;
  color: var(--km-text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.km-card-spacer {
  flex: 1 1 auto;
  min-width: 8px;
}

.km-rcpt-pill {
  border: none;
  border-radius: 6px;
  padding: 3px 9px;
  font-size: 11.5px;
  background: color-mix(in srgb, var(--km-text-muted) 12%, transparent);
  color: var(--km-text-muted);
  cursor: pointer;
  flex-shrink: 0;
}

.km-rcpt-pill:hover,
.km-rcpt-pill.open {
  color: var(--km-text);
  background: color-mix(in srgb, var(--km-text-muted) 18%, transparent);
}

.km-card-star {
  color: var(--km-star-gold);
  font-size: 13px;
  flex-shrink: 0;
}

.km-thread-date {
  font-size: 11.5px;
  color: var(--km-text-muted);
  white-space: nowrap;
  flex-shrink: 0;
}

.km-card-menu-wrap {
  position: relative;
  flex-shrink: 0;
}

.km-card-more-btn {
  width: 28px;
  height: 28px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--km-text-muted);
  font-size: 15px;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.km-card-more-btn:hover {
  background: var(--km-hover-bg);
  color: var(--km-text);
}

.km-card-menu {
  position: absolute;
  top: 30px;
  right: 0;
  min-width: 140px;
  padding: 4px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-md);
  background: var(--km-surface-raised);
  box-shadow: var(--km-shadow-popover);
  z-index: 30;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.km-card-menu-item {
  border: none;
  background: transparent;
  color: var(--km-text);
  text-align: left;
  padding: 6px 10px;
  border-radius: var(--km-radius-sm);
  font-size: 12px;
  cursor: pointer;
}

.km-card-menu-item:hover {
  background: var(--km-hover-bg);
}

.km-rcpts-detail {
  padding-left: 44px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-size: 11.5px;
  color: var(--km-text-muted);
  user-select: text;
}

.km-card-body {
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

.km-find-hit {
  background: rgba(255, 198, 0, 0.45);
  color: #111111;
  border-radius: 3px;
}

.km-find-hit.current {
  background: #ffc600;
  box-shadow: 0 0 0 1px #f59e0b;
}

.km-sandbox-frame {
  width: 100%;
  min-height: 240px;
  border: none;
  background: #ffffff;
}

.km-card-atts {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  padding-top: 8px;
}

.km-card-att-chip {
  display: inline-flex;
  align-items: stretch;
  border-radius: 8px;
  border: 1px solid var(--km-border);
  background: color-mix(in srgb, var(--km-text-muted) 7%, transparent);
  overflow: hidden;
  font-size: 12px;
}

.km-card-att-open {
  border: none;
  background: transparent;
  color: var(--km-text);
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 5px 10px;
  cursor: pointer;
}

.km-card-att-open:hover {
  background: var(--km-accent-soft);
}

.km-card-att-name {
  max-width: 210px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 500;
}

.km-card-att-size {
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-card-att-dl {
  border: none;
  border-left: 1px solid var(--km-border);
  background: transparent;
  color: var(--km-text-muted);
  padding: 0 9px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}

.km-card-att-dl:hover {
  background: var(--km-accent-soft);
  color: var(--km-text);
}
</style>

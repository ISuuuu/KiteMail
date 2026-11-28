<template>
  <div v-if="undoBanner || outboxItems.length > 0" class="km-outbox-dock">
    <!-- 可展开的发件箱排队/失败小面板 -->
    <div v-if="panelOpen && outboxItems.length > 0" class="km-outbox-panel" role="region" aria-label="发件队列状态">
      <div class="km-outbox-panel-hdr">
        <span class="km-outbox-panel-title">发件队列 ({{ outboxItems.length }})</span>
        <button
          type="button"
          class="km-toast-close"
          aria-label="收起发件队列"
          @click="setPanelOpen(false)"
        >
          ✕
        </button>
      </div>
      <ul class="km-outbox-list">
        <li v-for="item in outboxItems" :key="item.id" class="km-outbox-item">
          <div class="km-outbox-main">
            <div class="km-outbox-top">
              <span class="km-outbox-subj">{{ item.subject || "(无主题)" }}</span>
              <span class="km-outbox-badge" :class="`status-${item.status}`">
                {{ statusLabel(item.status) }}
              </span>
            </div>
            <div class="km-outbox-to">
              收件人：{{ item.toAddr }} · {{ formatSendTime(item.sendAt) }}
            </div>
            <div v-if="item.lastError" class="km-outbox-err">
              {{ item.lastError }}
            </div>
          </div>
          <div class="km-outbox-actions">
            <button
              v-if="item.status === 'failed' || item.status === 'uncertain'"
              type="button"
              class="km-outbox-btn km-outbox-retry"
              :disabled="busyOutboxId === item.id"
              @click="emit('retryItem', item.accountId, item.id)"
            >
              重试
            </button>
            <button
              v-if="item.status !== 'sending'"
              type="button"
              class="km-outbox-btn"
              :disabled="busyOutboxId === item.id"
              @click="emit('deleteItem', item.accountId, item.id)"
            >
              删除
            </button>
          </div>
        </li>
      </ul>
    </div>

    <div class="km-outbox-row">
      <!-- 发件箱队列状态胶囊 -->
      <button
        v-if="outboxItems.length > 0"
        type="button"
        class="km-outbox-pill"
        :class="{ 'has-failed': failedCount > 0 }"
        :aria-expanded="panelOpen"
        @click="setPanelOpen(!panelOpen)"
      >
        <span v-if="failedCount > 0">⚠ 发送失败 {{ failedCount }} 封</span>
        <span v-else>📤 待发队列 {{ outboxItems.length }} 封</span>
      </button>

      <!-- 撤销发送 Toast -->
      <div
        v-if="undoBanner"
        class="km-send-undo-toast"
        role="status"
        aria-live="polite"
      >
        <span class="km-toast-text">
          {{ undoBannerText }}
        </span>
        <span class="km-toast-timer">{{ undoBanner.remainingSecs }}s</span>
        <button
          type="button"
          class="km-toast-undo-btn"
          :disabled="undoing"
          @click="emit('undo')"
        >
          {{ undoing ? "撤销中…" : "撤销发送" }}
        </button>
        <button
          type="button"
          class="km-toast-close"
          title="关闭"
          aria-label="关闭撤销提示"
          @click="emit('dismissUndo')"
        >
          ✕
        </button>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import type { OutboxItemDto, OutboxItemStatus, UndoBannerState } from "./types";

const props = defineProps<{
  undoBanner: UndoBannerState | null;
  outboxItems: OutboxItemDto[];
  undoing?: boolean;
  busyOutboxId?: string | null;
}>();

const emit = defineEmits<{
  undo: [];
  dismissUndo: [];
  retryItem: [accountId: string, outboxId: string];
  deleteItem: [accountId: string, outboxId: string];
  overlayChange: [open: boolean];
}>();

const panelOpen = ref(false);

const failedCount = computed(
  () =>
    props.outboxItems.filter(
      (i) => i.status === "failed" || i.status === "uncertain"
    ).length
);

const undoBannerText = computed(() => {
  if (!props.undoBanner) return "";
  if (props.undoBanner.scheduledAt) {
    return `已排期发送至 ${props.undoBanner.to}`;
  }
  return `正在发送至 ${props.undoBanner.to}`;
});

function setPanelOpen(next: boolean) {
  if (panelOpen.value === next) return;
  panelOpen.value = next;
  emit("overlayChange", next);
}

function statusLabel(status: OutboxItemStatus): string {
  switch (status) {
    case "queued":
      return "排队中";
    case "sending":
      return "发送中";
    case "uncertain":
      return "待确认";
    case "failed":
      return "失败";
  }
}

function formatSendTime(epochSecs: number): string {
  const d = new Date(epochSecs * 1000);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `${hh}:${mm}`;
}

watch(
  () => props.outboxItems.length,
  (len) => {
    if (len === 0 && panelOpen.value) {
      setPanelOpen(false);
    }
  }
);

onUnmounted(() => {
  if (panelOpen.value) {
    emit("overlayChange", false);
  }
});
</script>

<style scoped>
.km-outbox-dock {
  position: fixed;
  right: 16px;
  bottom: 16px;
  z-index: 60;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 8px;
  max-width: min(460px, calc(100vw - 32px));
}

.km-outbox-row {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  flex-wrap: wrap;
  gap: 8px;
}

.km-outbox-pill {
  height: 32px;
  padding: 0 12px;
  border-radius: 999px;
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  color: var(--km-text);
  box-shadow: var(--km-shadow-card);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
}

.km-outbox-pill:hover {
  border-color: var(--km-accent);
}

.km-outbox-pill.has-failed {
  border-color: color-mix(in srgb, var(--km-danger) 55%, var(--km-border));
  color: var(--km-danger);
}

.km-send-undo-toast {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  color: var(--km-text);
  box-shadow: var(--km-shadow-popover);
  font-size: 12px;
}

.km-toast-text {
  font-weight: 500;
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-toast-timer {
  font-family: var(--km-font-mono);
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-toast-undo-btn {
  border: none;
  border-radius: var(--km-radius-sm);
  padding: 4px 10px;
  background: var(--km-accent);
  color: var(--km-accent-fg);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
}

.km-toast-undo-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.km-toast-close {
  border: none;
  background: transparent;
  color: var(--km-text-muted);
  cursor: pointer;
  padding: 2px 4px;
  font-size: 11px;
}

.km-toast-close:hover {
  color: var(--km-text);
}

.km-outbox-panel {
  width: min(400px, calc(100vw - 32px));
  max-height: 280px;
  display: flex;
  flex-direction: column;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-md);
  background: var(--km-surface-raised);
  box-shadow: var(--km-shadow-popover);
  overflow: hidden;
}

.km-outbox-panel-hdr {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  border-bottom: 1px solid var(--km-border);
  background: var(--km-sidebar);
}

.km-outbox-panel-title {
  font-size: 12px;
  font-weight: 600;
}

.km-outbox-list {
  list-style: none;
  margin: 0;
  padding: 6px;
  overflow: auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.km-outbox-item {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 8px;
  padding: 8px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface);
}

.km-outbox-main {
  flex: 1 1 auto;
  min-width: 0;
}

.km-outbox-top {
  display: flex;
  align-items: center;
  gap: 6px;
}

.km-outbox-subj {
  font-size: 12px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-outbox-badge {
  font-size: 10px;
  padding: 1px 6px;
  border-radius: 999px;
  background: var(--km-accent-soft);
  color: var(--km-accent);
  flex-shrink: 0;
}

.km-outbox-badge.status-failed,
.km-outbox-badge.status-uncertain {
  background: var(--km-danger-soft);
  color: var(--km-danger);
}

.km-outbox-to {
  font-size: 11px;
  color: var(--km-text-muted);
  margin-top: 2px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-outbox-err {
  font-size: 11px;
  color: var(--km-danger);
  margin-top: 3px;
  word-break: break-word;
}

.km-outbox-actions {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
}

.km-outbox-btn {
  height: 24px;
  padding: 0 8px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface-raised);
  font-size: 11px;
  cursor: pointer;
}

.km-outbox-btn:hover:not(:disabled) {
  background: var(--km-hover-bg);
}

.km-outbox-retry {
  border-color: var(--km-accent);
  color: var(--km-accent);
}
</style>

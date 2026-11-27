<template>
  <div
    role="option"
    tabindex="0"
    class="km-message-row"
    :class="{
      selected,
      unread: message.unread,
      wide,
      compact: !wide && compact,
    }"
    :aria-selected="selected"
    @click="emit('select', { accountId: message.accountId, messageId: message.id })"
    @keydown.enter.prevent="emit('select', { accountId: message.accountId, messageId: message.id })"
    @keydown.space.prevent="emit('select', { accountId: message.accountId, messageId: message.id })"
  >
    <!-- 单行宽表模式 (>= 600px) -->
    <template v-if="wide">
      <button
        type="button"
        class="km-row-action-btn km-unread-slot"
        :title="message.unread ? '标为已读' : '标为未读'"
        :aria-label="message.unread ? '标为已读' : '标为未读'"
        @click.stop="emit('toggleSeen', { accountId: message.accountId, messageId: message.id }, message.unread)"
      >
        <span class="km-unread-dot" :class="{ read: !message.unread }" />
      </button>
      <button
        type="button"
        class="km-row-action-btn km-star-slot"
        :class="{ flagged: message.flagged }"
        :title="message.flagged ? '取消星标' : '加上星标'"
        :aria-label="message.flagged ? '取消星标' : '加上星标'"
        @click.stop="emit('toggleFlagged', { accountId: message.accountId, messageId: message.id }, !message.flagged)"
      >
        {{ message.flagged ? "★" : "☆" }}
      </button>
      <span class="km-wide-sender">{{ senderDisplay }}</span>
      <span class="km-wide-subject-wrap">
        <span class="km-wide-subject">{{ message.subject || "(无主题)" }}</span>
        <span v-if="message.preview" class="km-wide-preview"> — {{ message.preview }}</span>
      </span>
      <span class="km-wide-meta">
        <span v-if="message.threadCount > 1" class="km-thread-chip">{{ message.threadCount }}</span>
        <span v-if="message.hasAttachment" class="km-attach-icon" title="含附件">📎</span>
        <span class="km-date-text">{{ message.dateDisplay }}</span>
      </span>
    </template>

    <!-- 卡片模式 (< 560px 退回) -->
    <template v-else>
      <div v-if="showAvatar" class="km-row-avatar" :style="{ backgroundColor: avatarBg }">
        {{ avatarInitial }}
      </div>
      <button
        type="button"
        class="km-row-action-btn km-unread-slot"
        :title="message.unread ? '标为已读' : '标为未读'"
        :aria-label="message.unread ? '标为已读' : '标为未读'"
        @click.stop="emit('toggleSeen', { accountId: message.accountId, messageId: message.id }, message.unread)"
      >
        <span class="km-unread-dot" :class="{ read: !message.unread }" />
      </button>
      <div class="km-card-content">
        <div class="km-card-top">
          <span class="km-card-sender">{{ senderDisplay }}</span>
          <span class="km-card-badges">
            <span v-if="message.threadCount > 1" class="km-thread-chip">{{ message.threadCount }}</span>
            <span v-if="message.hasAttachment" class="km-attach-icon" title="含附件">📎</span>
            <button
              type="button"
              class="km-row-action-btn km-star-slot km-card-star"
              :class="{ flagged: message.flagged }"
              :title="message.flagged ? '取消星标' : '加上星标'"
              :aria-label="message.flagged ? '取消星标' : '加上星标'"
              @click.stop="emit('toggleFlagged', { accountId: message.accountId, messageId: message.id }, !message.flagged)"
            >
              {{ message.flagged ? "★" : "☆" }}
            </button>
            <span class="km-date-text">{{ message.dateDisplay }}</span>
          </span>
        </div>
        <div class="km-card-subject">
          <span class="km-subject-text">{{ message.subject || "(无主题)" }}</span>
          <span
            v-for="tag in message.tags.slice(0, 2)"
            :key="tag"
            class="km-tag-chip"
          >
            {{ tag }}
          </span>
        </div>
        <div v-if="!compact" class="km-card-preview">
          {{ message.preview || " " }}
        </div>
      </div>
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed } from "vue";
import type { MessageRef, MessageSummaryDto } from "../ipc/types";

const props = defineProps<{
  message: MessageSummaryDto;
  selected: boolean;
  wide: boolean;
  compact: boolean;
  showAvatar: boolean;
}>();

const emit = defineEmits<{
  select: [ref: MessageRef];
  toggleSeen: [ref: MessageRef, seen: boolean];
  toggleFlagged: [ref: MessageRef, flagged: boolean];
}>();

const AVATAR_COLORS = [
  "#3584e4",
  "#26a269",
  "#9141ac",
  "#c64600",
  "#1c71d8",
  "#a51d2d",
  "#613583",
];

const senderDisplay = computed(() => {
  const raw = props.message.sender.trim();
  const idx = raw.indexOf("<");
  if (idx > 0) {
    return raw.slice(0, idx).trim().replace(/^"|"$/g, "");
  }
  return raw || "(未知发件人)";
});

const avatarInitial = computed(() => {
  const ch = senderDisplay.value.charAt(0);
  return ch ? ch.toUpperCase() : "?";
});

const avatarBg = computed(() => {
  let hash = 0;
  for (let i = 0; i < senderDisplay.value.length; i++) {
    hash = (hash * 31 + senderDisplay.value.charCodeAt(i)) | 0;
  }
  return AVATAR_COLORS[Math.abs(hash) % AVATAR_COLORS.length];
});
</script>

<style scoped>
.km-message-row {
  width: calc(100% - 12px);
  margin: 2px 6px;
  border: 1px solid transparent;
  border-radius: var(--km-radius-lg);
  background: transparent;
  color: var(--km-text);
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 10px 10px 8px;
  text-align: left;
  cursor: pointer;
  overflow: hidden;
}

.km-message-row.compact {
  padding: 8px 10px 8px 8px;
  border-radius: var(--km-radius-md);
}

.km-message-row.wide {
  width: calc(100% - 8px);
  margin: 1px 4px;
  height: 36px;
  padding: 0 10px;
  border-radius: var(--km-radius-sm);
}

.km-message-row:hover {
  background: var(--km-hover-bg);
}

.km-message-row.selected {
  background: var(--km-accent-soft);
  border-color: color-mix(in srgb, var(--km-accent) 36%, transparent);
}

.km-message-row.selected:hover {
  background: var(--km-accent-soft-hover);
}

.km-row-action-btn {
  border: none;
  background: transparent;
  padding: 0;
  margin: 0;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--km-radius-sm);
}

.km-unread-slot {
  width: 14px;
  height: 18px;
  flex-shrink: 0;
}

.km-unread-dot {
  width: 8px;
  height: 8px;
  border-radius: 999px;
  background: var(--km-accent);
  transition: opacity 120ms ease, transform 120ms ease;
}

.km-unread-dot.read {
  background: transparent;
  border: 1.5px solid var(--km-text-muted);
  opacity: 0;
}

.km-message-row:hover .km-unread-dot.read,
.km-unread-slot:focus-visible .km-unread-dot.read {
  opacity: 0.55;
}

.km-card-star:not(.flagged) {
  opacity: 0;
}

.km-message-row:hover .km-card-star:not(.flagged),
.km-card-star:focus-visible {
  opacity: 0.65;
}

.km-row-avatar {
  width: 34px;
  height: 34px;
  border-radius: 999px;
  color: #ffffff;
  font-size: 13px;
  font-weight: 700;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.km-card-content {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.km-card-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.km-card-sender {
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-message-row.unread .km-card-sender,
.km-message-row.unread .km-card-subject,
.km-message-row.unread .km-wide-sender,
.km-message-row.unread .km-wide-subject {
  font-weight: 700;
}

.km-card-badges,
.km-wide-meta {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  flex-shrink: 0;
}

.km-card-subject {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.km-subject-text {
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-card-preview {
  font-size: 12px;
  color: var(--km-text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-tag-chip,
.km-thread-chip {
  font-size: 11px;
  padding: 0 6px;
  height: 17px;
  border-radius: 999px;
  background: var(--km-accent-soft);
  color: var(--km-accent);
  display: inline-flex;
  align-items: center;
  flex-shrink: 0;
}

.km-star-slot {
  font-size: 13px;
  color: var(--km-text-muted);
  flex-shrink: 0;
}

.km-star-slot.flagged {
  color: var(--km-star-gold);
}

.km-attach-icon {
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-date-text {
  font-size: 11px;
  color: var(--km-text-muted);
  white-space: nowrap;
}

.km-wide-sender {
  width: 168px;
  flex-shrink: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-wide-subject-wrap {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-wide-preview {
  color: var(--km-text-muted);
  font-weight: 400;
}
</style>

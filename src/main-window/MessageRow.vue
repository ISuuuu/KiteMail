<template>
  <div class="km-row-shell" :class="{ 'is-child': isThreadChild }">
    <div v-if="isThreadChild" class="km-thread-rail" aria-hidden="true">
      <span class="km-thread-rail-line" :class="{ last: isLastThreadChild }" />
      <span class="km-thread-rail-dot" />
    </div>

    <div
      role="option"
      tabindex="0"
      class="km-message-row"
      :class="{
        selected,
        unread: message.unread,
        wide,
        compact: !wide && compact,
        child: isThreadChild,
      }"
      :style="rowInlineStyle"
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
          <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
            <path
              v-if="message.flagged"
              fill="currentColor"
              d="m8 1.9 1.88 3.81 4.2.61-3.04 2.96.72 4.19L8 11.49l-3.76 1.98.72-4.19L1.92 6.32l4.2-.61L8 1.9Z"
            />
            <path
              v-else
              fill="none"
              stroke="currentColor"
              stroke-width="1.3"
              stroke-linejoin="round"
              d="m8 2.1 1.78 3.6 3.97.58-2.87 2.8.68 3.95L8 11.16l-3.56 1.87.68-3.95-2.87-2.8 3.97-.58L8 2.1Z"
            />
          </svg>
        </button>

        <span class="km-wide-sender">{{ senderDisplay }}</span>

        <span class="km-wide-subject-wrap">
          <span class="km-wide-subject">{{ message.subject || "(无主题)" }}</span>
          <span v-if="message.preview" class="km-wide-preview"> — {{ message.preview }}</span>
        </span>

        <span class="km-wide-meta">
          <span
            v-for="tag in message.tags.slice(0, 2)"
            :key="tag"
            class="km-tag-chip"
            :style="tagStyle(tag)"
          >
            {{ tag }}
          </span>
          <button
            v-if="message.threadCount > 1"
            type="button"
            class="km-thread-chip"
            :class="{ expanded: threadExpanded }"
            :aria-expanded="threadExpanded"
            :title="threadExpanded ? '收起会话邮件' : '展开会话邮件'"
            @click.stop="emit('toggleThread', message)"
          >
            {{ message.threadCount }}
          </button>
          <span v-if="message.hasAttachment" class="km-attach-icon" title="含附件" aria-label="含附件">
            <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
              <path
                fill="none"
                stroke="currentColor"
                stroke-width="1.4"
                stroke-linecap="round"
                stroke-linejoin="round"
                d="m11.5 5.2-4.7 4.7a2 2 0 1 1-2.8-2.8l5-5a3.2 3.2 0 0 1 4.5 4.5l-5.3 5.3a4.3 4.3 0 0 1-6.1-6.1l4.4-4.4"
              />
            </svg>
          </span>
          <span class="km-date-text">{{ message.dateDisplay }}</span>
        </span>
      </template>

      <!-- 卡片模式 (< 560px 退回) -->
      <template v-else>
        <div
          v-if="showAvatar"
          class="km-row-avatar-ring"
          :style="{ borderColor: selected ? 'rgba(255,255,255,0.85)' : avatarBg }"
        >
          <div class="km-row-avatar" :style="{ backgroundColor: avatarBg }">
            {{ avatarInitial }}
          </div>
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
              <span v-if="message.answered || message.forwarded" class="km-reply-icon" :title="message.forwarded ? '已转发' : '已回复'">
                <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
                  <path
                    v-if="message.forwarded"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.4"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    d="m9.5 4 4 3.5-4 3.5M13.5 7.5H6a3.5 3.5 0 0 0-3.5 3.5V12"
                  />
                  <path
                    v-else
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.4"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    d="m6.5 4-4 3.5 4 3.5M2.5 7.5H10a3.5 3.5 0 0 1 3.5 3.5V12"
                  />
                </svg>
              </span>

              <button
                type="button"
                class="km-row-action-btn km-star-slot km-card-star"
                :class="{ flagged: message.flagged }"
                :title="message.flagged ? '取消星标' : '加上星标'"
                :aria-label="message.flagged ? '取消星标' : '加上星标'"
                @click.stop="emit('toggleFlagged', { accountId: message.accountId, messageId: message.id }, !message.flagged)"
              >
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
                  <path
                    v-if="message.flagged"
                    fill="currentColor"
                    d="m8 1.9 1.88 3.81 4.2.61-3.04 2.96.72 4.19L8 11.49l-3.76 1.98.72-4.19L1.92 6.32l4.2-.61L8 1.9Z"
                  />
                  <path
                    v-else
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.3"
                    stroke-linejoin="round"
                    d="m8 2.1 1.78 3.6 3.97.58-2.87 2.8.68 3.95L8 11.16l-3.56 1.87.68-3.95-2.87-2.8 3.97-.58L8 2.1Z"
                  />
                </svg>
              </button>

              <span v-if="message.hasAttachment" class="km-attach-icon" title="含附件" aria-label="含附件">
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
                  <path
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.4"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    d="m11.5 5.2-4.7 4.7a2 2 0 1 1-2.8-2.8l5-5a3.2 3.2 0 0 1 4.5 4.5l-5.3 5.3a4.3 4.3 0 0 1-6.1-6.1l4.4-4.4"
                  />
                </svg>
              </span>

              <span class="km-date-text">{{ message.dateDisplay }}</span>

              <button
                v-if="message.threadCount > 1"
                type="button"
                class="km-thread-chip"
                :class="{ expanded: threadExpanded }"
                :aria-expanded="threadExpanded"
                :title="threadExpanded ? '收起会话邮件' : '展开会话邮件'"
                @click.stop="emit('toggleThread', message)"
              >
                <span>{{ message.threadCount }}</span>
                <svg
                  v-if="threadExpanded"
                  viewBox="0 0 12 12"
                  width="9"
                  height="9"
                  aria-hidden="true"
                >
                  <path
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.6"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    d="M3 4.5 6 7.5l3-3"
                  />
                </svg>
              </button>
            </span>
          </div>

          <div class="km-card-subject">
            <span class="km-subject-text">{{ message.subject || "(无主题)" }}</span>
            <span
              v-for="tag in message.tags.slice(0, 2)"
              :key="tag"
              class="km-tag-chip"
              :style="tagStyle(tag)"
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
  </div>
</template>

<script setup lang="ts">
import { computed } from "vue";
import type { MessageRef, MessageSummaryDto, TagItemDto } from "../ipc/types";
import {
  avatarColorForKey,
  resolveMessageRowTint,
  resolveTagPillStyle,
  senderDisplayName,
  senderEmailAddress,
  senderInitials,
} from "./parityHelpers";

const props = withDefaults(
  defineProps<{
    message: MessageSummaryDto;
    selected: boolean;
    wide: boolean;
    compact: boolean;
    showAvatar: boolean;
    effectiveDark?: boolean;
    tagsMeta?: TagItemDto[];
    threadExpanded?: boolean;
    isThreadChild?: boolean;
    isLastThreadChild?: boolean;
  }>(),
  {
    effectiveDark: false,
    tagsMeta: () => [],
    threadExpanded: false,
    isThreadChild: false,
    isLastThreadChild: false,
  }
);

const emit = defineEmits<{
  select: [ref: MessageRef];
  toggleSeen: [ref: MessageRef, seen: boolean];
  toggleFlagged: [ref: MessageRef, flagged: boolean];
  toggleThread: [message: MessageSummaryDto];
}>();

const senderDisplay = computed(() => senderDisplayName(props.message.sender));
const avatarInitial = computed(() => senderInitials(senderDisplay.value));
const avatarBg = computed(() => {
  const email = senderEmailAddress(props.message.sender);
  return avatarColorForKey(email || senderDisplay.value);
});

const rowInlineStyle = computed(() => {
  const tint = resolveMessageRowTint(props.message.tags, props.selected, props.effectiveDark);
  return tint ? { backgroundColor: tint } : undefined;
});

function tagStyle(tag: string) {
  return resolveTagPillStyle(tag, props.selected, props.tagsMeta);
}
</script>

<style scoped>
.km-row-shell {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: stretch;
}

.km-row-shell.is-child {
  padding-left: 12px;
}

.km-thread-rail {
  width: 14px;
  flex-shrink: 0;
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
}

.km-thread-rail-line {
  position: absolute;
  left: 6px;
  top: 0;
  bottom: 0;
  width: 1.5px;
  background: color-mix(in srgb, var(--km-text-muted) 35%, transparent);
}

.km-thread-rail-line.last {
  bottom: 50%;
}

.km-thread-rail-dot {
  position: relative;
  z-index: 1;
  width: 7px;
  height: 7px;
  border-radius: 999px;
  background: var(--km-text-muted);
  border: 1.5px solid var(--km-surface, #fafafb);
}

.km-message-row {
  flex: 1 1 auto;
  min-width: 0;
  height: calc(100% - 4px);
  margin: 2px 6px;
  border: none;
  border-radius: 12px;
  background: transparent;
  color: var(--km-text);
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 9px 10px 9px 8px;
  text-align: left;
  cursor: pointer;
  overflow: hidden;
  transition: background-color 100ms ease;
}

.km-message-row.compact {
  border-radius: 10px;
  padding: 7px 10px 7px 8px;
}

.km-message-row.wide {
  margin: 1px 6px;
  height: calc(100% - 2px);
  padding: 0 10px;
  border-radius: 8px;
}

.km-message-row:hover {
  background: var(--km-hover-bg);
}

.km-message-row.selected {
  background: #3584e4 !important;
  color: #ffffff;
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
  width: 12px;
  height: 18px;
  flex-shrink: 0;
}

.km-unread-dot {
  width: 9px;
  height: 9px;
  border-radius: 999px;
  background: #3584e4;
  transition: opacity 120ms ease, transform 120ms ease;
}

.km-message-row.selected .km-unread-dot:not(.read) {
  background: #ffffff;
}

.km-unread-dot.read {
  background: transparent;
  border: 1.5px solid var(--km-text-muted);
  opacity: 0;
}

.km-message-row.selected .km-unread-dot.read {
  border-color: rgba(255, 255, 255, 0.75);
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

/* 44px 圆环头像 */
.km-row-avatar-ring {
  width: 44px;
  height: 44px;
  border-radius: 999px;
  border: 2px solid #3584e4;
  padding: 2px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.km-row-avatar {
  width: 100%;
  height: 100%;
  border-radius: 999px;
  color: #ffffff;
  font-size: 13px;
  font-weight: 700;
  display: inline-flex;
  align-items: center;
  justify-content: center;
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
  gap: 6px;
}

.km-card-sender {
  font-size: 13.5px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-message-row.unread .km-card-sender,
.km-message-row.unread .km-wide-sender {
  font-weight: 700;
}

.km-message-row.unread .km-subject-text,
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
  justify-content: space-between;
  gap: 6px;
  min-width: 0;
}

.km-subject-text {
  flex: 1 1 auto;
  min-width: 0;
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

.km-message-row.selected .km-card-preview,
.km-message-row.selected .km-wide-preview {
  color: rgba(255, 255, 255, 0.88);
}

.km-tag-chip {
  font-size: 10.5px;
  font-weight: 600;
  padding: 0 7px;
  height: 18px;
  border-radius: 999px;
  display: inline-flex;
  align-items: center;
  flex-shrink: 0;
}

.km-thread-chip {
  border: none;
  font-size: 11px;
  font-weight: 700;
  padding: 0 7px;
  min-width: 22px;
  height: 18px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--km-text) 10%, transparent);
  color: var(--km-text-muted);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 3px;
  flex-shrink: 0;
  cursor: pointer;
}

.km-thread-chip:hover {
  background: color-mix(in srgb, var(--km-text) 18%, transparent);
  color: var(--km-text);
}

.km-thread-chip.expanded {
  background: color-mix(in srgb, var(--km-accent) 20%, transparent);
  color: var(--km-accent);
}

.km-message-row.selected .km-thread-chip {
  background: #ffffff;
  color: #3584e4;
}

.km-star-slot {
  color: var(--km-text-muted);
  flex-shrink: 0;
}

.km-star-slot.flagged {
  color: var(--km-star-gold, #e5a50a);
}

.km-message-row.selected .km-star-slot:not(.flagged) {
  color: rgba(255, 255, 255, 0.85);
}

.km-attach-icon,
.km-reply-icon {
  display: inline-flex;
  align-items: center;
  color: var(--km-text-muted);
}

.km-date-text {
  font-size: 11px;
  color: var(--km-text-muted);
  white-space: nowrap;
}

.km-message-row.selected .km-attach-icon,
.km-message-row.selected .km-reply-icon,
.km-message-row.selected .km-date-text {
  color: rgba(255, 255, 255, 0.9);
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

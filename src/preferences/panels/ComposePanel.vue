<template>
  <section class="km-tab-panel">
    <h3 class="km-tab-title">写信、撤销发送与阅读行为</h3>

    <div class="km-card">
      <div class="km-field-row">
        <span class="km-label" id="km-pref-read-mark-label">自动标记已读策略</span>
        <ComboSelect
          :options="READ_MARK_OPTIONS"
          :modelValue="state.preferences.value.readMarkMode"
          aria-label="自动标记已读策略"
          @update:modelValue="(v) => onSelectStringValue(state, 'readMarkMode', v)"
        />
      </div>

      <label class="km-toggle-row">
        <input
          type="checkbox"
          :checked="state.preferences.value.threadNewestFirst"
          @change="onCheckbox(state, 'threadNewestFirst', $event)"
        />
        <span>会话卡片按最新回复置顶排序</span>
      </label>

      <div class="km-field-row">
        <label for="km-pref-undo-secs" class="km-label">撤销发送缓冲窗口（秒，0-30）</label>
        <input
          id="km-pref-undo-secs"
          type="number"
          min="0"
          max="30"
          class="km-input km-num-input"
          :value="state.preferences.value.undoSendSecs"
          @change="onNumberInput(state, 'undoSendSecs', $event)"
        />
      </div>

      <label class="km-toggle-row">
        <input
          type="checkbox"
          :checked="state.preferences.value.signatureAboveQuote"
          @change="onCheckbox(state, 'signatureAboveQuote', $event)"
        />
        <span>回复/转发时将签名置于引用正文上方</span>
      </label>
      <label class="km-toggle-row">
        <input
          type="checkbox"
          :checked="state.preferences.value.signatureDashes"
          @change="onCheckbox(state, 'signatureDashes', $event)"
        />
        <span>在签名前自动添加 `-- ` 标准分隔符</span>
      </label>

      <div class="km-field">
        <label for="km-pref-attach-kw" class="km-label">遗漏附件提醒关键词（逗号分隔）</label>
        <input
          id="km-pref-attach-kw"
          type="text"
          class="km-input"
          :value="state.preferences.value.attachmentReminderKeywords"
          @change="onTextInput(state, 'attachmentReminderKeywords', $event)"
        />
      </div>

      <div class="km-field">
        <label for="km-pref-reply-tpl" class="km-label">回复引用抬头模板</label>
        <input
          id="km-pref-reply-tpl"
          type="text"
          class="km-input"
          :value="state.preferences.value.replyHeaderTemplate"
          @change="onTextInput(state, 'replyHeaderTemplate', $event)"
        />
      </div>
    </div>
  </section>
</template>

<script setup lang="ts">
import ComboSelect from "../../settings/ComboSelect.vue";
import { onCheckbox, onNumberInput, onSelectStringValue, onTextInput } from "./fieldBinders";
import type { PrefsPanelState } from "./state";

defineProps<{ state: PrefsPanelState }>();

const READ_MARK_OPTIONS = [
  { label: "打开邮件立即标为已读", value: "shown" },
  { label: "停留 2 秒后标为已读", value: "delay" },
  { label: "仅手动标记已读", value: "manual" },
] as const;
</script>

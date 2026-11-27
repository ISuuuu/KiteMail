<template>
  <div class="km-sp-page">
    <!-- 保存反馈条（对齐 GPUI compose_inputs_feedback：accent 12% 圆角提示） -->
    <div v-if="feedback" class="km-sp-feedback" role="status">
      {{ feedback }}
    </div>

    <!-- 分组一：发件身份与签名 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">发件身份与签名</h4>
      <div class="km-sp-card">
        <div class="km-sp-row" id="sp-compose-default-from">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">新邮件默认发件账号</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="fromOptions"
              :modelValue="composeDefaultFrom"
              title="新邮件默认发件账号"
              @update:modelValue="composeDefaultFrom = $event"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-compose-inline">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">在主窗口中新建邮件</span>
            <span class="km-sp-row-desc">关闭时点击写信将在独立窗口中打开新邮件</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch v-model="composingState.composeInline" />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-reply-inline">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">在主窗口中回复与转发</span>
            <span class="km-sp-row-desc">关闭时回复与转发将在独立窗口中打开；开启时在主窗口分屏对照原邮件</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch v-model="composingState.replyInline" />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-signature-position">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">回复与转发时的签名位置</span>
            <span class="km-sp-row-desc">默认在引用内容上方，使新写正文与签名紧靠在一起</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="SIGNATURE_POSITION_OPTIONS"
              :modelValue="state.preferences.value.signatureAboveQuote ? 'above' : 'below'"
              title="回复与转发时的签名位置"
              @update:modelValue="setSignaturePosition"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-signatureDashes">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">签名前插入 "-- " 分隔符</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch
              :modelValue="state.preferences.value.signatureDashes"
              @update:modelValue="setBoolPref('signatureDashes', $event)"
            />
          </div>
        </div>
      </div>
    </section>

    <!-- 分组二：发送与引用头格式 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">发送与引用头格式</h4>
      <div class="km-sp-card">
        <div class="km-sp-row" id="sp-undo-send">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">撤销发送缓冲时间</span>
            <span class="km-sp-row-desc">点击发送后在指定秒数内可通过底部提示条「撤销发送」中止发出</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="UNDO_SEND_OPTIONS"
              :modelValue="String(state.preferences.value.undoSendSecs)"
              title="撤销发送缓冲时间"
              @update:modelValue="setUndoSend"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-edit-row" id="sp-reply-header">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">自定义回复引用头模板</span>
            <span class="km-sp-row-desc">
              支持 {name}、{address}、{date}、{time}、{subject}；留空即用默认中文格式
            </span>
          </div>
          <div class="km-sp-edit-inputs">
            <input
              v-model="replyHeaderInput"
              type="text"
              class="km-sp-input"
              :placeholder="'On {date} {time}, {name} <{address}> wrote:'"
              aria-label="自定义回复引用头模板"
              @input="replyHeaderEdited = true"
            />
            <button type="button" class="km-sp-primary-btn" @click="applyReplyHeader">应用</button>
            <button type="button" class="km-sp-outline-btn" @click="resetReplyHeader">恢复默认</button>
          </div>
        </div>
      </div>
    </section>

    <!-- 分组三：附件遗漏检测 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">附件遗漏检测</h4>
      <p class="km-sp-desc">
        多个关键词用逗号分隔；若发件主题或输入正文中出现关键词但附件列表为空，将在发送前提醒
      </p>
      <div class="km-sp-card">
        <div class="km-sp-edit-row" id="sp-attach-keywords">
          <div class="km-sp-edit-inputs">
            <input
              v-model="attachmentKeywordsInput"
              type="text"
              class="km-sp-input"
              aria-label="附件遗漏检测关键词"
              @input="keywordsEdited = true"
            />
            <button type="button" class="km-sp-primary-btn" @click="applyKeywords">应用</button>
            <button type="button" class="km-sp-outline-btn" @click="resetKeywords">恢复默认</button>
          </div>
          <div class="km-sp-row-text">
            <span class="km-sp-row-desc">默认列表：{{ DEFAULT_ATTACHMENT_KEYWORDS }}</span>
          </div>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import "./page.css";
import { onMounted, reactive, ref, watch } from "vue";
import ComboSelect from "./ComboSelect.vue";
import PillSwitch from "./PillSwitch.vue";
import { listAccounts } from "../accounts/ipc";
import type { FullPreferencesDto } from "../preferences/types";
import type { PrefsPanelState } from "../preferences/panels/state";

/**
 * 「写信」页（对齐 GPUI pages.rs#render_composing）。
 * 签名位置/分隔符/撤销时间/引用头模板/附件关键词直连 IPC；
 * 默认发件账号下拉（会话内状态）选项来自 list_accounts 已启用账号，
 * 新建/回复主窗口开关为会话内状态。
 */
const props = defineProps<{ state: PrefsPanelState }>();

const AUTO_FROM_LABEL = "自动（使用当前查看账号）";
const DEFAULT_ATTACHMENT_KEYWORDS =
  "附件, 附上, 随信, 见附, attachment, attached, enclose";

const SIGNATURE_POSITION_OPTIONS = [
  { label: "在引用内容上方", value: "above" },
  { label: "在引用内容下方", value: "below" },
] as const;

const UNDO_SEND_OPTIONS = [
  { label: "关闭（立即发送）", value: "0" },
  { label: "3 秒", value: "3" },
  { label: "5 秒（推荐）", value: "5" },
  { label: "10 秒", value: "10" },
  { label: "20 秒", value: "20" },
  { label: "30 秒", value: "30" },
] as const;

// ---- 会话内状态（对齐 mail-core 默认值） ----
const composingState = reactive<Record<string, boolean>>({
  composeInline: true,
  replyInline: true,
});
const composeDefaultFrom = ref("");
const fromOptions = ref<{ label: string; value: string }[]>([
  { label: AUTO_FROM_LABEL, value: "" },
]);

// ---- 文本编辑与保存反馈 ----
const feedback = ref<string | null>(null);
const replyHeaderInput = ref(props.state.preferences.value.replyHeaderTemplate);
const attachmentKeywordsInput = ref(
  props.state.preferences.value.attachmentReminderKeywords
);

// 偏好异步加载完成后同步未编辑的输入初值（用户编辑过则不覆盖）。
const replyHeaderEdited = ref(false);
const keywordsEdited = ref(false);
watch(
  () => props.state.preferences.value.replyHeaderTemplate,
  (value) => {
    if (!replyHeaderEdited.value) replyHeaderInput.value = value;
  }
);
watch(
  () => props.state.preferences.value.attachmentReminderKeywords,
  (value) => {
    if (!keywordsEdited.value) attachmentKeywordsInput.value = value;
  }
);

let feedbackTimer: ReturnType<typeof setTimeout> | null = null;
function showFeedback(text: string) {
  feedback.value = text;
  if (feedbackTimer) clearTimeout(feedbackTimer);
  feedbackTimer = setTimeout(() => {
    feedback.value = null;
  }, 2600);
}

function applyReplyHeader() {
  const value = replyHeaderInput.value.trim();
  void props.state.updatePreference({ replyHeaderTemplate: value });
  showFeedback("已应用自定义回复引用头模板");
}

function resetReplyHeader() {
  replyHeaderInput.value = "";
  void props.state.updatePreference({ replyHeaderTemplate: "" });
  showFeedback("已恢复默认中文回复引用头");
}

function applyKeywords() {
  const value = attachmentKeywordsInput.value.trim();
  void props.state.updatePreference({ attachmentReminderKeywords: value });
  showFeedback("已更新附件遗漏检测关键词");
}

function resetKeywords() {
  attachmentKeywordsInput.value = DEFAULT_ATTACHMENT_KEYWORDS;
  void props.state.updatePreference({
    attachmentReminderKeywords: DEFAULT_ATTACHMENT_KEYWORDS,
  });
  showFeedback("已恢复默认附件检测关键词");
}

function setSignaturePosition(value: string) {
  void props.state.updatePreference({
    signatureAboveQuote: value === "above",
  } as Partial<FullPreferencesDto>);
}

function setBoolPref(field: "signatureDashes", value: boolean) {
  void props.state.updatePreference({ [field]: value } as Partial<FullPreferencesDto>);
}

function setUndoSend(value: string) {
  const num = Number(value);
  if (!Number.isFinite(num)) return;
  void props.state.updatePreference({ undoSendSecs: num });
}

// 默认发件账号下拉：自动 + 已启用账号（list_accounts 只读；失败时仅保留自动）。
onMounted(async () => {
  try {
    const res = await listAccounts();
    const options = res.accounts
      .filter((acc) => acc.enabled)
      .map((acc) => {
        const primary =
          (acc.label || acc.displayName || "").trim() || acc.email;
        const label =
          primary === acc.email ? acc.email : `${primary} <${acc.email}>`;
        return { label, value: acc.email };
      });
    fromOptions.value = [{ label: AUTO_FROM_LABEL, value: "" }, ...options];
  } catch {
    /* 锁屏或账号 IPC 不可用时仅保留「自动」选项 */
  }
});
</script>

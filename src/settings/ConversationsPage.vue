<template>
  <div class="km-sp-page">
    <!-- 分组一：会话线程与卡片显示 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">会话线程与卡片显示</h4>
      <div class="km-sp-card">
        <div class="km-sp-row">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">按最新回复置顶排列会话卡片</span>
            <span class="km-sp-row-desc">关闭则按历史顺序从最早一封向下浏览</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch
              :modelValue="state.preferences.value.threadNewestFirst"
              @update:modelValue="setThreadNewestFirst"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">自动折叠多封会话卡片</span>
            <span class="km-sp-row-desc">切换后会立即重新应用当前打开会话的默认折叠状态</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="FOLD_MODE_OPTIONS"
              :modelValue="foldMode"
              title="自动折叠多封会话卡片"
              @update:modelValue="foldMode = $event"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">在各会话卡片中展示附件</span>
            <span class="km-sp-row-desc">每张卡片单独列出其附件预览、大小与保存动作</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch v-model="cardAttachments" />
          </div>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import "./page.css";
import { ref } from "vue";
import ComboSelect from "./ComboSelect.vue";
import PillSwitch from "./PillSwitch.vue";
import type { FullPreferencesDto } from "../preferences/types";
import type { PrefsPanelState } from "../preferences/panels/state";

/**
 * 「会话」页（对齐 GPUI pages.rs#render_conversations）。
 * 排序开关直连 threadNewestFirst；折叠策略与卡片附件为会话内状态。
 */
const props = defineProps<{ state: PrefsPanelState }>();

const FOLD_MODE_OPTIONS = [
  { label: "全部展开（不折叠）", value: "never" },
  { label: "仅折叠已读邮件", value: "read" },
  { label: "折叠较早邮件", value: "all_but_newest" },
] as const;

function setThreadNewestFirst(value: boolean) {
  void props.state.updatePreference({
    threadNewestFirst: value,
  } as Partial<FullPreferencesDto>);
}

// ---- 会话内状态（GPUI 默认：折叠较早邮件 / 展示附件） ----
const foldMode = ref("all_but_newest");
const cardAttachments = ref(true);
</script>

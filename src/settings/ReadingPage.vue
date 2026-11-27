<template>
  <div class="km-sp-page">
    <!-- 分组一：已读行为 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">已读行为</h4>
      <div class="km-sp-card">
        <div class="km-sp-row">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">自动标记为已读时机</span>
            <span class="km-sp-row-desc">控制点击选中未读邮件后何时提交已读状态</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="READ_MARK_OPTIONS"
              :modelValue="state.preferences.value.readMarkMode"
              title="自动标记为已读时机"
              @update:modelValue="setStringPref('readMarkMode', $event)"
            />
          </div>
        </div>
      </div>
    </section>

    <!-- 分组二：布局与正文字体主题 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">布局与正文字体主题</h4>
      <div class="km-sp-card">
        <div class="km-sp-row">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">阅读器窗格位置</span>
            <span class="km-sp-row-desc">可用 Ctrl+Alt+1 / 2 / 3 快捷键实时切换</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="READER_POSITION_OPTIONS"
              :modelValue="state.preferences.value.readerPosition"
              title="阅读器窗格位置"
              @update:modelValue="setStringPref('readerPosition', $event)"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">单封邮件显示卡片框线</span>
            <span class="km-sp-row-desc">关闭后单封邮件将边缘贴合铺满阅读区（仅单封会话生效）</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch v-model="readingState.singleMessageCard" />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">默认信纸主题</span>
            <span class="km-sp-row-desc">控制 HTML/文本正文底色（可被单封邮件顶部切换临时覆盖）</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="MESSAGE_THEME_OPTIONS"
              :modelValue="messageTheme"
              title="默认信纸主题"
              @update:modelValue="messageTheme = $event"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">阅读器正文字号</span>
            <span class="km-sp-row-desc">同时也可在阅读邮件时使用 Ctrl+= / Ctrl+- / Ctrl+0 快速调节</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="READER_ZOOM_OPTIONS"
              :modelValue="String(state.preferences.value.readerZoomPercent)"
              title="阅读器正文字号"
              @update:modelValue="setReaderZoom"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">始终显示收件人行</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch v-model="readingState.alwaysShowRecipients" />
          </div>
        </div>
      </div>
    </section>

    <!-- 分组三：阅读器顶栏与默认视图 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">阅读器顶栏与默认视图</h4>
      <div class="km-sp-card">
        <template v-for="(row, idx) in TOPBAR_ROWS" :key="row.key">
          <div v-if="idx > 0" class="km-sp-sep" />
          <div class="km-sp-row">
            <div class="km-sp-row-text">
              <span class="km-sp-row-title">{{ row.label }}</span>
            </div>
            <div class="km-sp-row-control">
              <PillSwitch v-model="readingState[row.key]" />
            </div>
          </div>
        </template>
        <div class="km-sp-sep" />
        <div class="km-sp-row">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">打开邮件时默认视图</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="READER_DEFAULT_MODE_OPTIONS"
              :modelValue="readerDefaultMode"
              title="打开邮件时默认视图"
              @update:modelValue="readerDefaultMode = $event"
            />
          </div>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import "./page.css";
import { reactive, ref } from "vue";
import ComboSelect from "./ComboSelect.vue";
import PillSwitch from "./PillSwitch.vue";
import type { FullPreferencesDto } from "../preferences/types";
import type { PrefsPanelState } from "../preferences/panels/state";

/**
 * 「阅读」页（对齐 GPUI pages.rs#render_reading）。
 * 已读时机/窗格位置/正文字号直连 IPC；信纸主题、卡片框线、
 * 收件人行、顶栏开关与默认视图为会话内状态。
 */
const props = defineProps<{ state: PrefsPanelState }>();

const READ_MARK_OPTIONS = [
  { label: "打开即标记已读", value: "shown" },
  { label: "停留 2 秒后标为已读", value: "delay" },
  { label: "仅手动标记", value: "manual" },
] as const;

const READER_POSITION_OPTIONS = [
  { label: "列表右侧（推荐）", value: "right" },
  { label: "列表下方", value: "below" },
  { label: "隐藏阅读器", value: "off" },
] as const;

const MESSAGE_THEME_OPTIONS = [
  { label: "跟随界面主题", value: "system" },
  { label: "始终浅色信纸", value: "light" },
  { label: "始终深色信纸", value: "dark" },
] as const;

const READER_ZOOM_OPTIONS = [
  { label: "50%", value: "50" },
  { label: "60%", value: "60" },
  { label: "70%", value: "70" },
  { label: "80%", value: "80" },
  { label: "90%", value: "90" },
  { label: "100%（默认）", value: "100" },
  { label: "110%", value: "110" },
  { label: "125%", value: "125" },
  { label: "150%", value: "150" },
  { label: "175%", value: "175" },
  { label: "200%", value: "200" },
  { label: "250%", value: "250" },
] as const;

const READER_DEFAULT_MODE_OPTIONS = [
  { label: "保持当前选择", value: "remember" },
  { label: "打开时默认简化视图", value: "on" },
  { label: "打开时默认完整 HTML", value: "off" },
] as const;

const TOPBAR_ROWS: ReadonlyArray<{ key: string; label: string }> = [
  { key: "readerShowAccountChip", label: "顶栏显示账号邮箱标签" },
  { key: "readerShowThemeSwitch", label: "顶栏显示明暗背景切换按钮" },
  { key: "readerShowViewSwitch", label: "顶栏显示简化阅读视图切换按钮" },
] as const;

function setStringPref(
  field: "readMarkMode" | "readerPosition",
  value: string
) {
  void props.state.updatePreference({ [field]: value } as Partial<FullPreferencesDto>);
}

function setReaderZoom(value: string) {
  const num = Number(value);
  if (!Number.isFinite(num)) return;
  void props.state.updatePreference({ readerZoomPercent: num });
}

// ---- 会话内状态（对齐 mail-core 字段默认值；信纸主题与默认视图为独立字符串项） ----
const messageTheme = ref("system");
const readerDefaultMode = ref("remember");
const readingState = reactive<Record<string, boolean>>({
  singleMessageCard: true,
  alwaysShowRecipients: false,
  readerShowAccountChip: true,
  readerShowThemeSwitch: true,
  readerShowViewSwitch: true,
});
</script>

<template>
  <div class="km-sp-page">
    <!-- 分组一：日期与时间格式 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">日期与时间格式</h4>
      <p class="km-sp-desc">实时影响邮件列表时间列与阅读器邮件头日期展示</p>
      <div class="km-sp-card">
        <div class="km-sp-row" id="sp-datetime-preview">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">当前格式实时预览</span>
          </div>
          <div class="km-sp-row-control">
            <span class="km-datetime-preview">{{ previewText }}</span>
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-date-style">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">日期排列顺序</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="DATE_STYLE_OPTIONS"
              :modelValue="dateTimeState.dateStyle"
              title="日期排列顺序"
              @update:modelValue="dateTimeState.dateStyle = $event"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-clock-style">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">时间制式</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="CLOCK_STYLE_OPTIONS"
              :modelValue="dateTimeState.clockStyle"
              title="时间制式"
              @update:modelValue="dateTimeState.clockStyle = $event"
            />
          </div>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import "./page.css";
import { computed, onBeforeUnmount, onMounted, reactive, ref } from "vue";
import ComboSelect from "./ComboSelect.vue";
import type { PrefsPanelState } from "../preferences/panels/state";

/**
 * 「日期和时间」页（对齐 GPUI pages.rs#render_date_time）。
 * date_style / clock_style 未进入前端 IPC 契约，会话内状态；
 * 预览行为 JS 镜像 format_message_datetime（每秒刷新当前时间）。
 */
defineProps<{ state: PrefsPanelState }>();

const DATE_STYLE_OPTIONS = [
  { label: "跟随系统 (YYYY-MM-DD)", value: "system" },
  { label: "月 / 日 / 年 (Nov 27, 2026)", value: "month_first" },
  { label: "日 / 月 / 年 (27 Nov 2026)", value: "day_first" },
  { label: "年 - 月 - 日 (2026-11-27)", value: "year_first" },
] as const;

const CLOCK_STYLE_OPTIONS = [
  { label: "跟随系统", value: "system" },
  { label: "12 小时制 (AM/PM)", value: "12h" },
  { label: "24 小时制", value: "24h" },
] as const;

const dateTimeState = reactive<Record<string, string>>({
  dateStyle: "system",
  clockStyle: "system",
});

const nowTs = ref(Math.floor(Date.now() / 1000));
let timer: ReturnType<typeof setInterval> | null = null;

onMounted(() => {
  timer = setInterval(() => {
    nowTs.value = Math.floor(Date.now() / 1000);
  }, 1000);
});

onBeforeUnmount(() => {
  if (timer) clearInterval(timer);
  timer = null;
});

const MONTH_ABBRS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

function formatClockPart(d: Date, clockStyle: string): string {
  if (clockStyle === "12h") {
    const h = d.getHours();
    const hour12 = h % 12 === 0 ? 12 : h % 12;
    const suffix = h >= 12 ? "PM" : "AM";
    return `${hour12}:${String(d.getMinutes()).padStart(2, "0")} ${suffix}`;
  }
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

function formatYearMonthDayPart(d: Date, dateStyle: string): string {
  const y = d.getFullYear();
  const m = d.getMonth() + 1;
  const day = d.getDate();
  switch (dateStyle) {
    case "month_first":
      return `${MONTH_ABBRS[m - 1]} ${day}, ${y}`;
    case "day_first":
      return `${day} ${MONTH_ABBRS[m - 1]} ${y}`;
    case "year_first":
      return `${y}-${String(m).padStart(2, "0")}-${String(day).padStart(2, "0")}`;
    default:
      return `${y}-${String(m).padStart(2, "0")}-${String(day).padStart(2, "0")}`;
  }
}

// 镜像 format_message_datetime：system/system 用本地 "YYYY-MM-DD HH:mm" 回落。
const previewText = computed(() => {
  const d = new Date(nowTs.value * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  const fallback = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
  if (dateTimeState.dateStyle === "system" && dateTimeState.clockStyle === "system") {
    return fallback;
  }
  return `${formatYearMonthDayPart(d, dateTimeState.dateStyle)} ${formatClockPart(d, dateTimeState.clockStyle)}`;
});
</script>

<style scoped>
.km-datetime-preview {
  font-size: 12px;
  font-weight: 600;
  color: var(--km-text-muted);
  font-variant-numeric: tabular-nums;
}
</style>

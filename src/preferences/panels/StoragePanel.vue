<template>
  <section class="km-tab-panel">
    <h3 class="km-tab-title">存储路径与正文缓存清理</h3>

    <div class="km-card">
      <div class="km-card-title">本地数据目录</div>
      <div class="km-path-row">
        <span class="km-path-label">配置与主数据目录</span>
        <code class="km-path-code">{{ state.dataDir.value || "-" }}</code>
      </div>
      <div class="km-path-row">
        <span class="km-path-label">邮件缓存目录</span>
        <code class="km-path-code">{{ state.mailDataDir.value || "-" }}</code>
      </div>
    </div>

    <div class="km-card">
      <div class="km-card-title">磁盘占用统计</div>
      <div class="km-stats-grid">
        <div class="km-stat-box">
          <span class="km-stat-label">总占用</span>
          <strong class="km-stat-value">
            {{ formatBytes(state.storageStats.value.totalBytes) }}
          </strong>
        </div>
        <div class="km-stat-box">
          <span class="km-stat-label">邮件正文缓存</span>
          <strong class="km-stat-value">
            {{ formatBytes(state.storageStats.value.mailBytes) }}
          </strong>
        </div>
        <div class="km-stat-box">
          <span class="km-stat-label">配置与索引</span>
          <strong class="km-stat-value">
            {{ formatBytes(state.storageStats.value.configBytes) }}
          </strong>
        </div>
        <div class="km-stat-box">
          <span class="km-stat-label">缓存文件数</span>
          <strong class="km-stat-value">
            {{ state.storageStats.value.fileCount }}
          </strong>
        </div>
      </div>

      <div class="km-field-row">
        <span class="km-label">清理范围</span>
        <ComboSelect
          :options="KEEP_DAYS_OPTIONS"
          :modelValue="state.cacheKeepDays.value === null ? 'all' : String(state.cacheKeepDays.value)"
          title="清理范围"
          @update:modelValue="(v) => onKeepDaysValue(state, v)"
        />
      </div>

      <div class="km-inline-actions">
        <button
          type="button"
          class="km-btn km-btn-danger-outline"
          :disabled="state.cacheBusy.value"
          @click="state.clearCaches()"
        >
          {{ state.cacheBusy.value ? "正在清理…" : "清理正文缓存" }}
        </button>
      </div>
    </div>
  </section>
</template>

<script setup lang="ts">
import ComboSelect from "../../settings/ComboSelect.vue";
import { formatBytes } from "../usePreferences";
import type { PrefsPanelState } from "./state";

defineProps<{ state: PrefsPanelState }>();

const KEEP_DAYS_OPTIONS = [
  { label: "清理全部本地正文缓存", value: "all" },
  { label: "保留最近 30 天正文缓存", value: "30" },
  { label: "保留最近 7 天正文缓存", value: "7" },
] as const;

function onKeepDaysValue(state: PrefsPanelState, value: string) {
  state.cacheKeepDays.value = value === "all" ? null : Number(value);
}
</script>

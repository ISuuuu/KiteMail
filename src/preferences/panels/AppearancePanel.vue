<template>
  <section class="km-tab-panel">
    <h3 class="km-tab-title">外观与阅读布局</h3>

    <div class="km-card">
      <div class="km-card-title">主题配色</div>
      <div class="km-theme-grid">
        <button
          v-for="t in BUILTIN_THEMES"
          :key="t.id"
          type="button"
          class="km-theme-chip"
          :class="{ active: state.preferences.value.themeId === t.id }"
          @click="state.updatePreference({ themeId: t.id })"
        >
          <span class="km-theme-swatch" :style="{ backgroundColor: t.swatch }" />
          <span>{{ t.label }}</span>
        </button>
      </div>

      <div class="km-field-row">
        <label for="km-pref-dark" class="km-label">深浅色模式</label>
        <select
          id="km-pref-dark"
          :value="darkModeSelectValue(state)"
          class="km-select"
          @change="onDarkModeChange(state, $event)"
        >
          <option value="system">跟随系统</option>
          <option value="light">始终浅色</option>
          <option value="dark">始终深色</option>
        </select>
      </div>

      <div class="km-field-row">
        <label for="km-pref-reader-pos" class="km-label">阅读窗格位置</label>
        <select
          id="km-pref-reader-pos"
          :value="state.preferences.value.readerPosition"
          class="km-select"
          @change="onSelectString(state, 'readerPosition', $event)"
        >
          <option value="right">右侧栏</option>
          <option value="below">列表下方</option>
          <option value="off">隐藏阅读区</option>
        </select>
      </div>
    </div>

    <div class="km-card">
      <div class="km-card-title">列表与缩放</div>
      <label class="km-toggle-row">
        <input
          type="checkbox"
          :checked="state.preferences.value.compactList"
          @change="onCheckbox(state, 'compactList', $event)"
        />
        <span>紧凑列表行高</span>
      </label>
      <label class="km-toggle-row">
        <input
          type="checkbox"
          :checked="state.preferences.value.showAvatars"
          @change="onCheckbox(state, 'showAvatars', $event)"
        />
        <span>显示发件人头像徽章</span>
      </label>
      <label class="km-toggle-row">
        <input
          type="checkbox"
          :checked="state.preferences.value.listGroupByDate"
          @change="onCheckbox(state, 'listGroupByDate', $event)"
        />
        <span>邮件列表按日期分组</span>
      </label>
      <label class="km-toggle-row">
        <input
          type="checkbox"
          :checked="state.preferences.value.singleKeyShortcuts"
          @change="onCheckbox(state, 'singleKeyShortcuts', $event)"
        />
        <span>启用单键快捷键（如 c 写信、e 归档、j/k 切邮件）</span>
      </label>

      <div class="km-field-row">
        <label for="km-pref-reader-zoom" class="km-label">
          阅读正文缩放 ({{ state.preferences.value.readerZoomPercent }}%)
        </label>
        <input
          id="km-pref-reader-zoom"
          type="range"
          min="50"
          max="250"
          step="10"
          :value="state.preferences.value.readerZoomPercent"
          @change="onNumberInput(state, 'readerZoomPercent', $event)"
        />
      </div>

      <div class="km-field-row">
        <label for="km-pref-ui-scale" class="km-label">
          界面文字比例 ({{ state.preferences.value.uiTextScalePercent }}%)
        </label>
        <input
          id="km-pref-ui-scale"
          type="range"
          min="80"
          max="150"
          step="5"
          :value="state.preferences.value.uiTextScalePercent"
          @change="onNumberInput(state, 'uiTextScalePercent', $event)"
        />
      </div>

      <div class="km-field-row">
        <label for="km-pref-default-view" class="km-label">启动默认视图</label>
        <select
          id="km-pref-default-view"
          :value="state.preferences.value.defaultView"
          class="km-select"
          @change="onSelectString(state, 'defaultView', $event)"
        >
          <option value="unified">统一收件箱</option>
          <option value="last">恢复上次打开的视图</option>
        </select>
      </div>
    </div>
  </section>
</template>

<script setup lang="ts">
import { BUILTIN_THEMES } from "../../ipc/types";
import {
  darkModeSelectValue,
  onCheckbox,
  onDarkModeChange,
  onNumberInput,
  onSelectString,
} from "./fieldBinders";
import type { PrefsPanelState } from "./state";

defineProps<{ state: PrefsPanelState }>();
</script>

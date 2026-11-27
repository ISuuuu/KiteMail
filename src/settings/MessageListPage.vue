<template>
  <div class="km-sp-page">
    <!-- 分组一：布局与摘要行数 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">布局与摘要行数</h4>
      <div class="km-sp-card">
        <div class="km-sp-row">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">布局</span>
            <span class="km-sp-row-desc">经典双行卡片，或宽表列视图（选宽表时自动切换为紧凑单行密度）</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="LAYOUT_OPTIONS"
              :modelValue="listLayout"
              title="布局"
              @update:modelValue="setLayout"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">摘要行数</span>
            <span class="km-sp-row-desc">列表卡片显示的正文预览行数（多行渲染随宽表模式工作落地）</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="PREVIEW_OPTIONS"
              :modelValue="String(listPreviewLines)"
              title="摘要行数"
              @update:modelValue="listPreviewLines = Number($event)"
            />
          </div>
        </div>
      </div>
    </section>

    <!-- 分组二：行样式与分组 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">行样式与分组</h4>
      <div class="km-sp-card">
        <div class="km-sp-row">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">紧凑列表行</span>
            <span class="km-sp-row-desc">以更紧凑的单/双行高度展示更多邮件条目</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch
              :modelValue="state.preferences.value.compactList"
              @update:modelValue="setBoolPref('compactList', $event)"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">显示发件人头像</span>
            <span class="km-sp-row-desc">隐藏后保留左侧未读状态蓝点与对齐间距</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch
              :modelValue="state.preferences.value.showAvatars"
              @update:modelValue="setBoolPref('showAvatars', $event)"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">按日期分组显示</span>
            <span class="km-sp-row-desc">按时间排序时显示「今天 / 昨天 / 本周 / 更早」分组头</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch
              :modelValue="state.preferences.value.listGroupByDate"
              @update:modelValue="setBoolPref('listGroupByDate', $event)"
            />
          </div>
        </div>
      </div>
    </section>

    <!-- 分组三：搜索栏与批量操作栏位置 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">搜索栏与批量操作栏位置</h4>
      <div class="km-sp-card">
        <template v-for="(row, idx) in BAR_ROWS" :key="row.key">
          <div v-if="idx > 0" class="km-sp-sep" />
          <div class="km-sp-row">
            <div class="km-sp-row-text">
              <span class="km-sp-row-title">{{ row.label }}</span>
            </div>
            <div class="km-sp-row-control">
              <PillSwitch v-model="listState[row.key]" />
            </div>
          </div>
        </template>
      </div>
    </section>

    <!-- 分组四：列顺序与显隐 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">列顺序与显隐</h4>
      <p class="km-sp-desc">单行宽表模式显示的列，从左到右排序；主题列为弹性列恒在显示</p>
      <div class="km-sp-card">
        <div class="km-sp-row">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">显示列标题栏</span>
            <span class="km-sp-row-desc">宽表模式在列表顶部显示可点击排序的列头</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch v-model="listState.listShowHeadings" />
          </div>
        </div>

        <!-- 已显示列：上移 / 下移 / 移除 -->
        <template v-for="(col, idx) in shownColumns" :key="`shown-${col}`">
          <div class="km-sp-sep" />
          <div class="km-sp-row">
            <div class="km-sp-row-text">
              <span class="km-sp-row-title">{{ columnLabel(col) }}</span>
            </div>
            <div class="km-sp-row-control km-sp-col-control">
              <button
                type="button"
                class="km-sp-icon-btn"
                title="上移"
                aria-label="上移"
                :disabled="idx === 0"
                @click="moveColumn(idx, -1)"
              >
                <SettingsIcon name="arrow-up" :size="13" />
              </button>
              <button
                type="button"
                class="km-sp-icon-btn"
                title="下移"
                aria-label="下移"
                :disabled="idx + 1 >= shownColumns.length"
                @click="moveColumn(idx, 1)"
              >
                <SettingsIcon name="arrow-down" :size="13" />
              </button>
              <button
                type="button"
                class="km-sp-outline-btn"
                @click="removeColumn(col)"
              >
                <SettingsIcon name="trash" :size="12" />
                <span>移除</span>
              </button>
            </div>
          </div>
        </template>

        <!-- 隐藏列：显示 -->
        <template v-for="col in hiddenColumns" :key="`hidden-${col}`">
          <div class="km-sp-sep" />
          <div class="km-sp-row">
            <div class="km-sp-row-text">
              <span class="km-sp-row-title">{{ columnLabel(col) }}</span>
            </div>
            <div class="km-sp-row-control">
              <button
                type="button"
                class="km-sp-outline-btn"
                @click="addColumn(col)"
              >
                <SettingsIcon name="plus" :size="12" />
                <span>显示</span>
              </button>
            </div>
          </div>
        </template>

        <div class="km-sp-footer">
          <button type="button" class="km-sp-outline-btn" @click="restoreColumns">
            恢复默认列
          </button>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import "./page.css";
import { computed, reactive, ref } from "vue";
import ComboSelect from "./ComboSelect.vue";
import PillSwitch from "./PillSwitch.vue";
import SettingsIcon from "./SettingsIcon.vue";
import type { FullPreferencesDto } from "../preferences/types";
import type { PrefsPanelState } from "../preferences/panels/state";

/**
 * 「邮件列表」页（对齐 GPUI pages.rs#render_message_list + ListColumn 列编辑器）。
 * 行样式三项直连 compactList/showAvatars/listGroupByDate；布局、摘要行数、
 * 搜索/批量栏位置与列编辑器为会话内状态（布局切换同步已契约的 compactList）。
 */
const props = defineProps<{ state: PrefsPanelState }>();

const LAYOUT_OPTIONS = [
  { label: "经典双行", value: "cards" },
  { label: "宽表列视图", value: "wide" },
] as const;

const PREVIEW_OPTIONS = [
  { label: "1 行", value: "1" },
  { label: "2 行", value: "2" },
  { label: "3 行", value: "3" },
] as const;

const BAR_ROWS = [
  { key: "listSearchAlwaysOpen", label: "始终显示搜索栏" },
  { key: "listSearchBottom", label: "搜索栏放在列表底部" },
  { key: "listBulkBarBottom", label: "多选批量操作栏放在列表底部" },
] as const;

/** 与 GPUI ListColumn::ALL 顺序一致。 */
const ALL_COLUMNS = [
  { key: "star", label: "星标" },
  { key: "replied", label: "已回复" },
  { key: "sender", label: "发件人" },
  { key: "recipients", label: "收件人" },
  { key: "correspondents", label: "往来联系人" },
  { key: "subject", label: "主题" },
  { key: "tags", label: "标签" },
  { key: "attachment", label: "附件" },
  { key: "importance", label: "重要性" },
  { key: "account", label: "账号" },
  { key: "due", label: "截止日期" },
  { key: "date", label: "日期" },
] as const;

/** 与 GPUI ListColumn::DEFAULT 一致。 */
const DEFAULT_COLUMNS = ["star", "replied", "sender", "subject", "tags", "attachment", "date"];

function columnLabel(key: string): string {
  return ALL_COLUMNS.find((c) => c.key === key)?.label ?? key;
}

// ---- 会话内状态（对齐 mail-core 默认值） ----
const listLayout = ref("cards");
const listPreviewLines = ref(1);
const shownColumnKeys = ref<string[]>([...DEFAULT_COLUMNS]);
const listState = reactive<Record<string, boolean>>({
  listSearchAlwaysOpen: false,
  listSearchBottom: false,
  listBulkBarBottom: false,
  listShowHeadings: true,
});

// 主题列为弹性列恒在显示：缺省插到发件人之后（对齐 list_columns_shown）。
const shownColumns = computed<string[]>(() => {
  const cols = shownColumnKeys.value.filter((k) => ALL_COLUMNS.some((c) => c.key === k));
  if (!cols.includes("subject")) {
    const senderIdx = cols.indexOf("sender");
    cols.splice(senderIdx >= 0 ? senderIdx + 1 : 0, 0, "subject");
  }
  return cols;
});

const hiddenColumns = computed<string[]>(() =>
  ALL_COLUMNS.map((c) => c.key).filter((k) => !shownColumns.value.includes(k))
);

function setLayout(value: string) {
  listLayout.value = value;
  // 行为映射（对齐 GPUI）：宽表列视图即紧凑单行密度，经典双行恢复卡片密度。
  void props.state.updatePreference({ compactList: value === "wide" });
}

function setBoolPref(
  field: "compactList" | "showAvatars" | "listGroupByDate",
  value: boolean
) {
  void props.state.updatePreference({ [field]: value } as Partial<FullPreferencesDto>);
}

function moveColumn(idx: number, delta: -1 | 1) {
  const cols = [...shownColumnKeys.value];
  const next = idx + delta;
  if (next < 0 || next >= cols.length) return;
  [cols[idx], cols[next]] = [cols[next], cols[idx]];
  shownColumnKeys.value = cols;
}

function removeColumn(key: string) {
  shownColumnKeys.value = shownColumnKeys.value.filter((k) => k !== key);
}

function addColumn(key: string) {
  if (!shownColumnKeys.value.includes(key)) {
    shownColumnKeys.value = [...shownColumnKeys.value, key];
  }
}

function restoreColumns() {
  shownColumnKeys.value = [...DEFAULT_COLUMNS];
}
</script>

<style scoped>
.km-sp-col-control {
  gap: 4px;
}
</style>

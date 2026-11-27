<template>
  <div class="km-sp-page">
    <!-- 分组一：侧边栏区段与折叠 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">侧边栏区段与折叠</h4>
      <div class="km-sp-card">
        <div class="km-sp-row" id="sp-default-view">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">启动时默认打开视图</span>
            <span class="km-sp-row-desc">选择每次冷启动定位的视图或恢复上一次阅读的文件夹</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="DEFAULT_VIEW_OPTIONS"
              :modelValue="state.preferences.value.defaultView"
              title="启动时默认打开视图"
              @update:modelValue="setDefaultView"
            />
          </div>
        </div>
        <template v-for="(row, idx) in TOP_ROWS" :key="row.key">
          <div class="km-sp-sep" />
          <div :id="`sp-${row.key}`" class="km-sp-row">
            <div class="km-sp-row-text">
              <span class="km-sp-row-title">{{ row.label }}</span>
              <span v-if="row.desc" class="km-sp-row-desc">{{ row.desc }}</span>
            </div>
            <div class="km-sp-row-control">
              <PillSwitch v-model="sidebarState[row.key]" />
            </div>
          </div>
        </template>
      </div>
    </section>

    <!-- 分组二：统一视图导航项 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">统一视图导航项</h4>
      <p class="km-sp-desc">控制侧边栏顶部跨账号统一视图入口的显示与隐藏</p>
      <div class="km-sp-card">
        <template v-for="(row, idx) in UNIFIED_ROWS" :key="row.key">
          <div v-if="idx > 0" class="km-sp-sep" />
          <div :id="`sp-${row.key}`" class="km-sp-row">
            <div class="km-sp-row-text">
              <span class="km-sp-row-title">{{ row.label }}</span>
            </div>
            <div class="km-sp-row-control">
              <PillSwitch v-model="sidebarState[row.key]" />
            </div>
          </div>
        </template>
      </div>
    </section>

    <!-- 分组三：分区分组 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">分区分组</h4>
      <p class="km-sp-desc">
        控制筛选文件夹 / 智能文件夹 / 标签等分区入口的显示（分区的实际渲染随侧边栏分区工作落地）
      </p>
      <div class="km-sp-card">
        <template v-for="(row, idx) in SECTION_ROWS" :key="row.key">
          <div v-if="idx > 0" class="km-sp-sep" />
          <div :id="`sp-${row.key}`" class="km-sp-row">
            <div class="km-sp-row-text">
              <span class="km-sp-row-title">{{ row.label }}</span>
            </div>
            <div class="km-sp-row-control">
              <PillSwitch v-model="sidebarState[row.key]" />
            </div>
          </div>
        </template>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import "./page.css";
import { reactive } from "vue";
import ComboSelect from "./ComboSelect.vue";
import PillSwitch from "./PillSwitch.vue";
import type { FullPreferencesDto } from "../preferences/types";
import type { PrefsPanelState } from "../preferences/panels/state";

/**
 * 「侧边栏」页（对齐 GPUI pages.rs#render_sidebar）。
 * 启动视图直连 defaultView；其余字段会话内状态。
 */
const props = defineProps<{ state: PrefsPanelState }>();

const DEFAULT_VIEW_OPTIONS = [
  { label: "统一收件箱", value: "unified" },
  { label: "恢复上次位置", value: "last" },
] as const;

const TOP_ROWS: ReadonlyArray<{ key: string; label: string; desc?: string }> = [
  {
    key: "showAccounts",
    label: "显示账号与文件夹列表",
    desc: "可用 Ctrl+Shift+A 快速显隐账号树",
  },
  { key: "sidebarShowContacts", label: "显示「联系人」入口" },
  { key: "sidebarShowAttachments", label: "显示「附件总览」入口" },
  {
    key: "rememberSidebarCollapsed",
    label: "记住侧边栏折叠状态",
    desc: "重启时恢复上次完整侧边栏或窄图标栏状态",
  },
  { key: "railUnreadDots", label: "窄栏模式下用未读圆点代替数字计数" },
] as const;

const UNIFIED_ROWS: ReadonlyArray<{ key: string; label: string }> = [
  { key: "unifiedShowInbox", label: "显示「统一收件箱」" },
  { key: "unifiedShowUnread", label: "显示「未读」" },
  { key: "unifiedShowStarred", label: "显示「已加星标」" },
  { key: "unifiedShowSent", label: "显示「已发送」" },
  { key: "unifiedShowDrafts", label: "显示「草稿箱」" },
  { key: "unifiedShowArchive", label: "显示「已归档」" },
  { key: "unifiedShowJunk", label: "显示「垃圾邮件」" },
  { key: "unifiedShowTrash", label: "显示「废纸篓」" },
  { key: "unifiedShowSnoozed", label: "显示「稍后处理」" },
] as const;

const SECTION_ROWS: ReadonlyArray<{ key: string; label: string }> = [
  { key: "sidebarShowFiltered", label: "显示「筛选文件夹」分区" },
  { key: "sidebarShowSmart", label: "显示「智能文件夹」分区" },
  { key: "sidebarShowTags", label: "显示「标签」分区" },
] as const;

function setDefaultView(value: string) {
  void props.state.updatePreference({
    defaultView: value,
  } as Partial<FullPreferencesDto>);
}

// ---- 会话内状态（对齐 mail-core 字段默认值） ----
const sidebarState = reactive<Record<string, boolean>>({
  showAccounts: true,
  sidebarShowContacts: true,
  sidebarShowAttachments: true,
  rememberSidebarCollapsed: true,
  railUnreadDots: true,
  unifiedShowInbox: true,
  unifiedShowUnread: true,
  unifiedShowStarred: true,
  unifiedShowSent: true,
  unifiedShowDrafts: true,
  unifiedShowArchive: true,
  unifiedShowJunk: true,
  unifiedShowTrash: true,
  unifiedShowSnoozed: true,
  sidebarShowFiltered: false,
  sidebarShowSmart: true,
  sidebarShowTags: true,
});
</script>

<template>
  <div class="km-sp-page">
    <!-- 分组一：专注模式 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">专注模式</h4>
      <p class="km-sp-desc">
        开启后由本页决定侧边栏、列表与阅读器的布局，替代其他设置页的同类项；Ctrl+Shift+F 或主菜单可随时切换
      </p>
      <div class="km-sp-card">
        <div class="km-sp-row" id="sp-focus-enabled">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">启用专注模式</span>
            <span class="km-sp-row-desc">
              沉浸阅读：按下方各分组开关裁剪界面（当前会话生效，随主窗口即时切换）
            </span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch v-model="focusEnabled" />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" :class="{ disabled: !focusEnabled }" id="sp-focusStartEnabled">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">启动时自动进入专注模式</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch v-model="focusStartEnabled" :disabled="!focusEnabled" />
          </div>
        </div>
      </div>
    </section>

    <!-- 分组二：侧边栏 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">侧边栏</h4>
      <div class="km-sp-card">
        <template v-for="(row, idx) in SIDEBAR_ROWS" :key="row.key">
          <div v-if="idx > 0" class="km-sp-sep" />
          <div :id="`sp-${row.key}`" class="km-sp-row" :class="{ disabled: !focusEnabled }">
            <div class="km-sp-row-text">
              <span class="km-sp-row-title">{{ row.label }}</span>
              <span v-if="row.desc" class="km-sp-row-desc">{{ row.desc }}</span>
            </div>
            <div class="km-sp-row-control">
              <PillSwitch
                v-model="focusState[row.key]"
                :disabled="!focusEnabled"
              />
            </div>
          </div>
        </template>
      </div>
    </section>

    <!-- 分组三：统一视图 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">统一视图</h4>
      <div class="km-sp-card">
        <template v-for="(row, idx) in UNIFIED_ROWS" :key="row.key">
          <div v-if="idx > 0" class="km-sp-sep" />
          <div :id="`sp-${row.key}`" class="km-sp-row" :class="{ disabled: !focusEnabled }">
            <div class="km-sp-row-text">
              <span class="km-sp-row-title">{{ row.label }}</span>
            </div>
            <div class="km-sp-row-control">
              <PillSwitch
                v-model="focusState[row.key]"
                :disabled="!focusEnabled"
              />
            </div>
          </div>
        </template>
      </div>
    </section>

    <!-- 分组四：分区 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">分区</h4>
      <div class="km-sp-card">
        <template v-for="(row, idx) in SECTION_ROWS" :key="row.key">
          <div v-if="idx > 0" class="km-sp-sep" />
          <div :id="`sp-${row.key}`" class="km-sp-row" :class="{ disabled: !focusEnabled }">
            <div class="km-sp-row-text">
              <span class="km-sp-row-title">{{ row.label }}</span>
            </div>
            <div class="km-sp-row-control">
              <PillSwitch
                v-model="focusState[row.key]"
                :disabled="!focusEnabled"
              />
            </div>
          </div>
        </template>
      </div>
    </section>

    <!-- 分组五：消息列表 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">消息列表</h4>
      <div class="km-sp-card">
        <div class="km-sp-row" :class="{ disabled: !focusEnabled }" id="sp-focus-layout">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">布局</span>
            <span class="km-sp-row-desc">卡片双行或单行表格；自动在宽窗使用单行</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="FOCUS_LAYOUT_OPTIONS"
              :modelValue="focusLayout"
              :disabled="!focusEnabled"
              title="布局"
              @update:modelValue="focusLayout = $event"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" :class="{ disabled: !focusEnabled }" id="sp-focusColumnHeadings">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">显示列标题</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch v-model="focusState.focusColumnHeadings" :disabled="!focusEnabled" />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" :class="{ disabled: !focusEnabled }" id="sp-focus-preview-lines">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">摘要行数</span>
            <span class="km-sp-row-desc">列表卡片显示的正文预览行数</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="FOCUS_PREVIEW_OPTIONS"
              :modelValue="focusPreviewLines"
              :disabled="!focusEnabled"
              title="摘要行数"
              @update:modelValue="focusPreviewLines = $event"
            />
          </div>
        </div>
        <template v-for="(row, idx) in LIST_ROWS" :key="row.key">
          <div class="km-sp-sep" />
          <div :id="`sp-${row.key}`" class="km-sp-row" :class="{ disabled: !focusEnabled }">
            <div class="km-sp-row-text">
              <span class="km-sp-row-title">{{ row.label }}</span>
              <span v-if="row.desc" class="km-sp-row-desc">{{ row.desc }}</span>
            </div>
            <div class="km-sp-row-control">
              <PillSwitch
                v-model="focusState[row.key]"
                :disabled="!focusEnabled"
              />
            </div>
          </div>
        </template>
      </div>
    </section>

    <!-- 分组六：阅读 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">阅读</h4>
      <div class="km-sp-card">
        <div class="km-sp-row" :class="{ disabled: !focusEnabled }" id="sp-focusFoldToolbar">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">折叠阅读器工具栏</span>
            <span class="km-sp-row-desc">每个按钮收进其 ⋯ 菜单。</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch v-model="focusState.focusFoldToolbar" :disabled="!focusEnabled" />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" :class="{ disabled: !focusEnabled }" id="sp-focusReaderView">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">简化阅读视图</span>
            <span class="km-sp-row-desc">打开邮件即进入简化阅读视图。</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch v-model="focusState.focusReaderView" :disabled="!focusEnabled" />
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
import type { PrefsPanelState } from "../preferences/panels/state";

/**
 * 「专注模式」页（对齐 GPUI pages.rs#render_focus_mode）。
 * 专注模式字段均未进入前端 IPC 契约，整页为会话内状态；
 * 总开关关闭时下方各分组联动禁用（本任务明确要求，GPUI 源码无此联动）。
 */
defineProps<{ state: PrefsPanelState }>();

const FOCUS_LAYOUT_OPTIONS = [
  { label: "卡片双行", value: "cards" },
  { label: "单行表格", value: "single_line" },
  { label: "自动（宽窗单行）", value: "automatic" },
] as const;

const FOCUS_PREVIEW_OPTIONS = [
  { label: "关闭", value: "0" },
  { label: "1 行", value: "1" },
  { label: "2 行", value: "2" },
  { label: "3 行", value: "3" },
] as const;

const SIDEBAR_ROWS: ReadonlyArray<{ key: string; label: string; desc?: string }> = [
  { key: "focusRailSidebar", label: "折叠侧边栏为图标栏" },
  { key: "focusUnreadCounts", label: "统一视图行显示未读计数" },
  { key: "focusRailDots", label: "图标栏以未读圆点代替计数" },
  { key: "focusHoverExpand", label: "悬停图标栏时滑出完整侧边栏" },
  { key: "focusHideAccounts", label: "隐藏账号分区", desc: "仅保留统一视图分区。" },
  { key: "focusFoldAccounts", label: "折叠账号为头像", desc: "展开的账号收起为其头像" },
  { key: "focusClickFolds", label: "单击账号即折叠" },
  { key: "focusFoldUnified", label: "折叠统一视图行" },
  {
    key: "focusHideSidebar",
    label: "隐藏整个侧边栏（Windows 行为）",
    desc: "保留既有专注模式行为：整栏隐藏而非折叠为图标栏",
  },
] as const;

const UNIFIED_ROWS: ReadonlyArray<{ key: string; label: string }> = [
  { key: "focusShowInboxes", label: "统一收件箱" },
  { key: "focusUnifiedStarred", label: "已加星标" },
  { key: "focusUnifiedSent", label: "已发送" },
  { key: "focusUnifiedDrafts", label: "草稿箱" },
  { key: "focusUnifiedArchive", label: "已归档" },
  { key: "focusUnifiedJunk", label: "垃圾邮件" },
  { key: "focusUnifiedTrash", label: "废纸篓" },
  { key: "focusUnifiedSnoozed", label: "稍后处理" },
] as const;

const SECTION_ROWS: ReadonlyArray<{ key: string; label: string }> = [
  { key: "focusShowFavorites", label: "收藏" },
  { key: "focusShowFilters", label: "筛选文件夹" },
  { key: "focusShowTags", label: "标签" },
  { key: "focusShowContacts", label: "联系人" },
  { key: "focusShowAttachments", label: "附件" },
] as const;

const LIST_ROWS: ReadonlyArray<{ key: string; label: string; desc?: string }> = [
  { key: "focusListHeader", label: "折叠列表头部", desc: "除侧边栏开关外的全部收进 ⋯。" },
  { key: "focusHideAvatars", label: "隐藏发件人头像" },
  { key: "focusHideSubject", label: "隐藏主题", desc: "标签胶囊随之隐藏。" },
  { key: "focusTagsBelow", label: "标签置于文本下方" },
  { key: "focusHideStars", label: "隐藏星标" },
  { key: "focusHideAttachmentIcons", label: "隐藏附件图标" },
  {
    key: "focusHideList",
    label: "隐藏中间邮件列表（Windows 行为）",
    desc: "聚焦阅读当前邮件，隐藏列表窗格",
  },
] as const;

// ---- 会话内状态（对齐 mail-core 字段默认值；布局与摘要行数为独立字符串项） ----
const focusEnabled = ref(false);
const focusStartEnabled = ref(false);
const focusLayout = ref("automatic");
const focusPreviewLines = ref("0");
const focusState = reactive<Record<string, boolean>>({
  focusRailSidebar: true,
  focusUnreadCounts: true,
  focusRailDots: true,
  focusHoverExpand: false,
  focusHideAccounts: true,
  focusFoldAccounts: true,
  focusClickFolds: false,
  focusFoldUnified: true,
  focusHideSidebar: true,
  focusShowInboxes: true,
  focusUnifiedStarred: true,
  focusUnifiedSent: true,
  focusUnifiedDrafts: true,
  focusUnifiedArchive: true,
  focusUnifiedJunk: true,
  focusUnifiedTrash: true,
  focusUnifiedSnoozed: true,
  focusShowFavorites: true,
  focusShowFilters: true,
  focusShowTags: true,
  focusShowContacts: true,
  focusShowAttachments: true,
  focusColumnHeadings: false,
  focusListHeader: true,
  focusHideAvatars: true,
  focusHideSubject: false,
  focusTagsBelow: false,
  focusHideStars: true,
  focusHideAttachmentIcons: true,
  focusHideList: true,
  focusFoldToolbar: true,
  focusReaderView: true,
});
</script>

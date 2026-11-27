<template>
  <div class="km-sp-page">
    <!-- 反馈条（对齐 GPUI mcp_feedback） -->
    <div v-if="feedback" class="km-sp-feedback" role="status">
      {{ feedback }}
    </div>

    <!-- 分组一：MCP 服务 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">MCP 服务</h4>
      <p class="km-sp-desc">
        允许外部 AI 助手通过 MCP (Model Context Protocol) 按权限访问本地邮件（Windows 命名管道监听待后台服务端接入）
      </p>
      <div class="km-sp-card">
        <div class="km-sp-row" id="sp-mcp-enabled">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">启用本地 MCP Server</span>
            <span class="km-sp-row-desc">开启后将按下方工具权限暴露只读/草稿能力</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch v-model="mcpState.mcpEnabled" />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-edit-row" id="sp-mcp-socket">
          <div class="km-sp-edit-inputs">
            <input
              v-model="socketPathInput"
              type="text"
              class="km-sp-input km-mono-input"
              aria-label="MCP 命名管道路径"
            />
            <button type="button" class="km-sp-outline-btn" @click="saveSocketPath">
              保存路径
            </button>
          </div>
        </div>
      </div>
    </section>

    <!-- 分组二：工具权限 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">工具权限</h4>
      <p class="km-sp-desc">外部 AI 助手仅可调用已授权的工具；不允许直接发信或永久删信</p>
      <div class="km-sp-card">
        <template v-for="(row, idx) in PERM_ROWS" :key="row.key">
          <div v-if="idx > 0" class="km-sp-sep" />
          <div :id="`sp-${row.key}`" class="km-sp-row">
            <div class="km-sp-row-text">
              <span class="km-sp-row-title">{{ row.label }}</span>
              <span class="km-sp-row-desc">{{ row.desc }}</span>
            </div>
            <div class="km-sp-row-control">
              <PillSwitch v-model="mcpState[row.key]" />
            </div>
          </div>
        </template>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import "./page.css";
import { reactive, ref } from "vue";
import PillSwitch from "./PillSwitch.vue";
import type { PrefsPanelState } from "../preferences/panels/state";

/**
 * 「MCP 服务」页（对齐 GPUI extra_pages.rs#render_mcp）。
 * mcp_* 字段均未进入前端 IPC 契约，整页为会话内状态；
 * 命名管道路径保存仅更新会话状态并给出与 GPUI 一致的反馈文案。
 */
defineProps<{ state: PrefsPanelState }>();

const DEFAULT_SOCKET_PATH = "\\\\.\\pipe\\hylki-mcp";

const PERM_ROWS = [
  { key: "mcpAllowRead", label: "允许读取邮件内容", desc: "读取本地缓存的邮件正文与发件人信息" },
  { key: "mcpAllowSearch", label: "允许搜索邮件", desc: "执行全文检索与条件筛选" },
  { key: "mcpAllowDraft", label: "允许创建草稿", desc: "生成待您审核发送的回复草稿" },
  { key: "mcpAllowModifyFlags", label: "允许标记已读 / 星标 / 标签", desc: "整理收件箱状态与分类标签" },
] as const;

const feedback = ref<string | null>(null);
const socketPathInput = ref(DEFAULT_SOCKET_PATH);

const mcpState = reactive<Record<string, boolean>>({
  mcpEnabled: false,
  mcpAllowRead: true,
  mcpAllowSearch: true,
  mcpAllowDraft: false,
  mcpAllowModifyFlags: false,
});

function saveSocketPath() {
  const path = socketPathInput.value.trim();
  // 空路径回落默认命名管道（对齐 GPUI 保存逻辑）。
  socketPathInput.value = path.length === 0 ? DEFAULT_SOCKET_PATH : path;
  feedback.value = "已保存 MCP 命名管道路径";
  window.setTimeout(() => {
    feedback.value = null;
  }, 2600);
}
</script>

<style scoped>
.km-mono-input {
  font-family: var(--km-font-mono);
}
</style>

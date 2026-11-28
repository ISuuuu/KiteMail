<template>
  <div class="km-sp-page">
    <!-- 反馈条（对齐 GPUI backup_feedback） -->
    <div
      v-if="feedback"
      class="km-sp-feedback"
      :class="{ error: !feedback.ok }"
      role="status"
    >
      {{ feedback.text }}
    </div>

    <!-- 分组一：导出配置备份 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">导出配置备份</h4>
      <p class="km-sp-desc">
        生成可移植的 TOML 备份文件（仅包含偏好设置与账号服务器参数；不包含密码、OAuth 令牌和本地邮件）
      </p>
      <div class="km-sp-card">
        <div class="km-sp-row" id="sp-backup-export">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">导出当前偏好与账号配置</span>
            <span class="km-sp-row-desc">保存为 .toml 文件以便迁移或重装后快速恢复</span>
          </div>
          <div class="km-sp-row-control">
            <button type="button" class="km-sp-primary-btn" @click="exportBackup">
              导出备份...
            </button>
          </div>
        </div>
      </div>
    </section>

    <!-- 分组二：导入与恢复配置 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">导入与恢复配置</h4>
      <p class="km-sp-desc">选择先前导出的 .toml 备份文件，预览内容后确认写回配置文件并立即应用</p>
      <div class="km-sp-card">
        <div class="km-sp-row" id="sp-backup-import">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">从备份文件恢复</span>
            <span class="km-sp-row-desc">不会覆盖现有 Windows 凭据管理器中的密码或令牌</span>
          </div>
          <div class="km-sp-row-control">
            <button type="button" class="km-sp-outline-btn" @click="pickBackupFile">
              选择备份文件...
            </button>
          </div>
        </div>
      </div>
    </section>

    <!-- 分组三：待确认恢复的备份预览（选中备份文件后出现） -->
    <section v-if="pendingImport" class="km-sp-group">
      <h4 class="km-sp-title">待确认恢复的备份预览</h4>
      <p class="km-sp-desc">确认后将立即覆盖偏好设置与账号清单，并重启同步相关配置</p>
      <div class="km-sp-card">
        <div class="km-sp-row">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">源文件</span>
          </div>
          <div class="km-sp-row-control">
            <span class="km-backup-path">{{ pendingImport.path }}</span>
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">包含邮箱账号</span>
          </div>
          <div class="km-sp-row-control">
            <span class="km-backup-value">{{ pendingImport.accountCount }} 个</span>
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">主题方案</span>
          </div>
          <div class="km-sp-row-control">
            <span class="km-backup-value">{{ pendingImport.themeId }}</span>
          </div>
        </div>
        <div class="km-sp-footer">
          <button type="button" class="km-sp-outline-btn" @click="cancelImport">
            取消
          </button>
          <button type="button" class="km-sp-primary-btn" @click="confirmImport">
            确认导入并生效
          </button>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import "./page.css";
import { ref } from "vue";
import { extractErrorMessage } from "../ipc/main-window";
import {
  exportSettingsBackup,
  importSettingsBackup,
  previewSettingsBackup,
} from "../preferences/ipc";
import type { PrefsPanelState } from "../preferences/panels/state";

/**
 * 「备份」页（对齐 GPUI pages.rs#render_backup）。
 * 导出 / 选择备份文件 / 确认导入均走真实 Tauri 命令：文件选择对话框由后端弹出，
 * 导入成功后后端会持久化并广播 preferencesUpdated / accountsUpdated 事件。
 */
defineProps<{ state: PrefsPanelState }>();

interface PendingBackupImport {
  path: string;
  accountCount: number;
  themeId: string;
}

const feedback = ref<{ ok: boolean; text: string } | null>(null);
const pendingImport = ref<PendingBackupImport | null>(null);

function showFeedback(ok: boolean, text: string) {
  feedback.value = { ok, text };
  window.setTimeout(() => {
    feedback.value = null;
  }, 3200);
}

async function exportBackup() {
  try {
    const res = await exportSettingsBackup();
    showFeedback(!res.cancelled, res.message);
  } catch (err) {
    showFeedback(false, extractErrorMessage(err));
  }
}

async function pickBackupFile() {
  try {
    const res = await previewSettingsBackup();
    if (res.cancelled) return;
    pendingImport.value = {
      path: res.path ?? "",
      accountCount: res.accountCount,
      themeId: res.themeId || "默认",
    };
    showFeedback(true, res.message);
  } catch (err) {
    showFeedback(false, extractErrorMessage(err));
  }
}

function cancelImport() {
  pendingImport.value = null;
}

async function confirmImport() {
  const target = pendingImport.value;
  if (!target) return;
  try {
    const res = await importSettingsBackup(target.path);
    pendingImport.value = null;
    showFeedback(res.imported, res.message);
  } catch (err) {
    showFeedback(false, extractErrorMessage(err));
  }
}
</script>

<style scoped>
.km-backup-path,
.km-backup-value {
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-backup-path {
  word-break: break-all;
  user-select: text;
  max-width: 360px;
  text-align: right;
}
</style>

<template>
  <div class="km-sp-page">
    <!-- 页级反馈（对齐 GPUI backup_feedback / mail_dir_feedback） -->
    <div
      v-if="feedback"
      class="km-sp-feedback"
      :class="{ error: !feedback.ok }"
      role="status"
    >
      {{ feedback.text }}
    </div>

    <!-- 分组一：窗口、托盘与快捷键 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">窗口、托盘与快捷键</h4>
      <div class="km-sp-card">
        <div class="km-sp-row" id="sp-startAtLogin">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">登录 Windows 时自动启动</span>
            <span class="km-sp-row-desc">读写 HKCU\Software\Microsoft\Windows\CurrentVersion\Run 注册表项</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch v-model="systemState.startAtLogin" />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-linkOpenMode">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">邮件内链接打开方式</span>
            <span class="km-sp-row-desc">选择点击外部链接或退订页面时使用的浏览器</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="LINK_OPEN_OPTIONS"
              :modelValue="linkOpenMode"
              title="邮件内链接打开方式"
              @update:modelValue="linkOpenMode = $event"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-closeToMinimize">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">关闭主窗口时最小化到托盘</span>
            <span class="km-sp-row-desc">点击关闭按钮时不退出程序，保持系统托盘后台收取新邮件</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch
              :modelValue="state.preferences.value.closeToMinimize"
              @update:modelValue="setBoolPref('closeToMinimize', $event)"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-minimizeToTray">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">最小化主窗口时隐藏到托盘</span>
            <span class="km-sp-row-desc">最小化主窗口后从任务栏隐藏，可通过托盘图标单/双击恢复</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch
              :modelValue="state.preferences.value.minimizeToTray"
              @update:modelValue="setBoolPref('minimizeToTray', $event)"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-startFullscreen">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">启动时进入全屏模式</span>
            <span class="km-sp-row-desc">冷启动创建主窗口后自动切换全屏（可随时按 F11 还原）</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch v-model="systemState.startFullscreen" />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-singleKeyShortcuts">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">启用单键快捷键</span>
            <span class="km-sp-row-desc">在非输入焦点下响应 j/k/r/a/d/s/m（按 ? 查看速查）</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch
              :modelValue="state.preferences.value.singleKeyShortcuts"
              @update:modelValue="setBoolPref('singleKeyShortcuts', $event)"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-statusBarTop">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">将状态栏置于窗口顶部</span>
            <span class="km-sp-row-desc">仅将连接与同步状态栏从窗口底部移至顶栏下方（操作提示条仍在底部显示）</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch v-model="systemState.statusBarTop" />
          </div>
        </div>
      </div>
    </section>

    <!-- 分组二：本地数据与缓存维护 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">本地数据与缓存维护</h4>
      <div class="km-sp-card">
        <div class="km-sp-row" id="sp-app-dir">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">程序配置目录</span>
            <span class="km-sp-row-desc km-sp-path">{{ state.dataDir.value || "-" }}</span>
          </div>
          <div class="km-sp-row-control">
            <button
              type="button"
              class="km-sp-outline-btn"
              @click="openDir('data')"
            >
              打开目录
            </button>
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-mail-dir">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">邮件存储目录</span>
            <span class="km-sp-row-desc km-sp-path">
              默认位置 · {{ state.mailDataDir.value || "-" }}
            </span>
          </div>
          <div class="km-sp-row-control km-sp-dir-actions">
            <button
              type="button"
              class="km-sp-outline-btn"
              @click="mailDirDialogOpen = true"
            >
              更改目录
            </button>
            <button
              type="button"
              class="km-sp-outline-btn"
              @click="openDir('mail')"
            >
              打开目录
            </button>
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-storage-stats">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">本地缓存与附件占用</span>
          </div>
          <div class="km-sp-row-control">
            <span class="km-sp-storage-text">{{ storageText }}</span>
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-keepDownloadsDays">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">已下载邮件保留时长</span>
            <span class="km-sp-row-desc">控制本地 SQLite 缓存的邮件正文保留期限</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="KEEP_DOWNLOADS_OPTIONS"
              :modelValue="keepDownloadsDays"
              title="已下载邮件保留时长"
              @update:modelValue="keepDownloadsDays = $event"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-run-mode">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">运行模式</span>
          </div>
          <div class="km-sp-row-control">
            <span class="km-sp-storage-text">{{ runModeText }}</span>
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-clear-cache">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">清除本地邮件正文缓存</span>
            <span class="km-sp-row-desc">仅移除已缓存的 HTML / 纯文本正文，保留账号配置与邮件列表索引</span>
          </div>
          <div class="km-sp-row-control">
            <button
              type="button"
              class="km-sp-outline-btn"
              :disabled="state.cacheBusy.value"
              @click="clearCacheDialogOpen = true"
            >
              {{ state.cacheBusy.value ? "正在清理…" : "清除正文缓存" }}
            </button>
          </div>
        </div>
      </div>
    </section>

    <!-- 分组三：软件更新 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">软件更新</h4>
      <div class="km-sp-card">
        <div class="km-sp-row" id="sp-checkUpdates">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">启动时检查更新</span>
            <span class="km-sp-row-desc">
              启动约 10 秒后读取 GitHub 最新发布；发现新版本时在「关于 KiteMail」中提示并支持安装
            </span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch
              :modelValue="state.preferences.value.checkUpdates"
              @update:modelValue="setBoolPref('checkUpdates', $event)"
            />
          </div>
        </div>
      </div>
    </section>

    <!-- 分组四：关于 KiteMail -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">关于 KiteMail</h4>
      <div class="km-sp-card">
        <div class="km-sp-row" id="sp-version">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">版本号</span>
          </div>
          <div class="km-sp-row-control">
            <span class="km-sp-storage-text">v{{ state.appVersion.value }}</span>
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-engine">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">界面与通信内核</span>
          </div>
          <div class="km-sp-row-control">
            <span class="km-sp-storage-text">Tauri WebView2 + IMAP IDLE + Microsoft Graph OAuth2</span>
          </div>
        </div>
      </div>
    </section>

    <!-- 清理正文缓存确认（GPUI 内联确认的弹层化，复用账号域 ConfirmDialog） -->
    <ConfirmDialog
      v-model:open="clearCacheDialogOpen"
      title="清除本地邮件正文缓存"
      message="将移除全部已缓存的 HTML / 纯文本正文，保留账号配置与邮件列表索引。"
      description="清理后重新打开邮件时需要重新从服务器拉取正文。"
      confirmLabel="确认清除"
      cancelLabel="取消"
      danger
      :busy="state.cacheBusy.value"
      @confirm="confirmClearCache"
    />

    <!-- 更改邮件存储目录（GPUI 目录切换弹层的等价说明；当前版本不提供运行时切换） -->
    <ConfirmDialog
      v-model:open="mailDirDialogOpen"
      title="更改邮件存储目录"
      message="当前版本不支持在运行中切换邮件存储目录：数据库、同步任务与发件箱都会独占当前目录，需要退出应用后手动迁移。"
      :description="mailDirDialogDescription"
      confirmLabel="查看手动步骤"
      cancelLabel="取消"
      @confirm="onMailDirDialogConfirm"
    />
  </div>
</template>

<script setup lang="ts">
import "./page.css";
import { computed, reactive, ref } from "vue";
import ConfirmDialog from "../accounts/ConfirmDialog.vue";
import ComboSelect from "./ComboSelect.vue";
import PillSwitch from "./PillSwitch.vue";
import { formatBytes } from "../preferences/usePreferences";
import { extractErrorMessage, openDataDirectory } from "../preferences/ipc";
import type { DataDirectoryKind, FullPreferencesDto } from "../preferences/types";
import type { PrefsPanelState } from "../preferences/panels/state";

/**
 * 「系统」页（对齐 GPUI pages.rs#render_system）。
 * 托盘/快捷键/打开数据目录/占用统计/清缓存/检查更新/关于直连既有 IPC；
 * 自启动、链接打开方式、全屏、状态栏置顶、保留时长为会话内状态；
 * 邮件存储目录切换后端无运行时迁移命令，改为指引用户在退出应用后手动迁移。
 */
const props = defineProps<{ state: PrefsPanelState }>();

const LINK_OPEN_OPTIONS = [
  { label: "系统默认浏览器", value: "system" },
  { label: "Microsoft Edge", value: "edge" },
  { label: "Google Chrome", value: "chrome" },
  { label: "Mozilla Firefox", value: "firefox" },
] as const;

const KEEP_DOWNLOADS_OPTIONS = [
  { label: "永久保留", value: "0" },
  { label: "1 年", value: "365" },
  { label: "90 天", value: "90" },
  { label: "30 天", value: "30" },
] as const;

const feedback = ref<{ ok: boolean; text: string } | null>(null);
const clearCacheDialogOpen = ref(false);
const mailDirDialogOpen = ref(false);

function showFeedback(ok: boolean, text: string, durationMs = 3200) {
  feedback.value = { ok, text };
  window.setTimeout(() => {
    feedback.value = null;
  }, durationMs);
}

function setBoolPref(
  field: "closeToMinimize" | "minimizeToTray" | "singleKeyShortcuts" | "checkUpdates",
  value: boolean
) {
  void props.state.updatePreference({ [field]: value } as Partial<FullPreferencesDto>);
}

const storageText = computed(() => {
  const s = props.state.storageStats.value;
  return `配置与索引 ${formatBytes(s.configBytes)} · 邮件与附件 ${formatBytes(s.mailBytes)} · 共 ${formatBytes(s.totalBytes)}（${s.fileCount} 个文件）`;
});

const runModeText = computed(() =>
  props.state.isPortable.value
    ? "便携版（EXE 同级 data 配置 + 分账号分库存储）"
    : "安装版（LocalAppData 配置 + 分账号分库存储）"
);

function confirmClearCache() {
  clearCacheDialogOpen.value = false;
  void props.state.clearCaches().then((accepted) => {
    if (accepted) showFeedback(true, "已请求清理本地邮件正文缓存");
  });
}

const mailDirDialogDescription = computed(() => {
  const dataDir = props.state.dataDir.value || "-";
  const mailDir = props.state.mailDataDir.value || "-";
  return [
    "手动迁移步骤：",
    "1) 完全退出 KiteMail（含托盘图标）；",
    `2) 将当前邮件目录 ${mailDir} 整体移动到新位置；`,
    `3) 编辑 ${dataDir} 下的 storage.toml，把 mail_data_dir 改为新路径；`,
    "4) 重新启动 KiteMail 生效。",
  ].join("");
});

function onMailDirDialogConfirm() {
  mailDirDialogOpen.value = false;
  showFeedback(true, mailDirDialogDescription.value, 15000);
}

async function openDir(kind: DataDirectoryKind) {
  try {
    const res = await openDataDirectory(kind);
    showFeedback(res.opened, res.message);
  } catch (err) {
    showFeedback(false, extractErrorMessage(err));
  }
}

// ---- 会话内状态（对齐 mail-core 字段默认值；链接打开方式与保留时长为独立字符串项） ----
const linkOpenMode = ref("system");
const keepDownloadsDays = ref("0");
const systemState = reactive<Record<string, boolean>>({
  startAtLogin: false,
  startFullscreen: false,
  statusBarTop: false,
});
</script>

<style scoped>
.km-sp-path {
  word-break: break-all;
  user-select: text;
}

.km-sp-storage-text {
  font-size: 12px;
  color: var(--km-text-muted);
  text-align: right;
}

.km-sp-dir-actions {
  gap: 6px;
  flex-wrap: wrap;
  justify-content: flex-end;
}
</style>

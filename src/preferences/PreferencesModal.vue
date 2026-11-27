<template>
  <div
    v-if="open"
    class="km-prefs-backdrop"
    role="dialog"
    aria-modal="true"
    aria-labelledby="km-prefs-modal-title"
    @click.self="closeModal"
  >
    <section class="km-prefs-modal">
      <header class="km-modal-header">
        <div class="km-modal-title-group">
          <h2 id="km-prefs-modal-title" class="km-modal-title">偏好设置</h2>
          <span class="km-modal-subtitle">外观、同步、隐私安全与本地存储</span>
        </div>

        <button
          type="button"
          class="km-close-btn"
          title="关闭 (Esc)"
          aria-label="关闭偏好设置窗口"
          @click="closeModal"
        >
          ✕
        </button>
      </header>

      <div class="km-modal-body">
        <!-- 左侧 6 分栏导航 -->
        <nav class="km-prefs-nav" aria-label="设置分类">
          <button
            v-for="tab in TABS"
            :key="tab.id"
            type="button"
            class="km-nav-item"
            :class="{ active: state.activeTab.value === tab.id }"
            @click="state.activeTab.value = tab.id"
          >
            <span class="km-nav-label">{{ tab.label }}</span>
            <span class="km-nav-desc">{{ tab.desc }}</span>
          </button>
        </nav>

        <!-- 右侧内容区 -->
        <main class="km-prefs-content">
          <div v-if="state.loading.value" class="km-state-empty">
            正在加载偏好设置…
          </div>
          <div v-else-if="state.loadError.value" class="km-state-error" role="alert">
            <p>{{ state.loadError.value }}</p>
            <button
              type="button"
              class="km-btn km-btn-secondary"
              @click="state.init(state.activeTab.value)"
            >
              重试
            </button>
          </div>
          <template v-else>
            <!-- 1. 外观与阅读布局 -->
            <section v-if="state.activeTab.value === 'appearance'" class="km-tab-panel">
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
                    :value="darkModeSelectValue"
                    class="km-select"
                    @change="onDarkModeChange"
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
                    @change="onSelectString('readerPosition', $event)"
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
                    @change="onCheckbox('compactList', $event)"
                  />
                  <span>紧凑列表行高</span>
                </label>
                <label class="km-toggle-row">
                  <input
                    type="checkbox"
                    :checked="state.preferences.value.showAvatars"
                    @change="onCheckbox('showAvatars', $event)"
                  />
                  <span>显示发件人头像徽章</span>
                </label>
                <label class="km-toggle-row">
                  <input
                    type="checkbox"
                    :checked="state.preferences.value.listGroupByDate"
                    @change="onCheckbox('listGroupByDate', $event)"
                  />
                  <span>邮件列表按日期分组</span>
                </label>
                <label class="km-toggle-row">
                  <input
                    type="checkbox"
                    :checked="state.preferences.value.singleKeyShortcuts"
                    @change="onCheckbox('singleKeyShortcuts', $event)"
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
                    @change="onNumberInput('readerZoomPercent', $event)"
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
                    @change="onNumberInput('uiTextScalePercent', $event)"
                  />
                </div>

                <div class="km-field-row">
                  <label for="km-pref-default-view" class="km-label">启动默认视图</label>
                  <select
                    id="km-pref-default-view"
                    :value="state.preferences.value.defaultView"
                    class="km-select"
                    @change="onSelectString('defaultView', $event)"
                  >
                    <option value="unified">统一收件箱</option>
                    <option value="last">恢复上次打开的视图</option>
                  </select>
                </div>
              </div>
            </section>

            <!-- 2. 同步与通知行为 -->
            <section v-else-if="state.activeTab.value === 'sync'" class="km-tab-panel">
              <div class="km-tab-header-row">
                <h3 class="km-tab-title">同步与通知行为</h3>
                <button
                  type="button"
                  class="km-btn km-btn-secondary"
                  @click="emit('openAccounts')"
                >
                  管理邮箱账号…
                </button>
              </div>

              <div class="km-card">
                <div class="km-field-row">
                  <label for="km-pref-sync-mins" class="km-label">自动同步周期（分钟，0 为手动）</label>
                  <input
                    id="km-pref-sync-mins"
                    type="number"
                    min="0"
                    max="1440"
                    class="km-input km-num-input"
                    :value="state.preferences.value.autoSyncMinutes"
                    @change="onNumberInput('autoSyncMinutes', $event)"
                  />
                </div>

                <label class="km-toggle-row">
                  <input
                    type="checkbox"
                    :checked="state.preferences.value.pushEnabled"
                    @change="onCheckbox('pushEnabled', $event)"
                  />
                  <span>启用实时推送 / IMAP IDLE 监听新邮件</span>
                </label>
                <label class="km-toggle-row">
                  <input
                    type="checkbox"
                    :checked="state.preferences.value.desktopNotifications"
                    @change="onCheckbox('desktopNotifications', $event)"
                  />
                  <span>收到新邮件时发送系统桌面通知</span>
                </label>
                <label class="km-toggle-row">
                  <input
                    type="checkbox"
                    :checked="state.preferences.value.notificationShowContent"
                    @change="onCheckbox('notificationShowContent', $event)"
                  />
                  <span>在桌面通知中显示发件人与主题预览</span>
                </label>
                <label class="km-toggle-row">
                  <input
                    type="checkbox"
                    :checked="state.preferences.value.closeToMinimize"
                    @change="onCheckbox('closeToMinimize', $event)"
                  />
                  <span>点击窗口关闭按钮时最小化而非退出</span>
                </label>
                <label class="km-toggle-row">
                  <input
                    type="checkbox"
                    :checked="state.preferences.value.minimizeToTray"
                    @change="onCheckbox('minimizeToTray', $event)"
                  />
                  <span>最小化时驻留系统托盘</span>
                </label>
                <label class="km-toggle-row">
                  <input
                    type="checkbox"
                    :checked="state.preferences.value.checkUpdates"
                    @change="onCheckbox('checkUpdates', $event)"
                  />
                  <span>启动时自动检查版本更新</span>
                </label>
              </div>
            </section>

            <!-- 3. 写信与阅读行为 -->
            <section v-else-if="state.activeTab.value === 'compose'" class="km-tab-panel">
              <h3 class="km-tab-title">写信、撤销发送与阅读行为</h3>

              <div class="km-card">
                <div class="km-field-row">
                  <label for="km-pref-read-mark" class="km-label">自动标记已读策略</label>
                  <select
                    id="km-pref-read-mark"
                    :value="state.preferences.value.readMarkMode"
                    class="km-select"
                    @change="onSelectString('readMarkMode', $event)"
                  >
                    <option value="shown">打开邮件立即标为已读</option>
                    <option value="delay">停留 2 秒后标为已读</option>
                    <option value="manual">仅手动标记已读</option>
                  </select>
                </div>

                <label class="km-toggle-row">
                  <input
                    type="checkbox"
                    :checked="state.preferences.value.threadNewestFirst"
                    @change="onCheckbox('threadNewestFirst', $event)"
                  />
                  <span>会话卡片按最新回复置顶排序</span>
                </label>

                <div class="km-field-row">
                  <label for="km-pref-undo-secs" class="km-label">撤销发送缓冲窗口（秒，0-30）</label>
                  <input
                    id="km-pref-undo-secs"
                    type="number"
                    min="0"
                    max="30"
                    class="km-input km-num-input"
                    :value="state.preferences.value.undoSendSecs"
                    @change="onNumberInput('undoSendSecs', $event)"
                  />
                </div>

                <label class="km-toggle-row">
                  <input
                    type="checkbox"
                    :checked="state.preferences.value.signatureAboveQuote"
                    @change="onCheckbox('signatureAboveQuote', $event)"
                  />
                  <span>回复/转发时将签名置于引用正文上方</span>
                </label>
                <label class="km-toggle-row">
                  <input
                    type="checkbox"
                    :checked="state.preferences.value.signatureDashes"
                    @change="onCheckbox('signatureDashes', $event)"
                  />
                  <span>在签名前自动添加 `-- ` 标准分隔符</span>
                </label>

                <div class="km-field">
                  <label for="km-pref-attach-kw" class="km-label">遗漏附件提醒关键词（逗号分隔）</label>
                  <input
                    id="km-pref-attach-kw"
                    type="text"
                    class="km-input"
                    :value="state.preferences.value.attachmentReminderKeywords"
                    @change="onTextInput('attachmentReminderKeywords', $event)"
                  />
                </div>

                <div class="km-field">
                  <label for="km-pref-reply-tpl" class="km-label">回复引用抬头模板</label>
                  <input
                    id="km-pref-reply-tpl"
                    type="text"
                    class="km-input"
                    :value="state.preferences.value.replyHeaderTemplate"
                    @change="onTextInput('replyHeaderTemplate', $event)"
                  />
                </div>
              </div>
            </section>

            <!-- 4. 隐私、远图白名单与安全锁 -->
            <section v-else-if="state.activeTab.value === 'privacy'" class="km-tab-panel">
              <h3 class="km-tab-title">隐私保护、远图白名单与安全锁</h3>

              <div class="km-card">
                <div class="km-card-title">远程图片与链接跟踪防护</div>
                <label class="km-toggle-row">
                  <input
                    type="checkbox"
                    :checked="state.preferences.value.blockRemoteImages"
                    @change="onCheckbox('blockRemoteImages', $event)"
                  />
                  <span>默认拦截 HTML 邮件中的外部远程图片</span>
                </label>
                <label class="km-toggle-row">
                  <input
                    type="checkbox"
                    :checked="state.preferences.value.showRemoteBanner"
                    @change="onCheckbox('showRemoteBanner', $event)"
                  />
                  <span>拦截远程图片时在阅读器顶部显示放行提示条</span>
                </label>
                <label class="km-toggle-row">
                  <input
                    type="checkbox"
                    :checked="state.preferences.value.cleanLinks"
                    @change="onCheckbox('cleanLinks', $event)"
                  />
                  <span>自动清理常见邮件跟踪链接参数 (UTM / fbclid)</span>
                </label>
              </div>

              <!-- 远图信任发件人白名单 -->
              <div class="km-card">
                <div class="km-card-header-row">
                  <div class="km-card-title">
                    远图白名单发件人 ({{ state.allowedRemoteSenders.value.length }})
                  </div>
                  <button
                    v-if="state.allowedRemoteSenders.value.length > 0"
                    type="button"
                    class="km-btn km-btn-ghost km-danger-text"
                    :disabled="state.sendersBusy.value"
                    @click="state.clearTrustedSenders()"
                  >
                    清空全部
                  </button>
                </div>

                <div class="km-inline-add">
                  <input
                    v-model="state.newTrustedSenderInput.value"
                    type="email"
                    class="km-input"
                    placeholder="输入信任的发件人邮箱，如 newsletter@example.com"
                    @keydown.enter.prevent="state.addTrustedSender()"
                  />
                  <button
                    type="button"
                    class="km-btn km-btn-secondary"
                    :disabled="state.sendersBusy.value"
                    @click="state.addTrustedSender()"
                  >
                    添加
                  </button>
                </div>

                <div v-if="state.allowedRemoteSenders.value.length === 0" class="km-muted-hint">
                  暂无白名单发件人，所有外部远程图片默认受拦截保护。
                </div>
                <ul v-else class="km-sender-list">
                  <li
                    v-for="sender in state.allowedRemoteSenders.value"
                    :key="sender"
                    class="km-sender-item"
                  >
                    <span class="km-mono">{{ sender }}</span>
                    <button
                      type="button"
                      class="km-btn km-btn-ghost km-danger-text"
                      :disabled="state.sendersBusy.value"
                      @click="state.removeTrustedSender(sender)"
                    >
                      移除
                    </button>
                  </li>
                </ul>
              </div>

              <!-- 应用安全锁口令 -->
              <div class="km-card">
                <div class="km-card-header-row">
                  <div class="km-card-title">应用安全锁与口令</div>
                  <span class="km-status-pill" :class="{ active: state.hasPasscode.value }">
                    {{ state.hasPasscode.value ? "已设置口令" : "未设置口令" }}
                  </span>
                </div>

                <div class="km-grid-2">
                  <div v-if="state.hasPasscode.value" class="km-field">
                    <label for="km-lock-current" class="km-label">当前口令 *</label>
                    <input
                      id="km-lock-current"
                      v-model="state.currentPasscodeInput.value"
                      type="password"
                      class="km-input"
                      autocomplete="current-password"
                      placeholder="验证当前口令"
                    />
                  </div>
                  <div class="km-field">
                    <label for="km-lock-new" class="km-label">
                      {{ state.hasPasscode.value ? "新口令（至少 4 位）" : "设置口令（至少 4 位）" }}
                    </label>
                    <input
                      id="km-lock-new"
                      v-model="state.newPasscodeInput.value"
                      type="password"
                      class="km-input"
                      autocomplete="new-password"
                      placeholder="输入新安全锁口令"
                    />
                  </div>
                  <div class="km-field">
                    <label for="km-lock-confirm" class="km-label">确认新口令</label>
                    <input
                      id="km-lock-confirm"
                      v-model="state.confirmPasscodeInput.value"
                      type="password"
                      class="km-input"
                      autocomplete="new-password"
                      placeholder="再次输入新口令"
                    />
                  </div>
                </div>

                <label class="km-toggle-row">
                  <input
                    type="checkbox"
                    :checked="state.preferences.value.appLockOnStartup"
                    :disabled="!state.hasPasscode.value"
                    @change="onCheckbox('appLockOnStartup', $event)"
                  />
                  <span>启动应用时自动进入锁屏状态</span>
                </label>

                <div class="km-field-row">
                  <label for="km-pref-idle-mins" class="km-label">闲置自动锁定时间（分钟）</label>
                  <input
                    id="km-pref-idle-mins"
                    type="number"
                    min="1"
                    max="240"
                    class="km-input km-num-input"
                    :value="state.preferences.value.appLockIdleMinutes"
                    @change="onNumberInput('appLockIdleMinutes', $event)"
                  />
                </div>

                <div v-if="state.passcodeError.value" class="km-field-err" role="alert">
                  {{ state.passcodeError.value }}
                </div>

                <div class="km-inline-actions">
                  <button
                    type="button"
                    class="km-btn km-btn-primary"
                    :disabled="state.passcodeBusy.value"
                    @click="state.submitPasscode()"
                  >
                    {{ state.hasPasscode.value ? "更新安全锁口令" : "设置并启用口令" }}
                  </button>
                  <button
                    v-if="state.hasPasscode.value"
                    type="button"
                    class="km-btn km-btn-danger-outline"
                    :disabled="state.passcodeBusy.value"
                    @click="state.clearPasscode()"
                  >
                    清除口令
                  </button>
                </div>
              </div>
            </section>

            <!-- 5. 存储路径与正文缓存 -->
            <section v-else-if="state.activeTab.value === 'storage'" class="km-tab-panel">
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
                  <label for="km-pref-keep-days" class="km-label">清理范围</label>
                  <select
                    id="km-pref-keep-days"
                    :value="state.cacheKeepDays.value === null ? 'all' : String(state.cacheKeepDays.value)"
                    class="km-select"
                    @change="onKeepDaysChange"
                  >
                    <option value="all">清理全部本地正文缓存</option>
                    <option value="30">保留最近 30 天正文缓存</option>
                    <option value="7">保留最近 7 天正文缓存</option>
                  </select>
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

            <!-- 6. 关于、版本与更新检查 -->
            <section v-else-if="state.activeTab.value === 'about'" class="km-tab-panel">
              <h3 class="km-tab-title">关于 KiteMail 与检查更新</h3>

              <div class="km-card">
                <div class="km-about-hero">
                  <div>
                    <div class="km-about-name">KiteMail Desktop</div>
                    <div class="km-about-meta">
                      当前版本：<strong>v{{ state.appVersion.value }}</strong>
                      · 运行形态：
                      <span class="km-status-pill">
                        {{ state.isPortable.value ? "便携版 (Portable)" : "标准安装版" }}
                      </span>
                    </div>
                  </div>
                  <button
                    type="button"
                    class="km-btn km-btn-primary"
                    :disabled="state.checkingUpdate.value"
                    @click="state.checkForUpdates()"
                  >
                    {{ state.checkingUpdate.value ? "检查中…" : "检查更新" }}
                  </button>
                </div>

                <div v-if="state.updateResult.value" class="km-update-box">
                  <div class="km-update-msg">{{ state.updateResult.value.message }}</div>
                  <div class="km-update-meta">
                    目标平台：{{ state.updateResult.value.target }}
                    <template v-if="state.updateResult.value.latestVersion">
                      · 最新版本：v{{ state.updateResult.value.latestVersion }}
                    </template>
                    <template v-if="state.updateResult.value.pubDate">
                      · 发布日期：{{ state.updateResult.value.pubDate }}
                    </template>
                  </div>
                  <p v-if="state.updateResult.value.notes" class="km-update-notes">
                    {{ state.updateResult.value.notes }}
                  </p>
                  <a
                    v-if="state.updateResult.value.downloadUrl"
                    :href="state.updateResult.value.downloadUrl"
                    target="_blank"
                    rel="noopener noreferrer"
                    class="km-update-link"
                  >
                    前往下载新版本
                  </a>
                </div>
              </div>
            </section>

            <!-- 全局操作状态横幅 -->
            <div
              v-if="state.banner.value"
              class="km-banner"
              :class="`is-${state.banner.value.kind}`"
              role="status"
            >
              {{ state.banner.value.text }}
            </div>
          </template>
        </main>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, watch } from "vue";
import { BUILTIN_THEMES } from "../ipc/types";
import type { FullPreferencesDto, PasscodeStatus, PreferenceTabId } from "./types";
import { formatBytes, usePreferences } from "./usePreferences";

const props = withDefaults(
  defineProps<{
    open: boolean;
    initialTab?: PreferenceTabId;
  }>(),
  {
    initialTab: "appearance",
  }
);

const emit = defineEmits<{
  "update:open": [open: boolean];
  close: [];
  preferencesUpdated: [prefs: FullPreferencesDto];
  passcodeChanged: [status: PasscodeStatus];
  openAccounts: [];
  overlayChange: [id: string, open: boolean];
}>();

const OVERLAY_ID = "preferences-modal";

const TABS: ReadonlyArray<{ id: PreferenceTabId; label: string; desc: string }> = [
  { id: "appearance", label: "外观与布局", desc: "主题、深浅色与窗格" },
  { id: "sync", label: "同步与通知", desc: "轮询、托盘与提示" },
  { id: "compose", label: "写信与阅读", desc: "撤销、签名与已读" },
  { id: "privacy", label: "隐私与安全锁", desc: "远图白名单与口令" },
  { id: "storage", label: "存储与缓存", desc: "目录统计与清理" },
  { id: "about", label: "关于与更新", desc: "版本形态与更新" },
];

const state = usePreferences({
  onLocked() {
    closeModal();
  },
  onPreferencesUpdated(prefs) {
    emit("preferencesUpdated", prefs);
  },
  onPasscodeChanged(status) {
    emit("passcodeChanged", status);
  },
});

const darkModeSelectValue = computed(() => {
  if (state.preferences.value.darkMode === true) return "dark";
  if (state.preferences.value.darkMode === false) return "light";
  return "system";
});

function onDarkModeChange(ev: Event) {
  const val = (ev.target as HTMLSelectElement).value;
  const darkMode = val === "dark" ? true : val === "light" ? false : null;
  void state.updatePreference({ darkMode });
}

function onSelectString(
  field: "readerPosition" | "readMarkMode" | "defaultView",
  ev: Event
) {
  const val = (ev.target as HTMLSelectElement).value;
  void state.updatePreference({ [field]: val } as Partial<FullPreferencesDto>);
}

function onCheckbox(
  field: keyof FullPreferencesDto,
  ev: Event
) {
  const checked = (ev.target as HTMLInputElement).checked;
  void state.updatePreference({ [field]: checked } as Partial<FullPreferencesDto>);
}

function onNumberInput(
  field:
    | "readerZoomPercent"
    | "uiTextScalePercent"
    | "autoSyncMinutes"
    | "undoSendSecs"
    | "appLockIdleMinutes",
  ev: Event
) {
  const num = Number((ev.target as HTMLInputElement).value);
  if (!Number.isFinite(num)) return;
  void state.updatePreference({ [field]: num });
}

function onTextInput(
  field: "attachmentReminderKeywords" | "replyHeaderTemplate",
  ev: Event
) {
  const val = (ev.target as HTMLInputElement).value;
  void state.updatePreference({ [field]: val });
}

function onKeepDaysChange(ev: Event) {
  const val = (ev.target as HTMLSelectElement).value;
  state.cacheKeepDays.value = val === "all" ? null : Number(val);
}

function closeModal() {
  if (!props.open) return;
  emit("overlayChange", OVERLAY_ID, false);
  emit("update:open", false);
  emit("close");
}

function handleKeydown(ev: KeyboardEvent) {
  if (!props.open) return;
  if (ev.key === "Escape") {
    ev.preventDefault();
    ev.stopPropagation();
    closeModal();
  }
}

function attachGlobalListeners() {
  if (typeof window !== "undefined") {
    window.addEventListener("keydown", handleKeydown);
  }
}

function detachGlobalListeners() {
  if (typeof window !== "undefined") {
    window.removeEventListener("keydown", handleKeydown);
  }
}

watch(
  () => props.open,
  (isOpen, prevOpen) => {
    if (isOpen && !prevOpen) {
      emit("overlayChange", OVERLAY_ID, true);
      attachGlobalListeners();
      void state.init(props.initialTab);
    } else if (!isOpen && prevOpen) {
      detachGlobalListeners();
      emit("overlayChange", OVERLAY_ID, false);
    }
  }
);

watch(
  () => props.initialTab,
  (tab) => {
    if (props.open && tab) {
      state.activeTab.value = tab;
    }
  }
);

onMounted(() => {
  if (props.open) {
    emit("overlayChange", OVERLAY_ID, true);
    attachGlobalListeners();
    void state.init(props.initialTab);
  }
});

onUnmounted(() => {
  detachGlobalListeners();
  if (props.open) {
    emit("overlayChange", OVERLAY_ID, false);
  }
  state.dispose();
});
</script>

<style scoped>
.km-prefs-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  background: rgba(0, 0, 0, 0.48);
  backdrop-filter: blur(4px);
}

.km-prefs-modal {
  width: min(920px, 100%);
  height: min(650px, calc(100vh - 32px));
  display: flex;
  flex-direction: column;
  border-radius: var(--km-radius-lg);
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  box-shadow: var(--km-shadow-popover);
  overflow: hidden;
}

.km-modal-header {
  height: 52px;
  padding: 0 18px;
  border-bottom: 1px solid var(--km-border);
  background: var(--km-surface);
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-shrink: 0;
}

.km-modal-title-group {
  display: flex;
  align-items: baseline;
  gap: 10px;
}

.km-modal-title {
  margin: 0;
  font-size: 15px;
  font-weight: 700;
  color: var(--km-text);
}

.km-modal-subtitle {
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-close-btn {
  width: 30px;
  height: 30px;
  border: none;
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-text-muted);
  font-size: 14px;
  cursor: pointer;
}

.km-close-btn:hover {
  background: var(--km-hover-bg);
  color: var(--km-text);
}

.km-modal-body {
  flex: 1;
  min-height: 0;
  display: flex;
}

.km-prefs-nav {
  width: 216px;
  border-right: 1px solid var(--km-border);
  background: var(--km-sidebar);
  padding: 10px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  overflow-y: auto;
}

.km-nav-item {
  border: 1px solid transparent;
  border-radius: var(--km-radius-md);
  background: transparent;
  padding: 9px 10px;
  text-align: left;
  display: flex;
  flex-direction: column;
  gap: 2px;
  cursor: pointer;
}

.km-nav-item:hover {
  background: var(--km-hover-bg);
}

.km-nav-item.active {
  background: var(--km-surface);
  border-color: var(--km-accent);
  box-shadow: var(--km-shadow-card);
}

.km-nav-label {
  font-size: 13px;
  font-weight: 600;
  color: var(--km-text);
}

.km-nav-desc {
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-prefs-content {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  padding: 18px 22px;
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.km-tab-panel {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.km-tab-header-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.km-tab-title {
  margin: 0;
  font-size: 15px;
  font-weight: 700;
  color: var(--km-text);
}

.km-card {
  padding: 14px;
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  background: var(--km-surface);
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.km-card-header-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.km-card-title {
  font-size: 12px;
  font-weight: 700;
  color: var(--km-text-muted);
}

.km-theme-grid {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.km-theme-chip {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  padding: 6px 10px;
  border-radius: var(--km-radius-sm);
  border: 1px solid var(--km-border);
  background: var(--km-canvas);
  font-size: 12px;
  cursor: pointer;
}

.km-theme-chip.active {
  border-color: var(--km-accent);
  background: var(--km-accent-soft);
  font-weight: 600;
}

.km-theme-swatch {
  width: 12px;
  height: 12px;
  border-radius: 999px;
  flex-shrink: 0;
}

.km-field-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.km-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.km-grid-2 {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}

.km-label {
  font-size: 12px;
  font-weight: 500;
  color: var(--km-text);
}

.km-input,
.km-select {
  height: 32px;
  padding: 0 10px;
  border-radius: var(--km-radius-sm);
  border: 1px solid var(--km-border);
  background: var(--km-canvas);
  color: var(--km-text);
  user-select: text;
}

.km-select {
  min-width: 190px;
}

.km-num-input {
  width: 110px;
}

.km-toggle-row {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--km-text);
  cursor: pointer;
}

.km-inline-add,
.km-inline-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.km-inline-add .km-input {
  flex: 1;
}

.km-sender-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
  max-height: 160px;
  overflow-y: auto;
}

.km-sender-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 10px;
  border-radius: var(--km-radius-sm);
  border: 1px solid var(--km-border);
  background: var(--km-canvas);
  font-size: 12px;
}

.km-mono {
  font-family: var(--km-font-mono);
}

.km-muted-hint {
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-status-pill {
  padding: 2px 8px;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 600;
  background: var(--km-hover-bg);
  color: var(--km-text-muted);
}

.km-status-pill.active {
  background: var(--km-accent-soft);
  color: var(--km-accent);
}

.km-field-err {
  font-size: 11px;
  color: var(--km-danger);
}

.km-path-row {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.km-path-label {
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-path-code {
  padding: 6px 8px;
  border-radius: var(--km-radius-sm);
  background: var(--km-canvas);
  border: 1px solid var(--km-border);
  font-family: var(--km-font-mono);
  font-size: 11px;
  word-break: break-all;
  user-select: text;
}

.km-stats-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 8px;
}

.km-stat-box {
  padding: 10px;
  border-radius: var(--km-radius-sm);
  border: 1px solid var(--km-border);
  background: var(--km-canvas);
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.km-stat-label {
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-stat-value {
  font-size: 14px;
  color: var(--km-text);
}

.km-about-hero {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.km-about-name {
  font-size: 16px;
  font-weight: 700;
  color: var(--km-text);
}

.km-about-meta {
  margin-top: 4px;
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-update-box {
  padding: 10px 12px;
  border-radius: var(--km-radius-sm);
  border: 1px solid var(--km-border);
  background: var(--km-canvas);
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 12px;
}

.km-update-msg {
  font-weight: 600;
}

.km-update-meta,
.km-update-notes {
  margin: 0;
  color: var(--km-text-muted);
  white-space: pre-wrap;
}

.km-update-link {
  color: var(--km-accent);
  font-weight: 600;
  text-decoration: none;
}

.km-banner {
  padding: 9px 12px;
  border-radius: var(--km-radius-sm);
  font-size: 12px;
  border: 1px solid var(--km-border);
  background: var(--km-canvas);
}

.km-banner.is-success {
  border-color: color-mix(in srgb, #26a269 45%, var(--km-border));
  background: color-mix(in srgb, #26a269 12%, transparent);
}

.km-banner.is-error {
  border-color: color-mix(in srgb, var(--km-danger) 50%, var(--km-border));
  background: var(--km-danger-soft);
  color: var(--km-danger);
}

.km-banner.is-info {
  border-color: color-mix(in srgb, var(--km-accent) 45%, var(--km-border));
  background: var(--km-accent-soft);
}

.km-btn {
  height: 32px;
  padding: 0 12px;
  border-radius: var(--km-radius-sm);
  border: 1px solid var(--km-border);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
}

.km-btn:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.km-btn-primary {
  border-color: var(--km-accent);
  background: var(--km-accent);
  color: var(--km-accent-fg);
}

.km-btn-secondary {
  background: var(--km-surface);
  color: var(--km-text);
}

.km-btn-secondary:hover:not(:disabled) {
  background: var(--km-hover-bg);
}

.km-btn-ghost {
  border-color: transparent;
  background: transparent;
  color: var(--km-accent);
}

.km-btn-ghost:hover:not(:disabled) {
  background: var(--km-hover-bg);
}

.km-btn-danger-outline {
  border-color: color-mix(in srgb, var(--km-danger) 50%, var(--km-border));
  background: transparent;
  color: var(--km-danger);
}

.km-btn-danger-outline:hover:not(:disabled) {
  background: var(--km-danger-soft);
}

.km-danger-text {
  color: var(--km-danger);
}

.km-state-empty,
.km-state-error {
  padding: 28px;
  text-align: center;
  color: var(--km-text-muted);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
}

.km-state-error {
  color: var(--km-danger);
}

@media (max-width: 760px) {
  .km-modal-body {
    flex-direction: column;
  }

  .km-prefs-nav {
    width: 100%;
    flex-direction: row;
    overflow-x: auto;
    border-right: none;
    border-bottom: 1px solid var(--km-border);
  }

  .km-stats-grid,
  .km-grid-2 {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
</style>

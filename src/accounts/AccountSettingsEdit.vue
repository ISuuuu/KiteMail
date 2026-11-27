<template>
  <div class="as-edit-root">
    <!-- 顶栏：返回 / 标题 / 移除 + 保存（preferences/mod.rs#L2465）；
         hostChrome=true 时由宿主壳层承载单条顶栏，此处隐藏避免双顶栏（WP-07） -->
    <header v-if="!hostChrome" class="as-edit-header">
      <div class="as-edit-header-left">
        <button type="button" class="as-btn as-btn-ghost as-btn-compact" @click="emit('back')">
          <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
            <path
              d="M9.5 3.5L5 8l4.5 4.5"
              fill="none"
              stroke="currentColor"
              stroke-width="1.6"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
          </svg>
          返回
        </button>
        <span class="as-edit-header-title">{{ editingId ? "编辑账号" : "添加账号" }}</span>
      </div>
      <div class="as-edit-header-actions">
        <button
          v-if="editingId && !demoMode"
          type="button"
          class="as-btn as-btn-danger as-btn-compact"
          @click="confirmRemove = !confirmRemove"
        >
          <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
            <path
              d="M3 4.5h10M6.5 4.5V3.2c0-.4.3-.7.7-.7h1.6c.4 0 .7.3.7.7v1.3M4.5 4.5l.6 8c0 .7.5 1.2 1.2 1.2h3.4c.7 0 1.2-.5 1.2-1.2l.6-8"
              fill="none"
              stroke="currentColor"
              stroke-width="1.3"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
          </svg>
          移除
        </button>
        <button
          v-if="!demoMode"
          type="button"
          class="as-btn as-btn-primary as-btn-compact"
          :disabled="state.saving.value || state.verifying.value"
          @click="handleSave"
        >
          {{ state.saving.value ? "正在保存…" : "保存" }}
        </button>
      </div>
    </header>

    <!-- 未保存提示横幅（preferences/mod.rs#L2393） -->
    <div v-if="pendingLeave" class="as-unsaved-banner" role="alert">
      <div class="as-unsaved-text">
        <p class="as-unsaved-title">当前账号有未保存的更改</p>
        <p class="as-unsaved-desc">是否保存更改后再离开编辑页面？</p>
      </div>
      <div class="as-unsaved-actions">
        <button
          type="button"
          class="as-btn as-btn-outline as-btn-compact"
          @click="emit('leave-cancel')"
        >
          取消
        </button>
        <button
          v-if="!demoMode"
          type="button"
          class="as-btn as-btn-danger as-btn-compact"
          :disabled="state.saving.value"
          @click="emit('leave-discard')"
        >
          放弃更改
        </button>
        <button
          v-if="!demoMode"
          type="button"
          class="as-btn as-btn-primary as-btn-compact"
          :disabled="state.saving.value || state.verifying.value"
          @click="emit('leave-save')"
        >
          {{ state.saving.value ? "正在保存…" : "保存" }}
        </button>
      </div>
    </div>

    <!-- 内容滚动区（px 24 / py 20，卡片列最大 600px） -->
    <div class="as-edit-scroll">
      <div class="as-page-inner">
        <!-- 品牌头部：56px 服务商标记 + 账号标识 -->
        <div class="as-edit-brand-head">
          <img
            v-if="topBrandId"
            class="as-edit-brand-img"
            :src="BRAND_IMAGE_URLS[topBrandId]"
            alt=""
          />
          <span v-else class="as-edit-brand-pill">IMAP</span>
          <p v-if="editingAccount" class="as-edit-brand-label">
            {{ displayLabel(editingAccount) }}
          </p>
        </div>

        <!-- 移除确认内联横幅（accounts.rs#L3749） -->
        <div v-if="confirmRemove && editingId" class="as-remove-banner">
          <div class="as-remove-text">
            <p class="as-remove-title">确定移除此邮箱账号？</p>
            <p class="as-remove-desc">将删除本地邮件缓存、配置与凭据管理器中的密钥。</p>
          </div>
          <div class="as-remove-actions">
            <button
              type="button"
              class="as-btn as-btn-outline as-btn-compact"
              @click="confirmRemove = false"
            >
              取消
            </button>
            <button
              type="button"
              class="as-btn as-btn-danger as-btn-compact"
              :disabled="state.deletingId.value === editingId"
              @click="handleRemove"
            >
              <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
                <path
                  d="M3 4.5h10M6.5 4.5V3.2c0-.4.3-.7.7-.7h1.6c.4 0 .7.3.7.7v1.3M4.5 4.5l.6 8c0 .7.5 1.2 1.2 1.2h3.4c.7 0 1.2-.5 1.2-1.2l.6-8"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.3"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                />
              </svg>
              {{ state.deletingId.value === editingId ? "删除中…" : "确认移除" }}
            </button>
          </div>
        </div>

        <!-- 保存 / 测试 / 校验反馈横幅 -->
        <div
          v-if="state.statusBanner.value"
          class="as-feedback-banner"
          :class="state.statusBanner.value.kind === 'error' ? 'is-error' : 'is-success'"
          role="status"
        >
          {{ state.statusBanner.value.text }}
        </div>

        <!-- 分组 1：邮箱账号（服务商 + 身份 + 服务器 + 收件箱显示） -->
        <section class="as-group">
          <h3 class="as-group-title">邮箱账号</h3>
          <div class="as-group-card">
            <!-- 服务商 -->
            <div class="as-trow">
              <div class="as-trow-main">
                <p class="as-trow-title">服务商</p>
                <p v-if="activePresetNote" class="as-trow-desc">{{ activePresetNote }}</p>
              </div>
              <div class="as-trow-side as-provider-side">
                <label class="as-combo-wrap">
                  <select
                    class="as-combo"
                    :value="state.form.value.presetId"
                    :disabled="oauthLocked || demoMode"
                    aria-label="邮箱服务商"
                    @change="onPresetChange"
                  >
                    <option
                      v-for="preset in state.presets.value"
                      :key="preset.id"
                      :value="preset.id"
                    >
                      {{ preset.name }}
                    </option>
                  </select>
                  <svg class="as-combo-chevron" viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
                    <path
                      d="M4 6.5L8 10.5l4-4"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="1.5"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                    />
                  </svg>
                </label>
                <button
                  v-if="state.hasLockedPresetFields.value && activePreset && activePreset.id !== 'manual'"
                  type="button"
                  class="as-btn as-btn-ghost as-btn-compact"
                  title="清空手动修改，恢复当前预设默认参数"
                  @click="state.resetToPreset()"
                >
                  重置预设
                </button>
              </div>
            </div>

            <!-- OAuth（Microsoft） -->
            <template v-if="isOAuth">
              <div v-if="editingAccount" class="as-frow">
                <span class="as-frow-label">账号邮箱</span>
                <span class="as-frow-static">{{ editingAccount.email }}</span>
              </div>
              <div class="as-frow">
                <label class="as-frow-label" for="as-acc-label-oauth">昵称</label>
                <div class="as-frow-control">
                  <input
                    id="as-acc-label-oauth"
                    v-model="state.form.value.label"
                    type="text"
                    class="as-input"
                    placeholder="例如 工作 / 个人"
                  />
                </div>
              </div>
              <div class="as-frow">
                <label class="as-frow-label" for="as-acc-display-oauth">寄件人名称</label>
                <div class="as-frow-control">
                  <input
                    id="as-acc-display-oauth"
                    v-model="state.form.value.displayName"
                    type="text"
                    class="as-input"
                    placeholder="例如 张三"
                  />
                  <span v-if="state.validationErrors.value.displayName" class="as-field-err">
                    {{ state.validationErrors.value.displayName }}
                  </span>
                </div>
              </div>
              <div class="as-trow">
                <div class="as-trow-main">
                  <p class="as-trow-title">Microsoft OAuth2 授权</p>
                  <p class="as-trow-desc">无需输入密码，通过系统浏览器完成 Microsoft 账号登录</p>
                </div>
                <button
                  type="button"
                  class="as-btn as-btn-primary as-btn-compact"
                  :disabled="demoMode || state.oauthStatus.value === 'pending'"
                  @click="state.startMicrosoftOAuthFlow()"
                >
                  {{
                    state.oauthStatus.value === "pending"
                      ? "等待授权回调…"
                      : editingId
                        ? "重新授权 Microsoft 账号"
                        : "使用 Microsoft 账号登录"
                  }}
                </button>
              </div>
              <div
                v-if="state.oauthMessage.value"
                class="as-inline-banner"
                :class="oauthBannerClass"
                role="status"
              >
                {{ state.oauthMessage.value }}
              </div>
              <div class="as-adv-wrap">
                <button
                  type="button"
                  class="as-adv-toggle"
                  :aria-expanded="msAdvancedExpanded"
                  @click="msAdvancedExpanded = !msAdvancedExpanded"
                >
                  <span>高级：自定义 Entra Client ID</span>
                  <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
                    <path
                      v-if="msAdvancedExpanded"
                      d="M4 10L8 6l4 4"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="1.5"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                    />
                    <path
                      v-else
                      d="M4 6.5L8 10.5l4-4"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="1.5"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                    />
                  </svg>
                </button>
                <div v-if="msAdvancedExpanded" class="as-adv-body">
                  <input
                    v-model="state.form.value.oauthClientId"
                    type="text"
                    class="as-input as-mono"
                    placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
                    aria-label="自定义 OAuth Client ID"
                  />
                </div>
              </div>
            </template>

            <!-- IMAP / SMTP 密码模式 -->
            <template v-else>
              <div class="as-frow">
                <label class="as-frow-label" for="as-acc-label">昵称</label>
                <div class="as-frow-control">
                  <input
                    id="as-acc-label"
                    v-model="state.form.value.label"
                    type="text"
                    class="as-input"
                    placeholder="例如 工作 / 个人"
                  />
                </div>
              </div>
              <div class="as-frow">
                <label class="as-frow-label" for="as-acc-display">寄件人名称</label>
                <div class="as-frow-control">
                  <input
                    id="as-acc-display"
                    v-model="state.form.value.displayName"
                    type="text"
                    class="as-input"
                    placeholder="例如 张三"
                  />
                  <span v-if="state.validationErrors.value.displayName" class="as-field-err">
                    {{ state.validationErrors.value.displayName }}
                  </span>
                </div>
              </div>
              <div class="as-frow">
                <label class="as-frow-label" for="as-acc-email">邮箱地址</label>
                <div class="as-frow-control">
                  <input
                    id="as-acc-email"
                    :value="state.form.value.email"
                    type="email"
                    class="as-input"
                    placeholder="name@example.com"
                    autocomplete="email"
                    @input="onEmailInput"
                  />
                  <span v-if="state.validationErrors.value.email" class="as-field-err">
                    {{ state.validationErrors.value.email }}
                  </span>
                </div>
              </div>
              <div class="as-frow">
                <span class="as-frow-label">IMAP 服务器</span>
                <div class="as-frow-control as-server-line">
                  <input
                    :value="state.form.value.imapHost"
                    type="text"
                    class="as-input as-mono as-server-host"
                    placeholder="imap.example.com"
                    aria-label="IMAP 服务器地址"
                    @input="onServerText('imapHost', $event)"
                  />
                  <input
                    :value="state.form.value.imapPort"
                    type="number"
                    min="1"
                    max="65535"
                    class="as-input as-mono as-server-port"
                    aria-label="IMAP 端口"
                    @input="onServerPort('imapPort', $event)"
                  />
                  <span class="as-combo-wrap as-server-sec">
                    <select
                      class="as-combo"
                      :value="state.form.value.imapSecurity"
                      :disabled="demoMode"
                      aria-label="IMAP 加密方式"
                      @change="onServerSecurity('imapSecurity', $event)"
                    >
                      <option value="tls">SSL/TLS</option>
                      <option value="startTls">STARTTLS</option>
                    </select>
                    <svg class="as-combo-chevron" viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
                      <path
                        d="M4 6.5L8 10.5l4-4"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.5"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                      />
                    </svg>
                  </span>
                </div>
                <span v-if="state.validationErrors.value.imapHost || state.validationErrors.value.imapPort" class="as-field-err as-server-err">
                  {{ state.validationErrors.value.imapHost ?? state.validationErrors.value.imapPort }}
                </span>
              </div>
              <div class="as-frow">
                <label class="as-frow-label" for="as-acc-imap-user">IMAP 用户名</label>
                <div class="as-frow-control">
                  <input
                    id="as-acc-imap-user"
                    :value="state.form.value.imapUsername"
                    type="text"
                    class="as-input"
                    placeholder="通常为完整邮箱地址"
                    @input="onImapUsernameInput"
                  />
                  <span v-if="state.validationErrors.value.imapUsername" class="as-field-err">
                    {{ state.validationErrors.value.imapUsername }}
                  </span>
                </div>
              </div>
              <div class="as-frow">
                <label class="as-frow-label" for="as-acc-imap-pwd">
                  {{ editingId && state.form.value.hasPassword ? "密码 / 授权码（留空保留）" : "密码 / 授权码" }}
                </label>
                <div class="as-frow-control">
                  <input
                    id="as-acc-imap-pwd"
                    v-model="state.form.value.imapPassword"
                    type="password"
                    class="as-input"
                    autocomplete="new-password"
                    :placeholder="
                      editingId && state.form.value.hasPassword
                        ? '已保存密码，留空则保持不变'
                        : '请输入邮箱密码或授权码'
                    "
                  />
                  <span v-if="state.validationErrors.value.imapPassword" class="as-field-err">
                    {{ state.validationErrors.value.imapPassword }}
                  </span>
                </div>
              </div>
              <div class="as-frow">
                <span class="as-frow-label">SMTP 服务器</span>
                <div class="as-frow-control as-server-line">
                  <input
                    :value="state.form.value.smtpHost"
                    type="text"
                    class="as-input as-mono as-server-host"
                    placeholder="smtp.example.com"
                    aria-label="SMTP 服务器地址"
                    @input="onServerText('smtpHost', $event)"
                  />
                  <input
                    :value="state.form.value.smtpPort"
                    type="number"
                    min="1"
                    max="65535"
                    class="as-input as-mono as-server-port"
                    aria-label="SMTP 端口"
                    @input="onServerPort('smtpPort', $event)"
                  />
                  <span class="as-combo-wrap as-server-sec">
                    <select
                      class="as-combo"
                      :value="state.form.value.smtpSecurity"
                      :disabled="demoMode"
                      aria-label="SMTP 加密方式"
                      @change="onServerSecurity('smtpSecurity', $event)"
                    >
                      <option value="tls">SSL/TLS</option>
                      <option value="startTls">STARTTLS</option>
                    </select>
                    <svg class="as-combo-chevron" viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
                      <path
                        d="M4 6.5L8 10.5l4-4"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.5"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                      />
                    </svg>
                  </span>
                </div>
                <span v-if="state.validationErrors.value.smtpHost || state.validationErrors.value.smtpPort" class="as-field-err as-server-err">
                  {{ state.validationErrors.value.smtpHost ?? state.validationErrors.value.smtpPort }}
                </span>
              </div>
              <div class="as-trow as-trow-switch">
                <p class="as-trow-title">SMTP 使用独立用户名与授权码</p>
                <button
                  type="button"
                  role="switch"
                  class="as-switch"
                  :class="{ checked: state.form.value.smtpSeparateCredentials, 'is-disabled': demoMode }"
                  :aria-checked="state.form.value.smtpSeparateCredentials"
                  :disabled="demoMode"
                  aria-label="SMTP 使用独立用户名与授权码"
                  @click="state.form.value.smtpSeparateCredentials = !state.form.value.smtpSeparateCredentials"
                >
                  <span class="as-switch-thumb" />
                </button>
              </div>
              <template v-if="state.form.value.smtpSeparateCredentials">
                <div class="as-frow">
                  <label class="as-frow-label" for="as-acc-smtp-user">SMTP 用户名</label>
                  <div class="as-frow-control">
                    <input
                      id="as-acc-smtp-user"
                      :value="state.form.value.smtpUsername"
                      type="text"
                      class="as-input"
                      placeholder="smtp-user@example.com"
                      @input="onSmtpUsernameInput"
                    />
                    <span v-if="state.validationErrors.value.smtpUsername" class="as-field-err">
                      {{ state.validationErrors.value.smtpUsername }}
                    </span>
                  </div>
                </div>
                <div class="as-frow">
                  <label class="as-frow-label" for="as-acc-smtp-pwd">
                    {{ editingId && state.form.value.hasPassword ? "SMTP 密码（留空保留）" : "SMTP 密码" }}
                  </label>
                  <div class="as-frow-control">
                    <input
                      id="as-acc-smtp-pwd"
                      v-model="state.form.value.smtpPassword"
                      type="password"
                      class="as-input"
                      autocomplete="new-password"
                      :placeholder="
                        editingId && state.form.value.hasPassword
                          ? '已保存密码，留空则保持不变'
                          : '请输入 SMTP 密码'
                      "
                    />
                    <span v-if="state.validationErrors.value.smtpPassword" class="as-field-err">
                      {{ state.validationErrors.value.smtpPassword }}
                    </span>
                  </div>
                </div>
              </template>
              <div class="as-trow as-verify-row">
                <p class="as-verify-hint">保存前可测试 IMAP / SMTP 服务器连通性与凭据</p>
                <button
                  type="button"
                  class="as-btn as-btn-outline as-btn-compact"
                  :disabled="demoMode || state.verifying.value || state.saving.value"
                  @click="state.verifyCurrentAccount()"
                >
                  {{ state.verifying.value ? "正在测试连接…" : "测试连接" }}
                </button>
              </div>
            </template>

            <!-- 在「所有收件箱」中显示 -->
            <div class="as-trow">
              <div class="as-trow-main">
                <p class="as-trow-title">在「所有收件箱」中显示</p>
                <p class="as-trow-desc">关闭后仅在左侧对应账号文件夹中查看，不混入统一视图</p>
              </div>
              <button
                type="button"
                role="switch"
                class="as-switch"
                :class="{ checked: state.form.value.inUnified, 'is-disabled': demoMode }"
                :aria-checked="state.form.value.inUnified"
                :disabled="demoMode"
                aria-label="在所有收件箱中显示"
                @click="state.form.value.inUnified = !state.form.value.inUnified"
              >
                <span class="as-switch-thumb" />
              </button>
            </div>
          </div>
        </section>

        <!-- 分组 2：账号外观（72px 实时预览 + 强调色 + 头像显示） -->
        <section class="as-group">
          <h3 class="as-group-title">账号外观</h3>
          <div class="as-group-card">
            <div class="as-appearance-preview">
              <span class="as-avatar as-avatar-lg" :style="{ borderColor: previewAccent }">
                <span class="as-avatar-inner" :style="{ background: previewAccent }">
                  <img
                    v-if="topBrandId"
                    class="as-avatar-brand"
                    :class="{ 'is-tile': isTileBrand(topBrandId) }"
                    :src="BRAND_IMAGE_URLS[topBrandId]"
                    alt=""
                  />
                  <span v-else class="as-avatar-initials as-avatar-initials-lg">
                    {{ previewInitials }}
                  </span>
                </span>
              </span>
            </div>
            <div class="as-trow as-appearance-disabled">
              <p class="as-trow-title">强调色</p>
              <div class="as-trow-side as-swatch-row">
                <span
                  v-for="([hex, name], idx) in ACCOUNT_ACCENT_COLORS"
                  :key="hex"
                  class="as-swatch"
                  :class="{ selected: idx === previewAccentIndex }"
                  :style="{ borderColor: idx === previewAccentIndex ? hex : 'var(--km-border)', background: hex }"
                  :title="name"
                  aria-hidden="true"
                >
                  <svg v-if="idx === previewAccentIndex" viewBox="0 0 16 16" width="10" height="10">
                    <path
                      d="M3.5 8.5l3 3 6-6.5"
                      fill="none"
                      stroke="#ffffff"
                      stroke-width="2"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                    />
                  </svg>
                </span>
                <button type="button" class="as-btn as-btn-ghost as-btn-compact" disabled>
                  默认
                </button>
              </div>
            </div>
            <div class="as-trow as-appearance-disabled">
              <p class="as-trow-title">头像显示</p>
              <div class="as-segmented" aria-hidden="true">
                <button
                  type="button"
                  class="as-segment"
                  :class="{ active: true }"
                  disabled
                  tabindex="-1"
                >
                  默认
                </button>
                <button type="button" class="as-segment" disabled tabindex="-1">
                  首字母或 Emoji
                </button>
                <button type="button" class="as-segment" disabled tabindex="-1">
                  图片
                </button>
              </div>
            </div>
          </div>
        </section>

        <!-- 分组 3：签名（编辑 + HTML 预览） -->
        <section class="as-group">
          <h3 class="as-group-title">签名</h3>
          <p class="as-group-desc">写信或回复时自动附加的签名，支持 HTML 源码与实时预览</p>
          <div class="as-group-card">
            <div class="as-signature-body-wrap">
              <div class="as-signature-toolbar">
                <span class="as-sig-tab selected">签名 1</span>
                <span class="as-signature-toolbar-spacer" />
                <button
                  type="button"
                  class="as-btn as-btn-outline as-btn-compact"
                  :disabled="demoMode"
                  @click="sigHtmlMode = !sigHtmlMode"
                >
                  {{ sigHtmlMode ? "HTML 模式" : "纯文本模式" }}
                </button>
                <button
                  type="button"
                  class="as-btn as-btn-ghost as-btn-compact"
                  :disabled="demoMode"
                  @click="pickSignatureFile"
                >
                  导入 HTML…
                </button>
                <button
                  v-if="sigHtmlMode"
                  type="button"
                  class="as-btn as-btn-ghost as-btn-compact"
                  @click="sigPreview = !sigPreview"
                >
                  {{ sigPreview ? "编辑源码" : "预览" }}
                </button>
              </div>
              <div
                class="as-signature-editor"
                :class="{ 'is-preview': sigHtmlMode && sigPreview }"
              >
                <textarea
                  v-if="!(sigHtmlMode && sigPreview)"
                  v-model="state.form.value.signature"
                  class="as-textarea"
                  rows="4"
                  placeholder="写信或回复时自动附加的默认签名（可选）"
                />
                <div
                  v-else
                  class="as-signature-preview"
                  v-html="signaturePreviewHtml"
                />
              </div>
            </div>
          </div>
        </section>

        <!-- 分组 4：同步（跟随全局的策略项以禁用态呈现，待 IPC 支持后接线） -->
        <section class="as-group">
          <h3 class="as-group-title">同步</h3>
          <div class="as-group-card">
            <div class="as-trow">
              <div class="as-trow-main">
                <p class="as-trow-title">实时推送（IMAP IDLE）</p>
                <p class="as-trow-desc">为此账号单独开启或关闭实时新邮件推送</p>
              </div>
              <span class="as-combo-static">
                跟随全局设置
                <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
                  <path d="M4 6.5L8 10.5l4-4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
                </svg>
              </span>
            </div>
            <div class="as-trow">
              <div class="as-trow-main">
                <p class="as-trow-title">轮询周期覆盖</p>
                <p class="as-trow-desc">覆盖全局定时拉取间隔</p>
              </div>
              <span class="as-combo-static">
                跟随全局设置
                <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
                  <path d="M4 6.5L8 10.5l4-4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
                </svg>
              </span>
            </div>
            <div class="as-trow">
              <div class="as-trow-main">
                <p class="as-trow-title">下载邮件时间范围</p>
                <p class="as-trow-desc">控制本地缓存同步的历史时间跨度</p>
              </div>
              <span class="as-combo-static">
                全部历史
                <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
                  <path d="M4 6.5L8 10.5l4-4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
                </svg>
              </span>
            </div>
            <div class="as-trow">
              <p class="as-trow-title">自动清空废纸篓</p>
              <span class="as-combo-static">
                从不自动清空
                <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
                  <path d="M4 6.5L8 10.5l4-4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
                </svg>
              </span>
            </div>
            <div class="as-trow">
              <p class="as-trow-title">自动清空垃圾邮件</p>
              <span class="as-combo-static">
                从不自动清空
                <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
                  <path d="M4 6.5L8 10.5l4-4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
                </svg>
              </span>
            </div>
            <div v-if="syncStatusText" class="as-trow">
              <div class="as-trow-main">
                <p class="as-trow-title">同步状态</p>
                <p class="as-trow-desc">{{ syncStatusText }}</p>
              </div>
              <button type="button" class="as-btn as-btn-outline as-btn-compact" disabled>
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
                  <path
                    d="M13 8a5 5 0 1 1-1.5-3.5M13 2.5V5h-2.5"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.4"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  />
                </svg>
                立即同步
              </button>
            </div>
          </div>
        </section>

        <p class="as-edit-footnote">
          密码与 OAuth 令牌安全保存在 Windows 凭据管理器中，本地配置仅保存非敏感设置。
        </p>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import {
  ACCOUNT_ACCENT_COLORS,
  BRAND_IMAGE_URLS,
  accountAccentHex,
  accountDisplayLabel,
  brandForAccountFields,
  isTileBrand,
  senderInitials,
} from "./brand";
import { buildSignaturePreviewHtml, looksLikeHtml } from "./signature";
import type { AccountConfigDto, SecurityKind } from "./types";
import type { AccountsStore } from "./useAccounts";

const props = withDefaults(
  defineProps<{
    /** 账号域共享状态（useAccounts 返回值），由宿主创建并传入 */
    state: AccountsStore;
    /** 正在编辑的账号 ID；null 表示新建 */
    editingId: string | null;
    demoMode?: boolean;
    /** 是否显示「未保存更改」横幅（由宿主基于脏检查驱动） */
    pendingLeave?: boolean;
    /** 同步状态描述文本（如「已同步：N 封邮件 / M 未读」），为空则隐藏该行 */
    syncStatusText?: string;
    /** 宿主壳层承载编辑顶栏（返回/移除/保存上提）；true 时隐藏本组件内置顶栏 */
    hostChrome?: boolean;
  }>(),
  {
    demoMode: false,
    pendingLeave: false,
    syncStatusText: "",
    hostChrome: false,
  }
);

const emit = defineEmits<{
  back: [];
  /** 保存成功（顶栏「保存」按钮） */
  saved: [account: AccountConfigDto];
  /** 移除成功 */
  removed: [];
  /** 未保存横幅按钮 */
  "leave-cancel": [];
  "leave-discard": [];
  "leave-save": [];
}>();

const confirmRemove = ref(false);
const msAdvancedExpanded = ref(false);
const sigHtmlMode = ref(false);
const sigPreview = ref(false);

const editingAccount = computed(() =>
  props.editingId
    ? props.state.accounts.value.find((a) => a.id === props.editingId) ?? null
    : null
);

const isOAuth = computed(() => props.state.form.value.authBackend === "microsoftGraphOAuth2");

/** 编辑已有 OAuth 账号时服务商锁定（accounts.rs#L1824 oauth_locked）。 */
const oauthLocked = computed(
  () => props.editingId !== null && editingAccount.value?.authBackend === "microsoftGraphOAuth2"
);

const activePreset = computed(() => props.state.activePreset.value);
const activePresetNote = computed(() => activePreset.value?.note ?? "");

const oauthBannerClass = computed(() => {
  switch (props.state.oauthStatus.value) {
    case "authorized":
      return "is-success";
    case "rejected":
    case "error":
      return "is-error";
    default:
      return "is-info";
  }
});

function displayLabel(acc: AccountConfigDto): string {
  return accountDisplayLabel(acc);
}

// ---- 品牌头部与外观预览 ----

const topBrandId = computed(() =>
  brandForAccountFields(
    props.state.form.value.authBackend,
    props.state.form.value.imapHost,
    Number(props.state.form.value.imapPort) || 993,
    props.state.form.value.email
  )
);

const previewLabel = computed(() => {
  const f = props.state.form.value;
  const label = f.label.trim();
  if (label) return label;
  const dn = f.displayName.trim();
  return dn || f.email.trim();
});

const previewAccent = computed(() => accountAccentHex(props.state.form.value.email.trim()));

const previewAccentIndex = computed(() => {
  const hex = previewAccent.value;
  return ACCOUNT_ACCENT_COLORS.findIndex(([candidate]) => candidate === hex);
});

const previewInitials = computed(() => senderInitials(previewLabel.value || "@"));

const signaturePreviewHtml = computed(() =>
  buildSignaturePreviewHtml(props.state.form.value.signature, sigHtmlMode.value)
);

// 账号切换 / 新建时重置编辑器局部状态
watch(
  () => [props.editingId, props.state.isCreating.value] as const,
  () => {
    confirmRemove.value = false;
    msAdvancedExpanded.value = false;
    sigHtmlMode.value = false;
    sigPreview.value = false;
  }
);

// ---- 输入处理（保持 useAccounts 的预设锁定语义） ----

function onEmailInput(ev: Event) {
  props.state.updateEmail((ev.target as HTMLInputElement).value);
}

function onPresetChange(ev: Event) {
  props.state.selectPreset((ev.target as HTMLSelectElement).value, true);
}

function onImapUsernameInput(ev: Event) {
  props.state.form.value.imapUsername = (ev.target as HTMLInputElement).value;
  props.state.markFieldEdited("imapUsername");
}

function onSmtpUsernameInput(ev: Event) {
  props.state.form.value.smtpUsername = (ev.target as HTMLInputElement).value;
  props.state.markFieldEdited("smtpUsername");
}

function onServerText(field: "imapHost" | "smtpHost", ev: Event) {
  props.state.form.value[field] = (ev.target as HTMLInputElement).value;
  props.state.markFieldEdited(field);
}

function onServerPort(field: "imapPort" | "smtpPort", ev: Event) {
  const raw = (ev.target as HTMLInputElement).value;
  props.state.form.value[field] = raw === "" ? Number.NaN : Number(raw);
  props.state.markFieldEdited(field);
}

function onServerSecurity(field: "imapSecurity" | "smtpSecurity", ev: Event) {
  props.state.form.value[field] = (ev.target as HTMLSelectElement).value as SecurityKind;
  props.state.markFieldEdited(field);
}

// ---- 动作 ----

async function handleSave() {
  const account = await props.state.saveCurrentAccount();
  if (account) {
    sigPreview.value = false;
    emit("saved", account);
  }
}

async function handleRemove() {
  if (!props.editingId) return;
  const removed = await props.state.confirmDelete(props.editingId);
  confirmRemove.value = false;
  if (removed) {
    emit("removed");
  }
}

// ---- 宿主壳层单顶栏（hostChrome）动作与状态（WP-07） ----

const editorTitle = computed(() => (props.editingId ? "编辑账号" : "添加账号"));
const canRemove = computed(() => props.editingId !== null && !props.demoMode);
const editorBusy = computed(() => props.state.saving.value || props.state.verifying.value);

defineExpose({
  editorTitle,
  canRemove,
  editorBusy,
  /** 壳层「保存」：与内置顶栏保存同一入口。 */
  requestSave: handleSave,
  /** 壳层「移除」：切换内联移除确认横幅。 */
  toggleRemoveConfirm() {
    confirmRemove.value = !confirmRemove.value;
  },
});

/** 「导入 HTML…」：读取本地 HTML / 文本文件放入签名（accounts.rs#import_signature_file）。 */
function pickSignatureFile() {
  const input = document.createElement("input");
  input.type = "file";
  input.accept = ".html,.htm,.txt,text/html,text/plain";
  input.onchange = () => {
    const file = input.files?.[0];
    if (!file) return;
    void file.text().then((content) => {
      props.state.form.value.signature = content;
      if (looksLikeHtml(content)) {
        sigHtmlMode.value = true;
        sigPreview.value = false;
      }
    });
  };
  input.click();
}
</script>

<style scoped>
.as-edit-root {
  flex: 1;
  min-height: 0;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

/* 顶栏：高 46px、横向 16px、底部 1px 分隔 */
.as-edit-header {
  height: 46px;
  flex-shrink: 0;
  padding: 0 16px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  border-bottom: 1px solid var(--km-border);
  background: var(--km-canvas);
}

.as-edit-header-left {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
}

.as-edit-header-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--km-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.as-edit-header-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

/* 未保存横幅：accent 12% 底 + 底边强调分隔 */
.as-unsaved-banner {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 24px;
  background: var(--km-accent-soft);
  border-bottom: 1px solid color-mix(in srgb, var(--km-accent) 40%, transparent);
}

.as-unsaved-text {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.as-unsaved-title {
  margin: 0;
  font-size: 13px;
  font-weight: 600;
  color: var(--km-text);
}

.as-unsaved-desc {
  margin: 0;
  font-size: 12px;
  color: var(--km-text-muted);
}

.as-unsaved-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

/* 滚动内容区 */
.as-edit-scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 20px 24px;
}

.as-page-inner {
  width: 100%;
  max-width: 600px;
  margin: 0 auto;
  display: flex;
  flex-direction: column;
  gap: 20px;
}

/* 品牌头部 */
.as-edit-brand-head {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding-top: 4px;
}

.as-edit-brand-img {
  width: 56px;
  height: 56px;
  object-fit: contain;
}

.as-edit-brand-pill {
  width: 56px;
  height: 56px;
  border-radius: 14px;
  background: #005bff;
  color: #ffffff;
  font-size: 14px;
  font-weight: 700;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.as-edit-brand-label {
  margin: 0;
  font-size: 13px;
  font-weight: 600;
  color: var(--km-text);
}

/* 移除确认横幅（danger 10% 底、danger 50% 边、圆角 12） */
.as-remove-banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px;
  border-radius: var(--km-radius-lg);
  background: color-mix(in srgb, var(--km-danger) 10%, transparent);
  border: 1px solid color-mix(in srgb, var(--km-danger) 50%, transparent);
}

.as-remove-text {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.as-remove-title {
  margin: 0;
  font-size: 13px;
  font-weight: 600;
  color: var(--km-danger);
}

.as-remove-desc {
  margin: 0;
  font-size: 12px;
  color: var(--km-text-muted);
}

.as-remove-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

/* 反馈横幅 */
.as-feedback-banner {
  padding: 8px 12px;
  border-radius: var(--km-radius-md);
  border: 1px solid;
  font-size: 12px;
  overflow-wrap: anywhere;
}

.as-feedback-banner.is-success {
  border-color: color-mix(in srgb, var(--km-accent) 45%, transparent);
  background: var(--km-accent-soft);
  color: var(--km-accent);
}

.as-feedback-banner.is-error {
  border-color: color-mix(in srgb, var(--km-danger) 45%, transparent);
  background: color-mix(in srgb, var(--km-danger) 10%, transparent);
  color: var(--km-danger);
}

/* 分组：标题与说明在卡片外（widgets.rs#setting_group） */
.as-group {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.as-group-title {
  margin: 0;
  padding: 0 4px;
  font-size: 13px;
  font-weight: 600;
  color: var(--km-text);
}

.as-group-desc {
  margin: 0;
  padding: 0 4px;
  font-size: 12px;
  color: var(--km-text-muted);
}

.as-group-card {
  width: 100%;
  border-radius: var(--km-radius-lg);
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  overflow: hidden;
}

.as-group-card > * + * {
  border-top: 1px solid var(--km-border);
}

/* 标题行：左标题/说明 + 右控件 */
.as-trow {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  width: 100%;
  min-height: 52px;
  padding: 10px 16px;
}

.as-trow-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.as-trow-title {
  margin: 0;
  font-size: 13px;
  font-weight: 500;
  color: var(--km-text);
}

.as-trow-desc {
  margin: 0;
  font-size: 12px;
  color: var(--km-text-muted);
  overflow-wrap: anywhere;
}

.as-trow-side {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

.as-trow-switch {
  min-height: 48px;
  padding-top: 8px;
  padding-bottom: 8px;
}

/* 表单行：左固定 128px 标签 + 右全宽控件（accounts.rs#form_entry_row） */
.as-frow {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  width: 100%;
  min-height: 52px;
  padding: 8px 16px;
}

.as-frow-label {
  width: 128px;
  flex-shrink: 0;
  padding-top: 6px;
  font-size: 12px;
  font-weight: 500;
  color: var(--km-text-muted);
}

.as-frow-control {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.as-frow-static {
  flex: 1;
  min-width: 0;
  padding-top: 6px;
  font-size: 13px;
  color: var(--km-text);
  overflow-wrap: anywhere;
  user-select: text;
}

/* 输入控件 */
.as-input,
.as-textarea {
  width: 100%;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-canvas);
  color: var(--km-text);
  font-size: 13px;
  user-select: text;
}

.as-input {
  height: 32px;
  padding: 0 10px;
}

.as-textarea {
  padding: 8px 10px;
  resize: vertical;
  min-height: 88px;
  line-height: 1.45;
}

.as-input:focus,
.as-textarea:focus {
  border-color: var(--km-accent);
  outline: none;
}

.as-input:disabled,
.as-textarea:disabled {
  opacity: 0.55;
}

.as-mono {
  font-family: var(--km-font-mono);
  font-size: 12px;
}

.as-field-err {
  font-size: 11px;
  color: var(--km-danger);
}

/* 服务器行：host 弹性 + 端口 74px + 加密下拉 */
.as-server-line {
  flex-direction: row;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.as-server-host {
  flex: 1;
  min-width: 120px;
}

.as-server-port {
  width: 74px;
  flex-shrink: 0;
}

.as-server-sec {
  flex-shrink: 0;
}

.as-server-err {
  width: 100%;
}

/* 下拉（outline compact 按钮 + ChevronDown 观感） */
.as-combo-wrap {
  position: relative;
  display: inline-flex;
  align-items: center;
}

.as-combo {
  appearance: none;
  height: 32px;
  min-width: 0;
  padding: 0 26px 0 10px;
  border-radius: var(--km-radius-sm);
  border: 1px solid var(--km-border);
  background: transparent;
  color: var(--km-text);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  max-width: 240px;
}

.as-combo:hover:not(:disabled) {
  background: var(--km-hover-bg);
}

.as-combo:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.as-combo:focus-visible {
  outline: 2px solid var(--km-accent);
  outline-offset: -2px;
}

.as-combo-chevron {
  position: absolute;
  right: 8px;
  color: var(--km-text-muted);
  pointer-events: none;
}

.as-provider-side {
  flex-wrap: wrap;
  justify-content: flex-end;
}

/* 禁用态静态下拉（同步分组占位） */
.as-combo-static {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 32px;
  padding: 0 10px;
  border-radius: var(--km-radius-sm);
  border: 1px solid var(--km-border);
  color: var(--km-text-muted);
  font-size: 12px;
  opacity: 0.55;
  flex-shrink: 0;
}

/* OAuth 提示与高级折叠 */
.as-inline-banner {
  padding: 8px 12px;
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  font-size: 12px;
}

.as-inline-banner.is-success {
  border-color: color-mix(in srgb, #26a269 45%, var(--km-border));
  background: color-mix(in srgb, #26a269 12%, transparent);
}

.as-inline-banner.is-error {
  border-color: color-mix(in srgb, var(--km-danger) 50%, var(--km-border));
  background: color-mix(in srgb, var(--km-danger) 12%, transparent);
  color: var(--km-danger);
}

.as-inline-banner.is-info {
  border-color: color-mix(in srgb, var(--km-accent) 45%, var(--km-border));
  background: var(--km-accent-soft);
}

.as-adv-wrap {
  display: flex;
  flex-direction: column;
}

.as-adv-toggle {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  width: 100%;
  min-height: 46px;
  padding: 8px 16px;
  border: none;
  background: transparent;
  color: var(--km-text-muted);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
}

.as-adv-toggle:hover {
  background: color-mix(in srgb, var(--km-surface) 55%, transparent);
}

.as-adv-body {
  padding: 0 16px 12px;
}

/* 测试连接行 */
.as-verify-row {
  min-height: 50px;
}

.as-verify-hint {
  margin: 0;
  font-size: 12px;
  color: var(--km-text-muted);
}

/* 账号外观 */
.as-appearance-preview {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px 0;
}

.as-appearance-disabled .as-trow-side,
.as-appearance-disabled .as-segmented,
.as-appearance-disabled .as-swatch-row {
  opacity: 0.45;
}

.as-appearance-disabled .as-btn,
.as-appearance-disabled .as-segment {
  pointer-events: none;
}

/* 72px 大头像（外环 3px + 留白 2px → 82px） */
.as-avatar-lg {
  width: 82px;
  height: 82px;
  border-width: 3px;
  padding: 2px;
}

.as-avatar-lg .as-avatar-inner {
  width: 72px;
  height: 72px;
}

.as-avatar-lg .as-avatar-inner.is-padded {
  padding: 10px;
}

.as-avatar-initials-lg {
  font-size: 30px;
}

/* 强调色板 */
.as-swatch-row {
  flex-wrap: wrap;
}

.as-swatch {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border-radius: 999px;
  border: 1px solid var(--km-border);
  padding: 2px;
  box-sizing: border-box;
}

.as-swatch.selected {
  border-width: 2px;
}

.as-swatch::before {
  content: "";
  display: block;
  width: 100%;
  height: 100%;
  border-radius: 999px;
  background: inherit;
}

/* 头像显示分段（禁用占位） */
.as-segmented {
  display: inline-flex;
  gap: 2px;
  padding: 2px;
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  background: var(--km-surface);
}

.as-segment {
  height: 26px;
  padding: 0 10px;
  border: none;
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-text-muted);
  font-size: 12px;
  cursor: pointer;
  white-space: nowrap;
}

.as-segment.active {
  background: var(--km-accent);
  color: var(--km-accent-fg);
  font-weight: 600;
}

/* 签名 */
.as-signature-body-wrap {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 12px;
}

.as-signature-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.as-signature-toolbar-spacer {
  flex: 1;
}

.as-sig-tab {
  height: 28px;
  display: inline-flex;
  align-items: center;
  padding: 0 10px;
  border-radius: var(--km-radius-sm);
  border: 1px solid var(--km-accent);
  background: var(--km-accent-soft);
  color: var(--km-accent);
  font-size: 12px;
  font-weight: 600;
}

.as-signature-editor {
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  background: var(--km-surface);
  padding: 6px;
}

.as-signature-editor.is-preview {
  padding: 0;
}

.as-signature-editor .as-textarea {
  border: none;
  background: transparent;
  min-height: 150px;
}

.as-signature-editor .as-textarea:focus {
  outline: none;
}

.as-signature-preview {
  height: 150px;
  overflow-y: auto;
  padding: 10px 12px;
  font-size: 13px;
  color: var(--km-text);
  line-height: 1.5;
  user-select: text;
  overflow-wrap: anywhere;
}

.as-signature-preview :deep(img) {
  max-width: 100%;
}

/* 同步状态行 */
.as-group-card .as-btn[disabled] {
  opacity: 0.55;
  cursor: not-allowed;
}

/* 底部备注 */
.as-edit-footnote {
  margin: 0;
  padding: 0 4px 16px;
  font-size: 12px;
  color: var(--km-text-muted);
}

/* 药丸开关（与列表一致：36×20，thumb 16） */
.as-switch {
  position: relative;
  width: 36px;
  height: 20px;
  border-radius: 999px;
  border: none;
  background: color-mix(in srgb, var(--km-text-muted) 35%, transparent);
  padding: 2px;
  cursor: pointer;
  flex-shrink: 0;
  transition: background-color 0.15s ease;
}

.as-switch.checked {
  background: var(--km-accent);
}

.as-switch.is-disabled {
  opacity: 0.45;
}

.as-switch:disabled {
  cursor: not-allowed;
}

.as-switch-thumb {
  display: block;
  width: 16px;
  height: 16px;
  border-radius: 999px;
  background: #ffffff;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
  transition: transform 0.15s ease;
}

.as-switch.checked .as-switch-thumb {
  transform: translateX(16px);
}

/* 按钮（GPUI compact Medium：高 32px） */
.as-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  height: 32px;
  border-radius: var(--km-radius-sm);
  border: 1px solid transparent;
  padding: 0 12px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  white-space: nowrap;
}

.as-btn:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.as-btn-compact {
  padding: 0 10px;
}

.as-btn-primary {
  border-color: var(--km-accent);
  background: var(--km-accent);
  color: var(--km-accent-fg);
}

.as-btn-primary:hover:not(:disabled) {
  background: color-mix(in srgb, var(--km-accent) 88%, transparent);
}

.as-btn-outline {
  border-color: var(--km-border);
  background: transparent;
  color: var(--km-text);
}

.as-btn-outline:hover:not(:disabled) {
  background: var(--km-hover-bg);
}

.as-btn-ghost {
  border-color: transparent;
  background: transparent;
  color: var(--km-text);
}

.as-btn-ghost:hover:not(:disabled) {
  background: var(--km-hover-bg);
}

.as-btn-danger {
  border-color: var(--km-danger);
  background: var(--km-danger);
  color: #ffffff;
}

.as-btn-danger:hover:not(:disabled) {
  background: color-mix(in srgb, var(--km-danger) 88%, transparent);
}

@media (max-width: 640px) {
  .as-frow {
    flex-direction: column;
    gap: 4px;
  }

  .as-frow-label {
    width: auto;
    padding-top: 0;
  }
}
</style>

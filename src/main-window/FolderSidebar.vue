<template>
  <aside
    class="km-sidebar"
    :class="{ rail: isRail }"
    :style="{ width: `${width}px`, minWidth: `${width}px` }"
    aria-label="邮件文件夹导航"
  >
    <!-- 顶部栏 48px：刷新 / KiteMail 标题（可拖拽）/ 主菜单 -->
    <header class="km-sidebar-header" data-tauri-drag-region>
      <template v-if="!isRail">
        <button
          type="button"
          class="km-icon-btn"
          :class="{ spinning: syncing }"
          title="刷新邮件 (F5)"
          aria-label="刷新邮件"
          @click="emit('refresh')"
        >
          <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
            <path
              fill="currentColor"
              d="M13.5 8a5.5 5.5 0 1 1-1.38-3.65L10.5 6h4V2l-1.52 1.52A7 7 0 1 0 15 8h-1.5Z"
            />
          </svg>
        </button>

        <div class="km-brand-drag" data-tauri-drag-region>
          <span class="km-brand-title" data-tauri-drag-region>KiteMail</span>
        </div>
      </template>

      <div ref="menuWrapRef" class="km-menu-wrap" :class="{ 'rail-menu-pair': isRail }">
        <button
          type="button"
          class="km-icon-btn"
          aria-haspopup="menu"
          :aria-expanded="mainMenuOpen"
          title="主菜单"
          aria-label="主菜单"
          @click="toggleMainMenu"
        >
          <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
            <path
              fill="currentColor"
              d="M2.5 4h11a.75.75 0 0 1 0 1.5h-11a.75.75 0 0 1 0-1.5Zm0 3.25h11a.75.75 0 0 1 0 1.5h-11a.75.75 0 0 1 0-1.5Zm0 3.25h11a.75.75 0 0 1 0 1.5h-11a.75.75 0 0 1 0-1.5Z"
            />
          </svg>
        </button>

        <button
          v-if="isRail"
          type="button"
          class="km-icon-btn"
          :class="{ spinning: syncing }"
          title="刷新邮件 (F5)"
          aria-label="刷新邮件"
          @click="emit('refresh')"
        >
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
            <path
              fill="currentColor"
              d="M13.5 8a5.5 5.5 0 1 1-1.38-3.65L10.5 6h4V2l-1.52 1.52A7 7 0 1 0 15 8h-1.5Z"
            />
          </svg>
        </button>

        <!-- 主菜单弹出层 -->
        <div v-if="mainMenuOpen" class="km-popover-menu km-main-menu" role="menu" aria-label="主菜单">
          <button type="button" class="km-menu-item" role="menuitem" @click="runMenuAction(() => emit('openAccounts'))">
            <span>账号管理</span>
            <span class="km-menu-shortcut">Accounts</span>
          </button>
          <button type="button" class="km-menu-item" role="menuitem" @click="runMenuAction(() => emit('openPreferences'))">
            <span>偏好设置…</span>
            <span class="km-menu-shortcut">Ctrl+,</span>
          </button>

          <div class="km-menu-divider" />
          <div class="km-menu-section-label">阅读布局</div>
          <div class="km-menu-segmented" role="group" aria-label="阅读布局切换">
            <button
              type="button"
              class="km-seg-btn"
              :class="{ active: readerPosition === 'right' }"
              title="右侧阅读 (Ctrl+Alt+1)"
              @click="runMenuAction(() => emit('setReaderPosition', 'right'))"
            >
              右侧
            </button>
            <button
              type="button"
              class="km-seg-btn"
              :class="{ active: readerPosition === 'below' }"
              title="下方阅读 (Ctrl+Alt+2)"
              @click="runMenuAction(() => emit('setReaderPosition', 'below'))"
            >
              下方
            </button>
            <button
              type="button"
              class="km-seg-btn"
              :class="{ active: readerPosition === 'off' }"
              title="隐藏阅读窗格 (Ctrl+Alt+3)"
              @click="runMenuAction(() => emit('setReaderPosition', 'off'))"
            >
              关闭
            </button>
          </div>

          <div class="km-menu-section-label">应用主题</div>
          <div class="km-menu-segmented" role="group" aria-label="应用主题切换">
            <button
              type="button"
              class="km-seg-btn"
              :class="{ active: darkMode === null || darkMode === undefined }"
              @click="runMenuAction(() => emit('setDarkMode', null))"
            >
              系统
            </button>
            <button
              type="button"
              class="km-seg-btn"
              :class="{ active: darkMode === false }"
              @click="runMenuAction(() => emit('setDarkMode', false))"
            >
              浅色
            </button>
            <button
              type="button"
              class="km-seg-btn"
              :class="{ active: darkMode === true }"
              @click="runMenuAction(() => emit('setDarkMode', true))"
            >
              深色
            </button>
          </div>

          <div class="km-theme-grid" role="group" aria-label="应用配色主题">
            <button
              v-for="theme in BUILTIN_THEMES"
              :key="theme.id"
              type="button"
              class="km-theme-swatch-btn"
              :class="{ active: themeId === theme.id }"
              :title="theme.label"
              :aria-pressed="themeId === theme.id"
              @click="runMenuAction(() => emit('setTheme', theme.id))"
            >
              <span
                class="km-theme-swatch-dot"
                :style="{ backgroundColor: theme.swatch }"
                aria-hidden="true"
              />
              <span class="km-theme-swatch-label">{{ theme.label }}</span>
              <svg
                v-if="themeId === theme.id"
                class="km-theme-swatch-check"
                viewBox="0 0 16 16"
                width="11"
                height="11"
                aria-hidden="true"
              >
                <path
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  d="m3.5 8.5 3 3 6-6.5"
                />
              </svg>
            </button>
          </div>

          <div class="km-menu-divider" />
          <button type="button" class="km-menu-item" role="menuitem" @click="runMenuAction(() => emit('openCommandPalette'))">
            <span>命令面板</span>
            <span class="km-menu-shortcut">Ctrl+K</span>
          </button>
          <button type="button" class="km-menu-item" role="menuitem" @click="runMenuAction(() => emit('openShortcutsHelp'))">
            <span>快捷键帮助</span>
            <span class="km-menu-shortcut">?</span>
          </button>

          <div class="km-menu-divider" />
          <button
            v-if="hasPasscode"
            type="button"
            class="km-menu-item"
            role="menuitem"
            @click="runMenuAction(() => emit('lock'))"
          >
            <span>立即锁定</span>
            <span class="km-menu-shortcut">Lock</span>
          </button>
          <button type="button" class="km-menu-item km-menu-danger" role="menuitem" @click="runMenuAction(() => emit('quit'))">
            <span>退出 KiteMail</span>
            <span class="km-menu-shortcut">Quit</span>
          </button>
        </div>
      </div>
    </header>

    <!-- 全宽 #3584E4 写信分割按钮 -->
    <div ref="composeWrapRef" class="km-compose-cta-wrap" :class="{ rail: isRail }">
      <template v-if="!isRail">
        <div class="km-compose-split">
          <button
            type="button"
            class="km-compose-main"
            title="写新邮件 (Ctrl+N)"
            aria-label="写邮件"
            @click="emit('compose')"
          >
            <svg viewBox="0 0 18 18" width="16" height="16" aria-hidden="true">
              <path
                fill="none"
                stroke="currentColor"
                stroke-width="1.5"
                stroke-linecap="round"
                stroke-linejoin="round"
                d="M2.5 4.75A1.75 1.75 0 0 1 4.25 3h9.5a1.75 1.75 0 0 1 1.75 1.75v7.5A1.75 1.75 0 0 1 13.75 14H4.25a1.75 1.75 0 0 1-1.75-1.75v-7.5Zm.6.4L9 9.5l5.9-4.35"
              />
              <circle cx="13.5" cy="12.5" r="3" fill="#3584e4" />
              <path
                fill="none"
                stroke="currentColor"
                stroke-width="1.4"
                stroke-linecap="round"
                d="M13.5 11.1v2.8M12.1 12.5h2.8"
              />
            </svg>
            <span>写邮件</span>
          </button>
          <span class="km-compose-sep" aria-hidden="true" />
          <button
            type="button"
            class="km-compose-caret"
            aria-haspopup="menu"
            :aria-expanded="composeMenuOpen"
            title="从模板创建邮件"
            aria-label="更多写信选项"
            @click="toggleComposeMenu"
          >
            <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
              <path
                fill="none"
                stroke="currentColor"
                stroke-width="1.7"
                stroke-linecap="round"
                stroke-linejoin="round"
                d="M4.25 6.25 8 10l3.75-3.75"
              />
            </svg>
          </button>
        </div>

        <!-- 写信模板菜单 -->
        <div
          v-if="composeMenuOpen"
          class="km-popover-menu km-compose-menu"
          role="menu"
          aria-label="写信模板菜单"
        >
          <button
            type="button"
            class="km-menu-item"
            role="menuitem"
            @click="runComposeAction(() => emit('compose'))"
          >
            <span>空白写信</span>
            <span class="km-menu-shortcut">Ctrl+N</span>
          </button>
          <div class="km-menu-divider" />
          <div class="km-menu-section-label">从模板创建</div>
          <div v-if="templates.length === 0" class="km-menu-empty">
            暂无写信模板
          </div>
          <button
            v-for="tpl in templates"
            :key="tpl.id"
            type="button"
            class="km-menu-item"
            role="menuitem"
            :title="tpl.subject || tpl.name"
            @click="runComposeAction(() => emit('composeFromTemplate', tpl))"
          >
            <span class="km-menu-item-label">{{ tpl.name }}</span>
          </button>
        </div>
      </template>

      <button
        v-else
        type="button"
        class="km-compose-rail-btn"
        title="写新邮件 (Ctrl+N)"
        aria-label="写邮件"
        @click="emit('compose')"
      >
        <svg viewBox="0 0 18 18" width="18" height="18" aria-hidden="true">
          <path
            fill="none"
            stroke="currentColor"
            stroke-width="1.6"
            stroke-linecap="round"
            stroke-linejoin="round"
            d="M3 5a1.5 1.5 0 0 1 1.5-1.5h9A1.5 1.5 0 0 1 15 5v7a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 3 12V5Zm.5.4L9 9.5l5.5-4.1"
          />
        </svg>
      </button>
    </div>

    <!-- 中部可滚动导航区 -->
    <div class="km-sidebar-scroll">
      <template v-if="!isRail">
        <!-- 统一导航顺序：统一收件箱 / 星标 / 已发 / 草稿 / 归档 -->
        <nav class="km-nav-section" aria-label="统一视图">
          <div v-for="item in unifiedItems" :key="item.kind" class="km-unified-block">
            <div
              role="button"
              tabindex="0"
              class="km-nav-row"
              :class="{ active: isUnifiedActive(item.kind) }"
              :title="item.label"
              @click="onSelectUnified(item.kind)"
              @keydown.enter.prevent="onSelectUnified(item.kind)"
              @keydown.space.prevent="onSelectUnified(item.kind)"
            >
              <span class="km-nav-icon" aria-hidden="true">
                <!-- 统一收件箱 -->
                <svg v-if="item.kind === 'unifiedInbox'" viewBox="0 0 16 16" width="16" height="16">
                  <path
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.4"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    d="M2.5 3.5h11l1 5.5v3.5a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1V9l1-5.5Zm-.7 5.5H5.4l1.1 1.8h3l1.1-1.8h3.6"
                  />
                </svg>
                <!-- 星标 -->
                <svg v-else-if="item.kind === 'starred'" viewBox="0 0 16 16" width="16" height="16">
                  <path
                    fill="currentColor"
                    d="m8 1.9 1.88 3.81 4.2.61-3.04 2.96.72 4.19L8 11.49l-3.76 1.98.72-4.19L1.92 6.32l4.2-.61L8 1.9Z"
                  />
                </svg>
                <!-- 已发送 -->
                <svg v-else-if="item.kind === 'sent'" viewBox="0 0 16 16" width="16" height="16">
                  <path
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.4"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    d="M2.2 7.8 13.5 2.5 10.6 13.6 7.5 9.6 2.2 7.8Zm5.3 1.8 6-7.1"
                  />
                </svg>
                <!-- 草稿箱 -->
                <svg v-else-if="item.kind === 'drafts'" viewBox="0 0 16 16" width="16" height="16">
                  <path
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.4"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    d="m10.8 2.4 2.8 2.8-7.7 7.7-3.3.6.6-3.3 7.6-7.8Z"
                  />
                </svg>
                <!-- 归档 -->
                <svg v-else viewBox="0 0 16 16" width="16" height="16">
                  <path
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.4"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    d="M2.2 3.2h11.6v2.6H2.2V3.2Zm1.1 2.6h9.4v7a1 1 0 0 1-1 1H4.3a1 1 0 0 1-1-1v-7Zm3.2 2.8h3"
                  />
                </svg>
              </span>

              <span class="km-nav-label">{{ item.label }}</span>

              <span
                v-if="item.badge > 0 && !expandedUnified.has(item.kind)"
                class="km-count-pill"
              >
                {{ formatBadge(item.badge) }}
              </span>

              <button
                type="button"
                class="km-row-chevron-btn"
                :class="{ expanded: expandedUnified.has(item.kind) }"
                :aria-expanded="expandedUnified.has(item.kind)"
                :aria-label="`展开${item.label}各账号子项`"
                @click.stop="toggleUnifiedExpand(item.kind)"
              >
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
                  <path
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.6"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    d="m6 3.75 4.25 4.25L6 12.25"
                  />
                </svg>
              </button>
            </div>

            <!-- 展开后的各账号子项 -->
            <div v-if="expandedUnified.has(item.kind)" class="km-subrow-list">
              <button
                v-for="sub in unifiedSubrows(item.kind)"
                :key="`${item.kind}-${sub.account.id}-${sub.folder?.id ?? 0}`"
                type="button"
                class="km-subrow-btn"
                :class="{ active: sub.active }"
                :title="sub.label"
                @click="onSelectUnifiedSubrow(item.kind, sub.account.id, sub.folder)"
              >
                <span
                  class="km-account-dot"
                  :style="{ backgroundColor: accountColor(sub.account) }"
                  aria-hidden="true"
                />
                <span class="km-subrow-label">{{ sub.label }}</span>
                <span v-if="sub.unread > 0" class="km-count-pill">
                  {{ formatBadge(sub.unread) }}
                </span>
              </button>
            </div>
          </div>

          <!-- Filters 分组 -->
          <div class="km-unified-block">
            <div
              role="button"
              tabindex="0"
              class="km-nav-row"
              :class="{ active: isFiltersCollectionActive }"
              title="筛选规则集合"
              @click="onSelectFiltersCollection"
              @keydown.enter.prevent="onSelectFiltersCollection"
              @keydown.space.prevent="onSelectFiltersCollection"
            >
              <span class="km-nav-icon" aria-hidden="true">
                <svg viewBox="0 0 16 16" width="16" height="16">
                  <path
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.4"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    d="M2.2 3.4h11.6l-4.4 5.1v3.8l-2.8 1.3V8.5L2.2 3.4Z"
                  />
                </svg>
              </span>
              <span class="km-nav-label">Filters</span>
              <span v-if="!filtersExpanded" class="km-count-pill">
                {{ SIDEBAR_FILTER_RULES.length }}
              </span>
              <button
                type="button"
                class="km-row-chevron-btn"
                :class="{ expanded: filtersExpanded }"
                :aria-expanded="filtersExpanded"
                aria-label="展开规则文件夹"
                @click.stop="filtersExpanded = !filtersExpanded"
              >
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
                  <path
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.6"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    d="m6 3.75 4.25 4.25L6 12.25"
                  />
                </svg>
              </button>
            </div>

            <div v-if="filtersExpanded" class="km-subrow-list">
              <button
                v-for="rule in SIDEBAR_FILTER_RULES"
                :key="rule.id"
                type="button"
                class="km-subrow-btn"
                :class="{ active: activeFilterRuleId === rule.id }"
                @click="onSelectFilterRule(rule)"
              >
                <span class="km-subrow-icon" aria-hidden="true">
                  <svg v-if="rule.id === 'unread'" viewBox="0 0 16 16" width="13" height="13">
                    <circle cx="8" cy="8" r="4" fill="#3584e4" />
                  </svg>
                  <svg v-else-if="rule.id === 'attachment'" viewBox="0 0 16 16" width="13" height="13">
                    <path
                      fill="none"
                      stroke="currentColor"
                      stroke-width="1.4"
                      stroke-linecap="round"
                      d="m11.5 5.2-4.7 4.7a2 2 0 1 1-2.8-2.8l5-5a3.2 3.2 0 0 1 4.5 4.5l-5.3 5.3a4.3 4.3 0 0 1-6.1-6.1l4.4-4.4"
                    />
                  </svg>
                  <svg v-else-if="rule.id === 'starred'" viewBox="0 0 16 16" width="13" height="13">
                    <path
                      fill="currentColor"
                      d="m8 2 1.7 3.5 3.9.56-2.8 2.73.66 3.87L8 10.83l-3.46 1.83.66-3.87L2.4 6.06l3.9-.56L8 2Z"
                    />
                  </svg>
                  <svg v-else viewBox="0 0 16 16" width="13" height="13">
                    <rect
                      x="2.5"
                      y="3.5"
                      width="11"
                      height="10"
                      rx="1.5"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="1.4"
                    />
                    <path fill="none" stroke="currentColor" stroke-width="1.4" d="M2.5 6.8h11M5.5 2v2.5M10.5 2v2.5" />
                  </svg>
                </span>
                <span class="km-subrow-label">{{ rule.label }}</span>
              </button>
            </div>
          </div>

          <!-- Tags 分组 -->
          <div class="km-unified-block">
            <div
              role="button"
              tabindex="0"
              class="km-nav-row"
              :class="{ active: isTagsCollectionActive }"
              title="带标签邮件视图"
              @click="onSelectTagsCollection"
              @keydown.enter.prevent="onSelectTagsCollection"
              @keydown.space.prevent="onSelectTagsCollection"
            >
              <span class="km-nav-icon" aria-hidden="true">
                <svg viewBox="0 0 16 16" width="16" height="16">
                  <path
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.4"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    d="M2.5 7.7V3.2A.7.7 0 0 1 3.2 2.5h4.5l5.8 5.8a1 1 0 0 1 0 1.4l-3.8 3.8a1 1 0 0 1-1.4 0L2.5 7.7Z"
                  />
                  <circle cx="5.6" cy="5.6" r="1" fill="currentColor" />
                </svg>
              </span>
              <span class="km-nav-label">Tags</span>
              <button
                type="button"
                class="km-row-chevron-btn"
                :class="{ expanded: tagsExpanded }"
                :aria-expanded="tagsExpanded"
                aria-label="展开标签列表"
                @click.stop="tagsExpanded = !tagsExpanded"
              >
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
                  <path
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.6"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    d="m6 3.75 4.25 4.25L6 12.25"
                  />
                </svg>
              </button>
            </div>

            <div v-if="tagsExpanded" class="km-subrow-list">
              <div v-if="tags.length === 0" class="km-subrow-empty">暂无标签</div>
              <button
                v-for="tag in tags"
                :key="tag.id"
                type="button"
                class="km-subrow-btn"
                :class="{ active: activeTagName === tag.name }"
                :title="tag.name"
                @click="onSelectTag(tag)"
              >
                <span
                  class="km-account-dot"
                  :style="{ backgroundColor: tag.color || '#3584e4' }"
                  aria-hidden="true"
                />
                <span class="km-subrow-label">{{ tag.name }}</span>
              </button>
            </div>
          </div>
        </nav>

        <!-- 账号列表 -->
        <div class="km-accounts-section">
          <div v-if="accounts.length === 0" class="km-accounts-empty">
            <span>尚未配置邮箱账号</span>
            <button
              type="button"
              class="km-accounts-add-link"
              @click="emit('openAccounts')"
            >
              添加账号
            </button>
          </div>

          <div
            v-for="acc in accounts"
            :key="acc.id"
            class="km-account-group"
          >
            <button
              type="button"
              class="km-account-header"
              :aria-expanded="isAccountExpanded(acc.id)"
              :title="`${accountPrimaryLabel(acc)}\n${acc.email}`"
              @click="toggleAccount(acc.id)"
            >
              <span class="km-account-avatar-wrap">
                <span
                  class="km-account-ring"
                  :style="{ borderColor: accountColor(acc) }"
                >
                  <span
                    class="km-account-avatar"
                    :style="{ backgroundColor: accountColor(acc) }"
                  >
                    {{ acc.emoji || accountInitials(acc) }}
                  </span>
                </span>
                <span
                  v-if="accountUnread(acc.id) > 0"
                  class="km-avatar-badge"
                  :aria-label="`${accountUnread(acc.id)} 封未读`"
                >
                  {{ formatCompactBadge(accountUnread(acc.id)) }}
                </span>
              </span>

              <span class="km-account-meta">
                <span class="km-account-name">{{ accountPrimaryLabel(acc) }}</span>
                <span class="km-account-email">{{ acc.email }}</span>
              </span>

              <span
                class="km-row-chevron-static"
                :class="{ expanded: isAccountExpanded(acc.id) }"
                aria-hidden="true"
              >
                <svg viewBox="0 0 16 16" width="13" height="13">
                  <path
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.6"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    d="m6 3.75 4.25 4.25L6 12.25"
                  />
                </svg>
              </span>
            </button>

            <div v-if="isAccountExpanded(acc.id)" class="km-folder-list">
              <div
                v-for="node in folderTreeForAccount(acc.id)"
                :key="node.folder.id"
                class="km-folder-tree-row"
                :style="{ paddingLeft: `${10 + node.depth * 14}px` }"
              >
                <button
                  v-if="node.hasChildren"
                  type="button"
                  class="km-tree-chev"
                  :class="{ expanded: node.expanded }"
                  :aria-expanded="node.expanded"
                  :aria-label="`展开或折叠 ${node.folder.name}`"
                  @click.stop="toggleFolderTreeNode(acc.id, node.folder.path)"
                >
                  <svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true">
                    <path
                      fill="none"
                      stroke="currentColor"
                      stroke-width="1.6"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      d="m6 3.75 4.25 4.25L6 12.25"
                    />
                  </svg>
                </button>
                <span v-else class="km-tree-spacer" aria-hidden="true" />

                <button
                  type="button"
                  class="km-folder-btn"
                  :class="{ active: isFolderActive(acc.id, node.folder.id) }"
                  :title="node.folder.name"
                  @click="onSelectFolder(acc.id, node.folder.id)"
                >
                  <span class="km-nav-icon" aria-hidden="true">
                    <svg v-if="node.role === 'inbox'" viewBox="0 0 16 16" width="15" height="15">
                      <path
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.4"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        d="M2.5 3.5h11l1 5.5v3.5a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1V9l1-5.5Zm-.7 5.5H5.4l1.1 1.8h3l1.1-1.8h3.6"
                      />
                    </svg>
                    <svg v-else-if="node.role === 'sent'" viewBox="0 0 16 16" width="15" height="15">
                      <path
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.4"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        d="M2.2 7.8 13.5 2.5 10.6 13.6 7.5 9.6 2.2 7.8Z"
                      />
                    </svg>
                    <svg v-else-if="node.role === 'drafts'" viewBox="0 0 16 16" width="15" height="15">
                      <path
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.4"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        d="m10.8 2.4 2.8 2.8-7.7 7.7-3.3.6.6-3.3 7.6-7.8Z"
                      />
                    </svg>
                    <svg v-else-if="node.role === 'archive'" viewBox="0 0 16 16" width="15" height="15">
                      <path
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.4"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        d="M2.2 3.2h11.6v2.6H2.2V3.2Zm1.1 2.6h9.4v7a1 1 0 0 1-1 1H4.3a1 1 0 0 1-1-1v-7Z"
                      />
                    </svg>
                    <svg v-else-if="node.role === 'trash'" viewBox="0 0 16 16" width="15" height="15">
                      <path
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.4"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        d="M3 4.5h10M6 4.5V3h4v1.5M4.5 4.5l.6 8.5h5.8l.6-8.5"
                      />
                    </svg>
                    <svg v-else-if="node.role === 'junk'" viewBox="0 0 16 16" width="15" height="15">
                      <path
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.4"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        d="M8 2.4 14 13H2L8 2.4Zm0 4.3v3M8 11.4v.2"
                      />
                    </svg>
                    <svg v-else viewBox="0 0 16 16" width="15" height="15">
                      <path
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.4"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        d="M2 4a1.2 1.2 0 0 1 1.2-1.2h3.1l1.3 1.4h5.2A1.2 1.2 0 0 1 14 5.4v6.8a1.2 1.2 0 0 1-1.2 1.2H3.2A1.2 1.2 0 0 1 2 12.2V4Z"
                      />
                    </svg>
                  </span>
                  <span class="km-nav-label">{{ node.folder.name }}</span>
                  <span v-if="node.folder.unreadCount > 0" class="km-count-pill">
                    {{ formatBadge(node.folder.unreadCount) }}
                  </span>
                  <span v-else-if="node.folder.totalCount > 0" class="km-folder-total">
                    {{ formatBadge(node.folder.totalCount) }}
                  </span>
                </button>
              </div>
            </div>
          </div>
        </div>
      </template>

      <!-- 窄轨模式（60–80px）：统一收件箱 + 账号圆环头像 -->
      <div v-else class="km-rail-stack">
        <button
          type="button"
          class="km-rail-icon-btn"
          :class="{ active: isUnifiedActive('unifiedInbox') }"
          title="统一收件箱"
          aria-label="统一收件箱"
          @click="onSelectUnified('unifiedInbox')"
        >
          <svg viewBox="0 0 16 16" width="17" height="17" aria-hidden="true">
            <path
              fill="none"
              stroke="currentColor"
              stroke-width="1.4"
              stroke-linecap="round"
              stroke-linejoin="round"
              d="M2.5 3.5h11l1 5.5v3.5a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1V9l1-5.5Zm-.7 5.5H5.4l1.1 1.8h3l1.1-1.8h3.6"
            />
          </svg>
          <span v-if="totalInboxUnread > 0" class="km-avatar-badge">
            {{ formatCompactBadge(totalInboxUnread) }}
          </span>
        </button>

        <div class="km-rail-divider" />

        <button
          v-for="acc in accounts"
          :key="acc.id"
          type="button"
          class="km-rail-account-btn"
          :class="{ active: isAccountActive(acc.id) }"
          :title="`${accountPrimaryLabel(acc)} (${acc.email})`"
          :aria-label="accountPrimaryLabel(acc)"
          @click="onRailAccountClick(acc.id)"
        >
          <span class="km-account-avatar-wrap">
            <span class="km-account-ring" :style="{ borderColor: accountColor(acc) }">
              <span class="km-account-avatar" :style="{ backgroundColor: accountColor(acc) }">
                {{ acc.emoji || accountInitials(acc) }}
              </span>
            </span>
            <span v-if="accountUnread(acc.id) > 0" class="km-avatar-badge">
              {{ formatCompactBadge(accountUnread(acc.id)) }}
            </span>
          </span>
        </button>
      </div>
    </div>

    <!-- 底部固定两行：通讯录与附件 -->
    <footer class="km-sidebar-footer" :class="{ rail: isRail }">
      <button
        type="button"
        class="km-footer-row"
        :class="{ active: activeWorkspaceView === 'contacts', 'rail-row': isRail }"
        title="通讯录"
        aria-label="通讯录"
        @click="emit('openContacts')"
      >
        <span class="km-nav-icon" aria-hidden="true">
          <svg viewBox="0 0 16 16" width="16" height="16">
            <rect
              x="2.8"
              y="2.2"
              width="10.4"
              height="11.6"
              rx="1.6"
              fill="none"
              stroke="currentColor"
              stroke-width="1.35"
            />
            <circle cx="8" cy="6.6" r="1.7" fill="none" stroke="currentColor" stroke-width="1.3" />
            <path
              fill="none"
              stroke="currentColor"
              stroke-width="1.3"
              stroke-linecap="round"
              d="M5.4 11.4a2.7 2.7 0 0 1 5.2 0"
            />
          </svg>
        </span>
        <span v-if="!isRail" class="km-nav-label">通讯录</span>
      </button>

      <button
        type="button"
        class="km-footer-row"
        :class="{ active: activeWorkspaceView === 'attachments', 'rail-row': isRail }"
        title="附件"
        aria-label="附件"
        @click="emit('openAttachments')"
      >
        <span class="km-nav-icon" aria-hidden="true">
          <svg viewBox="0 0 16 16" width="16" height="16">
            <path
              fill="none"
              stroke="currentColor"
              stroke-width="1.4"
              stroke-linecap="round"
              stroke-linejoin="round"
              d="m11.6 5.2-5 5a2 2 0 0 1-2.8-2.8l5.3-5.3a3.2 3.2 0 0 1 4.5 4.5l-5.6 5.6a4.4 4.4 0 0 1-6.2-6.2l4.7-4.7"
            />
          </svg>
        </span>
        <span v-if="!isRail" class="km-nav-label">附件</span>
      </button>
    </footer>
  </aside>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import {
  BUILTIN_THEMES,
  type AccountSummaryDto,
  type FolderSummaryDto,
  type MailTemplateDto,
  type MessageFilterDto,
  type ReaderPosition,
  type SearchScope,
  type TagItemDto,
  type UnifiedViewKind,
  type ViewDto,
} from "../ipc/types";
import {
  SIDEBAR_FILTER_RULES,
  accountAccentHex,
  buildAccountFolderTree,
  classifyFolderRole,
  findUnifiedSubrowFolder,
  senderInitials,
  type FolderTreeNode,
  type SidebarFilterRule,
} from "./parityHelpers";

const props = withDefaults(
  defineProps<{
    accounts: AccountSummaryDto[];
    folders: FolderSummaryDto[];
    tags?: TagItemDto[];
    templates?: MailTemplateDto[];
    currentView: ViewDto;
    searchQuery?: string;
    filterSet?: MessageFilterDto;
    isRail: boolean;
    width: number;
    syncing?: boolean;
    mode?: "demo" | "live";
    hasPasscode?: boolean;
    themeId?: string;
    darkMode?: boolean | null;
    effectiveDark?: boolean;
    readerPosition?: ReaderPosition;
    activeWorkspaceView?: "mail" | "contacts" | "attachments";
  }>(),
  {
    tags: () => [],
    templates: () => [],
    searchQuery: "",
    filterSet: () => ({}),
    syncing: false,
    mode: "demo",
    hasPasscode: false,
    themeId: "system",
    darkMode: null,
    effectiveDark: false,
    readerPosition: "right",
    activeWorkspaceView: "mail",
  }
);

const emit = defineEmits<{
  selectView: [view: ViewDto];
  applyFilter: [options: { view?: ViewDto; query?: string; scope?: SearchScope; filter?: MessageFilterDto }];
  toggleCollapse: [];
  refresh: [];
  lock: [];
  quit: [];
  toggleDarkMode: [];
  setDarkMode: [darkMode: boolean | null];
  setTheme: [themeId: string];
  setReaderPosition: [position: ReaderPosition];
  compose: [];
  composeFromTemplate: [template: MailTemplateDto];
  openAccounts: [];
  openAttachments: [];
  openContacts: [];
  openCommandPalette: [];
  openPreferences: [];
  openShortcutsHelp: [];
  overlayChange: [id: string, open: boolean];
}>();

const menuWrapRef = ref<HTMLElement | null>(null);
const composeWrapRef = ref<HTMLElement | null>(null);
const mainMenuOpen = ref(false);
const composeMenuOpen = ref(false);

function setMainMenu(open: boolean) {
  if (mainMenuOpen.value === open) return;
  mainMenuOpen.value = open;
  emit("overlayChange", "sidebar-main-menu", open);
}

function toggleMainMenu() {
  setComposeMenu(false);
  setMainMenu(!mainMenuOpen.value);
}

function setComposeMenu(open: boolean) {
  if (composeMenuOpen.value === open) return;
  composeMenuOpen.value = open;
  emit("overlayChange", "compose-template-menu", open);
}

function toggleComposeMenu() {
  setMainMenu(false);
  setComposeMenu(!composeMenuOpen.value);
}

function runMenuAction(fn: () => void) {
  setMainMenu(false);
  fn();
}

function runComposeAction(fn: () => void) {
  setComposeMenu(false);
  fn();
}

// 统一行展开 & Filters/Tags 展开
const expandedUnified = ref<Set<UnifiedViewKind>>(new Set());
const filtersExpanded = ref(false);
const tagsExpanded = ref(false);
const expandedAccounts = ref<Set<number>>(new Set());
const collapsedTreePaths = ref<Map<number, Set<string>>>(new Map());
const activeFilterRuleId = ref<string | null>(null);
const activeTagName = ref<string | null>(null);

watch(
  () => [props.currentView, props.searchQuery, props.filterSet] as const,
  ([view, q, f]) => {
    if (view.kind === "accountFolder") {
      if (!expandedAccounts.value.has(view.accountId)) {
        const next = new Set(expandedAccounts.value);
        next.add(view.accountId);
        expandedAccounts.value = next;
      }
    }
    if (view.kind !== "filters") {
      activeFilterRuleId.value = null;
    } else if (!q && !f?.unread && !f?.starred && !f?.hasAttachment && !f?.today) {
      activeFilterRuleId.value = null;
    }
    if (view.kind !== "tags") {
      activeTagName.value = null;
    } else if (!q.toLowerCase().startsWith("tag:")) {
      activeTagName.value = null;
    }
  },
  { immediate: true, deep: true }
);

function toggleUnifiedExpand(kind: UnifiedViewKind) {
  const next = new Set(expandedUnified.value);
  if (next.has(kind)) {
    next.delete(kind);
  } else {
    next.add(kind);
  }
  expandedUnified.value = next;
}

function toggleAccount(accountId: number) {
  const next = new Set(expandedAccounts.value);
  if (next.has(accountId)) {
    next.delete(accountId);
  } else {
    next.add(accountId);
  }
  expandedAccounts.value = next;
}

function isAccountExpanded(accountId: number): boolean {
  return expandedAccounts.value.has(accountId);
}

function toggleFolderTreeNode(accountId: number, folderPath: string) {
  const next = new Map(collapsedTreePaths.value);
  const current = new Set(next.get(accountId) ?? []);
  if (current.has(folderPath)) {
    current.delete(folderPath);
  } else {
    current.add(folderPath);
  }
  next.set(accountId, current);
  collapsedTreePaths.value = next;
}

function folderTreeForAccount(accountId: number): FolderTreeNode[] {
  const accFolders = props.folders.filter((f) => f.accountId === accountId);
  const collapsed = collapsedTreePaths.value.get(accountId) ?? new Set<string>();
  return buildAccountFolderTree(accFolders, collapsed);
}

const totalInboxUnread = computed(() =>
  props.folders
    .filter((f) => classifyFolderRole(f.path, f.name) === "inbox")
    .reduce((sum, f) => sum + f.unreadCount, 0)
);

const totalDraftsCount = computed(() =>
  props.folders
    .filter((f) => classifyFolderRole(f.path, f.name) === "drafts")
    .reduce((sum, f) => sum + f.totalCount, 0)
);

const unifiedItems = computed<
  Array<{ kind: UnifiedViewKind; label: string; badge: number }>
>(() => [
  { kind: "unifiedInbox", label: "统一收件箱", badge: totalInboxUnread.value },
  { kind: "starred", label: "星标", badge: 0 },
  { kind: "sent", label: "已发送", badge: 0 },
  { kind: "drafts", label: "草稿箱", badge: totalDraftsCount.value },
  { kind: "archive", label: "归档", badge: 0 },
]);

function unifiedSubrows(kind: UnifiedViewKind): Array<{
  account: AccountSummaryDto;
  folder: FolderSummaryDto | null;
  label: string;
  unread: number;
  active: boolean;
}> {
  return props.accounts.map((account) => {
    const folder = findUnifiedSubrowFolder(kind, account.id, props.folders);
    const label = accountPrimaryLabel(account);
    const unread =
      kind === "unifiedInbox"
        ? folder?.unreadCount ?? 0
        : kind === "drafts"
          ? folder?.totalCount ?? 0
          : 0;
    const active =
      props.activeWorkspaceView === "mail" &&
      (folder
        ? isFolderActive(account.id, folder.id)
        : props.currentView.kind === kind && props.filterSet?.accountId === account.id);
    return { account, folder, label, unread, active };
  });
}

function onSelectUnified(kind: UnifiedViewKind) {
  activeFilterRuleId.value = null;
  activeTagName.value = null;
  emit("selectView", { kind });
}

function onSelectUnifiedSubrow(
  kind: UnifiedViewKind,
  accountId: number,
  folder: FolderSummaryDto | null
) {
  activeFilterRuleId.value = null;
  activeTagName.value = null;
  if (folder) {
    emit("selectView", { kind: "accountFolder", accountId, folderId: folder.id });
  } else {
    emit("applyFilter", {
      view: { kind },
      filter: { accountId },
    });
  }
}

function onSelectFiltersCollection() {
  activeFilterRuleId.value = null;
  activeTagName.value = null;
  emit("selectView", { kind: "filters" });
}

function onSelectFilterRule(rule: SidebarFilterRule) {
  activeFilterRuleId.value = rule.id;
  activeTagName.value = null;
  emit("applyFilter", {
    view: { kind: "filters" },
    query: rule.query,
    scope: "allFolders",
    filter: rule.filter,
  });
}

function onSelectTagsCollection() {
  activeFilterRuleId.value = null;
  activeTagName.value = null;
  emit("selectView", { kind: "tags" });
}

function onSelectTag(tag: TagItemDto) {
  activeFilterRuleId.value = null;
  activeTagName.value = tag.name;
  const query = tag.name.includes(" ") ? `tag:"${tag.name}"` : `tag:${tag.name}`;
  emit("applyFilter", {
    view: { kind: "tags" },
    query,
    scope: "allFolders",
  });
}

function onSelectFolder(accountId: number, folderId: number) {
  activeFilterRuleId.value = null;
  activeTagName.value = null;
  emit("selectView", { kind: "accountFolder", accountId, folderId });
}

function onRailAccountClick(accountId: number) {
  const inbox = findUnifiedSubrowFolder("unifiedInbox", accountId, props.folders);
  if (inbox) {
    onSelectFolder(accountId, inbox.id);
  } else {
    emit("toggleCollapse");
  }
}

function isUnifiedActive(kind: UnifiedViewKind): boolean {
  return (
    props.activeWorkspaceView === "mail" &&
    props.currentView.kind === kind &&
    !props.filterSet?.accountId
  );
}

const isFiltersCollectionActive = computed(
  () =>
    props.activeWorkspaceView === "mail" &&
    props.currentView.kind === "filters" &&
    activeFilterRuleId.value === null
);

const isTagsCollectionActive = computed(
  () =>
    props.activeWorkspaceView === "mail" &&
    props.currentView.kind === "tags" &&
    activeTagName.value === null
);

function isFolderActive(accountId: number, folderId: number): boolean {
  return (
    props.activeWorkspaceView === "mail" &&
    props.currentView.kind === "accountFolder" &&
    props.currentView.accountId === accountId &&
    props.currentView.folderId === folderId
  );
}

function isAccountActive(accountId: number): boolean {
  return (
    props.activeWorkspaceView === "mail" &&
    props.currentView.kind === "accountFolder" &&
    props.currentView.accountId === accountId
  );
}

function accountPrimaryLabel(acc: AccountSummaryDto): string {
  return acc.label?.trim() || acc.displayName?.trim() || acc.email;
}

function accountColor(acc: AccountSummaryDto): string {
  return accountAccentHex(acc.email, acc.color);
}

function accountInitials(acc: AccountSummaryDto): string {
  return senderInitials(accountPrimaryLabel(acc));
}

function accountUnread(accountId: number): number {
  return props.folders
    .filter((f) => f.accountId === accountId && classifyFolderRole(f.path, f.name) === "inbox")
    .reduce((sum, f) => sum + f.unreadCount, 0);
}

function formatBadge(n: number): string {
  return n > 999 ? "999+" : String(n);
}

function formatCompactBadge(n: number): string {
  return n > 99 ? "99+" : String(n);
}

function onDocumentPointerDown(e: PointerEvent) {
  const target = e.target as Node;
  if (mainMenuOpen.value && menuWrapRef.value && !menuWrapRef.value.contains(target)) {
    setMainMenu(false);
  }
  if (composeMenuOpen.value && composeWrapRef.value && !composeWrapRef.value.contains(target)) {
    setComposeMenu(false);
  }
}

onMounted(() => {
  document.addEventListener("pointerdown", onDocumentPointerDown);
});

onUnmounted(() => {
  document.removeEventListener("pointerdown", onDocumentPointerDown);
  setMainMenu(false);
  setComposeMenu(false);
});
</script>

<style scoped>
.km-sidebar {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--km-sidebar, #ebebed);
  color: var(--km-text);
  flex-shrink: 0;
  overflow: hidden;
  position: relative;
  transition: width 120ms ease;
}

.km-sidebar-header {
  height: 48px;
  min-height: 48px;
  padding: 0 8px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 4px;
  flex-shrink: 0;
}

.km-sidebar.rail .km-sidebar-header {
  justify-content: center;
  padding: 0 4px;
}

.km-brand-drag {
  flex: 1 1 auto;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  min-width: 0;
}

.km-brand-title {
  font-size: 14px;
  font-weight: 700;
  letter-spacing: 0.01em;
}

.km-menu-wrap {
  position: relative;
  display: inline-flex;
  align-items: center;
}

.km-menu-wrap.rail-menu-pair {
  gap: 2px;
}

.km-icon-btn {
  width: 30px;
  height: 30px;
  border: none;
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-text-muted);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  flex-shrink: 0;
}

.km-icon-btn:hover {
  background: var(--km-hover-bg);
  color: var(--km-text);
}

.km-icon-btn.spinning svg {
  animation: km-spin 1s linear infinite;
}

@keyframes km-spin {
  to {
    transform: rotate(360deg);
  }
}

/* 写信分割按钮（x=6, y=52, h=46） */
.km-compose-cta-wrap {
  position: relative;
  padding: 4px 6px 8px;
  flex-shrink: 0;
}

.km-compose-cta-wrap.rail {
  display: flex;
  justify-content: center;
  padding: 4px 8px 8px;
}

.km-compose-split {
  width: 100%;
  height: 46px;
  border-radius: 10px;
  background: #3584e4;
  color: #ffffff;
  display: flex;
  align-items: stretch;
  overflow: hidden;
  box-shadow: var(--km-shadow-card);
}

.km-compose-main {
  flex: 1 1 auto;
  min-width: 0;
  border: none;
  background: transparent;
  color: #ffffff;
  font-size: 14px;
  font-weight: 600;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 0 8px 0 14px;
  cursor: pointer;
}

.km-compose-main:hover,
.km-compose-caret:hover {
  background: rgba(255, 255, 255, 0.12);
}

.km-compose-sep {
  width: 1px;
  margin: 10px 0;
  background: rgba(255, 255, 255, 0.28);
  flex-shrink: 0;
}

.km-compose-caret {
  width: 34px;
  border: none;
  background: transparent;
  color: #ffffff;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  flex-shrink: 0;
}

.km-compose-rail-btn {
  width: 44px;
  height: 44px;
  border: none;
  border-radius: 10px;
  background: #3584e4;
  color: #ffffff;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  box-shadow: var(--km-shadow-card);
}

.km-compose-rail-btn:hover {
  filter: brightness(1.06);
}

/* 弹出菜单 */
.km-popover-menu {
  position: absolute;
  top: 100%;
  background: var(--km-surface-raised);
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-md);
  box-shadow: var(--km-shadow-popover);
  padding: 6px;
  z-index: 60;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.km-main-menu {
  right: 0;
  top: 34px;
  width: 226px;
  max-height: calc(100vh - 52px);
  overflow-y: auto;
}

.km-sidebar.rail .km-main-menu {
  left: 0;
  right: auto;
}

.km-compose-menu {
  left: 6px;
  right: 6px;
  top: 52px;
}

.km-menu-item {
  border: none;
  background: transparent;
  color: var(--km-text);
  padding: 6px 8px;
  border-radius: var(--km-radius-sm);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  font-size: 12px;
  cursor: pointer;
  text-align: left;
  width: 100%;
}

.km-menu-item:hover {
  background: var(--km-hover-bg);
}

.km-menu-item.km-menu-danger {
  color: var(--km-danger);
}

.km-menu-item-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-menu-shortcut {
  font-size: 11px;
  color: var(--km-text-muted);
  flex-shrink: 0;
}

.km-menu-section-label {
  padding: 4px 8px 2px;
  font-size: 11px;
  font-weight: 600;
  color: var(--km-text-muted);
}

.km-menu-empty {
  padding: 6px 8px;
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-menu-divider {
  height: 1px;
  background: var(--km-border);
  margin: 4px 0;
}

.km-menu-segmented {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 4px;
  padding: 2px 4px 4px;
}

.km-seg-btn {
  border: 1px solid var(--km-border);
  background: transparent;
  color: var(--km-text);
  border-radius: var(--km-radius-sm);
  padding: 4px 0;
  font-size: 11px;
  cursor: pointer;
}

.km-seg-btn:hover {
  background: var(--km-hover-bg);
}

.km-seg-btn.active {
  background: var(--km-accent);
  border-color: var(--km-accent);
  color: #ffffff;
  font-weight: 600;
}

.km-theme-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 4px;
  padding: 2px 4px 4px;
}

.km-theme-swatch-btn {
  height: 26px;
  padding: 0 7px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-text);
  font-size: 11px;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  cursor: pointer;
  min-width: 0;
}

.km-theme-swatch-btn:hover {
  background: var(--km-hover-bg);
}

.km-theme-swatch-btn.active {
  border-color: var(--km-accent);
  background: var(--km-accent-soft);
  color: var(--km-accent);
  font-weight: 600;
}

.km-theme-swatch-dot {
  width: 9px;
  height: 9px;
  border-radius: 999px;
  flex-shrink: 0;
  box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.14);
}

.km-theme-swatch-label {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: left;
}

.km-theme-swatch-check {
  flex-shrink: 0;
}

/* 中部导航滚动区 */
.km-sidebar-scroll {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  padding: 4px 6px 8px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.km-sidebar.rail .km-sidebar-scroll {
  padding: 4px 6px;
  align-items: center;
}

.km-nav-section {
  display: flex;
  flex-direction: column;
  gap: 2px;
  width: 100%;
}

.km-unified-block {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.km-nav-row {
  height: 44px;
  padding: 0 10px;
  border: none;
  border-radius: 10px;
  background: transparent;
  color: var(--km-text);
  display: flex;
  align-items: center;
  gap: 10px;
  text-align: left;
  cursor: pointer;
  width: 100%;
  user-select: none;
}

.km-nav-row:hover {
  background: var(--km-hover-bg);
}

.km-nav-row.active {
  background: var(--km-sidebar-active-bg);
  color: var(--km-text);
  font-weight: 600;
}

.km-nav-icon {
  width: 18px;
  height: 18px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--km-text);
  flex-shrink: 0;
}

.km-nav-label {
  flex: 1 1 auto;
  min-width: 0;
  font-size: 14px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-count-pill {
  min-width: 22px;
  height: 20px;
  padding: 0 7px;
  border-radius: 999px;
  background: #3584e4;
  color: #ffffff;
  font-size: 11px;
  font-weight: 700;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.km-row-chevron-btn {
  width: 22px;
  height: 22px;
  border: none;
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-text-muted);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  flex-shrink: 0;
  transition: transform 120ms ease, background 120ms ease;
}

.km-row-chevron-btn:hover {
  background: var(--km-hover-bg);
  color: var(--km-text);
}

.km-row-chevron-btn.expanded,
.km-row-chevron-static.expanded,
.km-tree-chev.expanded {
  transform: rotate(90deg);
}

.km-row-chevron-static {
  width: 20px;
  height: 20px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--km-text-muted);
  flex-shrink: 0;
  transition: transform 120ms ease;
}

.km-subrow-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding-left: 18px;
}

.km-subrow-btn {
  height: 32px;
  padding: 0 10px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--km-text);
  display: flex;
  align-items: center;
  gap: 8px;
  text-align: left;
  cursor: pointer;
  width: 100%;
  font-size: 13px;
}

.km-subrow-btn:hover {
  background: var(--km-hover-bg);
}

.km-subrow-btn.active {
  background: var(--km-sidebar-active-bg);
  font-weight: 600;
}

.km-subrow-label {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-subrow-icon {
  width: 14px;
  height: 14px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--km-text-muted);
  flex-shrink: 0;
}

.km-subrow-empty {
  padding: 4px 10px;
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-account-dot {
  width: 9px;
  height: 9px;
  border-radius: 999px;
  flex-shrink: 0;
}

/* 账号区 */
.km-accounts-section {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding-top: 4px;
}

.km-accounts-empty {
  padding: 8px 10px;
  font-size: 12px;
  color: var(--km-text-muted);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
}

.km-accounts-add-link {
  border: none;
  background: transparent;
  color: var(--km-accent);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  padding: 0;
}

.km-account-group {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.km-account-header {
  min-height: 48px;
  padding: 6px 8px;
  border: none;
  border-radius: 10px;
  background: transparent;
  color: var(--km-text);
  display: flex;
  align-items: center;
  gap: 10px;
  text-align: left;
  cursor: pointer;
  width: 100%;
}

.km-account-header:hover {
  background: var(--km-hover-bg);
}

.km-account-avatar-wrap {
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.km-account-ring {
  width: 38px;
  height: 38px;
  border-radius: 999px;
  border: 2px solid #3584e4;
  padding: 1.5px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.km-account-avatar {
  width: 100%;
  height: 100%;
  border-radius: 999px;
  color: #ffffff;
  font-size: 12px;
  font-weight: 700;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.km-avatar-badge {
  position: absolute;
  top: -3px;
  right: -4px;
  min-width: 18px;
  height: 18px;
  padding: 0 4px;
  border-radius: 999px;
  background: #3584e4;
  color: #ffffff;
  border: 2px solid var(--km-sidebar, #ebebed);
  font-size: 10px;
  font-weight: 700;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  line-height: 1;
}

.km-account-meta {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  line-height: 1.25;
}

.km-account-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--km-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-account-email {
  font-size: 12px;
  color: var(--km-text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-folder-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.km-folder-tree-row {
  display: flex;
  align-items: center;
  gap: 2px;
}

.km-tree-chev {
  width: 18px;
  height: 28px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--km-text-muted);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  flex-shrink: 0;
  transition: transform 120ms ease;
}

.km-tree-chev:hover {
  background: var(--km-hover-bg);
}

.km-tree-spacer {
  width: 18px;
  flex-shrink: 0;
}

.km-folder-btn {
  flex: 1 1 auto;
  min-width: 0;
  height: 34px;
  padding: 0 8px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--km-text);
  display: flex;
  align-items: center;
  gap: 8px;
  text-align: left;
  cursor: pointer;
}

.km-folder-btn:hover {
  background: var(--km-hover-bg);
}

.km-folder-btn.active {
  background: var(--km-sidebar-active-bg);
  font-weight: 600;
}

.km-folder-total {
  font-size: 11px;
  color: var(--km-text-muted);
  flex-shrink: 0;
}

/* 窄轨模式 */
.km-rail-stack {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding-top: 4px;
}

.km-rail-icon-btn {
  position: relative;
  width: 44px;
  height: 40px;
  border: none;
  border-radius: 10px;
  background: transparent;
  color: var(--km-text);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}

.km-rail-icon-btn:hover {
  background: var(--km-hover-bg);
}

.km-rail-icon-btn.active {
  background: var(--km-sidebar-active-bg);
}

.km-rail-divider {
  width: 28px;
  height: 1px;
  background: var(--km-border);
}

.km-rail-account-btn {
  width: 46px;
  height: 46px;
  padding: 0;
  border: none;
  border-radius: 12px;
  background: transparent;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}

.km-rail-account-btn:hover {
  background: var(--km-hover-bg);
}

.km-rail-account-btn.active {
  background: var(--km-sidebar-active-bg);
}

/* 底部固定两行：通讯录与附件 */
.km-sidebar-footer {
  flex-shrink: 0;
  padding: 6px;
  border-top: 1px solid var(--km-border);
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.km-sidebar-footer.rail {
  align-items: center;
  padding: 6px 4px;
}

.km-footer-row {
  height: 42px;
  padding: 0 10px;
  border: none;
  border-radius: 10px;
  background: transparent;
  color: var(--km-text);
  display: flex;
  align-items: center;
  gap: 10px;
  text-align: left;
  cursor: pointer;
  width: 100%;
}

.km-footer-row.rail-row {
  width: 44px;
  height: 38px;
  padding: 0;
  justify-content: center;
}

.km-footer-row:hover {
  background: var(--km-hover-bg);
}

.km-footer-row.active {
  background: var(--km-sidebar-active-bg);
  font-weight: 600;
}
</style>

<template>
  <section class="km-reader-pane" :class="{ below: isBelow }" aria-label="邮件阅读区">
    <!-- 顶部一行无边框图标工具栏 (48px)，空白区支持拖拽窗口 -->
    <header ref="toolbarRef" class="km-reader-toolbar">
      <div v-if="message" class="km-toolbar-group km-toolbar-left">
        <button
          type="button"
          class="km-tool-icon-btn"
          title="回复"
          aria-label="回复"
          @click="emit('reply')"
        >
          <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
            <path
              fill="currentColor"
              d="M6.8 3.2 2.4 7.5l4.4 4.3v-2.6c3.2 0 5.3.95 6.8 3.2-.6-3.35-2.65-6.1-6.8-6.6V3.2Z"
            />
          </svg>
        </button>
        <button
          type="button"
          class="km-tool-icon-btn"
          title="全部回复"
          aria-label="全部回复"
          @click="emit('replyAll')"
        >
          <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
            <path
              fill="currentColor"
              d="M5.2 3.3 1.4 7.1l3.8 3.8V9.4L2.9 7.1l2.3-2.3V3.3Zm3.9-.1L4.7 7.5l4.4 4.3V9.2c2.95 0 4.8.9 6.2 3-.55-3.2-2.45-5.8-6.2-6.3V3.2Z"
            />
          </svg>
        </button>
        <button
          type="button"
          class="km-tool-icon-btn"
          title="转发"
          aria-label="转发"
          @click="emit('forward')"
        >
          <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
            <path
              fill="currentColor"
              d="M9.2 3.2v2.6C5.05 6.3 3 9.05 2.4 12.4c1.5-2.25 3.6-3.2 6.8-3.2v2.6l4.4-4.3-4.4-4.3Z"
            />
          </svg>
        </button>
        <button
          type="button"
          class="km-tool-icon-btn"
          :class="{ flagged: message.flagged }"
          :title="message.flagged ? '取消星标' : '加上星标'"
          :aria-label="message.flagged ? '取消星标' : '加上星标'"
          @click="emit('toggleFlagged', !message.flagged)"
        >
          <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
            <path
              v-if="message.flagged"
              fill="currentColor"
              d="M8 1.6 9.95 5.6l4.4.64-3.18 3.1.75 4.38L8 11.65l-3.92 2.07.75-4.38L1.65 6.24l4.4-.64L8 1.6Z"
            />
            <path
              v-else
              fill="currentColor"
              d="M8 1.6 9.95 5.6l4.4.64-3.18 3.1.75 4.38L8 11.65l-3.92 2.07.75-4.38L1.65 6.24l4.4-.64L8 1.6Zm0 2.46L6.7 6.7l-2.9.42 2.1 2.04-.5 2.9L8 10.7l2.6 1.36-.5-2.9 2.1-2.04-2.9-.42L8 4.06Z"
            />
          </svg>
        </button>
        <button
          type="button"
          class="km-tool-icon-btn"
          title="归档"
          aria-label="归档"
          :disabled="movingMessage"
          @click="emit('archive')"
        >
          <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
            <path
              fill="currentColor"
              d="M2.2 2.6h11.6v2.7H2.2V2.6Zm.8 3.8h10v6.8a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6.4Zm3.2 2v1.3h3.6V8.4H6.2Z"
            />
          </svg>
        </button>
        <button
          type="button"
          class="km-tool-icon-btn km-tool-danger"
          title="删除"
          aria-label="删除"
          :disabled="movingMessage"
          @click="emit('delete')"
        >
          <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
            <path
              fill="currentColor"
              d="M5.6 2h4.8l.6 1.2H13.5v1.3h-11V3.2H5L5.6 2ZM3.8 5.4h8.4l-.65 8.1a1 1 0 0 1-1 .9H5.45a1 1 0 0 1-1-.9L3.8 5.4Z"
            />
          </svg>
        </button>
      </div>

      <!-- 拖拽空白区 -->
      <div class="km-toolbar-drag" data-tauri-drag-region />

      <div class="km-toolbar-group km-toolbar-right">
        <template v-if="message">
          <!-- 标签弹出菜单 -->
          <div v-if="!isOverflowed('tags')" ref="tagsWrapRef" class="km-popover-wrap">
            <button
              type="button"
              class="km-tool-icon-btn"
              :class="{ active: selectedTags.length > 0 }"
              title="管理标签"
              aria-label="管理标签"
              aria-haspopup="menu"
              :aria-expanded="tagsMenuOpen"
              @click="toggleTagsMenu"
            >
              <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
                <path
                  fill="currentColor"
                  d="M2.2 2.8A.8.8 0 0 1 3 2h4.6a1 1 0 0 1 .7.3l5.2 5.2a1 1 0 0 1 0 1.4l-4.6 4.6a1 1 0 0 1-1.4 0L2.3 8.3a1 1 0 0 1-.3-.7V3a.8.8 0 0 1 .2-.2ZM4.8 4.2a1 1 0 1 0 0 2 1 1 0 0 0 0-2Z"
                />
              </svg>
            </button>
            <div v-if="tagsMenuOpen" class="km-theme-popover km-tags-popover" role="menu">
              <div class="km-menu-title">标签</div>
              <div v-if="availableTags.length === 0" class="km-menu-empty">暂无可用标签</div>
              <button
                v-for="tag in availableTags"
                :key="tag.id"
                type="button"
                class="km-theme-item"
                :class="{ active: hasTag(tag.name) }"
                @click="toggleTag(tag.name)"
              >
                <span class="km-theme-left">
                  <span class="km-swatch-dot" :style="{ backgroundColor: tag.color }" />
                  <span>{{ tag.name }}</span>
                </span>
                <svg
                  v-if="hasTag(tag.name)"
                  viewBox="0 0 16 16"
                  width="12"
                  height="12"
                  aria-hidden="true"
                >
                  <path
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.8"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    d="m3.5 8.5 3 3 6-6.5"
                  />
                </svg>
              </button>
            </div>
          </div>

          <!-- 已读 / 未读 -->
          <button
            v-if="!isOverflowed('toggleSeen')"
            type="button"
            class="km-tool-icon-btn"
            :class="{ active: selectedUnread }"
            :title="selectedUnread ? '标为已读' : '标为未读'"
            :aria-label="selectedUnread ? '标为已读' : '标为未读'"
            @click="emit('toggleSeen', Boolean(selectedUnread))"
          >
            <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
              <path
                v-if="selectedUnread"
                fill="currentColor"
                d="M2.2 4.2A1.2 1.2 0 0 1 3.4 3h9.2a1.2 1.2 0 0 1 1.2 1.2v7.6a1.2 1.2 0 0 1-1.2 1.2H3.4a1.2 1.2 0 0 1-1.2-1.2V4.2Zm1.5.5L8 7.8l4.3-3.1H3.7Z"
              />
              <path
                v-else
                fill="currentColor"
                d="M8 2.2 2.2 6v6.8a1 1 0 0 0 1 1h9.6a1 1 0 0 0 1-1V6L8 2.2ZM3.5 6.8 8 9.6l4.5-2.8v5.5h-9V6.8Z"
              />
            </svg>
          </button>

          <!-- 垃圾邮件 / 恢复 -->
          <button
            v-if="!isOverflowed('spam')"
            type="button"
            class="km-tool-icon-btn"
            :title="inSpamFolder ? '恢复（不是垃圾邮件）' : '标记为垃圾邮件'"
            :aria-label="inSpamFolder ? '不是垃圾邮件' : '标记为垃圾邮件'"
            :disabled="movingMessage"
            @click="inSpamFolder ? emit('unmarkSpam') : emit('markSpam')"
          >
            <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
              <path
                fill="currentColor"
                d="M5.4 2h5.2L14 5.4v5.2L10.6 14H5.4L2 10.6V5.4L5.4 2Zm2 2.8v4.3h1.2V4.8H7.4Zm0 5.5v1.3h1.2v-1.3H7.4Z"
              />
            </svg>
          </button>

          <!-- 移动到文件夹 -->
          <div v-if="!isOverflowed('move')" ref="moveWrapRef" class="km-popover-wrap">
            <button
              type="button"
              class="km-tool-icon-btn"
              title="移动到文件夹"
              aria-label="移动到文件夹"
              aria-haspopup="menu"
              :aria-expanded="moveMenuOpen"
              :disabled="movingMessage || accountFolders.length === 0"
              @click="toggleMoveMenu"
            >
              <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
                <path
                  fill="currentColor"
                  d="M2.2 3.5A1.3 1.3 0 0 1 3.5 2.2H6.4l1.3 1.4h4.8a1.3 1.3 0 0 1 1.3 1.3v7.6a1.3 1.3 0 0 1-1.3 1.3h-9a1.3 1.3 0 0 1-1.3-1.3V3.5Zm4.1 5.1H4.5l3.5 3.2 3.5-3.2H9.7V6.2H6.3v2.4Z"
                />
              </svg>
            </button>

            <div v-if="moveMenuOpen" class="km-theme-popover km-move-popover" role="menu">
              <div class="km-menu-title">移动到文件夹</div>
              <button
                v-for="folder in accountFolders"
                :key="`${folder.accountId}:${folder.id}`"
                type="button"
                class="km-theme-item"
                :class="{ active: isCurrentFolder(folder.path) }"
                :disabled="isCurrentFolder(folder.path)"
                @click="pickMoveFolder(folder.path)"
              >
                <span>{{ folder.name }}</span>
                <span v-if="isCurrentFolder(folder.path)" class="km-current-tag">当前</span>
              </button>
            </div>
          </div>

          <!-- 正文查找（放大镜） -->
          <button
            v-if="!isOverflowed('find')"
            type="button"
            class="km-tool-icon-btn"
            :class="{ active: findOpen }"
            title="在正文中查找"
            aria-label="在正文中查找"
            :aria-pressed="Boolean(findOpen)"
            @click="emit('toggleFind')"
          >
            <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
              <path
                fill="currentColor"
                d="M6.8 2.2a4.6 4.6 0 1 1 0 9.2 4.6 4.6 0 0 1 0-9.2Zm0 1.4a3.2 3.2 0 1 0 0 6.4 3.2 3.2 0 0 0 0-6.4Zm3.6 6.2 3.4 3.4-.95.95-3.4-3.4.95-.95Z"
              />
            </svg>
          </button>

          <!-- 打印 -->
          <button
            v-if="!isOverflowed('print')"
            type="button"
            class="km-tool-icon-btn"
            title="打印预览 / 打印"
            aria-label="打印"
            @click="emit('print')"
          >
            <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
              <path
                fill="currentColor"
                d="M4.5 2h7v2.8h-7V2Zm-2 3.8h11A1.5 1.5 0 0 1 15 7.3v3.5h-2.5v3.2h-9v-3.2H1V7.3a1.5 1.5 0 0 1 1.5-1.5Zm2.3 4.2v2.7h6.4V10H4.8Z"
              />
            </svg>
          </button>

          <!-- 窄窗溢出菜单 -->
          <div v-if="overflowActions.length > 0" ref="overflowWrapRef" class="km-popover-wrap">
            <button
              type="button"
              class="km-tool-icon-btn"
              title="更多操作"
              aria-label="更多阅读操作"
              aria-haspopup="menu"
              :aria-expanded="overflowMenuOpen"
              @click="toggleOverflowMenu"
            >
              ⋯
            </button>
            <div v-if="overflowMenuOpen" class="km-theme-popover km-overflow-popover" role="menu">
              <button
                v-if="isOverflowed('tags')"
                type="button"
                class="km-theme-item"
                @click="openTagsFromOverflow"
              >
                <span>标签</span>
              </button>
              <button
                v-if="isOverflowed('toggleSeen')"
                type="button"
                class="km-theme-item"
                @click="emit('toggleSeen', Boolean(selectedUnread)); setOverflowMenu(false)"
              >
                <span>{{ selectedUnread ? "标为已读" : "标为未读" }}</span>
              </button>
              <button
                v-if="isOverflowed('spam')"
                type="button"
                class="km-theme-item"
                :disabled="movingMessage"
                @click="inSpamFolder ? emit('unmarkSpam') : emit('markSpam'); setOverflowMenu(false)"
              >
                <span>{{ inSpamFolder ? "不是垃圾邮件" : "标记为垃圾邮件" }}</span>
              </button>
              <button
                v-if="isOverflowed('find')"
                type="button"
                class="km-theme-item"
                @click="emit('toggleFind'); setOverflowMenu(false)"
              >
                <span>查找正文</span>
              </button>
              <button
                v-if="isOverflowed('print')"
                type="button"
                class="km-theme-item"
                @click="emit('print'); setOverflowMenu(false)"
              >
                <span>打印</span>
              </button>
              <template v-if="isOverflowed('move') && accountFolders.length > 0">
                <div class="km-menu-divider" />
                <div class="km-menu-title">移动到</div>
                <button
                  v-for="folder in accountFolders"
                  :key="`ov-${folder.accountId}:${folder.id}`"
                  type="button"
                  class="km-theme-item"
                  :disabled="isCurrentFolder(folder.path)"
                  @click="pickMoveFolder(folder.path); setOverflowMenu(false)"
                >
                  <span>{{ folder.name }}</span>
                  <span v-if="isCurrentFolder(folder.path)" class="km-current-tag">当前</span>
                </button>
              </template>
            </div>
          </div>
        </template>

        <button
          v-if="(error || message?.bodyStatus === 'unavailable') && message"
          type="button"
          class="km-retry-pill"
          @click="emit('retry')"
        >
          重试正文
        </button>

        <!-- 窗口关闭按钮（并入顶部工具栏最右侧） -->
        <button
          v-if="showWindowClose"
          type="button"
          class="km-tool-icon-btn km-tool-close"
          title="关闭主窗口"
          aria-label="关闭窗口"
          @click="emit('closeWindow')"
        >
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
            <path
              fill="currentColor"
              d="m3.7 2.8 4.3 4.3 4.3-4.3.9.9L8.9 8l4.3 4.3-.9.9L8 8.9l-4.3 4.3-.9-.9L7.1 8 2.8 3.7l.9-.9Z"
            />
          </svg>
        </button>
      </div>
    </header>

    <!-- 空状态 -->
    <div v-if="!message && !loading && !error" class="km-reader-empty">
      <div class="km-empty-icon" aria-hidden="true">
        <svg viewBox="0 0 24 24" width="34" height="34" fill="none" aria-hidden="true">
          <rect
            x="3"
            y="5"
            width="18"
            height="14"
            rx="2.5"
            stroke="currentColor"
            stroke-width="1.5"
          />
          <path
            stroke="currentColor"
            stroke-width="1.5"
            stroke-linecap="round"
            stroke-linejoin="round"
            d="m3.8 6.2 8.2 6.3 8.2-6.3"
          />
        </svg>
      </div>
      <p>选择一封邮件查看正文</p>
    </div>

    <!-- 仅加载中骨架 -->
    <div v-else-if="loading && !message" class="km-reader-skeleton" aria-busy="true" aria-label="正在加载邮件">
      <div class="km-sk-line km-sk-title" />
      <div class="km-sk-line km-sk-meta" />
      <div class="km-sk-card">
        <div class="km-sk-line" />
        <div class="km-sk-line" />
        <div class="km-sk-line short" />
      </div>
    </div>

    <!-- 仅错误 -->
    <div v-else-if="error && !message" class="km-reader-empty" role="alert">
      <p class="km-error-msg">{{ error }}</p>
      <button type="button" class="km-retry-btn" @click="emit('retry')">重试</button>
    </div>

    <!-- 邮件阅读主区 -->
    <div v-else-if="message" class="km-reader-main">
      <!-- 固定标题区：邮箱胶囊 + 此邮件亮暗 / 阅读视图两枚开关 + 独立主题行 -->
      <div class="km-reader-subject-header">
        <div class="km-subject-top-row">
          <span v-if="resolvedAccountEmail" class="km-account-pill" :title="resolvedAccountEmail">
            {{ resolvedAccountEmail }}
          </span>
          <span v-else class="km-account-pill-spacer" />

          <div class="km-view-toggles">
            <button
              type="button"
              class="km-tool-icon-btn km-mode-toggle"
              :class="{ active: perMessageDark !== null && perMessageDark !== undefined }"
              :aria-pressed="Boolean(perMessageDark)"
              :title="perMessageDarkTitle"
              aria-label="切换此邮件亮暗渲染"
              @click="emit('togglePerMessageDark')"
            >
              <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
                <path
                  fill="currentColor"
                  d="M8 1.8a6.2 6.2 0 1 0 0 12.4A6.2 6.2 0 0 0 8 1.8Zm0 1.4a4.8 4.8 0 0 1 0 9.6V3.2Z"
                />
              </svg>
            </button>
            <button
              type="button"
              class="km-tool-icon-btn km-mode-toggle"
              :class="{ active: Boolean(readingView) }"
              :aria-pressed="Boolean(readingView)"
              title="切换阅读视图（统一排版显示邮件内容）"
              aria-label="切换阅读视图"
              @click="emit('toggleReadingView')"
            >
              <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
                <path
                  fill="currentColor"
                  d="M1.5 7.5a2.6 2.6 0 0 1 5.05-.86c.42-.2.9-.3 1.45-.3s1.03.1 1.45.3A2.6 2.6 0 1 1 14.5 8.5a2.6 2.6 0 0 1-5.12-.62A1.8 1.8 0 0 0 8 7.5a1.8 1.8 0 0 0-1.38.38A2.6 2.6 0 1 1 1.5 7.5Zm2.6-1.3a1.3 1.3 0 1 0 0 2.6 1.3 1.3 0 0 0 0-2.6Zm7.8 0a1.3 1.3 0 1 0 0 2.6 1.3 1.3 0 0 0 0-2.6Z"
                />
              </svg>
            </button>
          </div>
        </div>

        <h1 class="km-subject-title">{{ message.subject || "(无主题)" }}</h1>

        <!-- 正文查找栏 -->
        <div v-if="findOpen" class="km-find-bar" role="search" aria-label="正文查找">
          <input
            ref="findInputRef"
            :value="findQuery ?? ''"
            type="search"
            class="km-find-input"
            placeholder="在当前邮件正文中查找…"
            aria-label="查找关键词"
            @input="onFindInput"
            @keydown.enter.exact.prevent="emit('findStep', 1)"
            @keydown.shift.enter.prevent="emit('findStep', -1)"
            @keydown.esc.stop.prevent="emit('closeFind')"
          />
          <span class="km-find-count">
            {{ findCounterText }}
          </span>
          <button
            type="button"
            class="km-find-nav-btn"
            title="上一处 (Shift+Enter)"
            aria-label="上一处"
            :disabled="(findTotal ?? 0) === 0"
            @click="emit('findStep', -1)"
          >
            ↑
          </button>
          <button
            type="button"
            class="km-find-nav-btn"
            title="下一处 (Enter)"
            aria-label="下一处"
            :disabled="(findTotal ?? 0) === 0"
            @click="emit('findStep', 1)"
          >
            ↓
          </button>
          <button
            type="button"
            class="km-find-nav-btn"
            title="关闭查找 (Esc)"
            aria-label="关闭查找"
            @click="emit('closeFind')"
          >
            <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
              <path
                fill="none"
                stroke="currentColor"
                stroke-width="1.6"
                stroke-linecap="round"
                d="m4 4 8 8m0-8-8 8"
              />
            </svg>
          </button>
        </div>

        <!-- 远程图片隐私拦截提示条 -->
        <div
          v-if="message.remoteImagesBlocked === true"
          class="km-remote-img-banner"
          role="region"
          aria-label="远程图片隐私保护"
        >
          <span class="km-remote-img-text">已拦截远程图片以保护隐私</span>
          <div class="km-remote-img-actions">
            <button
              type="button"
              class="km-remote-img-btn"
              :disabled="allowingRemoteImages"
              @click="emit('allowRemoteImages', false)"
            >
              仅本次显示
            </button>
            <button
              type="button"
              class="km-remote-img-btn primary"
              :disabled="allowingRemoteImages"
              @click="emit('allowRemoteImages', true)"
            >
              始终信任此发件人
            </button>
          </div>
        </div>
      </div>

      <!-- 正文视口 -->
      <ReaderViewport
        :readerSessionId="message.readerSessionId"
        :bodyStatus="message.bodyStatus"
        :hasHtml="message.hasHtml"
        :plainText="message.plainText"
        :htmlBody="message.htmlBody"
        :threadMessages="message.threadMessages"
        :overlayOccluded="Boolean(overlayOccluded)"
        :errorMessage="error"
        :zoomPercent="zoomPercent"
        :accountId="message.accountId"
        :messageId="message.messageId"
        :sender="message.sender"
        :recipients="message.recipients"
        :cc="message.cc"
        :dateDisplay="message.dateDisplay"
        :attachments="message.attachments"
        :flagged="message.flagged"
        :effectiveDark="effectiveDark"
        :perMessageDark="perMessageDark"
        :readingView="readingView"
        :findQuery="findOpen ? findQuery : ''"
        :findStepToken="findStepToken"
        :findStepDelta="findStepDelta"
        @retry="emit('retry')"
        @cardAction="handleViewportCardAction"
        @findResult="(res) => emit('fallbackFindResult', res)"
      />

      <!-- 底部内嵌附件抽屉 -->
      <AttachmentDrawer
        v-if="aggregatedAttachments.length > 0"
        ref="drawerRef"
        :attachments="message.attachments"
        :threadMessages="message.threadMessages"
        :subject="message.subject"
        @overlayChange="(id, open) => emit('overlayChange', id, open)"
      />
    </div>
  </section>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type {
  FolderSummaryDto,
  OpenMessageOutput,
  ReaderPosition,
  TagItemDto,
  ThreadCardActionEvent,
} from "../ipc/types";
import AttachmentDrawer from "../reader-tools/AttachmentDrawer.vue";
import { aggregateThreadAttachments } from "../reader-tools/useAttachments";
import ReaderViewport from "./ReaderViewport.vue";
import {
  computeReaderOverflowActions,
  isSpamFolderPath,
  type ReaderOverflowActionId,
} from "./readerHelpers";

const props = withDefaults(
  defineProps<{
    message: OpenMessageOutput | null;
    loading: boolean;
    error: string | null;
    selectedUnread?: boolean;
    readerPosition: ReaderPosition;
    themeId: string;
    darkMode?: boolean | null;
    effectiveDark?: boolean;
    followSystem?: boolean;
    overlayOccluded?: boolean;
    zoomPercent?: number;
    isBelow?: boolean;
    folders?: FolderSummaryDto[];
    allowingRemoteImages?: boolean;
    movingMessage?: boolean;
    accountEmail?: string;
    availableTags?: TagItemDto[];
    selectedTags?: string[];
    perMessageDark?: boolean | null;
    readingView?: boolean;
    findOpen?: boolean;
    findQuery?: string;
    findCurrent?: number;
    findTotal?: number;
    findStepToken?: number;
    findStepDelta?: number;
    showWindowClose?: boolean;
  }>(),
  {
    availableTags: () => [],
    selectedTags: () => [],
    perMessageDark: null,
    readingView: false,
    findOpen: false,
    findQuery: "",
    findCurrent: 0,
    findTotal: 0,
    showWindowClose: true,
  }
);

const emit = defineEmits<{
  retry: [];
  reply: [];
  replyAll: [];
  forward: [];
  toggleSeen: [seen: boolean];
  toggleFlagged: [flagged: boolean];
  setReaderPosition: [pos: ReaderPosition];
  setTheme: [themeId: string, darkMode?: boolean | null];
  setDarkMode: [mode: boolean | null];
  toggleDarkMode: [];
  toggleFollowSystem: [];
  allowRemoteImages: [trustSender: boolean];
  archive: [];
  markSpam: [];
  unmarkSpam: [];
  delete: [];
  moveToFolder: [targetFolderPath: string];
  openStandalone: [];
  print: [];
  openAttachments: [];
  setTags: [tags: string[]];
  togglePerMessageDark: [];
  toggleReadingView: [];
  toggleFind: [];
  closeFind: [];
  findQuery: [query: string];
  findStep: [delta: number];
  fallbackFindResult: [result: { current: number; total: number }];
  cardAction: [event: ThreadCardActionEvent];
  closeWindow: [];
  overlayChange: [id: string, open: boolean];
}>();

const toolbarRef = ref<HTMLElement | null>(null);
const toolbarWidth = ref(673);
const tagsWrapRef = ref<HTMLElement | null>(null);
const tagsMenuOpen = ref(false);
const moveWrapRef = ref<HTMLElement | null>(null);
const moveMenuOpen = ref(false);
const overflowWrapRef = ref<HTMLElement | null>(null);
const overflowMenuOpen = ref(false);
const findInputRef = ref<HTMLInputElement | null>(null);
const drawerRef = ref<InstanceType<typeof AttachmentDrawer> | null>(null);

const overflowActions = computed(() => computeReaderOverflowActions(toolbarWidth.value));

function isOverflowed(id: ReaderOverflowActionId): boolean {
  return overflowActions.value.includes(id);
}

function setTagsMenu(open: boolean) {
  if (tagsMenuOpen.value === open) return;
  if (open) {
    setMoveMenu(false);
    setOverflowMenu(false);
  }
  tagsMenuOpen.value = open;
  emit("overlayChange", "reader-tags-menu", open);
}

function toggleTagsMenu() {
  setTagsMenu(!tagsMenuOpen.value);
}

function setMoveMenu(open: boolean) {
  if (moveMenuOpen.value === open) return;
  if (open) {
    setTagsMenu(false);
    setOverflowMenu(false);
  }
  moveMenuOpen.value = open;
  emit("overlayChange", "reader-move-menu", open);
}

function toggleMoveMenu() {
  setMoveMenu(!moveMenuOpen.value);
}

function setOverflowMenu(open: boolean) {
  if (overflowMenuOpen.value === open) return;
  if (open) {
    setTagsMenu(false);
    setMoveMenu(false);
  }
  overflowMenuOpen.value = open;
  emit("overlayChange", "reader-overflow-menu", open);
}

function toggleOverflowMenu() {
  setOverflowMenu(!overflowMenuOpen.value);
}

function openTagsFromOverflow() {
  setOverflowMenu(false);
  setTagsMenu(true);
}

function hasTag(name: string): boolean {
  return props.selectedTags.includes(name);
}

function toggleTag(name: string) {
  const next = hasTag(name)
    ? props.selectedTags.filter((item) => item !== name)
    : [...props.selectedTags, name];
  emit("setTags", next);
}

function pickMoveFolder(targetFolderPath: string) {
  emit("moveToFolder", targetFolderPath);
  setMoveMenu(false);
}

const accountFolders = computed(() => {
  if (!props.message || !props.folders) return [];
  return props.folders.filter((f) => f.accountId === props.message!.accountId);
});

const inSpamFolder = computed(() => isSpamFolderPath(props.message?.folderPath));

function isCurrentFolder(folderPath: string): boolean {
  const current = props.message?.folderPath;
  if (!current) return false;
  return current.toLowerCase() === folderPath.toLowerCase();
}

const resolvedAccountEmail = computed(() => {
  if (props.accountEmail) return props.accountEmail;
  return props.message?.recipients?.split(",")[0]?.trim() ?? "";
});

const perMessageDarkTitle = computed(() => {
  if (props.perMessageDark === true) return "当前邮件已强制深色（点击恢复默认）";
  if (props.perMessageDark === false) return "当前邮件已强制浅色（点击恢复默认）";
  return "切换此邮件独立亮暗外观";
});

const findCounterText = computed(() => {
  const total = props.findTotal ?? 0;
  const current = props.findCurrent ?? 0;
  if (total > 0) return `${current} / ${total}`;
  if ((props.findQuery ?? "").trim().length > 0) return "0 / 0";
  return "";
});

const aggregatedAttachments = computed(() => {
  if (!props.message) return [];
  return aggregateThreadAttachments(props.message.attachments, props.message.threadMessages);
});

function onFindInput(e: Event) {
  const val = (e.target as HTMLInputElement).value;
  emit("findQuery", val);
}

function handleViewportCardAction(event: ThreadCardActionEvent) {
  if (event.kind === "openAttachment" && event.targetId && drawerRef.value) {
    void drawerRef.value.openAttachmentById(event.targetId);
    return;
  }
  if (event.kind === "saveAttachment" && event.targetId && drawerRef.value) {
    void drawerRef.value.saveAttachmentById(event.targetId);
    return;
  }
  if (event.kind === "saveAllAttachments" && drawerRef.value) {
    void drawerRef.value.saveAllAttachmentsPicker();
    return;
  }
  emit("cardAction", event);
}

function openAttachmentFromNative(kind: "openAttachment" | "saveAttachment" | "saveAllAttachments", targetId?: number | null) {
  if (kind === "openAttachment" && targetId && drawerRef.value) {
    void drawerRef.value.openAttachmentById(targetId);
  } else if (kind === "saveAttachment" && targetId && drawerRef.value) {
    void drawerRef.value.saveAttachmentById(targetId);
  } else if (kind === "saveAllAttachments" && drawerRef.value) {
    void drawerRef.value.saveAllAttachmentsPicker();
  }
}

function expandAttachments() {
  drawerRef.value?.expandDrawer();
}

watch(
  () => props.findOpen,
  (isOpen) => {
    if (isOpen) {
      void nextTick(() => {
        findInputRef.value?.focus();
        findInputRef.value?.select();
      });
    }
  }
);

function onDocumentPointerDown(e: PointerEvent) {
  if (tagsMenuOpen.value && tagsWrapRef.value && !tagsWrapRef.value.contains(e.target as Node)) {
    setTagsMenu(false);
  }
  if (moveMenuOpen.value && moveWrapRef.value && !moveWrapRef.value.contains(e.target as Node)) {
    setMoveMenu(false);
  }
  if (
    overflowMenuOpen.value &&
    overflowWrapRef.value &&
    !overflowWrapRef.value.contains(e.target as Node)
  ) {
    setOverflowMenu(false);
  }
}

function onDocumentKeyDown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    setTagsMenu(false);
    setMoveMenu(false);
    setOverflowMenu(false);
  }
}

let toolbarRo: ResizeObserver | null = null;

onMounted(() => {
  if (toolbarRef.value && typeof ResizeObserver !== "undefined") {
    toolbarWidth.value = toolbarRef.value.getBoundingClientRect().width || 673;
    toolbarRo = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width;
      if (w && w > 0) toolbarWidth.value = w;
    });
    toolbarRo.observe(toolbarRef.value);
  }
  document.addEventListener("pointerdown", onDocumentPointerDown);
  document.addEventListener("keydown", onDocumentKeyDown);
});

onUnmounted(() => {
  toolbarRo?.disconnect();
  document.removeEventListener("pointerdown", onDocumentPointerDown);
  document.removeEventListener("keydown", onDocumentKeyDown);
  setTagsMenu(false);
  setMoveMenu(false);
  setOverflowMenu(false);
});

defineExpose({
  openAttachmentFromNative,
  expandAttachments,
});
</script>

<style scoped>
.km-reader-pane {
  flex: 1 1 auto;
  min-width: 0;
  min-height: 0;
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--km-reader-header-bg, #fafafb);
  color: var(--km-text);
  overflow: hidden;
}

:global(:root[data-color-mode="dark"]) .km-reader-pane {
  background: var(--km-reader-header-bg, #222226);
}

.km-reader-pane.below {
  min-height: 180px;
}

.km-reader-toolbar {
  height: 48px;
  min-height: 48px;
  padding: 0 10px 0 12px;
  display: flex;
  align-items: center;
  gap: 6px;
  background: var(--km-reader-header-bg, #fafafb);
  flex-shrink: 0;
}

:global(:root[data-color-mode="dark"]) .km-reader-toolbar {
  background: var(--km-reader-header-bg, #222226);
}

.km-toolbar-group {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
}

.km-toolbar-drag {
  flex: 1 1 auto;
  min-width: 16px;
  height: 100%;
}

.km-tool-icon-btn {
  width: 32px;
  height: 32px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--km-text);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  padding: 0;
}

.km-tool-icon-btn:hover:not(:disabled) {
  background: var(--km-hover-bg);
}

.km-tool-icon-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.km-tool-icon-btn.active {
  background: var(--km-accent-soft);
  color: var(--km-accent);
}

.km-tool-icon-btn.flagged {
  color: var(--km-star-gold);
}

.km-tool-danger:hover:not(:disabled) {
  color: var(--km-danger);
}

.km-tool-close {
  color: var(--km-text-muted);
}

.km-tool-close:hover {
  background: var(--km-danger-soft);
  color: var(--km-danger);
}

.km-retry-pill {
  height: 26px;
  padding: 0 10px;
  border: 1px solid color-mix(in srgb, var(--km-danger) 45%, var(--km-border));
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-danger);
  font-size: 12px;
  cursor: pointer;
}

.km-popover-wrap {
  position: relative;
}

.km-theme-popover {
  position: absolute;
  top: 34px;
  right: 0;
  width: 186px;
  padding: 6px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-md);
  background: var(--km-surface-raised);
  box-shadow: var(--km-shadow-popover);
  z-index: 40;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.km-menu-title {
  padding: 4px 8px;
  font-size: 11px;
  font-weight: 600;
  color: var(--km-text-muted);
}

.km-menu-empty {
  padding: 8px;
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-theme-item {
  border: none;
  background: transparent;
  color: var(--km-text);
  border-radius: var(--km-radius-sm);
  padding: 6px 8px;
  font-size: 12px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  cursor: pointer;
  text-align: left;
}

.km-theme-item:hover:not(:disabled) {
  background: var(--km-hover-bg);
}

.km-theme-item.active {
  color: var(--km-accent);
  font-weight: 600;
}

.km-theme-item:disabled {
  opacity: 0.5;
  cursor: default;
}

.km-theme-left {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}

.km-swatch-dot {
  width: 9px;
  height: 9px;
  border-radius: 999px;
  display: inline-block;
  flex-shrink: 0;
}

.km-current-tag {
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-menu-divider {
  height: 1px;
  background: var(--km-border);
  margin: 4px 0;
}

.km-reader-empty,
.km-reader-skeleton {
  flex: 1 1 auto;
  display: flex;
  flex-direction: column;
  padding: 20px;
}

.km-reader-empty {
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--km-text-muted);
}

.km-empty-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  opacity: 0.4;
}

.km-error-msg {
  color: var(--km-danger);
  margin: 0;
}

.km-retry-btn {
  padding: 5px 14px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface-raised);
  cursor: pointer;
}

.km-reader-skeleton {
  gap: 10px;
}

.km-sk-card {
  margin-top: 10px;
  padding: 18px;
  border-radius: var(--km-radius-lg);
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.km-sk-line {
  height: 12px;
  border-radius: 6px;
  background: color-mix(in srgb, var(--km-text-muted) 16%, transparent);
  animation: km-sk-pulse 1.3s ease-in-out infinite;
}

.km-sk-title {
  width: 62%;
  height: 20px;
}

.km-sk-meta {
  width: 44%;
}

.km-sk-line.short {
  width: 30%;
}

@keyframes km-sk-pulse {
  0%,
  100% {
    opacity: 0.45;
  }
  50% {
    opacity: 0.9;
  }
}

.km-reader-main {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.km-reader-subject-header {
  padding: 4px 16px 10px 21px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  background: var(--km-reader-header-bg, #fafafb);
  border-bottom: 1px solid var(--km-border);
  flex-shrink: 0;
}

:global(:root[data-color-mode="dark"]) .km-reader-subject-header {
  background: var(--km-reader-header-bg, #222226);
}

.km-subject-top-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  min-height: 28px;
}

.km-account-pill {
  display: inline-flex;
  align-items: center;
  max-width: 75%;
  padding: 2px 10px;
  border-radius: 999px;
  border: 1px solid var(--km-border);
  background: color-mix(in srgb, var(--km-text-muted) 8%, transparent);
  color: var(--km-text-muted);
  font-size: 11.5px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-account-pill-spacer {
  flex: 1 1 auto;
}

.km-view-toggles {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
}

.km-mode-toggle {
  width: 28px;
  height: 28px;
  color: var(--km-text-muted);
}

.km-mode-toggle.active {
  color: var(--km-accent);
  background: var(--km-accent-soft);
}

.km-subject-title {
  margin: 0;
  font-size: 22px;
  font-weight: 700;
  line-height: 1.28;
  user-select: text;
}

.km-find-bar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 8px;
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  background: var(--km-surface);
}

.km-find-input {
  flex: 1 1 auto;
  min-width: 0;
  border: none;
  background: transparent;
  font-size: 12.5px;
  outline: none;
}

.km-find-count {
  font-size: 11.5px;
  color: var(--km-text-muted);
  white-space: nowrap;
}

.km-find-nav-btn {
  width: 24px;
  height: 24px;
  padding: 0;
  border: none;
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-text-muted);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}

.km-find-nav-btn:hover:not(:disabled) {
  background: var(--km-hover-bg);
  color: var(--km-text);
}

.km-find-nav-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.km-remote-img-banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 8px;
  padding: 7px 12px;
  border: 1px solid color-mix(in srgb, var(--km-accent) 35%, var(--km-border));
  border-radius: var(--km-radius-md);
  background: var(--km-accent-soft);
  font-size: 12px;
}

.km-remote-img-text {
  color: var(--km-text);
  font-weight: 500;
}

.km-remote-img-actions {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.km-remote-img-btn {
  height: 25px;
  padding: 0 10px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface-raised);
  color: var(--km-text);
  font-size: 12px;
  cursor: pointer;
}

.km-remote-img-btn.primary {
  background: var(--km-accent);
  border-color: var(--km-accent);
  color: var(--km-accent-fg);
}

.km-remote-img-btn:hover:not(:disabled) {
  filter: brightness(0.96);
}

.km-remote-img-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
</style>

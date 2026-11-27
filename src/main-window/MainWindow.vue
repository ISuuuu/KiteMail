<template>
  <div ref="rootRef" class="km-main-window" :class="layoutClass">
    <FolderSidebar
      :accounts="state.accounts.value"
      :folders="state.folders.value"
      :currentView="state.currentView.value"
      :isRail="state.layout.isRail.value"
      :width="state.layout.sidebarVisualWidth.value"
      :syncing="state.syncStatus.value.syncing"
      :mode="state.mode.value"
      :hasPasscode="state.hasPasscode.value"
      :darkMode="state.layout.prefs.value.darkMode"
      :effectiveDark="state.layout.effectiveDark.value"
      @selectView="state.selectView"
      @toggleCollapse="state.layout.toggleSidebarCollapsed"
      @refresh="state.refresh"
      @lock="state.lock"
      @toggleDarkMode="state.layout.toggleDarkMode"
      @compose="openNewCompose()"
      @openAccounts="openAccountsModal()"
      @openAttachments="attachmentGalleryOpen = true"
      @openContacts="contactsModalOpen = true"
      @openCommandPalette="commandPaletteOpen = true"
      @openPreferences="openPreferencesModal('appearance')"
    />

    <!-- 侧栏分隔把手 -->
    <PaneSplitter
      v-if="!state.layout.isRail.value"
      direction="vertical"
      label="调整侧栏宽度"
      :value="state.layout.prefs.value.sidebarWidth"
      :min="SIDEBAR_MIN_WIDTH"
      :max="SIDEBAR_MAX_WIDTH"
      @dragStart="dragBaseSidebar = state.layout.prefs.value.sidebarWidth"
      @drag="(delta) => state.layout.onSidebarResize(dragBaseSidebar + delta)"
      @dragEnd="state.layout.flushSave()"
      @step="(delta) => state.layout.onSidebarResize(state.layout.prefs.value.sidebarWidth + delta)"
    />

    <!-- 中栏：列表（Right 模式定宽，Below/Off 模式铺满） -->
    <div class="km-list-column" :style="listColumnStyle">
      <div class="km-list-slot" :style="listSlotStyle">
        <MessageListPane
          :messages="state.messages.value"
          :groupedMessages="state.groupedMessages.value"
          :total="state.listTotal.value"
          :loading="state.listLoading.value"
          :error="state.listError.value"
          :hasMore="state.hasMore.value"
          :hasMoreRemote="state.currentFolderHasMoreRemote.value"
          :loadingOlder="state.loadingOlder.value"
          :searchQuery="state.searchQuery.value"
          :searchScope="state.searchScope.value"
          :sortMode="state.sortMode.value"
          :filterSet="state.filterSet.value"
          :selectedId="state.selectedId.value"
          :listWide="state.layout.listWide.value"
          :readerPosition="state.layout.prefs.value.readerPosition"
          :compactList="state.layout.prefs.value.compactList"
          :showAvatars="state.layout.prefs.value.showAvatars"
          :scrollResetToken="state.scrollResetToken.value"
          :syncStatus="state.syncStatus.value"
          :cacheReady="state.cacheReady.value"
          @selectMessage="state.selectMessage"
          @toggleSeen="state.toggleMessageSeen"
          @toggleFlagged="state.toggleMessageFlagged"
          @setSearch="state.setSearch"
          @setScope="state.setScope"
          @setSort="state.setSort"
          @setFilter="state.setFilter"
          @refresh="state.refresh"
          @loadMore="state.loadMore"
          @loadOlder="state.loadOlderMessages()"
          @overlayChange="state.layout.setOverlayOpen"
        />
      </div>

      <!-- Below 模式：列表下方阅读区 -->
      <template v-if="state.layout.prefs.value.readerPosition === 'below'">
        <PaneSplitter
          direction="horizontal"
          label="调整列表与阅读区高度"
          :value="state.layout.prefs.value.listBelowHeight"
          :min="BELOW_LIST_MIN_HEIGHT"
          @dragStart="dragBaseBelow = state.layout.prefs.value.listBelowHeight"
          @drag="(delta) => state.layout.onBelowResize(dragBaseBelow + delta)"
          @dragEnd="state.layout.flushSave()"
          @step="(delta) => state.layout.onBelowResize(state.layout.prefs.value.listBelowHeight + delta)"
        />
        <ReaderPane
          :isBelow="true"
          :message="state.openedMessage.value"
          :loading="state.readerLoading.value"
          :error="state.readerError.value"
          :selectedUnread="state.selectedUnread.value"
          :readerPosition="state.layout.prefs.value.readerPosition"
          :themeId="state.layout.prefs.value.themeId"
          :darkMode="state.layout.prefs.value.darkMode"
          :effectiveDark="state.layout.effectiveDark.value"
          :followSystem="state.layout.followSystemScheme.value"
          :overlayOccluded="state.layout.overlayOccluded.value"
          :zoomPercent="state.layout.prefs.value.readerZoomPercent"
          :folders="state.folders.value"
          :allowingRemoteImages="state.allowingRemoteImages.value"
          :movingMessage="state.movingMessage.value"
          @retry="state.retryOpen"
          @reply="openReplyCompose('reply')"
          @replyAll="openReplyCompose('replyAll')"
          @forward="openReplyCompose('forward')"
          @toggleSeen="(seen) => state.selectedId.value && state.toggleMessageSeen(state.selectedId.value, seen)"
          @toggleFlagged="(flagged) => state.selectedId.value && state.toggleMessageFlagged(state.selectedId.value, flagged)"
          @setReaderPosition="state.layout.setReaderPosition"
          @setTheme="state.layout.setTheme"
          @setDarkMode="state.layout.setDarkMode"
          @toggleDarkMode="state.layout.toggleDarkMode"
          @toggleFollowSystem="state.layout.toggleFollowSystem"
          @allowRemoteImages="state.allowOpenedRemoteImages"
          @archive="state.archiveSelectedMessage()"
          @markSpam="state.markSelectedSpam()"
          @delete="state.deleteSelectedMessage()"
          @moveToFolder="state.moveSelectedMessage"
          @openStandalone="openStandaloneReader('read')"
          @print="openStandaloneReader('print')"
          @openAttachments="attachmentDrawerOpen = true"
          @overlayChange="state.layout.setOverlayOpen"
        />
      </template>
    </div>

    <!-- Right 模式：右侧阅读区 -->
    <template v-if="state.layout.prefs.value.readerPosition === 'right'">
      <PaneSplitter
        direction="vertical"
        label="调整列表栏宽度"
        :value="state.layout.prefs.value.listWidth"
        :min="LIST_MIN_WIDTH"
        @dragStart="dragBaseList = state.layout.prefs.value.listWidth"
        @drag="(delta) => state.layout.onListResize(dragBaseList + delta)"
        @dragEnd="state.layout.flushSave()"
        @step="(delta) => state.layout.onListResize(state.layout.prefs.value.listWidth + delta)"
      />
      <ReaderPane
        class="km-reader-column"
        :message="state.openedMessage.value"
        :loading="state.readerLoading.value"
        :error="state.readerError.value"
        :selectedUnread="state.selectedUnread.value"
        :readerPosition="state.layout.prefs.value.readerPosition"
        :themeId="state.layout.prefs.value.themeId"
        :darkMode="state.layout.prefs.value.darkMode"
        :effectiveDark="state.layout.effectiveDark.value"
        :followSystem="state.layout.followSystemScheme.value"
        :overlayOccluded="state.layout.overlayOccluded.value"
        :zoomPercent="state.layout.prefs.value.readerZoomPercent"
        :folders="state.folders.value"
        :allowingRemoteImages="state.allowingRemoteImages.value"
        :movingMessage="state.movingMessage.value"
        @retry="state.retryOpen"
        @reply="openReplyCompose('reply')"
        @replyAll="openReplyCompose('replyAll')"
        @forward="openReplyCompose('forward')"
        @toggleSeen="(seen) => state.selectedId.value && state.toggleMessageSeen(state.selectedId.value, seen)"
        @toggleFlagged="(flagged) => state.selectedId.value && state.toggleMessageFlagged(state.selectedId.value, flagged)"
        @setReaderPosition="state.layout.setReaderPosition"
        @setTheme="state.layout.setTheme"
        @setDarkMode="state.layout.setDarkMode"
        @toggleDarkMode="state.layout.toggleDarkMode"
        @toggleFollowSystem="state.layout.toggleFollowSystem"
        @allowRemoteImages="state.allowOpenedRemoteImages"
        @archive="state.archiveSelectedMessage()"
        @markSpam="state.markSelectedSpam()"
        @delete="state.deleteSelectedMessage()"
        @moveToFolder="state.moveSelectedMessage"
        @openStandalone="openStandaloneReader('read')"
        @print="openStandaloneReader('print')"
        @openAttachments="attachmentDrawerOpen = true"
        @overlayChange="state.layout.setOverlayOpen"
      />
    </template>

    <!-- 底部操作撤销 Toast -->
    <div
      v-if="state.undoToast.value"
      class="km-undo-toast"
      role="status"
      aria-live="polite"
    >
      <span class="km-undo-toast-text">{{ state.undoToast.value.label }}</span>
      <button
        type="button"
        class="km-undo-toast-btn"
        :disabled="state.undoPending.value"
        @click="state.undoLastMove()"
      >
        {{ state.undoPending.value ? "撤销中…" : "撤销" }}
      </button>
      <button
        type="button"
        class="km-undo-toast-close"
        title="关闭提示"
        aria-label="关闭提示"
        @click="state.dismissUndoToast()"
      >
        ✕
      </button>
    </div>

    <!-- Off 模式浮动入口：允许在隐藏阅读区时切回阅读位置或主题 -->
    <div v-if="state.layout.prefs.value.readerPosition === 'off'" class="km-off-bar">
      <button
        type="button"
        class="km-off-chip"
        title="恢复右侧阅读区 (Ctrl+Alt+1)"
        @click="state.layout.setReaderPosition('right')"
      >
        显示阅读窗格
      </button>
    </div>

    <!-- 写邮件弹窗 / 撤销发送浮层 / 账号管理与首次欢迎引导 -->
    <ComposePane
      :open="compose.open.value"
      :accounts="composeAccounts"
      :composeState="compose"
      @update:open="(v) => (compose.open.value = v)"
      @overlayChange="(open) => state.layout.setOverlayOpen('compose', open)"
    />

    <OutboxUndoToast
      :undoBanner="compose.undoBanner.value"
      :outboxItems="compose.outboxItems.value"
      :undoing="compose.undoing.value"
      :busyOutboxId="compose.busyOutboxId.value"
      @undo="compose.undoSend()"
      @dismissUndo="compose.dismissUndoBanner()"
      @retryItem="(accountId, outboxId) => compose.retryOutbox(accountId, outboxId)"
      @deleteItem="(accountId, outboxId) => compose.deleteOutbox(accountId, outboxId)"
      @overlayChange="(open) => state.layout.setOverlayOpen('outbox-panel', open)"
    />

    <AccountsModal
      :open="accountsModalOpen"
      @update:open="(v) => (accountsModalOpen = v)"
      @close="accountsModalOpen = false"
      @changed="onAccountsChanged"
      @overlayChange="state.layout.setOverlayOpen"
    />

    <WelcomeOverlay
      :open="showWelcome"
      :allowSkip="true"
      @completed="onWelcomeCompleted"
      @skip="welcomeSkipped = true"
      @overlayChange="state.layout.setOverlayOpen"
    />

    <!-- Wave 3：附件抽屉 / 全局附件图库 / 独立读信打印 / 通讯录 / 设置 / 命令面板与快捷键 -->
    <AttachmentDrawer
      :open="attachmentDrawerOpen"
      :attachments="state.openedMessage.value?.attachments ?? []"
      :subject="state.openedMessage.value?.subject"
      @update:open="(v) => (attachmentDrawerOpen = v)"
      @close="attachmentDrawerOpen = false"
      @overlayChange="state.layout.setOverlayOpen"
    />

    <AttachmentGalleryModal
      :open="attachmentGalleryOpen"
      @update:open="(v) => (attachmentGalleryOpen = v)"
      @close="attachmentGalleryOpen = false"
      @openMessage="onGalleryOpenMessage"
      @overlayChange="state.layout.setOverlayOpen"
    />

    <StandaloneMessageModal
      :open="standaloneOpen"
      :target="standaloneTarget"
      :initialMode="standaloneMode"
      @update:open="(v) => (standaloneOpen = v)"
      @close="standaloneOpen = false"
      @reply="openComposeFromStandalone('reply', $event)"
      @replyAll="openComposeFromStandalone('replyAll', $event)"
      @forward="openComposeFromStandalone('forward', $event)"
      @overlayChange="state.layout.setOverlayOpen"
    />

    <ContactsModal
      :open="contactsModalOpen"
      @update:open="(v) => (contactsModalOpen = v)"
      @close="contactsModalOpen = false"
      @composeTo="onComposeToContact"
      @overlayChange="state.layout.setOverlayOpen"
    />

    <PreferencesModal
      :open="preferencesModalOpen"
      :initialTab="preferencesInitialTab"
      @update:open="(v) => (preferencesModalOpen = v)"
      @close="preferencesModalOpen = false"
      @preferencesUpdated="onFullPreferencesUpdated"
      @passcodeChanged="onPasscodeChanged"
      @openAccounts="openAccountsFromPreferences"
      @overlayChange="state.layout.setOverlayOpen"
    />

    <CommandPaletteModal
      :open="commandPaletteOpen"
      :items="commandPaletteItems"
      @update:open="(v) => (commandPaletteOpen = v)"
      @close="commandPaletteOpen = false"
      @execute="executePaletteCommand"
      @overlayChange="state.layout.setOverlayOpen"
    />

    <ShortcutsHelpModal
      :open="shortcutsHelpOpen"
      @update:open="(v) => (shortcutsHelpOpen = v)"
      @close="shortcutsHelpOpen = false"
      @overlayChange="state.layout.setOverlayOpen"
    />
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import FolderSidebar from "./FolderSidebar.vue";
import MessageListPane from "./MessageListPane.vue";
import ReaderPane from "./ReaderPane.vue";
import PaneSplitter from "./PaneSplitter.vue";
import ComposePane from "../compose/ComposePane.vue";
import OutboxUndoToast from "../compose/OutboxUndoToast.vue";
import { useCompose } from "../compose/useCompose";
import type { ComposeAccountOption, ComposeMode } from "../compose/types";
import AccountsModal from "../accounts/AccountsModal.vue";
import WelcomeOverlay from "../accounts/WelcomeOverlay.vue";
import AttachmentDrawer from "../reader-tools/AttachmentDrawer.vue";
import AttachmentGalleryModal from "../reader-tools/AttachmentGalleryModal.vue";
import StandaloneMessageModal from "../reader-tools/StandaloneMessageModal.vue";
import type {
  StandaloneMessageTarget,
  StandaloneReaderMode,
} from "../reader-tools/types";
import ContactsModal from "../contacts/ContactsModal.vue";
import PreferencesModal from "../preferences/PreferencesModal.vue";
import { getFullPreferences } from "../preferences/ipc";
import type {
  FullPreferencesDto,
  PasscodeStatus,
  PreferenceTabId,
} from "../preferences/types";
import CommandPaletteModal from "../command-palette/CommandPaletteModal.vue";
import ShortcutsHelpModal from "../command-palette/ShortcutsHelpModal.vue";
import type { CommandPaletteItem } from "../command-palette/types";
import { matchSingleKeyShortcut } from "../command-palette/useCommandPalette";
import type { OpenMessageOutput } from "../ipc/types";
import { injectMainWindow } from "./useMainWindow";
import {
  BELOW_LIST_MIN_HEIGHT,
  LIST_MIN_WIDTH,
  SIDEBAR_MAX_WIDTH,
  SIDEBAR_MIN_WIDTH,
} from "./usePaneLayout";

const state = injectMainWindow();
const rootRef = ref<HTMLElement | null>(null);

const composeAccounts = computed<ComposeAccountOption[]>(() =>
  state.accounts.value.map((acc) => ({
    id: String(acc.id),
    displayName: acc.displayName,
    email: acc.email,
  }))
);

const compose = useCompose({ accounts: composeAccounts.value });
watch(
  composeAccounts,
  (next) => {
    compose.setAccounts(next);
  },
  { deep: true, immediate: true }
);

const accountsModalOpen = ref(false);
const welcomeSkipped = ref(false);
const showWelcome = computed(
  () =>
    !state.loading.value &&
    !state.locked.value &&
    state.mode.value === "live" &&
    state.accounts.value.length === 0 &&
    !welcomeSkipped.value
);

function openNewCompose() {
  const defaultAccountId =
    state.currentView.value.kind === "accountFolder"
      ? String(state.currentView.value.accountId)
      : composeAccounts.value[0]?.id ?? null;
  compose.openCompose({
    mode: "new",
    accountId: defaultAccountId,
  });
}

function openReplyCompose(mode: ComposeMode) {
  const msg = state.openedMessage.value;
  if (!msg) return;
  const rfcMessageId =
    msg.rfcMessageId ??
    msg.threadMessages?.[msg.threadMessages.length - 1]?.rfcMessageId ??
    null;
  compose.openCompose({
    mode,
    accountId: String(msg.accountId),
    originalMessage: {
      accountId: String(msg.accountId),
      sender: msg.sender,
      recipients: msg.recipients,
      cc: msg.cc,
      subject: msg.subject,
      dateDisplay: msg.dateDisplay,
      plainText: msg.plainText,
      htmlBody: msg.htmlBody,
      rfcMessageId,
    },
  });
}

function openAccountsModal() {
  accountsModalOpen.value = true;
}

async function onAccountsChanged() {
  await state.init();
}

async function onWelcomeCompleted() {
  welcomeSkipped.value = false;
  await state.init();
}

const attachmentDrawerOpen = ref(false);
const attachmentGalleryOpen = ref(false);
const standaloneOpen = ref(false);
const standaloneTarget = ref<StandaloneMessageTarget | null>(null);
const standaloneMode = ref<StandaloneReaderMode>("read");
const contactsModalOpen = ref(false);
const preferencesModalOpen = ref(false);
const preferencesInitialTab = ref<PreferenceTabId>("appearance");
const commandPaletteOpen = ref(false);
const shortcutsHelpOpen = ref(false);
const singleKeyShortcuts = ref(true);

const hasAnyOpenModal = computed(
  () =>
    compose.open.value ||
    accountsModalOpen.value ||
    showWelcome.value ||
    attachmentDrawerOpen.value ||
    attachmentGalleryOpen.value ||
    standaloneOpen.value ||
    contactsModalOpen.value ||
    preferencesModalOpen.value ||
    commandPaletteOpen.value ||
    shortcutsHelpOpen.value
);

const commandPaletteItems = computed<CommandPaletteItem[]>(() => {
  const hasSelection = Boolean(state.selectedId.value);
  const hasMessage = Boolean(state.openedMessage.value);
  const hasAttachments = Boolean(
    state.openedMessage.value && state.openedMessage.value.attachments.length > 0
  );
  return [
    {
      id: "compose.new",
      title: "写新邮件",
      category: "写信",
      shortcut: "c / Ctrl+N",
      keywords: ["compose", "new", "write", "xie"],
    },
    {
      id: "message.reply",
      title: "回复当前邮件",
      category: "邮件操作",
      shortcut: "r",
      disabled: !hasMessage,
    },
    {
      id: "message.replyAll",
      title: "全部回复当前邮件",
      category: "邮件操作",
      shortcut: "a",
      disabled: !hasMessage,
    },
    {
      id: "message.forward",
      title: "转发当前邮件",
      category: "邮件操作",
      shortcut: "f",
      disabled: !hasMessage,
    },
    {
      id: "message.archive",
      title: "归档当前邮件",
      category: "邮件操作",
      shortcut: "e",
      disabled: !hasSelection,
    },
    {
      id: "message.toggleStar",
      title: "切换星标状态",
      category: "邮件操作",
      shortcut: "s",
      disabled: !hasSelection,
    },
    {
      id: "message.toggleRead",
      title: "切换已读 / 未读",
      category: "邮件操作",
      shortcut: "u",
      disabled: !hasSelection,
    },
    {
      id: "message.delete",
      title: "删除当前邮件",
      category: "邮件操作",
      shortcut: "# / Delete",
      disabled: !hasSelection,
    },
    {
      id: "reader.standalone",
      title: "独立窗口阅读当前邮件",
      category: "阅读",
      disabled: !hasMessage,
    },
    {
      id: "reader.print",
      title: "打印预览 / 打印当前邮件",
      category: "阅读",
      disabled: !hasMessage,
    },
    {
      id: "reader.attachments",
      title: "打开当前邮件附件抽屉",
      category: "阅读",
      disabled: !hasAttachments,
    },
    {
      id: "tools.attachments",
      title: "打开全局附件图库",
      category: "工作区工具",
      keywords: ["gallery", "attachments", "images"],
    },
    {
      id: "tools.contacts",
      title: "打开通讯录管理",
      category: "工作区工具",
      keywords: ["contacts", "vcard", "csv"],
    },
    {
      id: "tools.accounts",
      title: "打开邮箱账号管理",
      category: "工作区工具",
      keywords: ["accounts", "imap", "smtp", "oauth"],
    },
    {
      id: "tools.preferences",
      title: "打开完整设置与偏好",
      category: "工作区工具",
      keywords: ["settings", "preferences", "lock", "privacy"],
    },
    {
      id: "search.focus",
      title: "聚焦邮件搜索框",
      category: "导航",
      shortcut: "/ / Ctrl+F",
    },
    {
      id: "mail.refresh",
      title: "刷新同步邮件",
      category: "导航",
      shortcut: "F5",
    },
    {
      id: "layout.right",
      title: "右侧阅读布局",
      category: "布局与外观",
      shortcut: "Ctrl+Alt+1",
    },
    {
      id: "layout.below",
      title: "下方阅读布局",
      category: "布局与外观",
      shortcut: "Ctrl+Alt+2",
    },
    {
      id: "layout.off",
      title: "隐藏阅读窗格",
      category: "布局与外观",
      shortcut: "Ctrl+Alt+3",
    },
    {
      id: "layout.toggleDark",
      title: "切换亮暗主题模式",
      category: "布局与外观",
    },
    {
      id: "help.shortcuts",
      title: "查看键盘快捷键速查表",
      category: "帮助",
      shortcut: "?",
    },
    {
      id: "app.lock",
      title: "立即锁定 KiteMail",
      category: "安全",
      disabled: !state.hasPasscode.value,
    },
  ];
});

function openPreferencesModal(tab: PreferenceTabId = "appearance") {
  preferencesInitialTab.value = tab;
  preferencesModalOpen.value = true;
}

function openAccountsFromPreferences() {
  preferencesModalOpen.value = false;
  accountsModalOpen.value = true;
}

function openStandaloneReader(mode: StandaloneReaderMode) {
  const msg = state.openedMessage.value;
  if (!msg) return;
  standaloneTarget.value = {
    accountId: msg.accountId,
    messageId: msg.messageId,
  };
  standaloneMode.value = mode;
  standaloneOpen.value = true;
}

function openComposeFromStandalone(mode: ComposeMode, msg: OpenMessageOutput) {
  standaloneOpen.value = false;
  compose.openCompose({
    mode,
    accountId: String(msg.accountId),
    originalMessage: {
      accountId: String(msg.accountId),
      sender: msg.sender,
      recipients: msg.recipients,
      cc: msg.cc,
      subject: msg.subject,
      dateDisplay: msg.dateDisplay,
      plainText: msg.plainText,
      htmlBody: msg.htmlBody,
      rfcMessageId: msg.rfcMessageId ?? null,
    },
  });
}

function onGalleryOpenMessage(accountId: number, messageId: number) {
  attachmentGalleryOpen.value = false;
  standaloneTarget.value = { accountId, messageId };
  standaloneMode.value = "read";
  standaloneOpen.value = true;
}

function onComposeToContact(email: string, name: string) {
  contactsModalOpen.value = false;
  const formatted = name && name.trim() ? `${name.trim()} <${email}>` : email;
  const defaultAccountId =
    state.currentView.value.kind === "accountFolder"
      ? String(state.currentView.value.accountId)
      : composeAccounts.value[0]?.id ?? null;
  compose.openCompose({
    mode: "new",
    accountId: defaultAccountId,
    to: formatted,
  });
}

function onFullPreferencesUpdated(prefs: FullPreferencesDto) {
  singleKeyShortcuts.value = prefs.singleKeyShortcuts;
  state.layout.applyPrefs({
    ...state.layout.prefs.value,
    themeId: prefs.themeId,
    darkMode: prefs.darkMode,
    readerPosition: prefs.readerPosition,
    compactList: prefs.compactList,
    showAvatars: prefs.showAvatars,
    listGroupByDate: prefs.listGroupByDate,
    readerZoomPercent: prefs.readerZoomPercent,
  });
}

function onPasscodeChanged(status: PasscodeStatus) {
  state.hasPasscode.value = status.hasPasscode;
}

function executePaletteCommand(commandId: string) {
  switch (commandId) {
    case "compose.new":
      openNewCompose();
      break;
    case "message.reply":
      openReplyCompose("reply");
      break;
    case "message.replyAll":
      openReplyCompose("replyAll");
      break;
    case "message.forward":
      openReplyCompose("forward");
      break;
    case "message.archive":
      if (state.selectedId.value) void state.archiveSelectedMessage();
      break;
    case "message.toggleStar":
      if (state.selectedId.value) {
        void state.toggleMessageFlagged(
          state.selectedId.value,
          !state.selectedMessage.value?.flagged
        );
      }
      break;
    case "message.toggleRead":
      if (state.selectedId.value) {
        void state.toggleMessageSeen(
          state.selectedId.value,
          Boolean(state.selectedUnread.value)
        );
      }
      break;
    case "message.next":
      state.selectNext();
      break;
    case "message.prev":
      state.selectPrev();
      break;
    case "message.delete":
      if (state.selectedId.value) void state.deleteSelectedMessage();
      break;
    case "reader.standalone":
      openStandaloneReader("read");
      break;
    case "reader.print":
      openStandaloneReader("print");
      break;
    case "reader.attachments":
      if (state.openedMessage.value?.attachments.length) {
        attachmentDrawerOpen.value = true;
      }
      break;
    case "tools.attachments":
      attachmentGalleryOpen.value = true;
      break;
    case "tools.contacts":
      contactsModalOpen.value = true;
      break;
    case "tools.accounts":
      openAccountsModal();
      break;
    case "tools.preferences":
      openPreferencesModal("appearance");
      break;
    case "search.focus":
      window.dispatchEvent(new CustomEvent("km-focus-search"));
      break;
    case "mail.refresh":
      void state.refresh();
      break;
    case "layout.right":
      state.layout.setReaderPosition("right");
      break;
    case "layout.below":
      state.layout.setReaderPosition("below");
      break;
    case "layout.off":
      state.layout.setReaderPosition("off");
      break;
    case "layout.toggleDark":
      state.layout.toggleDarkMode();
      break;
    case "help.shortcuts":
      shortcutsHelpOpen.value = true;
      break;
    case "app.lock":
      if (state.hasPasscode.value) void state.lock();
      break;
  }
}

const dragBaseSidebar = ref(state.layout.prefs.value.sidebarWidth);
const dragBaseList = ref(state.layout.prefs.value.listWidth);
const dragBaseBelow = ref(state.layout.prefs.value.listBelowHeight);

let ro: ResizeObserver | null = null;

onMounted(() => {
  if (rootRef.value) {
    const rect = rootRef.value.getBoundingClientRect();
    if (rect.width > 0) {
      state.layout.setWindowSize(rect.width, rect.height);
    }
    ro = new ResizeObserver((entries) => {
      const cr = entries[0].contentRect;
      state.layout.setWindowSize(cr.width, cr.height);
    });
    ro.observe(rootRef.value);
  }
  window.addEventListener("keydown", onKeyDown);
  void getFullPreferences()
    .then((res) => {
      singleKeyShortcuts.value = res.preferences.singleKeyShortcuts;
    })
    .catch(() => {
      /* 锁屏或预览初始阶段忽略 */
    });
});

onUnmounted(() => {
  ro?.disconnect();
  window.removeEventListener("keydown", onKeyDown);
  state.layout.flushSave();
});

const listColumnStyle = computed(() => {
  const prefs = state.layout.prefs.value;
  if (prefs.readerPosition === "right") {
    return {
      width: `${prefs.listWidth}px`,
      minWidth: `${LIST_MIN_WIDTH}px`,
      flexShrink: 0,
    };
  }
  return {
    flex: "1 1 auto",
    minWidth: `${LIST_MIN_WIDTH}px`,
  };
});

const listSlotStyle = computed(() => {
  const prefs = state.layout.prefs.value;
  if (prefs.readerPosition === "below") {
    return {
      height: `${prefs.listBelowHeight}px`,
      minHeight: `${BELOW_LIST_MIN_HEIGHT}px`,
      flexShrink: 0,
      display: "flex",
      flexDirection: "column" as const,
      overflow: "hidden",
    };
  }
  return {
    flex: "1 1 auto",
    minHeight: "0",
    display: "flex",
    flexDirection: "column" as const,
    overflow: "hidden",
  };
});

const layoutClass = computed(() => ({
  "km-layout-right": state.layout.prefs.value.readerPosition === "right",
  "km-layout-below": state.layout.prefs.value.readerPosition === "below",
  "km-layout-off": state.layout.prefs.value.readerPosition === "off",
  "km-rail": state.layout.isRail.value,
}));

function isEditableFocused(): boolean {
  const el = document.activeElement as HTMLElement | null;
  if (!el) return false;
  if (el.isContentEditable) return true;
  return el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT";
}

function onKeyDown(e: KeyboardEvent) {
  // 中文 IME 组合输入期间保持编辑键优先
  if (e.isComposing || e.key === "Process" || e.keyCode === 229) return;

  // Ctrl+F: 聚焦搜索框
  if (e.ctrlKey && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "f") {
    e.preventDefault();
    window.dispatchEvent(new CustomEvent("km-focus-search"));
    return;
  }

  // Ctrl+N: 写新邮件（输入聚焦或模态框已打开时不拦截）
  if (
    (e.ctrlKey || e.metaKey) &&
    !e.altKey &&
    !e.shiftKey &&
    e.key.toLowerCase() === "n" &&
    !isEditableFocused() &&
    !hasAnyOpenModal.value
  ) {
    e.preventDefault();
    openNewCompose();
    return;
  }

  // Ctrl+K 命令面板 & 单键快捷键（严格尊重 IME / 输入焦 / 模态弹窗与设置开关）
  const shortcutMatch = matchSingleKeyShortcut(e, {
    singleKeyShortcuts: singleKeyShortcuts.value,
    hasOpenModal: hasAnyOpenModal.value,
    isInputFocused: isEditableFocused(),
  });
  if (shortcutMatch.matched && shortcutMatch.commandId) {
    if (shortcutMatch.preventDefault) e.preventDefault();
    if (shortcutMatch.isPaletteToggle) {
      commandPaletteOpen.value = !commandPaletteOpen.value;
      return;
    }
    executePaletteCommand(shortcutMatch.commandId);
    return;
  }

  // F5: 刷新
  if (!e.ctrlKey && !e.altKey && e.key === "F5") {
    e.preventDefault();
    void state.refresh();
    return;
  }

  // Ctrl+Alt+1/2/3: 切换阅读位置 right/below/off
  if (e.ctrlKey && e.altKey) {
    if (e.key === "1" || e.code === "Digit1") {
      e.preventDefault();
      state.layout.setReaderPosition("right");
      return;
    }
    if (e.key === "2" || e.code === "Digit2") {
      e.preventDefault();
      state.layout.setReaderPosition("below");
      return;
    }
    if (e.key === "3" || e.code === "Digit3") {
      e.preventDefault();
      state.layout.setReaderPosition("off");
      return;
    }
  }

  // 上/下键选邮件 & Delete 删除选中邮件（输入控件聚焦时不拦截）
  if (isEditableFocused()) return;
  if (!e.ctrlKey && !e.altKey && !e.shiftKey) {
    if (e.key === "ArrowUp") {
      e.preventDefault();
      state.selectPrev();
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      state.selectNext();
      return;
    }
    if (e.key === "Delete") {
      if (state.selectedId.value) {
        e.preventDefault();
        void state.deleteSelectedMessage();
      }
      return;
    }
  }
}
</script>

<style scoped>
.km-main-window {
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: row;
  overflow: hidden;
  background: var(--km-canvas);
  position: relative;
}

.km-list-column {
  display: flex;
  flex-direction: column;
  overflow: hidden;
  min-width: 0;
  height: 100%;
}

.km-reader-column {
  flex: 1 1 auto;
  min-width: min(441px, 100%);
  overflow: hidden;
}

.km-off-bar {
  position: absolute;
  right: 14px;
  bottom: 14px;
  z-index: 15;
}

.km-off-chip {
  padding: 6px 12px;
  border-radius: 999px;
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  box-shadow: var(--km-shadow-card);
  font-size: 12px;
  cursor: pointer;
}

.km-off-chip:hover {
  border-color: var(--km-accent);
  color: var(--km-accent);
}

.km-undo-toast {
  position: absolute;
  left: 16px;
  bottom: 16px;
  z-index: 50;
  max-width: calc(100% - 32px);
  display: inline-flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px;
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  color: var(--km-text);
  box-shadow: var(--km-shadow-popover);
  font-size: 12px;
}

.km-undo-toast-text {
  font-weight: 500;
}

.km-undo-toast-btn {
  border: none;
  border-radius: var(--km-radius-sm);
  padding: 4px 10px;
  background: var(--km-accent);
  color: var(--km-accent-fg);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
}

.km-undo-toast-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.km-undo-toast-close {
  border: none;
  background: transparent;
  color: var(--km-text-muted);
  cursor: pointer;
  padding: 2px 4px;
  font-size: 11px;
}

.km-undo-toast-close:hover {
  color: var(--km-text);
}
</style>

<template>
  <aside
    class="km-sidebar"
    :class="{ rail: isRail }"
    :style="{ width: `${width}px`, minWidth: `${width}px` }"
    aria-label="邮件文件夹导航"
  >
    <!-- 顶部栏 48px -->
    <header class="km-sidebar-header">
      <button
        type="button"
        class="km-icon-btn"
        :title="isRail ? '展开侧栏' : '折叠侧栏'"
        :aria-label="isRail ? '展开侧栏' : '折叠侧栏'"
        @click="emit('toggleCollapse')"
      >
        <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
          <path
            fill="currentColor"
            d="M2.5 3.5A1.5 1.5 0 0 1 4 2h8a1.5 1.5 0 0 1 1.5 1.5v9A1.5 1.5 0 0 1 12 14H4a1.5 1.5 0 0 1-1.5-1.5v-9ZM6 3v10h6a.5.5 0 0 0 .5-.5v-9A.5.5 0 0 0 12 3H6Z"
          />
        </svg>
      </button>

      <span v-if="!isRail" class="km-brand-title">KiteMail</span>

      <button
        v-if="!isRail"
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
    </header>

    <!-- 滚动导航区 -->
    <div class="km-sidebar-scroll">
      <!-- 统一视图 -->
      <nav class="km-nav-section" aria-label="统一视图">
        <button
          v-for="item in unifiedItems"
          :key="item.kind"
          type="button"
          class="km-nav-row"
          :class="{ active: isUnifiedActive(item.kind), 'rail-row': isRail }"
          :title="item.label"
          @click="emit('selectView', { kind: item.kind })"
        >
          <span class="km-nav-icon" aria-hidden="true">{{ item.icon }}</span>
          <span v-if="!isRail" class="km-nav-label">{{ item.label }}</span>
          <span
            v-if="!isRail && item.badge > 0"
            class="km-count-pill"
            :class="{ active: isUnifiedActive(item.kind) }"
          >
            {{ formatBadge(item.badge) }}
          </span>
        </button>
      </nav>

      <!-- 账号文件夹树 -->
      <div v-if="!isRail" class="km-accounts-section">
        <div
          v-for="acc in accounts"
          :key="acc.id"
          class="km-account-group"
        >
          <button
            type="button"
            class="km-account-header"
            :aria-expanded="!collapsedAccounts.has(acc.id)"
            @click="toggleAccount(acc.id)"
          >
            <span class="km-account-avatar">{{ avatarInitial(acc.displayName || acc.email) }}</span>
            <span class="km-account-meta">
              <span class="km-account-name">{{ acc.displayName }}</span>
              <span class="km-account-email">{{ acc.email }}</span>
            </span>
            <span class="km-chevron" :class="{ collapsed: collapsedAccounts.has(acc.id) }">▾</span>
          </button>

          <div v-if="!collapsedAccounts.has(acc.id)" class="km-folder-list">
            <button
              v-for="folder in foldersForAccount(acc.id)"
              :key="folder.id"
              type="button"
              class="km-nav-row km-folder-row"
              :class="{ active: isFolderActive(acc.id, folder.id) }"
              @click="emit('selectView', { kind: 'accountFolder', accountId: acc.id, folderId: folder.id })"
            >
              <span class="km-nav-icon" aria-hidden="true">{{ folderIcon(folder.path, folder.name) }}</span>
              <span class="km-nav-label">{{ folder.name }}</span>
              <span
                v-if="folder.unreadCount > 0"
                class="km-count-pill"
                :class="{ active: isFolderActive(acc.id, folder.id) }"
              >
                {{ formatBadge(folder.unreadCount) }}
              </span>
            </button>
          </div>
        </div>
      </div>

      <!-- 窄轨模式下展示收件箱与首账号快捷文件夹图标 -->
      <div v-else class="km-rail-folders">
        <button
          v-for="folder in railFolders"
          :key="`${folder.accountId}:${folder.id}`"
          type="button"
          class="km-nav-row rail-row"
          :class="{ active: isFolderActive(folder.accountId, folder.id) }"
          :title="folder.name"
          @click="emit('selectView', { kind: 'accountFolder', accountId: folder.accountId, folderId: folder.id })"
        >
          <span class="km-nav-icon" aria-hidden="true">{{ folderIcon(folder.path, folder.name) }}</span>
        </button>
      </div>
    </div>

    <!-- 底部状态 -->
    <footer class="km-sidebar-footer">
      <span v-if="!isRail" class="km-mode-badge">
        {{ mode === "demo" ? "演示模式 (--demo)" : "本地工作区" }}
      </span>
      <span v-else class="km-mode-dot" :title="mode === 'demo' ? '演示模式' : '本地工作区'" />
    </footer>
  </aside>
</template>

<script setup lang="ts">
import { computed, ref } from "vue";
import type {
  AccountSummaryDto,
  FolderSummaryDto,
  UnifiedViewKind,
  ViewDto,
} from "../ipc/types";

const props = defineProps<{
  accounts: AccountSummaryDto[];
  folders: FolderSummaryDto[];
  currentView: ViewDto;
  isRail: boolean;
  width: number;
  syncing?: boolean;
  mode?: "demo" | "live";
}>();

const emit = defineEmits<{
  selectView: [view: ViewDto];
  toggleCollapse: [];
  refresh: [];
}>();

const collapsedAccounts = ref<Set<number>>(new Set());

function toggleAccount(accountId: number) {
  const next = new Set(collapsedAccounts.value);
  if (next.has(accountId)) {
    next.delete(accountId);
  } else {
    next.add(accountId);
  }
  collapsedAccounts.value = next;
}

const totalInboxUnread = computed(() =>
  props.folders
    .filter((f) => f.path.toUpperCase() === "INBOX" || f.name.includes("收件箱"))
    .reduce((sum, f) => sum + f.unreadCount, 0)
);

const unifiedItems = computed<
  Array<{ kind: UnifiedViewKind; label: string; icon: string; badge: number }>
>(() => [
  { kind: "unifiedInbox", label: "收件箱", icon: "📥", badge: totalInboxUnread.value },
  { kind: "unread", label: "未读", icon: "🔵", badge: 0 },
  { kind: "starred", label: "已加星标", icon: "★", badge: 0 },
  { kind: "sent", label: "已发送", icon: "📤", badge: 0 },
  { kind: "drafts", label: "草稿箱", icon: "📝", badge: 0 },
  { kind: "archive", label: "归档", icon: "🗄", badge: 0 },
  { kind: "junk", label: "垃圾邮件", icon: "⚠", badge: 0 },
  { kind: "trash", label: "已删除", icon: "🗑", badge: 0 },
]);

const railFolders = computed(() => props.folders.slice(0, 6));

function foldersForAccount(accountId: number): FolderSummaryDto[] {
  return props.folders.filter((f) => f.accountId === accountId);
}

function isUnifiedActive(kind: UnifiedViewKind): boolean {
  return props.currentView.kind === kind;
}

function isFolderActive(accountId: number, folderId: number): boolean {
  return (
    props.currentView.kind === "accountFolder" &&
    props.currentView.accountId === accountId &&
    props.currentView.folderId === folderId
  );
}

function folderIcon(path: string, name: string): string {
  const p = path.toUpperCase();
  if (p === "INBOX" || name.includes("收件箱")) return "📥";
  if (p.includes("SENT") || name.includes("已发送")) return "📤";
  if (p.includes("DRAFT") || name.includes("草稿")) return "📝";
  if (p.includes("ARCHIVE") || name.includes("归档")) return "🗄";
  if (p.includes("TRASH") || p.includes("DELETED") || name.includes("已删除")) return "🗑";
  if (p.includes("JUNK") || p.includes("SPAM") || name.includes("垃圾")) return "⚠";
  return "📁";
}

function formatBadge(n: number): string {
  return n > 999 ? "999+" : String(n);
}

function avatarInitial(name: string): string {
  const ch = name.trim().charAt(0);
  return ch ? ch.toUpperCase() : "K";
}
</script>

<style scoped>
.km-sidebar {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--km-sidebar);
  color: var(--km-text);
  flex-shrink: 0;
  overflow: hidden;
  transition: width 120ms ease;
}

.km-sidebar-header {
  height: 48px;
  min-height: 48px;
  padding: 0 10px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  border-bottom: 1px solid var(--km-border);
}

.km-sidebar.rail .km-sidebar-header {
  justify-content: center;
  padding: 0 6px;
}

.km-brand-title {
  font-size: 14px;
  font-weight: 700;
  letter-spacing: 0.01em;
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

.km-sidebar-scroll {
  flex: 1 1 auto;
  overflow-y: auto;
  padding: 8px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.km-sidebar.rail .km-sidebar-scroll {
  padding: 8px 6px;
  align-items: center;
}

.km-nav-section,
.km-folder-list,
.km-rail-folders {
  display: flex;
  flex-direction: column;
  gap: 2px;
  width: 100%;
}

.km-nav-row {
  height: 34px;
  padding: 0 10px;
  border: none;
  border-radius: var(--km-radius-md);
  background: transparent;
  color: var(--km-text);
  display: flex;
  align-items: center;
  gap: 8px;
  text-align: left;
  cursor: pointer;
  width: 100%;
}

.km-nav-row.rail-row {
  justify-content: center;
  padding: 0;
  width: 42px;
  height: 36px;
  margin: 0 auto;
}

.km-nav-row:hover {
  background: var(--km-hover-bg);
}

.km-nav-row.active {
  background: var(--km-accent-soft);
  color: var(--km-accent);
  font-weight: 600;
}

.km-nav-icon {
  width: 18px;
  text-align: center;
  font-size: 13px;
  flex-shrink: 0;
}

.km-nav-label {
  flex: 1 1 auto;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-count-pill {
  min-width: 20px;
  height: 18px;
  padding: 0 6px;
  border-radius: 999px;
  background: var(--km-accent);
  color: var(--km-accent-fg);
  font-size: 11px;
  font-weight: 700;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.km-accounts-section {
  display: flex;
  flex-direction: column;
  gap: 8px;
  border-top: 1px solid var(--km-border);
  padding-top: 10px;
}

.km-account-group {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.km-account-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  border: none;
  border-radius: var(--km-radius-md);
  background: transparent;
  cursor: pointer;
  text-align: left;
  width: 100%;
}

.km-account-header:hover {
  background: var(--km-hover-bg);
}

.km-account-avatar {
  width: 24px;
  height: 24px;
  border-radius: 999px;
  background: var(--km-accent);
  color: var(--km-accent-fg);
  font-size: 11px;
  font-weight: 700;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.km-account-meta {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.km-account-name {
  font-size: 12px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-account-email {
  font-size: 11px;
  color: var(--km-text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-chevron {
  font-size: 11px;
  color: var(--km-text-muted);
  transition: transform 120ms ease;
}

.km-chevron.collapsed {
  transform: rotate(-90deg);
}

.km-folder-row {
  padding-left: 14px;
}

.km-sidebar-footer {
  height: 32px;
  padding: 0 12px;
  border-top: 1px solid var(--km-border);
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-mode-badge {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-mode-dot {
  width: 8px;
  height: 8px;
  border-radius: 999px;
  background: var(--km-accent);
}
</style>

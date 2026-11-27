/**
 * KiteMail 主窗口 Tauri IPC 契约类型定义。
 * 所有枚举值与结构体字段严格使用 camelCase，与 `src-tauri/src/dto.rs` 保持 100% 一致。
 */

export type ThemeId =
  | "system"
  | "default"
  | "midnight"
  | "tidal"
  | "rose"
  | "earth"
  | "forest";

export interface ThemeMeta {
  id: ThemeId;
  label: string;
  swatch: string;
}

export const BUILTIN_THEMES: readonly ThemeMeta[] = [
  { id: "system", label: "系统", swatch: "#3584e4" },
  { id: "default", label: "经典蓝", swatch: "#3584e4" },
  { id: "midnight", label: "子夜", swatch: "#7253b9" },
  { id: "tidal", label: "海潮", swatch: "#2672af" },
  { id: "rose", label: "玫瑰", swatch: "#b75077" },
  { id: "earth", label: "大地", swatch: "#ae552a" },
  { id: "forest", label: "森林", swatch: "#1b7d50" },
] as const;

export type ReaderPosition = "right" | "below" | "off";

export type UnifiedViewKind =
  | "unifiedInbox"
  | "unread"
  | "starred"
  | "sent"
  | "drafts"
  | "archive"
  | "junk"
  | "trash"
  | "snoozed";

export type ViewDto =
  | { kind: UnifiedViewKind }
  | { kind: "accountFolder"; accountId: number; folderId: number };

export interface AccountSummaryDto {
  id: number;
  displayName: string;
  email: string;
}

export interface FolderSummaryDto {
  id: number;
  accountId: number;
  name: string;
  path: string;
  unreadCount: number;
  totalCount: number;
  hasMoreRemote: boolean;
}

export interface MainPreferencesDto {
  themeId: ThemeId | string;
  darkMode: boolean;
  readerPosition: ReaderPosition;
  sidebarWidth: number;
  listWidth: number;
  listBelowHeight: number;
  sidebarCollapsed: boolean;
  compactList: boolean;
  showAvatars: boolean;
  listGroupByDate: boolean;
  readerZoomPercent: number;
}

export type MainPreferencesPatch = Partial<MainPreferencesDto>;

export interface SyncStatusDto {
  syncing: boolean;
  message: string | null;
  lastSyncAt: number | null;
}

export interface MainBootstrapDto {
  mode: "demo" | "live";
  locked: boolean;
  hasPasscode?: boolean;
  cacheReady?: boolean;
  stateVersion: number;
  accounts: AccountSummaryDto[];
  folders: FolderSummaryDto[];
  currentView: ViewDto;
  preferences: MainPreferencesDto;
  syncStatus: SyncStatusDto;
}

export interface UnlockAppInput {
  passcode: string;
}

export interface UnlockAppOutput {
  unlocked: boolean;
  locked: boolean;
  error?: string | null;
  retryAfterSecs?: number | null;
}

export interface LockAppOutput {
  locked: boolean;
}

export interface SetMessageSeenInput {
  accountId: number;
  messageId: number;
  seen: boolean;
}

export interface SetMessageSeenOutput {
  accountId: number;
  messageId: number;
  unread: boolean;
  stateVersion: number;
}

export interface SetMessageFlaggedInput {
  accountId: number;
  messageId: number;
  flagged: boolean;
}

export interface SetMessageFlaggedOutput {
  accountId: number;
  messageId: number;
  flagged: boolean;
  stateVersion: number;
}

export type SearchScope = "thisFolder" | "allFolders";

export type MessageSort =
  | "dateNewest"
  | "dateOldest"
  | "senderAsc"
  | "senderDesc"
  | "subjectAsc"
  | "subjectDesc"
  | "unreadFirst"
  | "starredFirst"
  | "attachmentFirst"
  | "importanceFirst";

export interface MessageFilterDto {
  unread?: boolean;
  starred?: boolean;
  hasAttachment?: boolean;
  today?: boolean;
}

export interface ListMessagesInput {
  requestId: string;
  view: ViewDto;
  query: string;
  scope: SearchScope;
  sort: MessageSort;
  filter: MessageFilterDto;
  offset: number;
  limit: number;
}

export interface MessageSummaryDto {
  id: number;
  accountId: number;
  folderId: number;
  folderPath: string;
  sender: string;
  recipients: string;
  subject: string;
  preview: string;
  dateTs: number;
  dateDisplay: string;
  unread: boolean;
  flagged: boolean;
  hasAttachment: boolean;
  threadCount: number;
  importance: number;
  tags: string[];
  answered: boolean;
  forwarded: boolean;
}

export interface ListMessagesOutput {
  requestId: string;
  stateVersion: number;
  offset: number;
  total: number;
  hasMore: boolean;
  items: MessageSummaryDto[];
}

export interface OpenMessageInput {
  requestId: string;
  accountId: number;
  messageId: number;
}

export type BodyStatus = "loading" | "ready" | "unavailable";

export interface AttachmentDto {
  id?: string | number;
  filename: string;
  mimeType: string;
  sizeBytes: number;
}

export interface ThreadMessageDto {
  id: number;
  accountId: number;
  sender: string;
  recipients: string;
  cc: string;
  dateDisplay: string;
  preview: string;
  plainText: string | null;
  hasHtml: boolean;
  htmlBody: string | null;
  unread: boolean;
  flagged: boolean;
}

export interface OpenMessageOutput {
  requestId: string;
  readerSessionId: string;
  accountId: number;
  messageId: number;
  subject: string;
  sender: string;
  recipients: string;
  cc: string;
  dateDisplay: string;
  bodyStatus: BodyStatus;
  plainText: string | null;
  hasHtml: boolean;
  htmlBody: string | null;
  flagged: boolean;
  attachments: AttachmentDto[];
  threadMessages: ThreadMessageDto[];
}

export interface RectDto {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface SetReaderLayoutInput {
  readerSessionId: string;
  layoutRevision: number;
  contentRect: RectDto;
  clipRect: RectDto;
  visible: boolean;
  devicePixelRatio?: number;
}

export interface SetReaderLayoutOutput {
  applied: boolean;
  readerSessionId: string;
  layoutRevision: number;
}

export interface RefreshMailInput {
  view: ViewDto;
}

export interface RefreshMailOutput {
  accepted: boolean;
  stateVersion: number;
}

export interface IpcError {
  code: string;
  message: string;
  retryable: boolean;
  requestId?: string | null;
}

export interface MailChangedEvent {
  stateVersion: number;
  view?: ViewDto | null;
  reason?: string | null;
  accountId?: number | null;
  folderId?: number | null;
  messageId?: number | null;
}

export type SyncStatusEvent =
  | (SyncStatusDto & { stateVersion?: number })
  | { stateVersion?: number; syncStatus: SyncStatusDto };

export interface ReaderStatusEvent {
  readerSessionId: string;
  accountId?: number;
  messageId?: number;
  bodyStatus: BodyStatus;
  detail?: string | null;
  message?: string | null;
  retryable?: boolean;
  plainText?: string | null;
  hasHtml?: boolean;
  htmlBody?: string | null;
}

export interface LockStateEvent {
  locked: boolean;
  hasPasscode: boolean;
  lockedReason?: string | null;
  reason?: string | null;
  stateVersion?: number;
}

export interface MessageRef {
  accountId: number;
  messageId: number;
}

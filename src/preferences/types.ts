/**
 * KiteMail 完整偏好设置、安全锁口令、远图白名单、缓存与更新模块类型定义与冻结 IPC 契约。
 * 所有枚举值与结构体字段严格使用 camelCase，与 Rust 后端保持一致。
 */

export type PreferenceTabId =
  | "appearance"
  | "sync"
  | "compose"
  | "privacy"
  | "storage"
  | "about";

export type ReaderPosition = "right" | "below" | "off";
export type ReadMarkMode = "shown" | "delay" | "manual";
export type DefaultViewMode = "unified" | "last";

export interface FullPreferencesDto {
  themeId: string;
  darkMode: boolean | null;
  readerPosition: ReaderPosition;
  compactList: boolean;
  showAvatars: boolean;
  listGroupByDate: boolean;
  readerZoomPercent: number;
  uiTextScalePercent: number;
  autoSyncMinutes: number;
  pushEnabled: boolean;
  desktopNotifications: boolean;
  notificationShowContent: boolean;
  closeToMinimize: boolean;
  minimizeToTray: boolean;
  checkUpdates: boolean;
  blockRemoteImages: boolean;
  showRemoteBanner: boolean;
  cleanLinks: boolean;
  readMarkMode: ReadMarkMode;
  threadNewestFirst: boolean;
  undoSendSecs: number;
  signatureAboveQuote: boolean;
  signatureDashes: boolean;
  attachmentReminderKeywords: string;
  replyHeaderTemplate: string;
  singleKeyShortcuts: boolean;
  defaultView: DefaultViewMode;
  appLockEnabled: boolean;
  appLockIdleMinutes: number;
  appLockOnStartup: boolean;
}

export type FullPreferencesPatch = Partial<FullPreferencesDto>;

export interface StorageStatsDto {
  totalBytes: number;
  mailBytes: number;
  configBytes: number;
  fileCount: number;
}

export interface GetFullPreferencesOutput {
  preferences: FullPreferencesDto;
  hasPasscode: boolean;
  allowedRemoteSenders: string[];
  dataDir: string;
  mailDataDir: string;
  storageStats: StorageStatsDto;
  appVersion: string;
  isPortable: boolean;
  stateVersion: number;
}

export interface UpdateFullPreferencesOutput {
  preferences: FullPreferencesDto;
  stateVersion: number;
}

export interface SetAppLockPasscodeInput {
  currentPasscode?: string | null;
  newPasscode: string | null;
  enableOnStartup?: boolean;
  idleMinutes?: number;
}

export interface SetAppLockPasscodeOutput {
  hasPasscode: boolean;
  appLockEnabled: boolean;
  message: string;
  stateVersion: number;
}

export type TrustedSenderAction = "list" | "add" | "remove" | "clear";

export interface ManageTrustedSendersInput {
  action: TrustedSenderAction;
  sender?: string | null;
}

export interface ManageTrustedSendersOutput {
  senders: string[];
  stateVersion: number;
}

export interface ClearBodyCachesInput {
  keepDays?: number | null;
}

export interface ClearBodyCachesOutput {
  accepted: boolean;
  message: string;
  storageStats: StorageStatsDto;
  stateVersion: number;
}

export interface CheckAppUpdateOutput {
  checked: boolean;
  updateAvailable: boolean;
  currentVersion: string;
  target: string;
  latestVersion?: string | null;
  notes?: string | null;
  pubDate?: string | null;
  downloadUrl?: string | null;
  message: string;
}

export interface InstallAppUpdateOutput {
  started: boolean;
  target: string;
  version?: string | null;
  message: string;
}

export interface ExportSettingsBackupOutput {
  saved: boolean;
  cancelled: boolean;
  path?: string | null;
  message: string;
}

export interface PreviewSettingsBackupOutput {
  cancelled: boolean;
  path?: string | null;
  accountCount: number;
  themeId?: string | null;
  message: string;
}

export interface ImportSettingsBackupOutput {
  imported: boolean;
  accountCount: number;
  message: string;
  stateVersion: number;
}

export type DataDirectoryKind = "data" | "mail";

export interface OpenDataDirectoryOutput {
  opened: boolean;
  path: string;
  message: string;
}

export interface PasscodeStatus {
  hasPasscode: boolean;
  appLockEnabled: boolean;
}

export interface PreferencesBanner {
  kind: "success" | "error" | "info";
  text: string;
}

/**
 * KiteMail 完整偏好设置与系统工具 IPC 客户端与可控 Mock 传输层。
 *
 * 严格遵循后端参数包装约定：
 *   get_full_preferences()                无参
 *   update_full_preferences({ input })    包在 input 字段
 *   set_app_lock_passcode({ input })      包在 input 字段
 *   manage_trusted_senders({ input })     包在 input 字段
 *   clear_body_caches({ input })          包在 input 字段
 *   check_app_update()                    无参
 */

import { isTauri, extractErrorMessage, isLockedError } from "../ipc/main-window";
import type {
  CheckAppUpdateOutput,
  ClearBodyCachesInput,
  ClearBodyCachesOutput,
  FullPreferencesDto,
  FullPreferencesPatch,
  GetFullPreferencesOutput,
  ManageTrustedSendersInput,
  ManageTrustedSendersOutput,
  SetAppLockPasscodeInput,
  SetAppLockPasscodeOutput,
  StorageStatsDto,
  UpdateFullPreferencesOutput,
} from "./types";

export { isTauri, extractErrorMessage, isLockedError };

async function tauriInvoke<T>(cmd: string, args?: unknown): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args as Record<string, unknown>);
}

// ---------- 默认偏好与 Mock 状态 ----------

export const DEFAULT_FULL_PREFERENCES: FullPreferencesDto = {
  themeId: "system",
  darkMode: null,
  readerPosition: "right",
  compactList: false,
  showAvatars: true,
  listGroupByDate: true,
  readerZoomPercent: 100,
  uiTextScalePercent: 100,
  autoSyncMinutes: 5,
  pushEnabled: true,
  desktopNotifications: true,
  notificationShowContent: true,
  closeToMinimize: true,
  minimizeToTray: true,
  checkUpdates: true,
  blockRemoteImages: true,
  showRemoteBanner: true,
  cleanLinks: true,
  readMarkMode: "shown",
  threadNewestFirst: false,
  undoSendSecs: 5,
  signatureAboveQuote: false,
  signatureDashes: true,
  attachmentReminderKeywords: "附件, 附上, attach, attachment, 详看",
  replyHeaderTemplate: "在 {date}，{sender} 写道：",
  singleKeyShortcuts: false,
  defaultView: "unified",
  appLockEnabled: false,
  appLockIdleMinutes: 15,
  appLockOnStartup: false,
};

const DEFAULT_STORAGE_STATS: StorageStatsDto = {
  totalBytes: 128_540_000,
  mailBytes: 124_200_000,
  configBytes: 4_340_000,
  fileCount: 1420,
};

export interface MockPreferencesOptions {
  preferences?: FullPreferencesPatch;
  hasPasscode?: boolean;
  passcode?: string | null;
  allowedRemoteSenders?: string[];
  dataDir?: string;
  mailDataDir?: string;
  storageStats?: Partial<StorageStatsDto>;
  appVersion?: string;
  isPortable?: boolean;
  locked?: boolean;
  stateVersion?: number;
  updateResponder?: () => CheckAppUpdateOutput | Promise<CheckAppUpdateOutput>;
}

let mockPrefs: FullPreferencesDto = { ...DEFAULT_FULL_PREFERENCES };
let mockHasPasscode = false;
let mockPasscode: string | null = null;
let mockAllowedSenders: string[] = ["newsletter@kitemail.example"];
let mockDataDir = "C:\\Users\\Demo\\AppData\\Local\\KiteMailPreview";
let mockMailDataDir = "C:\\Users\\Demo\\AppData\\Local\\KiteMailPreview\\mail";
let mockStorageStats: StorageStatsDto = { ...DEFAULT_STORAGE_STATS };
let mockAppVersion = "0.1.0";
let mockIsPortable = false;
let mockLocked = false;
let mockStateVersion = 1;
let mockUpdateResponder: MockPreferencesOptions["updateResponder"] | null = null;

function clonePrefs(p: FullPreferencesDto): FullPreferencesDto {
  return { ...p };
}

function cloneStats(s: StorageStatsDto): StorageStatsDto {
  return { ...s };
}

function makeLockedError() {
  return {
    code: "LOCKED",
    message: "应用已锁定，请先解锁后再访问偏好设置",
    retryable: false,
  };
}

function makeError(code: string, message: string) {
  return {
    code,
    message,
    retryable: false,
  };
}

function clampNumber(val: number, min: number, max: number): number {
  if (!Number.isFinite(val)) return min;
  return Math.min(max, Math.max(min, Math.round(val)));
}

function sanitizePrefsPatch(
  base: FullPreferencesDto,
  patch: FullPreferencesPatch
): FullPreferencesDto {
  const next: FullPreferencesDto = { ...base, ...patch };
  next.readerZoomPercent = clampNumber(next.readerZoomPercent, 50, 250);
  next.uiTextScalePercent = clampNumber(next.uiTextScalePercent, 80, 150);
  next.autoSyncMinutes = clampNumber(next.autoSyncMinutes, 0, 1440);
  next.undoSendSecs = clampNumber(next.undoSendSecs, 0, 30);
  next.appLockIdleMinutes = clampNumber(next.appLockIdleMinutes, 1, 240);
  if (
    next.readerPosition !== "right" &&
    next.readerPosition !== "below" &&
    next.readerPosition !== "off"
  ) {
    next.readerPosition = "right";
  }
  if (
    next.readMarkMode !== "shown" &&
    next.readMarkMode !== "delay" &&
    next.readMarkMode !== "manual"
  ) {
    next.readMarkMode = "shown";
  }
  if (next.defaultView !== "unified" && next.defaultView !== "last") {
    next.defaultView = "unified";
  }
  return next;
}

export function __resetPreferencesMock(options?: MockPreferencesOptions): void {
  mockPrefs = sanitizePrefsPatch(DEFAULT_FULL_PREFERENCES, options?.preferences ?? {});
  const hasPasscode = options?.hasPasscode ?? false;
  mockHasPasscode = hasPasscode;
  mockPasscode =
    options?.passcode !== undefined
      ? options.passcode
      : hasPasscode
        ? "1234"
        : null;
  mockAllowedSenders = [...(options?.allowedRemoteSenders ?? ["newsletter@kitemail.example"])];
  mockDataDir = options?.dataDir ?? "C:\\Users\\Demo\\AppData\\Local\\KiteMailPreview";
  mockMailDataDir =
    options?.mailDataDir ?? "C:\\Users\\Demo\\AppData\\Local\\KiteMailPreview\\mail";
  mockStorageStats = { ...DEFAULT_STORAGE_STATS, ...(options?.storageStats ?? {}) };
  mockAppVersion = options?.appVersion ?? "0.1.0";
  mockIsPortable = options?.isPortable ?? false;
  mockLocked = options?.locked ?? false;
  mockStateVersion = options?.stateVersion ?? 1;
  mockUpdateResponder = options?.updateResponder ?? null;
}

// ---------- 公共 IPC API ----------

export async function getFullPreferences(): Promise<GetFullPreferencesOutput> {
  if (isTauri()) return tauriInvoke<GetFullPreferencesOutput>("get_full_preferences");
  if (mockLocked) return Promise.reject(makeLockedError());

  return {
    preferences: clonePrefs(mockPrefs),
    hasPasscode: mockHasPasscode,
    allowedRemoteSenders: [...mockAllowedSenders],
    dataDir: mockDataDir,
    mailDataDir: mockMailDataDir,
    storageStats: cloneStats(mockStorageStats),
    appVersion: mockAppVersion,
    isPortable: mockIsPortable,
    stateVersion: mockStateVersion,
  };
}

export async function updateFullPreferences(
  input: FullPreferencesPatch
): Promise<UpdateFullPreferencesOutput> {
  if (isTauri()) {
    return tauriInvoke<UpdateFullPreferencesOutput>("update_full_preferences", { input });
  }
  if (mockLocked) return Promise.reject(makeLockedError());

  mockPrefs = sanitizePrefsPatch(mockPrefs, input);
  mockStateVersion += 1;
  return {
    preferences: clonePrefs(mockPrefs),
    stateVersion: mockStateVersion,
  };
}

export async function setAppLockPasscode(
  input: SetAppLockPasscodeInput
): Promise<SetAppLockPasscodeOutput> {
  if (isTauri()) {
    return tauriInvoke<SetAppLockPasscodeOutput>("set_app_lock_passcode", { input });
  }
  if (mockLocked) return Promise.reject(makeLockedError());

  // 绝不在任何报错或返回信息中回显口令明文
  if (mockHasPasscode) {
    if (!input.currentPasscode || input.currentPasscode !== mockPasscode) {
      return Promise.reject(
        makeError("INVALID_PASSCODE", "当前安全锁口令不正确，请重新输入")
      );
    }
  }

  if (input.newPasscode === null || input.newPasscode === "") {
    mockPasscode = null;
    mockHasPasscode = false;
    mockPrefs.appLockEnabled = false;
    mockPrefs.appLockOnStartup = false;
    mockStateVersion += 1;
    return {
      hasPasscode: false,
      appLockEnabled: false,
      message: "安全锁口令已清除",
      stateVersion: mockStateVersion,
    };
  }

  if (input.newPasscode.length < 4) {
    return Promise.reject(
      makeError("INVALID_INPUT", "安全锁口令长度至少为 4 个字符")
    );
  }

  mockPasscode = input.newPasscode;
  mockHasPasscode = true;
  mockPrefs.appLockEnabled = true;
  if (typeof input.enableOnStartup === "boolean") {
    mockPrefs.appLockOnStartup = input.enableOnStartup;
  }
  if (typeof input.idleMinutes === "number") {
    mockPrefs.appLockIdleMinutes = clampNumber(input.idleMinutes, 1, 240);
  }
  mockStateVersion += 1;
  return {
    hasPasscode: true,
    appLockEnabled: mockPrefs.appLockEnabled,
    message: "安全锁口令已设置并启用",
    stateVersion: mockStateVersion,
  };
}

export async function manageTrustedSenders(
  input: ManageTrustedSendersInput
): Promise<ManageTrustedSendersOutput> {
  if (isTauri()) {
    return tauriInvoke<ManageTrustedSendersOutput>("manage_trusted_senders", { input });
  }
  if (mockLocked) return Promise.reject(makeLockedError());

  const sender = input.sender?.trim().toLowerCase() ?? "";
  switch (input.action) {
    case "list":
      break;
    case "add": {
      if (!sender || !sender.includes("@")) {
        return Promise.reject(
          makeError("INVALID_INPUT", "请输入有效的发件人邮箱地址")
        );
      }
      if (!mockAllowedSenders.includes(sender)) {
        mockAllowedSenders.push(sender);
        mockStateVersion += 1;
      }
      break;
    }
    case "remove": {
      const before = mockAllowedSenders.length;
      mockAllowedSenders = mockAllowedSenders.filter((s) => s !== sender);
      if (mockAllowedSenders.length !== before) {
        mockStateVersion += 1;
      }
      break;
    }
    case "clear": {
      if (mockAllowedSenders.length > 0) {
        mockAllowedSenders = [];
        mockStateVersion += 1;
      }
      break;
    }
  }

  return {
    senders: [...mockAllowedSenders],
    stateVersion: mockStateVersion,
  };
}

export async function clearBodyCaches(
  input: ClearBodyCachesInput = {}
): Promise<ClearBodyCachesOutput> {
  if (isTauri()) {
    return tauriInvoke<ClearBodyCachesOutput>("clear_body_caches", { input });
  }
  if (mockLocked) return Promise.reject(makeLockedError());

  const keepDays = input.keepDays ?? null;
  if (keepDays === null || keepDays <= 0) {
    mockStorageStats = {
      ...mockStorageStats,
      mailBytes: 0,
      totalBytes: mockStorageStats.configBytes,
      fileCount: Math.min(mockStorageStats.fileCount, 12),
    };
  } else {
    const retainedMailBytes = Math.round(mockStorageStats.mailBytes * 0.25);
    mockStorageStats = {
      ...mockStorageStats,
      mailBytes: retainedMailBytes,
      totalBytes: retainedMailBytes + mockStorageStats.configBytes,
      fileCount: Math.max(12, Math.round(mockStorageStats.fileCount * 0.25)),
    };
  }
  mockStateVersion += 1;
  return {
    accepted: true,
    message:
      keepDays && keepDays > 0
        ? `已清理 ${keepDays} 天前的邮件正文缓存`
        : "已清理全部本地邮件正文缓存",
    storageStats: cloneStats(mockStorageStats),
    stateVersion: mockStateVersion,
  };
}

export async function checkAppUpdate(): Promise<CheckAppUpdateOutput> {
  if (isTauri()) return tauriInvoke<CheckAppUpdateOutput>("check_app_update");
  if (mockLocked) return Promise.reject(makeLockedError());

  if (mockUpdateResponder) {
    return await mockUpdateResponder();
  }

  return {
    checked: true,
    updateAvailable: false,
    currentVersion: mockAppVersion,
    target: "windows-x86_64",
    latestVersion: mockAppVersion,
    notes: null,
    pubDate: null,
    downloadUrl: null,
    message: `当前版本 v${mockAppVersion} 已是最新版本`,
  };
}

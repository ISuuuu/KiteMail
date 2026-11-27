import { getCurrentInstance, onUnmounted, ref } from "vue";
import {
  DEFAULT_FULL_PREFERENCES,
  checkAppUpdate,
  clearBodyCaches,
  extractErrorMessage,
  getFullPreferences,
  isLockedError,
  manageTrustedSenders,
  setAppLockPasscode,
  updateFullPreferences,
} from "./ipc";
import type {
  CheckAppUpdateOutput,
  FullPreferencesDto,
  FullPreferencesPatch,
  PasscodeStatus,
  PreferenceTabId,
  PreferencesBanner,
  StorageStatsDto,
} from "./types";

// ---------- 纯函数工具 ----------

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unitIdx = 0;
  while (value >= 1024 && unitIdx < units.length - 1) {
    value /= 1024;
    unitIdx += 1;
  }
  const fixed = unitIdx === 0 ? value.toFixed(0) : value.toFixed(value >= 100 ? 0 : 1);
  return `${fixed} ${units[unitIdx]}`;
}

export function validatePasscodeChange(params: {
  hasPasscode: boolean;
  currentPasscode: string;
  newPasscode: string;
  confirmPasscode: string;
  clearing: boolean;
}): string | null {
  if (params.hasPasscode && !params.currentPasscode) {
    return "请输入当前安全锁口令以验证身份";
  }
  if (params.clearing) {
    return null;
  }
  if (!params.newPasscode || params.newPasscode.length < 4) {
    return "新口令长度至少为 4 个字符";
  }
  if (params.newPasscode !== params.confirmPasscode) {
    return "两次输入的新口令不一致";
  }
  return null;
}

export interface UsePreferencesOptions {
  onLocked?: () => void;
  onPreferencesUpdated?: (prefs: FullPreferencesDto) => void;
  onPasscodeChanged?: (status: PasscodeStatus) => void;
}

export function usePreferences(options?: UsePreferencesOptions) {
  const activeTab = ref<PreferenceTabId>("appearance");
  const loading = ref(false);
  const saving = ref(false);
  const passcodeBusy = ref(false);
  const sendersBusy = ref(false);
  const cacheBusy = ref(false);
  const checkingUpdate = ref(false);

  const loadError = ref<string | null>(null);
  const banner = ref<PreferencesBanner | null>(null);

  const preferences = ref<FullPreferencesDto>({ ...DEFAULT_FULL_PREFERENCES });
  const hasPasscode = ref(false);
  const allowedRemoteSenders = ref<string[]>([]);
  const dataDir = ref("");
  const mailDataDir = ref("");
  const storageStats = ref<StorageStatsDto>({
    totalBytes: 0,
    mailBytes: 0,
    configBytes: 0,
    fileCount: 0,
  });
  const appVersion = ref("0.1.0");
  const isPortable = ref(false);

  // 口令表单本地缓冲（提交/失败后立即清空）
  const currentPasscodeInput = ref("");
  const newPasscodeInput = ref("");
  const confirmPasscodeInput = ref("");
  const passcodeError = ref<string | null>(null);

  // 白名单输入
  const newTrustedSenderInput = ref("");
  // 缓存清理保留天数（null=全部清理）
  const cacheKeepDays = ref<number | null>(null);
  // 检查更新结果
  const updateResult = ref<CheckAppUpdateOutput | null>(null);

  let disposed = false;

  function clearPasscodeFields(): void {
    currentPasscodeInput.value = "";
    newPasscodeInput.value = "";
    confirmPasscodeInput.value = "";
  }

  function handleError(err: unknown, fallbackPrefix: string): string {
    if (isLockedError(err)) {
      const msg = extractErrorMessage(err);
      banner.value = { kind: "error", text: msg };
      options?.onLocked?.();
      return msg;
    }
    const msg = `${fallbackPrefix}：${extractErrorMessage(err)}`;
    banner.value = { kind: "error", text: msg };
    return msg;
  }

  async function init(initialTab?: PreferenceTabId): Promise<void> {
    if (initialTab) activeTab.value = initialTab;
    loading.value = true;
    loadError.value = null;
    banner.value = null;
    passcodeError.value = null;
    clearPasscodeFields();

    try {
      const res = await getFullPreferences();
      if (disposed) return;
      preferences.value = { ...res.preferences };
      hasPasscode.value = res.hasPasscode;
      allowedRemoteSenders.value = [...res.allowedRemoteSenders];
      dataDir.value = res.dataDir;
      mailDataDir.value = res.mailDataDir;
      storageStats.value = { ...res.storageStats };
      appVersion.value = res.appVersion;
      isPortable.value = res.isPortable;
    } catch (err) {
      if (disposed) return;
      if (isLockedError(err)) {
        loadError.value = extractErrorMessage(err);
        options?.onLocked?.();
      } else {
        loadError.value = extractErrorMessage(err);
      }
    } finally {
      if (!disposed) loading.value = false;
    }
  }

  async function updatePreference(
    patch: FullPreferencesPatch
  ): Promise<FullPreferencesDto | null> {
    saving.value = true;
    banner.value = null;
    try {
      const res = await updateFullPreferences(patch);
      if (disposed) return null;
      preferences.value = { ...res.preferences };
      options?.onPreferencesUpdated?.({ ...res.preferences });
      banner.value = { kind: "success", text: "偏好设置已保存" };
      return res.preferences;
    } catch (err) {
      handleError(err, "保存偏好设置失败");
      return null;
    } finally {
      saving.value = false;
    }
  }

  async function submitPasscode(params?: {
    currentPasscode?: string;
    newPasscode?: string;
    confirmPasscode?: string;
    enableOnStartup?: boolean;
    idleMinutes?: number;
  }): Promise<boolean> {
    const current = params?.currentPasscode ?? currentPasscodeInput.value;
    const next = params?.newPasscode ?? newPasscodeInput.value;
    const confirm = params?.confirmPasscode ?? confirmPasscodeInput.value;

    const validationErr = validatePasscodeChange({
      hasPasscode: hasPasscode.value,
      currentPasscode: current,
      newPasscode: next,
      confirmPasscode: confirm,
      clearing: false,
    });
    if (validationErr) {
      passcodeError.value = validationErr;
      return false;
    }

    passcodeBusy.value = true;
    passcodeError.value = null;
    banner.value = null;
    try {
      const res = await setAppLockPasscode({
        currentPasscode: hasPasscode.value ? current : null,
        newPasscode: next,
        enableOnStartup: params?.enableOnStartup ?? preferences.value.appLockOnStartup,
        idleMinutes: params?.idleMinutes ?? preferences.value.appLockIdleMinutes,
      });
      if (disposed) return false;
      hasPasscode.value = res.hasPasscode;
      preferences.value = {
        ...preferences.value,
        appLockEnabled: res.appLockEnabled,
        appLockOnStartup:
          params?.enableOnStartup ?? preferences.value.appLockOnStartup,
        appLockIdleMinutes:
          params?.idleMinutes ?? preferences.value.appLockIdleMinutes,
      };
      options?.onPasscodeChanged?.({
        hasPasscode: res.hasPasscode,
        appLockEnabled: res.appLockEnabled,
      });
      options?.onPreferencesUpdated?.({ ...preferences.value });
      banner.value = { kind: "success", text: res.message };
      return true;
    } catch (err) {
      const msg = isLockedError(err)
        ? extractErrorMessage(err)
        : extractErrorMessage(err);
      passcodeError.value = msg;
      if (isLockedError(err)) options?.onLocked?.();
      return false;
    } finally {
      // 无论成功或失败均立即清除内存中的口令输入，绝不滞留或打印明文
      clearPasscodeFields();
      passcodeBusy.value = false;
    }
  }

  async function clearPasscode(currentPasscodeOverride?: string): Promise<boolean> {
    const current = currentPasscodeOverride ?? currentPasscodeInput.value;
    const validationErr = validatePasscodeChange({
      hasPasscode: hasPasscode.value,
      currentPasscode: current,
      newPasscode: "",
      confirmPasscode: "",
      clearing: true,
    });
    if (validationErr) {
      passcodeError.value = validationErr;
      return false;
    }

    passcodeBusy.value = true;
    passcodeError.value = null;
    banner.value = null;
    try {
      const res = await setAppLockPasscode({
        currentPasscode: hasPasscode.value ? current : null,
        newPasscode: null,
      });
      if (disposed) return false;
      hasPasscode.value = res.hasPasscode;
      preferences.value = {
        ...preferences.value,
        appLockEnabled: res.appLockEnabled,
        appLockOnStartup: false,
      };
      options?.onPasscodeChanged?.({
        hasPasscode: res.hasPasscode,
        appLockEnabled: res.appLockEnabled,
      });
      options?.onPreferencesUpdated?.({ ...preferences.value });
      banner.value = { kind: "success", text: res.message };
      return true;
    } catch (err) {
      const msg = extractErrorMessage(err);
      passcodeError.value = msg;
      if (isLockedError(err)) options?.onLocked?.();
      return false;
    } finally {
      clearPasscodeFields();
      passcodeBusy.value = false;
    }
  }

  async function addTrustedSender(senderOverride?: string): Promise<boolean> {
    const sender = (senderOverride ?? newTrustedSenderInput.value).trim();
    if (!sender) {
      banner.value = { kind: "error", text: "请输入要加入白名单的发件人邮箱" };
      return false;
    }
    sendersBusy.value = true;
    banner.value = null;
    try {
      const res = await manageTrustedSenders({ action: "add", sender });
      if (disposed) return false;
      allowedRemoteSenders.value = [...res.senders];
      newTrustedSenderInput.value = "";
      banner.value = { kind: "success", text: `已将 ${sender} 加入远图白名单` };
      return true;
    } catch (err) {
      handleError(err, "添加白名单失败");
      return false;
    } finally {
      sendersBusy.value = false;
    }
  }

  async function removeTrustedSender(sender: string): Promise<boolean> {
    sendersBusy.value = true;
    banner.value = null;
    try {
      const res = await manageTrustedSenders({ action: "remove", sender });
      if (disposed) return false;
      allowedRemoteSenders.value = [...res.senders];
      banner.value = { kind: "info", text: `已移除 ${sender}` };
      return true;
    } catch (err) {
      handleError(err, "移除白名单失败");
      return false;
    } finally {
      sendersBusy.value = false;
    }
  }

  async function clearTrustedSenders(): Promise<boolean> {
    sendersBusy.value = true;
    banner.value = null;
    try {
      const res = await manageTrustedSenders({ action: "clear" });
      if (disposed) return false;
      allowedRemoteSenders.value = [...res.senders];
      banner.value = { kind: "info", text: "已清空全部远图白名单发件人" };
      return true;
    } catch (err) {
      handleError(err, "清空白名单失败");
      return false;
    } finally {
      sendersBusy.value = false;
    }
  }

  async function clearCaches(keepDaysOverride?: number | null): Promise<boolean> {
    const keepDays =
      keepDaysOverride !== undefined ? keepDaysOverride : cacheKeepDays.value;
    cacheBusy.value = true;
    banner.value = null;
    try {
      const res = await clearBodyCaches({ keepDays });
      if (disposed) return false;
      storageStats.value = { ...res.storageStats };
      banner.value = { kind: res.accepted ? "success" : "info", text: res.message };
      return res.accepted;
    } catch (err) {
      handleError(err, "清理缓存失败");
      return false;
    } finally {
      cacheBusy.value = false;
    }
  }

  async function checkForUpdates(): Promise<CheckAppUpdateOutput | null> {
    checkingUpdate.value = true;
    banner.value = null;
    try {
      const res = await checkAppUpdate();
      if (disposed) return null;
      updateResult.value = res;
      banner.value = {
        kind: res.updateAvailable ? "info" : "success",
        text: res.message,
      };
      return res;
    } catch (err) {
      handleError(err, "检查更新失败");
      return null;
    } finally {
      checkingUpdate.value = false;
    }
  }

  function dispose(): void {
    disposed = true;
    clearPasscodeFields();
  }

  if (getCurrentInstance()) {
    onUnmounted(() => dispose());
  }

  return {
    activeTab,
    loading,
    saving,
    passcodeBusy,
    sendersBusy,
    cacheBusy,
    checkingUpdate,
    loadError,
    banner,
    preferences,
    hasPasscode,
    allowedRemoteSenders,
    dataDir,
    mailDataDir,
    storageStats,
    appVersion,
    isPortable,
    currentPasscodeInput,
    newPasscodeInput,
    confirmPasscodeInput,
    passcodeError,
    newTrustedSenderInput,
    cacheKeepDays,
    updateResult,
    init,
    updatePreference,
    submitPasscode,
    clearPasscode,
    addTrustedSender,
    removeTrustedSender,
    clearTrustedSenders,
    clearCaches,
    checkForUpdates,
    dispose,
  };
}

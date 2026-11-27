import {
  computed,
  getCurrentInstance,
  onUnmounted,
  ref,
} from "vue";
import {
  BUILTIN_PROVIDER_PRESETS,
  deleteAccount,
  extractErrorMessage,
  isLockedError,
  listAccounts,
  onAccountEvent,
  saveAccount,
  setAccountEnabled,
  startMicrosoftOAuth,
  verifyAccount,
  type UnlistenFn,
} from "./ipc";
import type {
  AccountConfigDto,
  AccountEvent,
  AccountFormModel,
  AccountValidationErrors,
  AccountValidationResult,
  LockableAccountField,
  OAuthFlowStatus,
  PresetMatchResult,
  ProviderPresetDto,
  SaveAccountInput,
  StatusBanner,
  VerifyAccountOutput,
} from "./types";

const EMAIL_PATTERN = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

const PRESET_LOCKABLE_FIELDS: readonly LockableAccountField[] = [
  "authBackend",
  "imapHost",
  "imapPort",
  "imapSecurity",
  "smtpHost",
  "smtpPort",
  "smtpSecurity",
];

// ---------- 纯函数工具（可在 Node 无 DOM 环境直接单测） ----------

/** 从邮箱地址或 `Name <email>` 中提取小写域名 */
export function extractEmailDomain(email: string): string {
  const trimmed = email.trim();
  const angleMatch = trimmed.match(/<([^>]+)>/);
  const address = (angleMatch ? angleMatch[1] : trimmed).trim();
  const atIndex = address.lastIndexOf("@");
  if (atIndex <= 0 || atIndex === address.length - 1) return "";
  return address.slice(atIndex + 1).toLowerCase();
}

/** 根据邮箱域名匹配主流服务商预设 */
export function matchPresetByEmail(
  email: string,
  presets: readonly ProviderPresetDto[] = BUILTIN_PROVIDER_PRESETS
): PresetMatchResult {
  const domain = extractEmailDomain(email);
  if (!domain) {
    return { matched: false, domain: "", preset: null };
  }
  const preset =
    presets.find((p) =>
      p.domains.some((d) => d.trim().toLowerCase() === domain)
    ) ?? null;
  return {
    matched: preset !== null,
    domain,
    preset,
  };
}

export function createEmptyAccountForm(
  preset?: ProviderPresetDto | null
): AccountFormModel {
  const target = preset ?? null;
  return {
    id: null,
    presetId: target?.id ?? "manual",
    authBackend: target?.authBackend ?? "password",
    oauthClientId: "",
    displayName: "",
    email: "",
    label: "",
    enabled: true,
    inUnified: true,
    imapHost: target?.imapHost ?? "",
    imapPort: target?.imapPort ?? 993,
    imapSecurity: target?.imapSecurity ?? "tls",
    imapUsername: "",
    imapPassword: "",
    smtpHost: target?.smtpHost ?? "",
    smtpPort: target?.smtpPort ?? 465,
    smtpSecurity: target?.smtpSecurity ?? "tls",
    smtpUsername: "",
    smtpPassword: "",
    smtpSeparateCredentials: false,
    signature: "",
    hasPassword: false,
  };
}

export function inferPresetForAccount(
  account: AccountConfigDto,
  presets: readonly ProviderPresetDto[]
): ProviderPresetDto | null {
  if (account.authBackend === "microsoftGraphOAuth2") {
    return presets.find((p) => p.authBackend === "microsoftGraphOAuth2") ?? null;
  }
  const byDomain = matchPresetByEmail(account.email, presets);
  if (byDomain.preset) return byDomain.preset;
  const host = account.imapHost.trim().toLowerCase();
  if (host) {
    const byHost = presets.find(
      (p) => p.imapHost && p.imapHost.toLowerCase() === host
    );
    if (byHost) return byHost;
  }
  return presets.find((p) => p.id === "manual") ?? null;
}

export function accountDtoToForm(
  account: AccountConfigDto,
  presets: readonly ProviderPresetDto[] = BUILTIN_PROVIDER_PRESETS
): AccountFormModel {
  const matchedPreset = inferPresetForAccount(account, presets);
  return {
    id: account.id,
    presetId: matchedPreset?.id ?? "manual",
    authBackend: account.authBackend,
    oauthClientId: account.oauthClientId ?? "",
    displayName: account.displayName,
    email: account.email,
    label: account.label,
    enabled: account.enabled,
    inUnified: account.inUnified,
    imapHost: account.imapHost,
    imapPort: account.imapPort,
    imapSecurity: account.imapSecurity,
    imapUsername: account.imapUsername,
    imapPassword: "",
    smtpHost: account.smtpHost,
    smtpPort: account.smtpPort,
    smtpSecurity: account.smtpSecurity,
    smtpUsername: account.smtpUsername,
    smtpPassword: "",
    smtpSeparateCredentials: account.smtpSeparateCredentials,
    signature: account.signature,
    hasPassword: account.hasPassword,
  };
}

function isValidPort(port: unknown): boolean {
  if (typeof port !== "number" || !Number.isInteger(port)) return false;
  return port >= 1 && port <= 65535;
}

/** 校验账号表单：邮箱格式、端口范围、密码非空（编辑态 hasPassword 留空保留原密码）、独立 SMTP 凭据 */
export function validateAccountForm(
  form: AccountFormModel
): AccountValidationResult {
  const errors: AccountValidationErrors = {};
  const email = form.email.trim();

  if (!email) {
    errors.email = "请输入邮箱地址";
  } else if (!EMAIL_PATTERN.test(email)) {
    errors.email = "邮箱地址格式不正确";
  }

  if (!form.displayName.trim()) {
    errors.displayName = "请输入发件人显示名称";
  }

  if (form.authBackend === "password") {
    if (!form.imapHost.trim()) {
      errors.imapHost = "请输入 IMAP 服务器地址";
    }
    if (!isValidPort(form.imapPort)) {
      errors.imapPort = "IMAP 端口必须是 1 到 65535 之间的整数";
    }
    if (!form.imapUsername.trim()) {
      errors.imapUsername = "请输入 IMAP 登录用户名";
    }

    if (!form.smtpHost.trim()) {
      errors.smtpHost = "请输入 SMTP 服务器地址";
    }
    if (!isValidPort(form.smtpPort)) {
      errors.smtpPort = "SMTP 端口必须是 1 到 65535 之间的整数";
    }

    const canKeepExistingPassword = Boolean(form.id && form.hasPassword);
    if (!canKeepExistingPassword && form.imapPassword.length === 0) {
      errors.imapPassword = "请输入邮箱密码或客户端授权码";
    }

    if (form.smtpSeparateCredentials) {
      if (!form.smtpUsername.trim()) {
        errors.smtpUsername = "已开启独立 SMTP 凭据，请输入 SMTP 用户名";
      }
      if (!canKeepExistingPassword && form.smtpPassword.length === 0) {
        errors.smtpPassword = "已开启独立 SMTP 凭据，请输入 SMTP 密码";
      }
    }
  } else {
    // OAuth2 模式下若填写了服务器端口，仍校验端口合法性
    if (form.imapHost.trim() && !isValidPort(form.imapPort)) {
      errors.imapPort = "IMAP 端口必须是 1 到 65535 之间的整数";
    }
    if (form.smtpHost.trim() && !isValidPort(form.smtpPort)) {
      errors.smtpPort = "SMTP 端口必须是 1 到 65535 之间的整数";
    }
  }

  const keys = Object.keys(errors) as Array<keyof AccountValidationErrors>;
  const firstError = keys.length > 0 ? (errors[keys[0]] ?? null) : null;
  return {
    valid: keys.length === 0,
    errors,
    firstError,
  };
}

/** 构造保存/验证账号 IPC 载荷 */
export function buildSaveAccountInput(form: AccountFormModel): SaveAccountInput {
  const email = form.email.trim();
  const imapUsername = form.imapUsername.trim() || email;
  const isOAuth = form.authBackend === "microsoftGraphOAuth2";
  const separate = !isOAuth && form.smtpSeparateCredentials;

  return {
    id: form.id ?? null,
    authBackend: form.authBackend,
    oauthClientId: isOAuth ? form.oauthClientId.trim() || null : null,
    displayName: form.displayName.trim(),
    email,
    label: form.label.trim(),
    enabled: form.enabled,
    inUnified: form.inUnified,
    imapHost: form.imapHost.trim(),
    imapPort: Number(form.imapPort),
    imapSecurity: form.imapSecurity,
    imapUsername,
    smtpHost: form.smtpHost.trim(),
    smtpPort: Number(form.smtpPort),
    smtpSecurity: form.smtpSecurity,
    smtpUsername: separate ? form.smtpUsername.trim() : imapUsername,
    smtpSeparateCredentials: separate,
    signature: form.signature,
    imapPassword: !isOAuth && form.imapPassword.length > 0 ? form.imapPassword : null,
    smtpPassword: separate && form.smtpPassword.length > 0 ? form.smtpPassword : null,
  };
}

/** 脱敏错误信息，避免意外泄露输入的密码文本 */
function sanitizeSensitiveError(rawMessage: string, form: AccountFormModel): string {
  let msg = rawMessage;
  for (const secret of [form.imapPassword, form.smtpPassword]) {
    if (secret && secret.length >= 3 && msg.includes(secret)) {
      msg = msg.split(secret).join("***");
    }
  }
  return msg;
}

export interface UseAccountsOptions {
  onChanged?: () => void;
  onLocked?: () => void;
  onOAuthAuthorized?: (account: AccountConfigDto | null, event: AccountEvent) => void;
}

/** useAccounts 返回的共享状态类型，供账号域组件 props 使用。 */
export type AccountsStore = ReturnType<typeof useAccounts>;

export function useAccounts(options: UseAccountsOptions = {}) {
  const accounts = ref<AccountConfigDto[]>([]);
  const presets = ref<ProviderPresetDto[]>([...BUILTIN_PROVIDER_PRESETS]);
  const stateVersion = ref(0);

  const loading = ref(false);
  const loadError = ref<string | null>(null);

  const selectedAccountId = ref<string | null>(null);
  const isCreating = ref(true);
  const form = ref<AccountFormModel>(createEmptyAccountForm());

  const lockedFields = ref<Set<LockableAccountField>>(new Set());
  const validationErrors = ref<AccountValidationErrors>({});

  const verifying = ref(false);
  const verifyResult = ref<VerifyAccountOutput | null>(null);

  const saving = ref(false);
  const saveError = ref<string | null>(null);

  const togglingEnabledId = ref<string | null>(null);
  const confirmDeleteId = ref<string | null>(null);
  const deletingId = ref<string | null>(null);

  const oauthStatus = ref<OAuthFlowStatus>("idle");
  const oauthMessage = ref<string | null>(null);
  const statusBanner = ref<StatusBanner | null>(null);

  let unlistenAccountEvent: UnlistenFn | null = null;
  let initialized = false;

  const selectedAccount = computed(
    () => accounts.value.find((a) => a.id === selectedAccountId.value) ?? null
  );

  const activePreset = computed(
    () => presets.value.find((p) => p.id === form.value.presetId) ?? null
  );

  const hasLockedPresetFields = computed(() =>
    PRESET_LOCKABLE_FIELDS.some((f) => lockedFields.value.has(f))
  );

  function clearTransientFeedback() {
    validationErrors.value = {};
    verifyResult.value = null;
    saveError.value = null;
    statusBanner.value = null;
  }

  function applyPresetFields(preset: ProviderPresetDto, force = false): void {
    const locked = lockedFields.value;
    const next = { ...form.value, presetId: preset.id };

    if (force || !locked.has("authBackend")) {
      next.authBackend = preset.authBackend;
    }
    if (force || !locked.has("imapHost")) {
      next.imapHost = preset.imapHost;
    }
    if (force || !locked.has("imapPort")) {
      next.imapPort = preset.imapPort;
    }
    if (force || !locked.has("imapSecurity")) {
      next.imapSecurity = preset.imapSecurity;
    }
    if (force || !locked.has("smtpHost")) {
      next.smtpHost = preset.smtpHost;
    }
    if (force || !locked.has("smtpPort")) {
      next.smtpPort = preset.smtpPort;
    }
    if (force || !locked.has("smtpSecurity")) {
      next.smtpSecurity = preset.smtpSecurity;
    }
    form.value = next;
  }

  function markFieldEdited(field: LockableAccountField): void {
    if (!lockedFields.value.has(field)) {
      const next = new Set(lockedFields.value);
      next.add(field);
      lockedFields.value = next;
    }
    verifyResult.value = null;
    saveError.value = null;
  }

  function selectPreset(presetId: string, force = true): void {
    const preset = presets.value.find((p) => p.id === presetId);
    if (!preset) return;
    if (force) {
      const next = new Set(lockedFields.value);
      for (const f of PRESET_LOCKABLE_FIELDS) {
        next.delete(f);
      }
      lockedFields.value = next;
    }
    applyPresetFields(preset, force);
    clearTransientFeedback();
  }

  function resetToPreset(presetId?: string): void {
    const targetId =
      presetId ??
      matchPresetByEmail(form.value.email, presets.value).preset?.id ??
      form.value.presetId;
    const preset = presets.value.find((p) => p.id === targetId);
    if (!preset) return;
    selectPreset(preset.id, true);
    statusBanner.value = {
      kind: "info",
      text: `已恢复「${preset.name}」默认服务器参数`,
    };
  }

  function updateEmail(email: string): void {
    const prevEmail = form.value.email.trim();
    const trimmed = email.trim();
    const next: AccountFormModel = { ...form.value, email };

    if (!lockedFields.value.has("imapUsername")) {
      if (!next.imapUsername || next.imapUsername === prevEmail) {
        next.imapUsername = trimmed;
      }
    }
    if (!next.smtpSeparateCredentials && !lockedFields.value.has("smtpUsername")) {
      if (!next.smtpUsername || next.smtpUsername === prevEmail) {
        next.smtpUsername = next.imapUsername || trimmed;
      }
    }
    if (!next.displayName.trim() && trimmed.includes("@")) {
      const localPart = trimmed.split("@")[0];
      if (localPart) {
        next.displayName = localPart;
      }
    }
    form.value = next;

    const match = matchPresetByEmail(email, presets.value);
    if (match.matched && match.preset) {
      applyPresetFields(match.preset, false);
    }
  }

  function startCreate(presetId?: string): void {
    isCreating.value = true;
    selectedAccountId.value = null;
    confirmDeleteId.value = null;
    lockedFields.value = new Set();
    oauthStatus.value = "idle";
    oauthMessage.value = null;
    const preset = presetId
      ? presets.value.find((p) => p.id === presetId) ?? null
      : null;
    form.value = createEmptyAccountForm(preset);
    clearTransientFeedback();
  }

  function selectAccount(
    accountId: string,
    selectOpts?: { preserveFeedback?: boolean }
  ): void {
    const acc = accounts.value.find((a) => a.id === accountId);
    if (!acc) return;
    isCreating.value = false;
    selectedAccountId.value = acc.id;
    confirmDeleteId.value = null;
    if (!selectOpts?.preserveFeedback) {
      oauthStatus.value = "idle";
      oauthMessage.value = null;
      clearTransientFeedback();
    }
    // 编辑已有账号时默认锁定现有服务器与认证字段，避免输入邮箱时意外覆写用户原有配置
    lockedFields.value = new Set<LockableAccountField>([
      ...PRESET_LOCKABLE_FIELDS,
      "imapUsername",
      "smtpUsername",
    ]);
    form.value = accountDtoToForm(acc, presets.value);
  }

  function handleAccountEvent(event: AccountEvent): void {
    if (typeof event.stateVersion === "number" && event.stateVersion > stateVersion.value) {
      stateVersion.value = event.stateVersion;
    }

    if (event.kind === "oauthReauthorized") {
      oauthStatus.value = event.ok ? "authorized" : "error";
      oauthMessage.value = event.message;
      statusBanner.value = {
        kind: event.ok ? "success" : "error",
        text: event.message,
      };
      if (event.ok) {
        void loadAccounts({
          preserveSelection: false,
          preferAccountId: event.accountId ?? undefined,
          preserveFeedback: true,
        }).then(() => {
          const acc = event.accountId
            ? accounts.value.find((a) => a.id === event.accountId) ?? null
            : null;
          options.onChanged?.();
          options.onOAuthAuthorized?.(acc, event);
        });
      }
      return;
    }

    if (event.kind === "oauthRebindRejected") {
      oauthStatus.value = "rejected";
      oauthMessage.value = event.message;
      statusBanner.value = {
        kind: "error",
        text: event.message,
      };
      return;
    }
  }

  async function loadAccounts(opts?: {
    preserveSelection?: boolean;
    preferAccountId?: string;
    initialPresetId?: string | null;
    preserveFeedback?: boolean;
  }): Promise<void> {
    loading.value = true;
    loadError.value = null;
    try {
      const res = await listAccounts();
      accounts.value = res.accounts;
      if (res.presets.length > 0) {
        presets.value = res.presets;
      }
      stateVersion.value = res.stateVersion;

      if (opts?.initialPresetId) {
        startCreate(opts.initialPresetId);
        return;
      }

      const targetId = opts?.preferAccountId ?? (opts?.preserveSelection !== false ? selectedAccountId.value : null);
      const selectOpts = { preserveFeedback: opts?.preserveFeedback };
      if (targetId && accounts.value.some((a) => a.id === targetId)) {
        selectAccount(targetId, selectOpts);
      } else if (accounts.value.length > 0 && !isCreating.value) {
        selectAccount(accounts.value[0].id, selectOpts);
      } else if (accounts.value.length > 0 && selectedAccountId.value === null && !opts?.initialPresetId) {
        selectAccount(accounts.value[0].id, selectOpts);
      } else if (accounts.value.length === 0) {
        startCreate(opts?.initialPresetId ?? undefined);
      }
    } catch (err) {
      if (isLockedError(err)) {
        options.onLocked?.();
      }
      loadError.value = sanitizeSensitiveError(extractErrorMessage(err), form.value);
    } finally {
      loading.value = false;
    }
  }

  async function init(opts?: {
    initialAccountId?: string | null;
    initialPresetId?: string | null;
  }): Promise<void> {
    if (!initialized) {
      initialized = true;
      unlistenAccountEvent = await onAccountEvent(handleAccountEvent);
    }
    await loadAccounts({
      preserveSelection: false,
      preferAccountId: opts?.initialAccountId ?? undefined,
      initialPresetId: opts?.initialPresetId ?? null,
    });
  }

  function dispose(): void {
    if (unlistenAccountEvent) {
      unlistenAccountEvent();
      unlistenAccountEvent = null;
    }
    initialized = false;
  }

  if (getCurrentInstance()) {
    onUnmounted(() => dispose());
  }

  function validateCurrentForm(): boolean {
    const res = validateAccountForm(form.value);
    validationErrors.value = res.errors;
    if (!res.valid && res.firstError) {
      statusBanner.value = { kind: "error", text: res.firstError };
    }
    return res.valid;
  }

  async function verifyCurrentAccount(): Promise<VerifyAccountOutput | null> {
    if (!validateCurrentForm()) return null;
    verifying.value = true;
    verifyResult.value = null;
    saveError.value = null;
    try {
      const payload = buildSaveAccountInput(form.value);
      const res = await verifyAccount(payload);
      verifyResult.value = res;
      statusBanner.value = {
        kind: res.ok ? "success" : "error",
        text: sanitizeSensitiveError(res.message, form.value),
      };
      return res;
    } catch (err) {
      if (isLockedError(err)) {
        options.onLocked?.();
      }
      const message = sanitizeSensitiveError(extractErrorMessage(err), form.value);
      const failed = { ok: false, message };
      verifyResult.value = failed;
      statusBanner.value = { kind: "error", text: message };
      return failed;
    } finally {
      verifying.value = false;
    }
  }

  async function saveCurrentAccount(): Promise<AccountConfigDto | null> {
    if (!validateCurrentForm()) return null;
    saving.value = true;
    saveError.value = null;
    try {
      const payload = buildSaveAccountInput(form.value);
      const res = await saveAccount(payload);
      stateVersion.value = res.stateVersion;

      const idx = accounts.value.findIndex((a) => a.id === res.account.id);
      if (idx >= 0) {
        accounts.value[idx] = res.account;
      } else {
        accounts.value.push(res.account);
      }

      isCreating.value = false;
      selectedAccountId.value = res.account.id;
      lockedFields.value = new Set<LockableAccountField>([
        ...PRESET_LOCKABLE_FIELDS,
        "imapUsername",
        "smtpUsername",
      ]);
      // 保存成功后立即清空内存中的密码输入，保留 hasPassword 状态
      form.value = accountDtoToForm(res.account, presets.value);
      validationErrors.value = {};
      statusBanner.value = {
        kind: "success",
        text: `已保存账号 ${res.account.email}`,
      };
      options.onChanged?.();
      return res.account;
    } catch (err) {
      if (isLockedError(err)) {
        options.onLocked?.();
      }
      const message = sanitizeSensitiveError(extractErrorMessage(err), form.value);
      saveError.value = message;
      statusBanner.value = { kind: "error", text: message };
      return null;
    } finally {
      saving.value = false;
    }
  }

  async function toggleAccountEnabled(
    accountId: string,
    enabled?: boolean
  ): Promise<boolean> {
    const target = accounts.value.find((a) => a.id === accountId);
    if (!target) return false;
    const nextEnabled = enabled ?? !target.enabled;
    togglingEnabledId.value = accountId;
    try {
      const res = await setAccountEnabled({ accountId, enabled: nextEnabled });
      stateVersion.value = res.stateVersion;
      target.enabled = res.enabled;
      if (form.value.id === accountId) {
        form.value.enabled = res.enabled;
      }
      statusBanner.value = {
        kind: "info",
        text: res.enabled ? `已启用 ${target.email}` : `已停用 ${target.email}`,
      };
      options.onChanged?.();
      return true;
    } catch (err) {
      if (isLockedError(err)) {
        options.onLocked?.();
      }
      statusBanner.value = {
        kind: "error",
        text: sanitizeSensitiveError(extractErrorMessage(err), form.value),
      };
      return false;
    } finally {
      togglingEnabledId.value = null;
    }
  }

  function requestDelete(accountId: string): void {
    confirmDeleteId.value = accountId;
  }

  function cancelDelete(): void {
    confirmDeleteId.value = null;
  }

  async function confirmDelete(accountId?: string): Promise<boolean> {
    const targetId = accountId ?? confirmDeleteId.value;
    if (!targetId) return false;
    deletingId.value = targetId;
    try {
      const removed = accounts.value.find((a) => a.id === targetId);
      const res = await deleteAccount({ accountId: targetId });
      stateVersion.value = res.stateVersion;
      confirmDeleteId.value = null;

      if (!res.deleted) return false;

      accounts.value = accounts.value.filter((a) => a.id !== targetId);
      if (selectedAccountId.value === targetId) {
        if (accounts.value.length > 0) {
          selectAccount(accounts.value[0].id);
        } else {
          startCreate();
        }
      }
      statusBanner.value = {
        kind: "info",
        text: removed ? `已删除账号 ${removed.email}` : "已删除账号",
      };
      options.onChanged?.();
      return true;
    } catch (err) {
      if (isLockedError(err)) {
        options.onLocked?.();
      }
      statusBanner.value = {
        kind: "error",
        text: sanitizeSensitiveError(extractErrorMessage(err), form.value),
      };
      return false;
    } finally {
      deletingId.value = null;
    }
  }

  async function startMicrosoftOAuthFlow(): Promise<boolean> {
    oauthStatus.value = "pending";
    oauthMessage.value = null;
    statusBanner.value = null;
    try {
      const res = await startMicrosoftOAuth({
        customClientId: form.value.oauthClientId.trim() || null,
        loginHintEmail: form.value.email.trim() || null,
        editingAccountId: form.value.id ?? null,
      });
      if (!res.started) {
        oauthStatus.value = "error";
        oauthMessage.value = res.message;
        statusBanner.value = { kind: "error", text: res.message };
        return false;
      }
      oauthMessage.value = res.message;
      statusBanner.value = { kind: "info", text: res.message };
      return true;
    } catch (err) {
      if (isLockedError(err)) {
        options.onLocked?.();
      }
      const message = sanitizeSensitiveError(extractErrorMessage(err), form.value);
      oauthStatus.value = "error";
      oauthMessage.value = message;
      statusBanner.value = { kind: "error", text: message };
      return false;
    }
  }

  return {
    // 状态
    accounts,
    presets,
    stateVersion,
    loading,
    loadError,
    selectedAccountId,
    selectedAccount,
    isCreating,
    form,
    lockedFields,
    activePreset,
    hasLockedPresetFields,
    validationErrors,
    verifying,
    verifyResult,
    saving,
    saveError,
    togglingEnabledId,
    confirmDeleteId,
    deletingId,
    oauthStatus,
    oauthMessage,
    statusBanner,

    // 方法
    init,
    dispose,
    loadAccounts,
    startCreate,
    selectAccount,
    updateEmail,
    markFieldEdited,
    selectPreset,
    resetToPreset,
    validateCurrentForm,
    verifyCurrentAccount,
    saveCurrentAccount,
    toggleAccountEnabled,
    requestDelete,
    cancelDelete,
    confirmDelete,
    startMicrosoftOAuthFlow,
  };
}

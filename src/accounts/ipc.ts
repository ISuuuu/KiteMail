/**
 * KiteMail 账号管理 IPC 客户端与可控 Mock 传输层。
 *
 * 严格遵循后端参数包装约定：
 *   list_accounts()                  无参
 *   verify_account({ input })        包在 input 字段
 *   save_account({ input })          包在 input 字段
 *   delete_account({ input })        包在 input 字段
 *   set_account_enabled({ input })   包在 input 字段
 *   start_microsoft_oauth({ input }) 包在 input 字段
 */

import { isTauri, extractErrorMessage, isLockedError } from "../ipc/main-window";
import type {
  AccountConfigDto,
  AccountEvent,
  DeleteAccountInput,
  DeleteAccountOutput,
  ListAccountsOutput,
  ProviderPresetDto,
  SaveAccountInput,
  SaveAccountOutput,
  SetAccountEnabledInput,
  SetAccountEnabledOutput,
  StartMicrosoftOAuthInput,
  StartMicrosoftOAuthOutput,
  VerifyAccountOutput,
} from "./types";

export { isTauri, extractErrorMessage, isLockedError };

export type UnlistenFn = () => void;

export const EVENT_ACCOUNT_EVENT = "main://account-event";

// ---------- 内置主流邮箱服务商预设 ----------

export const BUILTIN_PROVIDER_PRESETS: readonly ProviderPresetDto[] = [
  {
    id: "outlook",
    name: "Outlook / Microsoft 365",
    domains: [
      "outlook.com",
      "hotmail.com",
      "live.com",
      "msn.com",
      "outlook.cn",
      "office365.com",
    ],
    authBackend: "microsoftGraphOAuth2",
    imapHost: "outlook.office365.com",
    imapPort: 993,
    imapSecurity: "tls",
    smtpHost: "smtp.office365.com",
    smtpPort: 587,
    smtpSecurity: "startTls",
    note: "通过系统浏览器完成 Microsoft Graph OAuth2 授权登录（支持 Outlook.com / Hotmail / Live / Microsoft 365）。",
  },
  {
    id: "gmail",
    name: "Gmail",
    domains: ["gmail.com", "googlemail.com"],
    authBackend: "password",
    imapHost: "imap.gmail.com",
    imapPort: 993,
    imapSecurity: "tls",
    smtpHost: "smtp.gmail.com",
    smtpPort: 465,
    smtpSecurity: "tls",
    note: "Gmail 需在 Google 账号开启「两步验证」，并在安全性设置中生成 16 位「应用专用密码 (App Password)」。",
  },
  {
    id: "qq",
    name: "QQ 邮箱",
    domains: ["qq.com", "foxmail.com", "vip.qq.com"],
    authBackend: "password",
    imapHost: "imap.qq.com",
    imapPort: 993,
    imapSecurity: "tls",
    smtpHost: "smtp.qq.com",
    smtpPort: 465,
    smtpSecurity: "tls",
    note: "QQ 邮箱需在网页版「设置 → 账号」开启 IMAP/SMTP 服务，并填写生成的 16 位授权码（非 QQ 登录密码）。",
  },
  {
    id: "netease163",
    name: "网易 163 邮箱",
    domains: ["163.com", "vip.163.com"],
    authBackend: "password",
    imapHost: "imap.163.com",
    imapPort: 993,
    imapSecurity: "tls",
    smtpHost: "smtp.163.com",
    smtpPort: 465,
    smtpSecurity: "tls",
    note: "网易 163 需在网页版「设置 → POP3/SMTP/IMAP」开启 IMAP/SMTP 服务，并填写客户端授权密码。",
  },
  {
    id: "netease126",
    name: "网易 126 / Yeah",
    domains: ["126.com", "yeah.net"],
    authBackend: "password",
    imapHost: "imap.126.com",
    imapPort: 993,
    imapSecurity: "tls",
    smtpHost: "smtp.126.com",
    smtpPort: 465,
    smtpSecurity: "tls",
    note: "网易 126 需在网页版「设置 → POP3/SMTP/IMAP」开启 IMAP/SMTP 服务，并填写客户端授权密码。",
  },
  {
    id: "icloud",
    name: "iCloud 邮箱",
    domains: ["icloud.com", "me.com", "mac.com"],
    authBackend: "password",
    imapHost: "imap.mail.me.com",
    imapPort: 993,
    imapSecurity: "tls",
    smtpHost: "smtp.mail.me.com",
    smtpPort: 587,
    smtpSecurity: "startTls",
    note: "iCloud 需在 account.apple.com 生成「App 专用密码」后填写，不支持直接使用 Apple ID 登录密码。",
  },
  {
    id: "exmail",
    name: "腾讯企业邮",
    domains: ["exmail.qq.com"],
    authBackend: "password",
    imapHost: "imap.exmail.qq.com",
    imapPort: 993,
    imapSecurity: "tls",
    smtpHost: "smtp.exmail.qq.com",
    smtpPort: 465,
    smtpSecurity: "tls",
    note: "若管理员开启了安全登录或微信绑定，请在腾讯企业邮网页端「设置 → 账号安全」生成客户端专用密码。",
  },
  {
    id: "aliyun",
    name: "阿里邮箱 / 企业邮",
    domains: ["aliyun.com", "qiye.aliyun.com"],
    authBackend: "password",
    imapHost: "imap.qiye.aliyun.com",
    imapPort: 993,
    imapSecurity: "tls",
    smtpHost: "smtp.qiye.aliyun.com",
    smtpPort: 465,
    smtpSecurity: "tls",
    note: "请使用完整邮箱地址作为用户名；如开启三方客户端安全密码，请在网页端生成专用密码。",
  },
  {
    id: "manual",
    name: "通用 IMAP / SMTP（手动配置）",
    domains: [],
    authBackend: "password",
    imapHost: "",
    imapPort: 993,
    imapSecurity: "tls",
    smtpHost: "",
    smtpPort: 465,
    smtpSecurity: "tls",
    note: "手动填写邮件服务商提供的 IMAP 与 SMTP 服务器地址、端口与加密方式。",
  },
];

// ---------- Tauri invoke / listen ----------

async function tauriInvoke<T>(cmd: string, args?: unknown): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args as Record<string, unknown>);
}

async function tauriListen<T>(
  event: string,
  handler: (payload: T) => void
): Promise<UnlistenFn> {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<T>(event, (ev) => handler(ev.payload));
}

// ---------- 可控 Mock 传输层 ----------

const DEFAULT_MOCK_ACCOUNTS: AccountConfigDto[] = [
  {
    id: "1",
    authBackend: "password",
    oauthClientId: null,
    displayName: "测试工作邮箱",
    email: "demo@kitemail.example",
    label: "工作",
    enabled: true,
    inUnified: true,
    imapHost: "imap.kitemail.example",
    imapPort: 993,
    imapSecurity: "tls",
    imapUsername: "demo@kitemail.example",
    smtpHost: "smtp.kitemail.example",
    smtpPort: 465,
    smtpSecurity: "tls",
    smtpUsername: "demo@kitemail.example",
    smtpSeparateCredentials: false,
    signature: "-- \nSent from KiteMail",
    hasPassword: true,
  },
];

function cloneAccount(acc: AccountConfigDto): AccountConfigDto {
  return { ...acc };
}

function clonePreset(preset: ProviderPresetDto): ProviderPresetDto {
  return { ...preset, domains: [...preset.domains] };
}

export type MockOAuthMode = "autoSuccess" | "rebindRejected" | "manual";

export interface MockAccountsOptions {
  accounts?: AccountConfigDto[];
  presets?: ProviderPresetDto[];
  locked?: boolean;
  stateVersion?: number;
  oauthMode?: MockOAuthMode;
  oauthDelayMs?: number;
  verifyResponder?: (input: SaveAccountInput) => VerifyAccountOutput | Promise<VerifyAccountOutput>;
}

let mockStateVersion = 1;
let mockLocked = false;
let mockNextId = 2;
let mockAccounts: AccountConfigDto[] = DEFAULT_MOCK_ACCOUNTS.map(cloneAccount);
let mockPresets: ProviderPresetDto[] = BUILTIN_PROVIDER_PRESETS.map(clonePreset);
let mockOAuthMode: MockOAuthMode = "autoSuccess";
let mockOAuthDelayMs = 20;
let mockVerifyResponder: MockAccountsOptions["verifyResponder"] | null = null;
const mockOAuthTimers = new Set<ReturnType<typeof setTimeout>>();

type AccountEventHandler = (payload: AccountEvent) => void;
const mockAccountListeners: AccountEventHandler[] = [];

function emitAccountEventLocal(event: AccountEvent): void {
  for (const fn of [...mockAccountListeners]) {
    fn({ ...event });
  }
}

function makeLockedError() {
  return {
    code: "LOCKED",
    message: "应用已锁定，请先解锁后再管理邮箱账号",
    retryable: false,
  };
}

function makeInvalidInputError(message: string) {
  return {
    code: "INVALID_INPUT",
    message,
    retryable: false,
  };
}

function computeNextNumericId(accounts: readonly AccountConfigDto[]): number {
  let maxId = 0;
  for (const acc of accounts) {
    const num = Number.parseInt(acc.id, 10);
    if (Number.isFinite(num) && num > maxId) {
      maxId = num;
    }
  }
  return maxId + 1;
}

export function resetMockAccounts(options?: MockAccountsOptions): void {
  for (const timer of mockOAuthTimers) {
    clearTimeout(timer);
  }
  mockOAuthTimers.clear();

  mockAccounts = (options?.accounts ?? DEFAULT_MOCK_ACCOUNTS).map(cloneAccount);
  mockPresets = (options?.presets ?? BUILTIN_PROVIDER_PRESETS).map(clonePreset);
  mockLocked = options?.locked ?? false;
  mockStateVersion = options?.stateVersion ?? 1;
  mockNextId = computeNextNumericId(mockAccounts);
  mockOAuthMode = options?.oauthMode ?? "autoSuccess";
  mockOAuthDelayMs = options?.oauthDelayMs ?? 20;
  mockVerifyResponder = options?.verifyResponder ?? null;
}

export function emitMockAccountEvent(event: AccountEvent): void {
  emitAccountEventLocal(event);
}

// ---------- 公共 IPC API ----------

export async function listAccounts(): Promise<ListAccountsOutput> {
  if (isTauri()) return tauriInvoke<ListAccountsOutput>("list_accounts");
  if (mockLocked) return Promise.reject(makeLockedError());
  return {
    accounts: mockAccounts.map(cloneAccount),
    presets: mockPresets.map(clonePreset),
    stateVersion: mockStateVersion,
  };
}

export async function verifyAccount(input: SaveAccountInput): Promise<VerifyAccountOutput> {
  if (isTauri()) return tauriInvoke<VerifyAccountOutput>("verify_account", { input });
  if (mockLocked) return Promise.reject(makeLockedError());

  if (mockVerifyResponder) {
    const result = await mockVerifyResponder(input);
    emitAccountEventLocal({
      stateVersion: mockStateVersion,
      kind: "verified",
      ok: result.ok,
      accountId: input.id ?? null,
      email: input.email,
      message: result.message,
    });
    return result;
  }

  const email = input.email.trim();
  const isOAuth = input.authBackend === "microsoftGraphOAuth2";
  const existing = input.id ? mockAccounts.find((a) => a.id === input.id) : undefined;
  const hasImapPwd = Boolean(input.imapPassword && input.imapPassword.length > 0) || Boolean(existing?.hasPassword);

  let ok = true;
  let message = isOAuth
    ? "Microsoft Graph 连接与授权状态验证成功"
    : "IMAP 与 SMTP 服务器连接验证成功";

  if (!email.includes("@")) {
    ok = false;
    message = "邮箱地址格式无效，无法验证连接";
  } else if (!input.imapHost.trim() || !input.smtpHost.trim()) {
    ok = false;
    message = "请填写完整的 IMAP 与 SMTP 服务器地址";
  } else if (
    !Number.isInteger(input.imapPort) ||
    input.imapPort < 1 ||
    input.imapPort > 65535 ||
    !Number.isInteger(input.smtpPort) ||
    input.smtpPort < 1 ||
    input.smtpPort > 65535
  ) {
    ok = false;
    message = "IMAP/SMTP 端口号必须在 1 到 65535 之间";
  } else if (!isOAuth && !hasImapPwd) {
    ok = false;
    message = "请输入 IMAP 密码或授权码后再测试连接";
  } else if (input.imapHost.includes("fail.invalid") || input.smtpHost.includes("fail.invalid")) {
    ok = false;
    message = "无法连接到指定邮件服务器，请检查主机名、端口与加密方式";
  }

  emitAccountEventLocal({
    stateVersion: mockStateVersion,
    kind: "verified",
    ok,
    accountId: input.id ?? null,
    email,
    message,
  });

  return { ok, message };
}

export async function saveAccount(input: SaveAccountInput): Promise<SaveAccountOutput> {
  if (isTauri()) return tauriInvoke<SaveAccountOutput>("save_account", { input });
  if (mockLocked) return Promise.reject(makeLockedError());

  const email = input.email.trim();
  if (!email) {
    return Promise.reject(makeInvalidInputError("邮箱地址不能为空"));
  }

  const existingIdx = input.id ? mockAccounts.findIndex((a) => a.id === input.id) : -1;
  const existing = existingIdx >= 0 ? mockAccounts[existingIdx] : undefined;

  const isOAuth = input.authBackend === "microsoftGraphOAuth2";
  const providedImapPassword = Boolean(input.imapPassword && input.imapPassword.length > 0);
  const separate = Boolean(input.smtpSeparateCredentials);

  if (!isOAuth && !existing?.hasPassword && !providedImapPassword) {
    return Promise.reject(makeInvalidInputError("新增账号必须填写密码或授权码"));
  }

  const saved: AccountConfigDto = {
    id: existing ? existing.id : String(mockNextId++),
    authBackend: input.authBackend,
    oauthClientId: input.oauthClientId?.trim() ? input.oauthClientId.trim() : null,
    displayName: input.displayName.trim() || email,
    email,
    label: input.label?.trim() ?? "",
    enabled: input.enabled ?? existing?.enabled ?? true,
    inUnified: input.inUnified ?? existing?.inUnified ?? true,
    imapHost: input.imapHost.trim(),
    imapPort: input.imapPort,
    imapSecurity: input.imapSecurity,
    imapUsername: input.imapUsername.trim() || email,
    smtpHost: input.smtpHost.trim(),
    smtpPort: input.smtpPort,
    smtpSecurity: input.smtpSecurity,
    smtpUsername: separate ? input.smtpUsername.trim() || email : input.imapUsername.trim() || email,
    smtpSeparateCredentials: separate,
    signature: input.signature ?? existing?.signature ?? "",
    hasPassword: isOAuth ? Boolean(existing?.hasPassword) : Boolean(providedImapPassword || existing?.hasPassword),
  };

  if (existingIdx >= 0) {
    mockAccounts[existingIdx] = saved;
  } else {
    mockAccounts.push(saved);
  }

  mockStateVersion += 1;
  emitAccountEventLocal({
    stateVersion: mockStateVersion,
    kind: "saved",
    ok: true,
    accountId: saved.id,
    email: saved.email,
    message: `账号 ${saved.email} 已保存`,
  });

  return {
    account: cloneAccount(saved),
    stateVersion: mockStateVersion,
  };
}

export async function deleteAccount(input: DeleteAccountInput): Promise<DeleteAccountOutput> {
  if (isTauri()) return tauriInvoke<DeleteAccountOutput>("delete_account", { input });
  if (mockLocked) return Promise.reject(makeLockedError());

  const idx = mockAccounts.findIndex((a) => a.id === input.accountId);
  if (idx < 0) {
    return { deleted: false, stateVersion: mockStateVersion };
  }
  const [removed] = mockAccounts.splice(idx, 1);
  mockStateVersion += 1;
  emitAccountEventLocal({
    stateVersion: mockStateVersion,
    kind: "deleted",
    ok: true,
    accountId: removed.id,
    email: removed.email,
    message: `已移除账号 ${removed.email}`,
  });
  return { deleted: true, stateVersion: mockStateVersion };
}

export async function setAccountEnabled(
  input: SetAccountEnabledInput
): Promise<SetAccountEnabledOutput> {
  if (isTauri()) return tauriInvoke<SetAccountEnabledOutput>("set_account_enabled", { input });
  if (mockLocked) return Promise.reject(makeLockedError());

  const target = mockAccounts.find((a) => a.id === input.accountId);
  if (!target) {
    return Promise.reject(makeInvalidInputError(`未找到账号 #${input.accountId}`));
  }
  target.enabled = input.enabled;
  mockStateVersion += 1;
  emitAccountEventLocal({
    stateVersion: mockStateVersion,
    kind: "updated",
    ok: true,
    accountId: target.id,
    email: target.email,
    message: input.enabled ? `已启用账号 ${target.email}` : `已停用账号 ${target.email}`,
  });
  return {
    accountId: target.id,
    enabled: target.enabled,
    stateVersion: mockStateVersion,
  };
}

export async function startMicrosoftOAuth(
  input: StartMicrosoftOAuthInput
): Promise<StartMicrosoftOAuthOutput> {
  if (isTauri()) {
    return tauriInvoke<StartMicrosoftOAuthOutput>("start_microsoft_oauth", { input });
  }
  if (mockLocked) return Promise.reject(makeLockedError());

  if (mockOAuthMode !== "manual") {
    const timer = setTimeout(() => {
      mockOAuthTimers.delete(timer);
      if (mockLocked) return;

      if (mockOAuthMode === "rebindRejected") {
        emitAccountEventLocal({
          stateVersion: mockStateVersion,
          kind: "oauthRebindRejected",
          ok: false,
          accountId: input.editingAccountId ?? null,
          email: input.loginHintEmail ?? null,
          message: "授权返回的 Microsoft 账号邮箱与当前编辑账号不一致，已拒绝改绑",
        });
        return;
      }

      const resolvedEmail = input.loginHintEmail?.trim() || "outlook-user@outlook.com";
      let accountId = input.editingAccountId ?? null;
      const existing = accountId ? mockAccounts.find((a) => a.id === accountId) : undefined;
      if (existing) {
        existing.authBackend = "microsoftGraphOAuth2";
        existing.oauthClientId = input.customClientId?.trim() || null;
        existing.hasPassword = true;
      } else {
        const created: AccountConfigDto = {
          id: String(mockNextId++),
          authBackend: "microsoftGraphOAuth2",
          oauthClientId: input.customClientId?.trim() || null,
          displayName: resolvedEmail.split("@")[0] || "Outlook",
          email: resolvedEmail,
          label: "",
          enabled: true,
          inUnified: true,
          imapHost: "outlook.office365.com",
          imapPort: 993,
          imapSecurity: "tls",
          imapUsername: resolvedEmail,
          smtpHost: "smtp.office365.com",
          smtpPort: 587,
          smtpSecurity: "startTls",
          smtpUsername: resolvedEmail,
          smtpSeparateCredentials: false,
          signature: "",
          hasPassword: true,
        };
        mockAccounts.push(created);
        accountId = created.id;
      }

      mockStateVersion += 1;
      emitAccountEventLocal({
        stateVersion: mockStateVersion,
        kind: "oauthReauthorized",
        ok: true,
        accountId,
        email: resolvedEmail,
        message: `Microsoft Graph OAuth2 授权成功（${resolvedEmail}）`,
      });
    }, mockOAuthDelayMs);

    mockOAuthTimers.add(timer);
  }

  return {
    started: true,
    message: "已在系统浏览器中发起 Microsoft OAuth2 授权，请在浏览器中完成登录",
  };
}

export async function onAccountEvent(
  handler: (event: AccountEvent) => void
): Promise<UnlistenFn> {
  if (isTauri()) {
    return tauriListen<AccountEvent>(EVENT_ACCOUNT_EVENT, handler);
  }
  mockAccountListeners.push(handler);
  return () => {
    const idx = mockAccountListeners.indexOf(handler);
    if (idx >= 0) mockAccountListeners.splice(idx, 1);
  };
}

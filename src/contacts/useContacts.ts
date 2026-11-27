import { computed, getCurrentInstance, onUnmounted, ref } from "vue";
import {
  deleteContact,
  exportContacts,
  extractErrorMessage,
  importContacts,
  isLockedError,
  listContacts,
  saveContact,
} from "./ipc";
import type {
  ContactDto,
  ContactExportFormat,
  ContactFormModel,
  ContactImportFormat,
  ContactStatusBanner,
  ContactValidationErrors,
  ContactValidationResult,
  ExportContactsOutput,
  ImportContactsOutput,
  SaveContactInput,
} from "./types";

const EMAIL_PATTERN = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
const PAGE_LIMIT = 50;

// ---------- 纯函数工具（可在 Node 环境直接单测） ----------

export function createEmptyContactForm(): ContactFormModel {
  return {
    id: null,
    name: "",
    email: "",
    phone: "",
    organization: "",
    jobTitle: "",
    groupsText: "",
    notes: "",
    isUserContact: true,
    messageCount: 0,
    lastContactTs: 0,
  };
}

export function contactToForm(contact: ContactDto): ContactFormModel {
  return {
    id: contact.id,
    name: contact.name || contact.email.split("@")[0] || "",
    email: contact.email,
    phone: contact.phone,
    organization: contact.organization,
    jobTitle: contact.jobTitle,
    groupsText: contact.groups.join(", "),
    notes: contact.notes,
    isUserContact: contact.isUserContact,
    messageCount: contact.messageCount,
    lastContactTs: contact.lastContactTs,
  };
}

export function parseGroupsText(raw: string): string[] {
  return raw
    .split(/[,，;；]/)
    .map((item) => item.trim())
    .filter((item, idx, arr) => item.length > 0 && arr.indexOf(item) === idx);
}

export function validateContactForm(
  form: ContactFormModel
): ContactValidationResult {
  const errors: ContactValidationErrors = {};
  const email = form.email.trim();
  const name = form.name.trim();

  if (!name) {
    errors.name = "请输入联系人姓名";
  }
  if (!email) {
    errors.email = "请输入邮箱地址";
  } else if (!EMAIL_PATTERN.test(email)) {
    errors.email = "邮箱地址格式不正确";
  }

  const firstError = errors.name ?? errors.email ?? null;
  return {
    valid: Object.keys(errors).length === 0,
    errors,
    firstError,
  };
}

export function buildSaveContactInput(form: ContactFormModel): SaveContactInput {
  return {
    id: form.id,
    name: form.name.trim(),
    email: form.email.trim(),
    phone: form.phone.trim(),
    organization: form.organization.trim(),
    jobTitle: form.jobTitle.trim(),
    groups: parseGroupsText(form.groupsText),
    notes: form.notes.trim(),
  };
}

export function getContactKey(contact: Pick<ContactDto, "id" | "email">): string {
  return contact.id !== null
    ? `id:${contact.id}`
    : `auto:${contact.email.trim().toLowerCase()}`;
}

export interface UseContactsOptions {
  onLocked?: () => void;
  onChanged?: () => void;
}

export function useContacts(options?: UseContactsOptions) {
  const contacts = ref<ContactDto[]>([]);
  const total = ref(0);
  const hasMore = ref(false);
  const query = ref("");
  const loading = ref(false);
  const loadingMore = ref(false);
  const saving = ref(false);
  const deletingId = ref<number | null>(null);
  const importing = ref(false);
  const exporting = ref(false);
  const loadError = ref<string | null>(null);
  const statusBanner = ref<ContactStatusBanner | null>(null);

  const selectedKey = ref<string | null>(null);
  const isCreating = ref(false);
  const confirmDeleteId = ref<number | null>(null);
  const form = ref<ContactFormModel>(createEmptyContactForm());
  const validationErrors = ref<ContactValidationErrors>({});

  const importPanelOpen = ref(false);
  const importFormat = ref<ContactImportFormat>("auto");
  const importContent = ref("");
  const lastImportResult = ref<ImportContactsOutput | null>(null);
  const lastExportResult = ref<ExportContactsOutput | null>(null);

  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  let disposed = false;

  const selectedContact = computed<ContactDto | null>(() => {
    if (!selectedKey.value) return null;
    return contacts.value.find((c) => getContactKey(c) === selectedKey.value) ?? null;
  });

  function handleError(err: unknown, fallbackPrefix: string): string {
    if (isLockedError(err)) {
      const msg = extractErrorMessage(err);
      statusBanner.value = { kind: "error", text: msg };
      options?.onLocked?.();
      return msg;
    }
    const msg = `${fallbackPrefix}：${extractErrorMessage(err)}`;
    statusBanner.value = { kind: "error", text: msg };
    return msg;
  }

  async function loadContacts(reset = true): Promise<void> {
    if (reset) {
      loading.value = true;
      loadError.value = null;
    } else {
      loadingMore.value = true;
    }
    try {
      const offset = reset ? 0 : contacts.value.length;
      const res = await listContacts({
        query: query.value,
        offset,
        limit: PAGE_LIMIT,
      });
      if (disposed) return;

      contacts.value = reset ? res.items : [...contacts.value, ...res.items];
      total.value = res.total;
      hasMore.value = res.hasMore;

      if (!isCreating.value) {
        if (selectedKey.value) {
          const matched = contacts.value.find(
            (c) => getContactKey(c) === selectedKey.value
          );
          if (matched) {
            form.value = contactToForm(matched);
          } else if (contacts.value.length > 0) {
            selectContact(contacts.value[0]);
          } else {
            selectedKey.value = null;
            startCreate();
          }
        } else if (contacts.value.length > 0) {
          selectContact(contacts.value[0]);
        } else {
          startCreate();
        }
      }
    } catch (err) {
      if (disposed) return;
      if (isLockedError(err)) {
        loadError.value = extractErrorMessage(err);
        options?.onLocked?.();
      } else {
        loadError.value = extractErrorMessage(err);
      }
    } finally {
      if (!disposed) {
        loading.value = false;
        loadingMore.value = false;
      }
    }
  }

  async function init(): Promise<void> {
    statusBanner.value = null;
    validationErrors.value = {};
    confirmDeleteId.value = null;
    importPanelOpen.value = false;
    await loadContacts(true);
  }

  function setQuery(nextQuery: string, immediate = false): void {
    query.value = nextQuery;
    if (searchTimer) {
      clearTimeout(searchTimer);
      searchTimer = null;
    }
    if (immediate) {
      void loadContacts(true);
      return;
    }
    searchTimer = setTimeout(() => {
      searchTimer = null;
      void loadContacts(true);
    }, 180);
  }

  function selectContact(contact: ContactDto): void {
    isCreating.value = false;
    importPanelOpen.value = false;
    confirmDeleteId.value = null;
    validationErrors.value = {};
    selectedKey.value = getContactKey(contact);
    form.value = contactToForm(contact);
  }

  function startCreate(): void {
    isCreating.value = true;
    importPanelOpen.value = false;
    confirmDeleteId.value = null;
    validationErrors.value = {};
    selectedKey.value = null;
    form.value = createEmptyContactForm();
  }

  async function saveCurrentContact(): Promise<ContactDto | null> {
    const check = validateContactForm(form.value);
    validationErrors.value = check.errors;
    if (!check.valid) {
      statusBanner.value = {
        kind: "error",
        text: check.firstError ?? "请检查联系人表单字段",
      };
      return null;
    }

    saving.value = true;
    statusBanner.value = null;
    try {
      const res = await saveContact(buildSaveContactInput(form.value));
      if (disposed) return null;
      isCreating.value = false;
      selectedKey.value = getContactKey(res.contact);
      form.value = contactToForm(res.contact);
      await loadContacts(true);
      statusBanner.value = {
        kind: "success",
        text: `已保存联系人 ${res.contact.name} (${res.contact.email})`,
      };
      options?.onChanged?.();
      return res.contact;
    } catch (err) {
      handleError(err, "保存联系人失败");
      return null;
    } finally {
      saving.value = false;
    }
  }

  function requestDelete(contactId: number): void {
    confirmDeleteId.value = contactId;
  }

  function cancelDelete(): void {
    confirmDeleteId.value = null;
  }

  async function confirmDelete(): Promise<boolean> {
    const id = confirmDeleteId.value;
    if (id == null) return false;

    deletingId.value = id;
    statusBanner.value = null;
    try {
      const res = await deleteContact({ contactId: id });
      if (disposed) return false;
      confirmDeleteId.value = null;
      if (res.deleted) {
        selectedKey.value = null;
        await loadContacts(true);
        statusBanner.value = { kind: "success", text: "已删除联系人" };
        options?.onChanged?.();
        return true;
      }
      statusBanner.value = { kind: "info", text: "未找到可删除的个人联系人" };
      return false;
    } catch (err) {
      handleError(err, "删除联系人失败");
      return false;
    } finally {
      deletingId.value = null;
    }
  }

  function openImportPanel(): void {
    importPanelOpen.value = true;
    statusBanner.value = null;
  }

  function closeImportPanel(): void {
    importPanelOpen.value = false;
  }

  async function runImport(
    contentOverride?: string,
    formatOverride?: ContactImportFormat
  ): Promise<ImportContactsOutput | null> {
    const content = contentOverride ?? importContent.value;
    const format = formatOverride ?? importFormat.value;
    if (!content.trim()) {
      statusBanner.value = { kind: "error", text: "请先选择文件或粘贴 vCard / CSV 内容" };
      return null;
    }

    importing.value = true;
    statusBanner.value = null;
    try {
      const res = await importContacts({ format, content });
      if (disposed) return null;
      lastImportResult.value = res;
      importContent.value = "";
      importPanelOpen.value = false;
      await loadContacts(true);
      statusBanner.value = {
        kind: "success",
        text: `导入完成：解析 ${res.totalParsed} 条，新增 ${res.added} 条，跳过 ${res.skipped} 条`,
      };
      options?.onChanged?.();
      return res;
    } catch (err) {
      handleError(err, "导入联系人失败");
      return null;
    } finally {
      importing.value = false;
    }
  }

  async function runExport(
    format: ContactExportFormat
  ): Promise<ExportContactsOutput | null> {
    exporting.value = true;
    statusBanner.value = null;
    try {
      const res = await exportContacts({ format });
      if (disposed) return null;
      lastExportResult.value = res;
      statusBanner.value = {
        kind: "success",
        text: `已生成 ${res.filename}（共 ${res.count} 位联系人）`,
      };
      return res;
    } catch (err) {
      handleError(err, "导出联系人失败");
      return null;
    } finally {
      exporting.value = false;
    }
  }

  function dispose(): void {
    disposed = true;
    if (searchTimer) {
      clearTimeout(searchTimer);
      searchTimer = null;
    }
  }

  if (getCurrentInstance()) {
    onUnmounted(() => dispose());
  }

  return {
    contacts,
    total,
    hasMore,
    query,
    loading,
    loadingMore,
    saving,
    deletingId,
    importing,
    exporting,
    loadError,
    statusBanner,
    selectedKey,
    selectedContact,
    isCreating,
    confirmDeleteId,
    form,
    validationErrors,
    importPanelOpen,
    importFormat,
    importContent,
    lastImportResult,
    lastExportResult,
    init,
    loadContacts,
    setQuery,
    selectContact,
    startCreate,
    saveCurrentContact,
    requestDelete,
    cancelDelete,
    confirmDelete,
    openImportPanel,
    closeImportPanel,
    runImport,
    runExport,
    dispose,
  };
}

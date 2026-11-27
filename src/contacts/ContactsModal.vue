<template>
  <div
    v-if="open"
    class="km-contacts-backdrop"
    role="dialog"
    aria-modal="true"
    aria-labelledby="km-contacts-modal-title"
    @click.self="closeModal"
  >
    <section class="km-contacts-modal">
      <header class="km-modal-header">
        <div class="km-modal-title-group">
          <h2 id="km-contacts-modal-title" class="km-modal-title">通讯录管理</h2>
          <span class="km-modal-count">{{ state.total.value }} 位联系人</span>
        </div>

        <div class="km-modal-header-actions">
          <button
            type="button"
            class="km-btn km-btn-secondary"
            @click="state.openImportPanel()"
          >
            导入 vCard / CSV
          </button>
          <button
            type="button"
            class="km-btn km-btn-secondary"
            :disabled="state.exporting.value"
            @click="handleExport('vcard')"
          >
            导出 vCard
          </button>
          <button
            type="button"
            class="km-btn km-btn-secondary"
            :disabled="state.exporting.value"
            @click="handleExport('csv')"
          >
            导出 CSV
          </button>
          <button
            type="button"
            class="km-btn km-btn-primary"
            @click="handleStartCreate"
          >
            + 新建联系人
          </button>
          <button
            type="button"
            class="km-close-btn"
            title="关闭 (Esc)"
            aria-label="关闭通讯录窗口"
            @click="closeModal"
          >
            ✕
          </button>
        </div>
      </header>

      <div class="km-modal-body">
        <!-- 左侧：搜索与联系人列表 -->
        <aside class="km-contacts-sidebar" aria-label="联系人列表">
          <div class="km-search-box">
            <input
              ref="searchInputRef"
              :value="state.query.value"
              type="search"
              class="km-input"
              placeholder="搜索姓名、邮箱、公司或分组…"
              aria-label="搜索联系人"
              @input="onSearchInput"
            />
          </div>

          <div v-if="state.loading.value" class="km-sidebar-empty">
            正在加载通讯录…
          </div>
          <div v-else-if="state.loadError.value" class="km-sidebar-error" role="alert">
            <p>{{ state.loadError.value }}</p>
            <button type="button" class="km-btn km-btn-secondary" @click="state.loadContacts(true)">
              重试
            </button>
          </div>
          <div v-else-if="state.contacts.value.length === 0" class="km-sidebar-empty">
            <p>{{ state.query.value ? "未找到匹配的联系人" : "暂无联系人" }}</p>
            <button type="button" class="km-btn km-btn-primary" @click="handleStartCreate">
              新建联系人
            </button>
          </div>
          <template v-else>
            <ul class="km-contact-list">
              <li
                v-for="item in state.contacts.value"
                :key="getContactKey(item)"
                class="km-contact-item"
                :class="{
                  active: !state.isCreating.value && state.selectedKey.value === getContactKey(item),
                }"
              >
                <button
                  type="button"
                  class="km-contact-select-btn"
                  @click="state.selectContact(item)"
                >
                  <span class="km-contact-avatar" aria-hidden="true">
                    {{ avatarInitial(item.name || item.email) }}
                  </span>
                  <span class="km-contact-info">
                    <span class="km-contact-name-row">
                      <span class="km-contact-name">{{ item.name || item.email }}</span>
                      <span
                        class="km-contact-badge"
                        :class="{ auto: !item.isUserContact }"
                      >
                        {{ item.isUserContact ? "个人" : "往来聚合" }}
                      </span>
                    </span>
                    <span class="km-contact-email">{{ item.email }}</span>
                    <span
                      v-if="item.organization || item.messageCount > 0"
                      class="km-contact-meta"
                    >
                      {{ item.organization || `往来 ${item.messageCount} 封` }}
                    </span>
                  </span>
                </button>
              </li>
            </ul>
            <div v-if="state.hasMore.value" class="km-load-more-wrap">
              <button
                type="button"
                class="km-btn km-btn-secondary km-load-more-btn"
                :disabled="state.loadingMore.value"
                @click="state.loadContacts(false)"
              >
                {{ state.loadingMore.value ? "加载中…" : "加载更多联系人" }}
              </button>
            </div>
          </template>
        </aside>

        <!-- 右侧：导入面板 或 联系人详情/编辑表单 -->
        <main class="km-contact-editor">
          <!-- 导入面板 -->
          <section v-if="state.importPanelOpen.value" class="km-import-panel">
            <div class="km-editor-header">
              <div>
                <h3 class="km-editor-title">导入联系人 (vCard / CSV)</h3>
                <p class="km-editor-subtitle">
                  支持标准 `.vcf` / `.vcard` 与 `.csv`（含表头 Name,Email,Phone,Organization,JobTitle,Groups,Notes）
                </p>
              </div>
              <button
                type="button"
                class="km-btn km-btn-ghost"
                @click="state.closeImportPanel()"
              >
                返回编辑
              </button>
            </div>

            <div class="km-grid-2">
              <div class="km-field">
                <label for="km-import-format" class="km-label">导入格式</label>
                <select
                  id="km-import-format"
                  v-model="state.importFormat.value"
                  class="km-input"
                >
                  <option value="auto">自动识别 (vCard / CSV)</option>
                  <option value="vcard">vCard (.vcf)</option>
                  <option value="csv">CSV (.csv)</option>
                </select>
              </div>

              <div class="km-field">
                <label for="km-import-file" class="km-label">从本地文件读取</label>
                <input
                  id="km-import-file"
                  type="file"
                  accept=".vcf,.vcard,.csv,text/vcard,text/csv"
                  class="km-file-input"
                  @change="onImportFileSelected"
                />
              </div>
            </div>

            <div class="km-field">
              <label for="km-import-text" class="km-label">或直接粘贴导入内容</label>
              <textarea
                id="km-import-text"
                v-model="state.importContent.value"
                class="km-textarea km-mono"
                rows="10"
                placeholder="BEGIN:VCARD&#10;VERSION:3.0&#10;FN:张三&#10;EMAIL:zhangsan@example.com&#10;END:VCARD"
              />
            </div>

            <div
              v-if="state.statusBanner.value"
              class="km-banner"
              :class="`is-${state.statusBanner.value.kind}`"
              role="status"
            >
              {{ state.statusBanner.value.text }}
            </div>

            <footer class="km-editor-footer">
              <button
                type="button"
                class="km-btn km-btn-secondary"
                @click="state.closeImportPanel()"
              >
                取消
              </button>
              <button
                type="button"
                class="km-btn km-btn-primary"
                :disabled="state.importing.value"
                @click="state.runImport()"
              >
                {{ state.importing.value ? "正在导入…" : "开始导入" }}
              </button>
            </footer>
          </section>

          <!-- 联系人编辑/详情表单 -->
          <form v-else class="km-editor-form" @submit.prevent="state.saveCurrentContact()">
            <div class="km-editor-header">
              <div>
                <h3 class="km-editor-title">
                  {{
                    state.isCreating.value
                      ? "新建联系人"
                      : state.form.value.name || state.form.value.email
                  }}
                </h3>
                <p class="km-editor-subtitle">
                  {{
                    state.isCreating.value
                      ? "填写联系人资料后保存至个人通讯录"
                      : state.form.value.isUserContact
                        ? `个人联系人${state.form.value.messageCount > 0 ? ` · 往来 ${state.form.value.messageCount} 封` : ""}`
                        : `历史往来自动聚合（${state.form.value.messageCount} 封）· 点击保存可纳入个人通讯录`
                  }}
                </p>
              </div>

              <button
                v-if="state.form.value.email.trim()"
                type="button"
                class="km-btn km-btn-primary"
                @click="handleComposeTo"
              >
                给 TA 写信
              </button>
            </div>

            <div v-if="!state.isCreating.value && !state.form.value.isUserContact" class="km-note-banner">
              此联系人来自历史邮件自动聚合，补充备注或直接保存即可升级为个人联系人。
            </div>

            <section class="km-form-section">
              <div class="km-grid-2">
                <div class="km-field">
                  <label for="km-contact-name" class="km-label">姓名 *</label>
                  <input
                    id="km-contact-name"
                    ref="nameInputRef"
                    v-model="state.form.value.name"
                    type="text"
                    class="km-input"
                    placeholder="例如 张三"
                  />
                  <span v-if="state.validationErrors.value.name" class="km-field-err">
                    {{ state.validationErrors.value.name }}
                  </span>
                </div>

                <div class="km-field">
                  <label for="km-contact-email" class="km-label">邮箱地址 *</label>
                  <input
                    id="km-contact-email"
                    v-model="state.form.value.email"
                    type="email"
                    class="km-input"
                    placeholder="name@example.com"
                  />
                  <span v-if="state.validationErrors.value.email" class="km-field-err">
                    {{ state.validationErrors.value.email }}
                  </span>
                </div>

                <div class="km-field">
                  <label for="km-contact-phone" class="km-label">电话</label>
                  <input
                    id="km-contact-phone"
                    v-model="state.form.value.phone"
                    type="text"
                    class="km-input"
                    placeholder="例如 +86 138-0000-0000"
                  />
                </div>

                <div class="km-field">
                  <label for="km-contact-org" class="km-label">公司 / 组织</label>
                  <input
                    id="km-contact-org"
                    v-model="state.form.value.organization"
                    type="text"
                    class="km-input"
                    placeholder="例如 KiteMail 团队"
                  />
                </div>

                <div class="km-field">
                  <label for="km-contact-title" class="km-label">职位</label>
                  <input
                    id="km-contact-title"
                    v-model="state.form.value.jobTitle"
                    type="text"
                    class="km-input"
                    placeholder="例如 产品经理"
                  />
                </div>

                <div class="km-field">
                  <label for="km-contact-groups" class="km-label">分组（逗号分隔）</label>
                  <input
                    id="km-contact-groups"
                    v-model="state.form.value.groupsText"
                    type="text"
                    class="km-input"
                    placeholder="工作, 项目组"
                  />
                </div>
              </div>

              <div class="km-field">
                <label for="km-contact-notes" class="km-label">备注</label>
                <textarea
                  id="km-contact-notes"
                  v-model="state.form.value.notes"
                  class="km-textarea"
                  rows="3"
                  placeholder="联系人备注信息（可选）"
                />
              </div>
            </section>

            <!-- 最近一次导出预览快捷复制 -->
            <div v-if="state.lastExportResult.value" class="km-export-box">
              <div class="km-export-box-row">
                <span>
                  最近导出：<strong>{{ state.lastExportResult.value.filename }}</strong>
                  （{{ state.lastExportResult.value.count }} 条）
                </span>
                <button
                  type="button"
                  class="km-btn km-btn-secondary"
                  @click="copyLastExport"
                >
                  {{ copiedExport ? "已复制到剪贴板" : "复制导出文本" }}
                </button>
              </div>
            </div>

            <div
              v-if="state.statusBanner.value"
              class="km-banner"
              :class="`is-${state.statusBanner.value.kind}`"
              role="status"
            >
              {{ state.statusBanner.value.text }}
            </div>

            <footer class="km-editor-footer">
              <div class="km-footer-left">
                <template
                  v-if="
                    !state.isCreating.value &&
                    state.form.value.isUserContact &&
                    state.form.value.id !== null
                  "
                >
                  <div
                    v-if="state.confirmDeleteId.value === state.form.value.id"
                    class="km-delete-confirm"
                  >
                    <span class="km-delete-confirm-text">确认删除该联系人？</span>
                    <button
                      type="button"
                      class="km-btn km-btn-danger"
                      :disabled="state.deletingId.value === state.form.value.id"
                      @click="state.confirmDelete()"
                    >
                      {{ state.deletingId.value ? "删除中…" : "确认删除" }}
                    </button>
                    <button
                      type="button"
                      class="km-btn km-btn-ghost"
                      @click="state.cancelDelete()"
                    >
                      取消
                    </button>
                  </div>
                  <button
                    v-else
                    type="button"
                    class="km-btn km-btn-danger-outline"
                    @click="state.requestDelete(state.form.value.id)"
                  >
                    删除联系人
                  </button>
                </template>
              </div>

              <div class="km-footer-right">
                <button
                  type="submit"
                  class="km-btn km-btn-primary"
                  :disabled="state.saving.value"
                >
                  {{ state.saving.value ? "正在保存…" : "保存联系人" }}
                </button>
              </div>
            </footer>
          </form>
        </main>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type { ContactExportFormat } from "./types";
import { getContactKey, useContacts } from "./useContacts";

const props = defineProps<{
  open: boolean;
}>();

const emit = defineEmits<{
  "update:open": [open: boolean];
  close: [];
  composeTo: [email: string, name: string];
  overlayChange: [id: string, open: boolean];
}>();

const OVERLAY_ID = "contacts-modal";
const searchInputRef = ref<HTMLInputElement | null>(null);
const nameInputRef = ref<HTMLInputElement | null>(null);
const copiedExport = ref(false);

const state = useContacts({
  onLocked() {
    closeModal();
  },
});

function avatarInitial(text: string): string {
  const trimmed = text.trim();
  return trimmed ? trimmed.slice(0, 1).toUpperCase() : "@";
}

function onSearchInput(ev: Event) {
  state.setQuery((ev.target as HTMLInputElement).value);
}

function handleStartCreate() {
  state.startCreate();
  void nextTick(() => {
    nameInputRef.value?.focus();
  });
}

function handleComposeTo() {
  const email = state.form.value.email.trim();
  if (!email) return;
  const name = state.form.value.name.trim() || email;
  emit("composeTo", email, name);
  closeModal();
}

function onImportFileSelected(ev: Event) {
  const input = ev.target as HTMLInputElement;
  const file = input.files?.[0];
  if (!file) return;
  if (file.name.toLowerCase().endsWith(".csv")) {
    state.importFormat.value = "csv";
  } else if (
    file.name.toLowerCase().endsWith(".vcf") ||
    file.name.toLowerCase().endsWith(".vcard")
  ) {
    state.importFormat.value = "vcard";
  }
  const reader = new FileReader();
  reader.onload = () => {
    state.importContent.value = typeof reader.result === "string" ? reader.result : "";
  };
  reader.readAsText(file, "utf-8");
  input.value = "";
}

async function handleExport(format: ContactExportFormat) {
  copiedExport.value = false;
  const res = await state.runExport(format);
  if (!res || typeof document === "undefined") return;
  if (typeof Blob !== "undefined" && typeof URL.createObjectURL === "function") {
    const mime = format === "vcard" ? "text/vcard;charset=utf-8" : "text/csv;charset=utf-8";
    const blob = new Blob([res.content], { type: mime });
    const href = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = href;
    link.download = res.filename;
    document.body.appendChild(link);
    link.click();
    link.remove();
    setTimeout(() => URL.revokeObjectURL(href), 1000);
  }
}

async function copyLastExport() {
  const content = state.lastExportResult.value?.content;
  if (!content || typeof navigator === "undefined" || !navigator.clipboard) return;
  try {
    await navigator.clipboard.writeText(content);
    copiedExport.value = true;
    setTimeout(() => {
      copiedExport.value = false;
    }, 2000);
  } catch {
    state.statusBanner.value = { kind: "error", text: "复制到剪贴板失败" };
  }
}

function closeModal() {
  if (!props.open) return;
  emit("overlayChange", OVERLAY_ID, false);
  emit("update:open", false);
  emit("close");
}

function handleKeydown(ev: KeyboardEvent) {
  if (!props.open) return;
  if (ev.key === "Escape") {
    ev.preventDefault();
    ev.stopPropagation();
    if (state.importPanelOpen.value) {
      state.closeImportPanel();
    } else {
      closeModal();
    }
  }
}

function attachGlobalListeners() {
  if (typeof window !== "undefined") {
    window.addEventListener("keydown", handleKeydown);
  }
}

function detachGlobalListeners() {
  if (typeof window !== "undefined") {
    window.removeEventListener("keydown", handleKeydown);
  }
}

watch(
  () => props.open,
  (isOpen, prevOpen) => {
    if (isOpen && !prevOpen) {
      emit("overlayChange", OVERLAY_ID, true);
      attachGlobalListeners();
      void state.init();
      void nextTick(() => searchInputRef.value?.focus());
    } else if (!isOpen && prevOpen) {
      detachGlobalListeners();
      emit("overlayChange", OVERLAY_ID, false);
    }
  }
);

onMounted(() => {
  if (props.open) {
    emit("overlayChange", OVERLAY_ID, true);
    attachGlobalListeners();
    void state.init();
    void nextTick(() => searchInputRef.value?.focus());
  }
});

onUnmounted(() => {
  detachGlobalListeners();
  if (props.open) {
    emit("overlayChange", OVERLAY_ID, false);
  }
  state.dispose();
});
</script>

<style scoped>
.km-contacts-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  background: rgba(0, 0, 0, 0.48);
  backdrop-filter: blur(4px);
}

.km-contacts-modal {
  width: min(940px, 100%);
  height: min(660px, calc(100vh - 32px));
  display: flex;
  flex-direction: column;
  border-radius: var(--km-radius-lg);
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  box-shadow: var(--km-shadow-popover);
  overflow: hidden;
}

.km-modal-header {
  height: 52px;
  padding: 0 18px;
  border-bottom: 1px solid var(--km-border);
  background: var(--km-surface);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  flex-shrink: 0;
}

.km-modal-title-group {
  display: flex;
  align-items: baseline;
  gap: 10px;
}

.km-modal-title {
  margin: 0;
  font-size: 15px;
  font-weight: 700;
  color: var(--km-text);
}

.km-modal-count {
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-modal-header-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.km-close-btn {
  width: 30px;
  height: 30px;
  border: none;
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-text-muted);
  font-size: 14px;
  cursor: pointer;
}

.km-close-btn:hover {
  background: var(--km-hover-bg);
  color: var(--km-text);
}

.km-modal-body {
  flex: 1;
  min-height: 0;
  display: flex;
}

.km-contacts-sidebar {
  width: 290px;
  min-width: 240px;
  border-right: 1px solid var(--km-border);
  background: var(--km-sidebar);
  display: flex;
  flex-direction: column;
  overflow-y: auto;
  padding: 10px;
  gap: 10px;
}

.km-search-box {
  flex-shrink: 0;
}

.km-sidebar-empty,
.km-sidebar-error {
  padding: 24px 12px;
  text-align: center;
  color: var(--km-text-muted);
  font-size: 12px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
}

.km-sidebar-error {
  color: var(--km-danger);
}

.km-contact-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.km-contact-item {
  border-radius: var(--km-radius-md);
  border: 1px solid transparent;
  padding: 6px 8px;
}

.km-contact-item:hover {
  background: var(--km-hover-bg);
}

.km-contact-item.active {
  background: var(--km-surface);
  border-color: var(--km-accent);
  box-shadow: var(--km-shadow-card);
}

.km-contact-select-btn {
  width: 100%;
  border: none;
  background: transparent;
  padding: 0;
  display: flex;
  align-items: center;
  gap: 9px;
  text-align: left;
  cursor: pointer;
}

.km-contact-avatar {
  width: 32px;
  height: 32px;
  border-radius: 999px;
  background: var(--km-accent-soft);
  color: var(--km-accent);
  font-weight: 700;
  font-size: 13px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.km-contact-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.km-contact-name-row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.km-contact-name {
  font-size: 12px;
  font-weight: 600;
  color: var(--km-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-contact-badge {
  padding: 0 5px;
  border-radius: 4px;
  font-size: 10px;
  font-weight: 600;
  background: var(--km-accent-soft);
  color: var(--km-accent);
  flex-shrink: 0;
}

.km-contact-badge.auto {
  background: var(--km-hover-bg);
  color: var(--km-text-muted);
}

.km-contact-email,
.km-contact-meta {
  font-size: 11px;
  color: var(--km-text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-load-more-wrap {
  padding-top: 4px;
}

.km-load-more-btn {
  width: 100%;
}

.km-contact-editor {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  padding: 18px 22px;
  background: var(--km-surface-raised);
}

.km-editor-form,
.km-import-panel {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.km-editor-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.km-editor-title {
  margin: 0;
  font-size: 15px;
  font-weight: 700;
  color: var(--km-text);
}

.km-editor-subtitle {
  margin: 2px 0 0;
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-note-banner {
  padding: 9px 12px;
  border-radius: var(--km-radius-sm);
  border: 1px solid color-mix(in srgb, var(--km-accent) 28%, var(--km-border));
  background: var(--km-accent-soft);
  font-size: 12px;
  color: var(--km-text);
}

.km-form-section {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding-top: 10px;
  border-top: 1px solid var(--km-border);
}

.km-grid-2 {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

.km-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.km-label {
  font-size: 12px;
  font-weight: 500;
  color: var(--km-text);
}

.km-input,
.km-textarea {
  width: 100%;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-canvas);
  color: var(--km-text);
  user-select: text;
}

.km-input {
  height: 32px;
  padding: 0 10px;
}

.km-textarea {
  padding: 8px 10px;
  resize: vertical;
  min-height: 68px;
}

.km-input:focus,
.km-textarea:focus {
  border-color: var(--km-accent);
  outline: none;
}

.km-file-input {
  font-size: 12px;
  color: var(--km-text-muted);
  padding: 4px 0;
}

.km-mono {
  font-family: var(--km-font-mono);
  font-size: 12px;
}

.km-field-err {
  font-size: 11px;
  color: var(--km-danger);
}

.km-export-box {
  padding: 8px 12px;
  border-radius: var(--km-radius-sm);
  border: 1px solid var(--km-border);
  background: var(--km-canvas);
  font-size: 12px;
}

.km-export-box-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.km-banner {
  padding: 9px 12px;
  border-radius: var(--km-radius-sm);
  font-size: 12px;
  border: 1px solid var(--km-border);
  background: var(--km-canvas);
}

.km-banner.is-success {
  border-color: color-mix(in srgb, #26a269 45%, var(--km-border));
  background: color-mix(in srgb, #26a269 12%, transparent);
}

.km-banner.is-error {
  border-color: color-mix(in srgb, var(--km-danger) 50%, var(--km-border));
  background: var(--km-danger-soft);
  color: var(--km-danger);
}

.km-banner.is-info {
  border-color: color-mix(in srgb, var(--km-accent) 45%, var(--km-border));
  background: var(--km-accent-soft);
}

.km-editor-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding-top: 12px;
  border-top: 1px solid var(--km-border);
  flex-wrap: wrap;
}

.km-footer-left,
.km-footer-right {
  display: flex;
  align-items: center;
  gap: 8px;
}

.km-delete-confirm {
  display: flex;
  align-items: center;
  gap: 6px;
}

.km-delete-confirm-text {
  font-size: 12px;
  font-weight: 600;
  color: var(--km-danger);
}

.km-btn {
  height: 32px;
  padding: 0 12px;
  border-radius: var(--km-radius-sm);
  border: 1px solid var(--km-border);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
}

.km-btn:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.km-btn-primary {
  border-color: var(--km-accent);
  background: var(--km-accent);
  color: var(--km-accent-fg);
}

.km-btn-secondary {
  background: var(--km-surface);
  color: var(--km-text);
}

.km-btn-secondary:hover:not(:disabled) {
  background: var(--km-hover-bg);
}

.km-btn-ghost {
  border-color: transparent;
  background: transparent;
  color: var(--km-accent);
}

.km-btn-ghost:hover:not(:disabled) {
  background: var(--km-hover-bg);
}

.km-btn-danger-outline {
  border-color: color-mix(in srgb, var(--km-danger) 50%, var(--km-border));
  background: transparent;
  color: var(--km-danger);
}

.km-btn-danger-outline:hover:not(:disabled) {
  background: var(--km-danger-soft);
}

.km-btn-danger {
  border-color: var(--km-danger);
  background: var(--km-danger);
  color: #ffffff;
}

@media (max-width: 760px) {
  .km-modal-body {
    flex-direction: column;
  }

  .km-contacts-sidebar {
    width: 100%;
    max-height: 200px;
    border-right: none;
    border-bottom: 1px solid var(--km-border);
  }

  .km-grid-2 {
    grid-template-columns: 1fr;
  }
}
</style>

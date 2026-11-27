<template>
  <div
    v-if="open"
    class="km-accounts-backdrop"
    role="dialog"
    aria-modal="true"
    aria-labelledby="km-accounts-modal-title"
    @click.self="requestClose"
  >
    <section class="km-accounts-modal">
      <!-- 顶部栏：标题 + 计数 + 关闭 -->
      <header class="km-modal-header">
        <div class="km-modal-title-group">
          <h2 id="km-accounts-modal-title" class="km-modal-title">邮箱账号管理</h2>
          <span class="km-modal-count">{{ state.accounts.value.length }} 个已配置账号</span>
        </div>

        <div class="km-modal-header-actions">
          <button
            type="button"
            class="km-close-btn"
            title="关闭 (Esc)"
            aria-label="关闭账号管理窗口"
            @click="requestClose"
          >
            ✕
          </button>
        </div>
      </header>

      <!-- 主体：账号列表层 ↔ 编辑层（对照 GPUI 设置窗账号页两层结构） -->
      <div class="km-modal-body">
        <AccountSettingsList
          v-if="viewLayer === 'list'"
          :state="state"
          @add="startCreateFlow"
          @select="onRowSelect"
          @toggle="(id, enabled) => void state.toggleAccountEnabled(id, enabled)"
          @retry="state.loadAccounts()"
        />
        <AccountSettingsEdit
          v-else
          :state="state"
          :editingId="state.isCreating.value ? null : state.selectedAccountId.value"
          :pendingLeave="pendingLeave !== null"
          @back="requestBack"
          @saved="onEditorSaved"
          @removed="onEditorRemoved"
          @leave-cancel="pendingLeave = null"
          @leave-discard="discardPendingLeave"
          @leave-save="saveAndLeave"
        />
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import AccountSettingsEdit from "./AccountSettingsEdit.vue";
import AccountSettingsList from "./AccountSettingsList.vue";
import { useAccounts } from "./useAccounts";

const props = withDefaults(
  defineProps<{
    open: boolean;
    initialAccountId?: string | null;
    initialPresetId?: string | null;
  }>(),
  {
    initialAccountId: null,
    initialPresetId: null,
  }
);

const emit = defineEmits<{
  close: [];
  "update:open": [open: boolean];
  changed: [];
  overlayChange: [id: string, open: boolean];
}>();

const OVERLAY_ID = "accounts-modal";

const state = useAccounts({
  onChanged() {
    emit("changed");
  },
  onLocked() {
    closeModal();
  },
});

/** 当前层级：账号列表 ↔ 编辑表单 */
const viewLayer = ref<"list" | "editor">("list");

type PendingLeave = { type: "close" } | { type: "list" } | { type: "select"; id: string };

/** 未保存确认：待执行的离开动作（对照 GPUI pending_leave_action） */
const pendingLeave = ref<PendingLeave | null>(null);

// ---- 脏检查：表单签名与基线快照对比 ----

function formSignature(): string {
  return JSON.stringify(state.form.value);
}

const baselineJson = ref(formSignature());
const dirty = computed(() => formSignature() !== baselineJson.value);

function snapshotBaseline() {
  baselineJson.value = formSignature();
}

// 选择 / 新建切换后重新取基线（覆盖 init、内部重载与宿主驱动的选择）
watch(
  () => [state.selectedAccountId.value, state.isCreating.value] as const,
  () => {
    snapshotBaseline();
  }
);

// 非编辑态下收到后台事件（启停 / OAuth 重授权等）后同步基线，避免误报未保存
watch(
  () => state.stateVersion.value,
  () => {
    if (!dirty.value) {
      snapshotBaseline();
    }
  }
);

// ---- 层级切换 ----

function goList() {
  pendingLeave.value = null;
  viewLayer.value = "list";
}

function openEditor(accountId: string) {
  state.selectAccount(accountId);
  snapshotBaseline();
  pendingLeave.value = null;
  viewLayer.value = "editor";
}

function startCreateFlow() {
  state.startCreate();
  snapshotBaseline();
  pendingLeave.value = null;
  viewLayer.value = "editor";
}

// ---- 未保存确认 ----

function requestBack() {
  if (dirty.value) {
    pendingLeave.value = { type: "list" };
    return;
  }
  goList();
}

function requestClose() {
  if (!props.open) return;
  if (viewLayer.value === "editor" && dirty.value) {
    pendingLeave.value = { type: "close" };
    return;
  }
  closeModal();
}

function onRowSelect(accountId: string) {
  if (viewLayer.value === "editor" && dirty.value) {
    pendingLeave.value = { type: "select", id: accountId };
    return;
  }
  openEditor(accountId);
}

function executePendingLeave() {
  const action = pendingLeave.value;
  pendingLeave.value = null;
  if (!action) return;
  if (action.type === "close") {
    closeModal();
  } else if (action.type === "list") {
    goList();
  } else {
    openEditor(action.id);
  }
}

function discardPendingLeave() {
  snapshotBaseline();
  executePendingLeave();
}

async function saveAndLeave() {
  const saved = await state.saveCurrentAccount();
  if (saved) {
    snapshotBaseline();
    executePendingLeave();
  } else {
    // 保存失败：留在编辑页展示错误反馈
    pendingLeave.value = null;
  }
}

function onEditorSaved() {
  snapshotBaseline();
  // 对照 GPUI：顶栏保存成功后回到账号列表
  goList();
}

function onEditorRemoved() {
  snapshotBaseline();
  goList();
}

// ---- 弹窗生命周期 ----

function closeModal() {
  if (!props.open) return;
  pendingLeave.value = null;
  viewLayer.value = "list";
  emit("overlayChange", OVERLAY_ID, false);
  emit("update:open", false);
  emit("close");
}

function handleKeydown(ev: KeyboardEvent) {
  if (!props.open) return;
  if (ev.key === "Escape") {
    ev.preventDefault();
    ev.stopPropagation();
    requestClose();
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

async function initialize() {
  await state.init({
    initialAccountId: props.initialAccountId,
    initialPresetId: props.initialPresetId,
  });
  // 直达表单：显式指定账号（编辑）或预设（新建），对照 GPUI 窄窗行为
  if (props.initialPresetId) {
    state.startCreate(props.initialPresetId);
    snapshotBaseline();
    viewLayer.value = "editor";
  } else if (props.initialAccountId) {
    snapshotBaseline();
    viewLayer.value = "editor";
  } else {
    snapshotBaseline();
    viewLayer.value = "list";
  }
}

watch(
  () => props.open,
  (isOpen, prevOpen) => {
    if (isOpen && !prevOpen) {
      emit("overlayChange", OVERLAY_ID, true);
      attachGlobalListeners();
      void initialize();
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
    void initialize();
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
.km-accounts-backdrop {
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

.km-accounts-modal {
  width: min(920px, 100%);
  height: min(848px, calc(100vh - 32px));
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
</style>

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
import { onMounted, onUnmounted, watch } from "vue";
import AccountSettingsEdit from "./AccountSettingsEdit.vue";
import AccountSettingsList from "./AccountSettingsList.vue";
import { useAccountLayers } from "./useAccountLayers";
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

// 两级层级编排（列表 ↔ 编辑、未保存守卫）与设置窗「邮箱账号」页共用同一 composable
const {
  layer: viewLayer,
  pendingLeave,
  snapshotBaseline,
  startCreateFlow,
  requestBack,
  requestClose: guardRequestClose,
  onRowSelect,
  discardPendingLeave,
  saveAndLeave,
  onEditorSaved,
  onEditorRemoved,
} = useAccountLayers({ state, onClose: closeModal });

// ---- 弹窗生命周期 ----

function requestClose() {
  if (!props.open) return;
  // 编辑中且有未保存更改：先显示「未保存提示」横幅，暂不关闭
  if (guardRequestClose()) return;
  closeModal();
}

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

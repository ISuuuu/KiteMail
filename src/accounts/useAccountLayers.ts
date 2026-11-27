/**
 * 账号两级层级编排（列表 ↔ 编辑），供独立账号弹窗与设置窗「邮箱账号」页共用。
 *
 * 职责（对照 GPUI SettingsWindow 的账号编辑流程）：
 * - 表单脏检查：以 useAccounts 表单签名为基线快照，选择 / 新建切换与保存成功后重取基线；
 * - 离开守卫（pending_leave_action）：编辑中且有未保存更改时，返回 / 关闭 / 切换账号
 *   均先挂起动作并显示「未保存提示」横幅（保存 / 放弃更改 / 取消）；
 * - 顶栏保存成功后回到列表（GPUI commit_account_editor_save(BackToAccountList)）。
 */

import { computed, ref, watch } from "vue";
import type { AccountsStore } from "./useAccounts";

export type AccountLayer = "list" | "editor";

export type PendingAccountLeave =
  | { type: "close" }
  | { type: "list" }
  | { type: "select"; id: string };

export interface UseAccountLayersOptions {
  state: AccountsStore;
  /** 执行「关闭」离开动作（弹窗宿主=关闭弹窗；设置窗宿主由壳层管理窗口生命周期，可传空）。 */
  onClose?: () => void;
}

export function useAccountLayers({ state, onClose }: UseAccountLayersOptions) {
  const layer = ref<AccountLayer>("list");
  /** 未保存确认：待执行的离开动作（对照 GPUI pending_leave_action）。 */
  const pendingLeave = ref<PendingAccountLeave | null>(null);

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
    layer.value = "list";
  }

  function openEditor(accountId: string) {
    state.selectAccount(accountId);
    snapshotBaseline();
    pendingLeave.value = null;
    layer.value = "editor";
  }

  function startCreateFlow() {
    state.startCreate();
    snapshotBaseline();
    pendingLeave.value = null;
    layer.value = "editor";
  }

  // ---- 未保存确认 ----

  function requestBack() {
    if (dirty.value) {
      pendingLeave.value = { type: "list" };
      return;
    }
    goList();
  }

  /** 宿主关闭入口：编辑中且有未保存更改时挂起动作并返回 true（宿主暂缓关闭）。 */
  function requestClose(): boolean {
    if (layer.value === "editor" && dirty.value) {
      pendingLeave.value = { type: "close" };
      return true;
    }
    return false;
  }

  function onRowSelect(accountId: string) {
    if (layer.value === "editor" && dirty.value) {
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
      onClose?.();
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

  /** 顶栏「保存」成功：按 GPUI 行为回到账号列表。 */
  function onEditorSaved() {
    snapshotBaseline();
    goList();
  }

  /** 移除成功：回到账号列表。 */
  function onEditorRemoved() {
    snapshotBaseline();
    goList();
  }

  return {
    layer,
    pendingLeave,
    dirty,
    snapshotBaseline,
    goList,
    openEditor,
    startCreateFlow,
    requestBack,
    requestClose,
    onRowSelect,
    discardPendingLeave,
    saveAndLeave,
    onEditorSaved,
    onEditorRemoved,
  };
}

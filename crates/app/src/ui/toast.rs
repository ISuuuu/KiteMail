//! 应用内 Toast 与「撤销发送」底部反馈组件（方案第 4、5 节）。
//!
//! 职责边界：
//! - `WindowToastQueue`：每窗口独立的短时提示队列（同操作键合并、最多展示 2 条、
//!   普通短提示约 4.5s 自动关闭、恢复入口不被普通提示挤掉）。
//! - `SendToastViewModel` / `render_send_toast`：由发送状态驱动的底部发送卡片，
//!   根据持久化 UTC 截止时刻计算剩余秒数与底边 3px 主题色进度线。
//! - WebView2 防遮挡几何辅助：主窗常规三栏优先落在邮件列表下缘 GPUI 区；
//!   窄窗、阅读栏在下方、专注模式与独立阅读窗固定在底部预留 GPUI 反馈区，
//!   使 WebView2 裁切矩形止于反馈区上方，提示出入不改变正文视口高度。

use std::time::{Duration, Instant};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{h_flex, v_flex, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, relative, AnyElement, App, FontWeight, IntoElement, ParentElement, SharedString,
    Styled, Window,
};

use crate::theme;

/// 普通短时 Toast 默认显示时长（4.5 秒）。
pub(crate) const TRANSIENT_TOAST_DURATION: Duration = Duration::from_millis(4500);
/// 移动 / 归档 / 删除邮件撤销提示时长（对齐既有 15 秒 Ctrl+Z 时效）。
pub(crate) const MOVE_UNDO_TOAST_DURATION: Duration = Duration::from_secs(15);
/// 同窗最多同时展示的普通 Toast 数量。
pub(crate) const MAX_VISIBLE_TOASTS: usize = 2;
/// 列表栏内嵌底部 Toast 所需的最小宽度（逻辑像素）。
pub(crate) const LIST_PANE_TOAST_MIN_WIDTH: f32 = 300.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ToastActionKind {
    UndoLastMove,
    ViewOutbox { account_id: u32, outbox_id: u32 },
    ViewDrafts,
    RecoverCompose,
    RefreshFolders { account_id: u32 },
    OpenAccountSettings { account_id: Option<u32> },
    FocusActiveCompose,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToastAction {
    pub(crate) label: SharedString,
    pub(crate) primary: bool,
    pub(crate) kind: ToastActionKind,
}

#[derive(Clone, Debug)]
pub(crate) struct AppToast {
    pub(crate) key: String,
    pub(crate) kind: ToastKind,
    pub(crate) message: SharedString,
    pub(crate) detail: Option<SharedString>,
    pub(crate) action: Option<ToastAction>,
    pub(crate) expires_at: Option<Instant>,
    pub(crate) persistent_recovery: bool,
    pub(crate) dismissible: bool,
}

#[derive(Default, Debug)]
pub(crate) struct WindowToastQueue {
    items: Vec<AppToast>,
}

impl WindowToastQueue {
    pub(crate) fn push(&mut self, toast: AppToast) {
        if let Some(existing) = self.items.iter_mut().find(|item| item.key == toast.key) {
            *existing = toast;
            return;
        }
        self.items.push(toast);
        self.trim_overflow();
    }

    pub(crate) fn push_transient(
        &mut self,
        key: impl Into<String>,
        kind: ToastKind,
        message: impl Into<SharedString>,
        action: Option<ToastAction>,
    ) {
        let now = Instant::now();
        self.push(AppToast {
            key: key.into(),
            kind,
            message: message.into(),
            detail: None,
            action,
            expires_at: Some(now + TRANSIENT_TOAST_DURATION),
            persistent_recovery: false,
            dismissible: true,
        });
    }

    pub(crate) fn push_with_duration(
        &mut self,
        key: impl Into<String>,
        kind: ToastKind,
        message: impl Into<SharedString>,
        action: Option<ToastAction>,
        duration: Duration,
    ) {
        let now = Instant::now();
        self.push(AppToast {
            key: key.into(),
            kind,
            message: message.into(),
            detail: None,
            action,
            expires_at: Some(now + duration),
            persistent_recovery: false,
            dismissible: true,
        });
    }

    pub(crate) fn push_recovery(
        &mut self,
        key: impl Into<String>,
        kind: ToastKind,
        message: impl Into<SharedString>,
        detail: Option<SharedString>,
        action: Option<ToastAction>,
    ) {
        self.push(AppToast {
            key: key.into(),
            kind,
            message: message.into(),
            detail,
            action,
            expires_at: None,
            persistent_recovery: true,
            dismissible: true,
        });
    }

    pub(crate) fn dismiss_by_key(&mut self, key: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|item| item.key != key);
        self.items.len() != before
    }

    pub(crate) fn prune_expired(&mut self, now: Instant) -> bool {
        let before = self.items.len();
        self.items
            .retain(|item| item.expires_at.is_none_or(|exp| exp > now));
        self.items.len() != before
    }

    pub(crate) fn visible(&self) -> Vec<&AppToast> {
        // 优先保留等待用户处理的恢复入口，其余按最新入队顺序截断至 MAX_VISIBLE_TOASTS。
        let mut recovery: Vec<&AppToast> = self
            .items
            .iter()
            .filter(|t| t.persistent_recovery)
            .collect();
        let mut transient: Vec<&AppToast> = self
            .items
            .iter()
            .filter(|t| !t.persistent_recovery)
            .collect();
        if recovery.len() >= MAX_VISIBLE_TOASTS {
            recovery.truncate(MAX_VISIBLE_TOASTS);
            return recovery;
        }
        let slots = MAX_VISIBLE_TOASTS - recovery.len();
        if transient.len() > slots {
            transient = transient.split_off(transient.len() - slots);
        }
        recovery.extend(transient);
        recovery
    }

    pub(crate) fn next_expiry(&self) -> Option<Instant> {
        self.items.iter().filter_map(|t| t.expires_at).min()
    }

    fn trim_overflow(&mut self) {
        // 仅清理超出上限的最旧普通短提示，绝不挤掉持久恢复入口。
        while self.items.iter().filter(|t| !t.persistent_recovery).count() > 6 {
            if let Some(pos) = self.items.iter().position(|t| !t.persistent_recovery) {
                self.items.remove(pos);
            } else {
                break;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SendToastStage {
    Saving {
        cancellable: bool,
    },
    Countdown {
        deadline_utc_secs: i64,
        total_delay_secs: u8,
    },
    Undoing,
    SendingNow,
    EnqueueFailedNeedsRecovery {
        message: SharedString,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct SendToastViewModel {
    pub(crate) token: u64,
    pub(crate) account_id: u32,
    pub(crate) outbox_id: Option<u32>,
    pub(crate) subject: SharedString,
    pub(crate) stage: SendToastStage,
    pub(crate) extra_waiting_count: usize,
    pub(crate) first_extra_target: Option<(u32, u32)>,
}

/// 根据持久化 UTC 截止时刻计算剩余整秒与进度比例（0.0 = 刚起步，1.0 = 到期）。
pub(crate) fn compute_countdown_state(
    now_utc_secs: i64,
    now_subsec_nanos: u32,
    deadline_utc_secs: i64,
    total_delay_secs: u8,
) -> (u8, f32) {
    if total_delay_secs == 0 {
        return (0, 1.0);
    }
    let total = f64::from(total_delay_secs);
    let now_exact = now_utc_secs as f64 + (now_subsec_nanos.min(999_999_999) as f64 / 1e9);
    let remaining_exact = (deadline_utc_secs as f64 - now_exact).clamp(0.0, total);
    let remaining_secs = remaining_exact.ceil().clamp(0.0, total) as u8;
    let progress = ((total - remaining_exact) / total).clamp(0.0, 1.0) as f32;
    (remaining_secs, progress)
}

/// 独立反馈条高度（随界面字号缩放，提示出入时保持恒定，不抖动 WebView2 视口）。
pub(crate) fn feedback_strip_height(ui_text_scale_percent: u32) -> f32 {
    let scale =
        kitemail_mail_core::config::clamp_ui_text_scale(ui_text_scale_percent) as f32 / 100.0;
    (74.0 * scale).round()
}

pub(crate) fn list_pane_fits_toast(list_width: f32) -> bool {
    list_width >= LIST_PANE_TOAST_MIN_WIDTH
}

/// 判断主窗当前布局是否需要底部预留固定 GPUI 反馈条来避免 WebView2 遮挡。
pub(crate) fn main_window_uses_reserved_strip(
    focus_mode: bool,
    reader_is_below: bool,
    reader_is_hidden: bool,
    auto_rail_active: bool,
    list_width: f32,
) -> bool {
    if reader_is_hidden {
        return false;
    }
    focus_mode || reader_is_below || auto_rail_active || !list_pane_fits_toast(list_width)
}

#[cfg(test)]
pub(crate) fn rects_intersect(a: [i32; 4], b: [i32; 4]) -> bool {
    a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
}

pub(crate) fn render_send_toast(
    model: &SendToastViewModel,
    compact_width: bool,
    p: &theme::HylkiPalette,
    on_undo: impl Fn(&mut Window, &mut App) + 'static,
    on_send_now: impl Fn(&mut Window, &mut App) + 'static,
    on_recover: impl Fn(&mut Window, &mut App) + 'static,
    on_view_outbox: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let now_dt = chrono::Utc::now();
    let (title, progress, is_error, show_undo, undo_disabled, show_send_now, show_recover) =
        match &model.stage {
            SendToastStage::Saving { cancellable } => (
                SharedString::from("正在保存待发送邮件…"),
                0.08_f32,
                false,
                *cancellable,
                false,
                false,
                false,
            ),
            SendToastStage::Countdown {
                deadline_utc_secs,
                total_delay_secs,
            } => {
                let (secs, frac) = compute_countdown_state(
                    now_dt.timestamp(),
                    now_dt.timestamp_subsec_nanos(),
                    *deadline_utc_secs,
                    *total_delay_secs,
                );
                let text = if secs == 0 {
                    "正在等待发件…".to_string()
                } else {
                    format!("邮件将在 {secs} 秒后发送")
                };
                (
                    SharedString::from(text),
                    frac,
                    false,
                    secs > 0,
                    secs == 0,
                    secs > 0,
                    false,
                )
            }
            SendToastStage::Undoing => (
                SharedString::from("正在撤销…"),
                0.0,
                false,
                true,
                true,
                false,
                false,
            ),
            SendToastStage::SendingNow => (
                SharedString::from("正在发送…"),
                1.0,
                false,
                false,
                true,
                false,
                false,
            ),
            SendToastStage::EnqueueFailedNeedsRecovery { message } => (
                SharedString::from(format!("未能保存邮件：{message}")),
                0.0,
                true,
                false,
                true,
                false,
                true,
            ),
        };

    let subject_preview = {
        let s = model.subject.trim();
        if s.is_empty() {
            None
        } else {
            let truncated: String = s.chars().take(36).collect();
            Some(SharedString::from(if s.chars().count() > 36 {
                format!("「{truncated}…」")
            } else {
                format!("「{truncated}」")
            }))
        }
    };

    let border_color = if is_error {
        p.danger.opacity(0.6)
    } else {
        p.border
    };
    let card_bg = if is_error {
        p.danger.opacity(0.10)
    } else {
        p.surface_raised
    };

    let actions_row = h_flex()
        .gap_1p5()
        .flex_shrink_0()
        .items_center()
        .when(show_recover, |row| {
            row.child(
                Button::new(("send-toast-recover", model.token))
                    .primary()
                    .compact()
                    .label("恢复编辑")
                    .on_click(move |_, window, cx| on_recover(window, cx)),
            )
        })
        .when(show_undo, |row| {
            let mut btn = Button::new(("send-toast-undo", model.token))
                .primary()
                .compact()
                .label("撤销发送");
            if undo_disabled {
                btn = btn.tooltip("正在撤销或已到发信临界时间");
            } else {
                btn = btn
                    .tooltip("撤销本次发送并恢复草稿（Esc）")
                    .on_click(move |_, window, cx| on_undo(window, cx));
            }
            row.child(btn)
        })
        .when(show_send_now, |row| {
            row.child(
                Button::new(("send-toast-now", model.token))
                    .outline()
                    .compact()
                    .label("立即发送")
                    .on_click(move |_, window, cx| on_send_now(window, cx)),
            )
        });

    let headline = v_flex()
        .flex_1()
        .min_w_0()
        .gap_0p5()
        .child(
            h_flex()
                .min_w_0()
                .gap_1p5()
                .items_center()
                .child(
                    Icon::new(if is_error {
                        IconName::TriangleAlert
                    } else {
                        IconName::Send
                    })
                    .with_size(px(14.0))
                    .text_color(if is_error { p.danger } else { p.accent }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if is_error { p.danger } else { p.text })
                        .truncate()
                        .child(title),
                ),
        )
        .when_some(subject_preview, |col, subj| {
            col.child(
                div()
                    .text_xs()
                    .text_color(p.text_muted)
                    .truncate()
                    .child(subj),
            )
        })
        .when(model.extra_waiting_count > 0, |col| {
            let extra = model.extra_waiting_count;
            col.child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.text_muted)
                            .child(SharedString::from(format!("还有 {extra} 封等待发送"))),
                    )
                    .child(
                        Button::new(("send-toast-outbox", model.token))
                            .ghost()
                            .compact()
                            .text_size(px(11.0))
                            .label("查看发件箱")
                            .on_click(move |_, window, cx| on_view_outbox(window, cx)),
                    ),
            )
        });

    let body = if compact_width {
        v_flex()
            .px_3()
            .py_2()
            .gap_1p5()
            .child(headline)
            .child(h_flex().justify_end().child(actions_row))
            .into_any_element()
    } else {
        h_flex()
            .px_3()
            .py_2()
            .gap_3()
            .items_center()
            .justify_between()
            .child(headline)
            .child(actions_row)
            .into_any_element()
    };

    v_flex()
        .w_full()
        .when(!compact_width, |el| el.min_w(px(320.0)).max_w(px(480.0)))
        .rounded(px(10.0))
        .border_1()
        .border_color(border_color)
        .bg(card_bg)
        .shadow_md()
        .overflow_hidden()
        .child(body)
        .when(!is_error, |card| {
            card.child(
                div().w_full().h(px(3.0)).bg(p.border.opacity(0.45)).child(
                    div()
                        .h_full()
                        .w(relative(progress.clamp(0.0, 1.0)))
                        .bg(p.accent),
                ),
            )
        })
        .into_any_element()
}

pub(crate) fn render_toast_stack(
    toasts: &[&AppToast],
    compact_width: bool,
    p: &theme::HylkiPalette,
    on_action: impl Fn(ToastActionKind, String, &mut Window, &mut App) + Clone + 'static,
    on_dismiss: impl Fn(String, &mut Window, &mut App) + Clone + 'static,
) -> Vec<AnyElement> {
    toasts
        .iter()
        .enumerate()
        .map(|(idx, toast)| {
            let (icon_name, accent_color, bg_color, border_color) = match toast.kind {
                ToastKind::Info => (IconName::Info, p.accent, p.surface_raised, p.border),
                ToastKind::Success => (IconName::CircleCheck, p.accent, p.surface_raised, p.border),
                ToastKind::Warning => (
                    IconName::TriangleAlert,
                    p.danger,
                    p.surface_raised,
                    p.danger.opacity(0.45),
                ),
                ToastKind::Error => (
                    IconName::CircleX,
                    p.danger,
                    p.danger.opacity(0.10),
                    p.danger.opacity(0.55),
                ),
            };
            let key_for_action = toast.key.clone();
            let key_for_close = toast.key.clone();
            let action_cb = on_action.clone();
            let dismiss_cb = on_dismiss.clone();
            let action_opt = toast.action.clone();
            let dismissible = toast.dismissible;

            h_flex()
                .w_full()
                .when(!compact_width, |el| el.min_w(px(280.0)).max_w(px(480.0)))
                .px_3()
                .py_2()
                .gap_2()
                .items_center()
                .justify_between()
                .rounded(px(10.0))
                .border_1()
                .border_color(border_color)
                .bg(bg_color)
                .shadow_md()
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_2()
                        .items_center()
                        .child(
                            Icon::new(icon_name)
                                .with_size(px(14.0))
                                .text_color(accent_color),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(if toast.kind == ToastKind::Error {
                                            p.danger
                                        } else {
                                            p.text
                                        })
                                        .child(toast.message.clone()),
                                )
                                .when_some(toast.detail.clone(), |col, d| {
                                    col.child(
                                        div()
                                            .text_xs()
                                            .text_color(p.text_muted)
                                            .truncate()
                                            .child(d),
                                    )
                                }),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .flex_shrink_0()
                        .items_center()
                        .when_some(action_opt, |row, action| {
                            let kind = action.kind.clone();
                            let k = key_for_action.clone();
                            let cb = action_cb.clone();
                            let btn = Button::new(("toast-act", idx))
                                .compact()
                                .label(action.label.clone())
                                .on_click(move |_, window, cx| {
                                    cb(kind.clone(), k.clone(), window, cx)
                                });
                            let btn = if action.primary {
                                btn.primary()
                            } else {
                                btn.outline()
                            };
                            row.child(btn)
                        })
                        .when(dismissible, |row| {
                            let k = key_for_close.clone();
                            let cb = dismiss_cb.clone();
                            row.child(
                                Button::new(("toast-close", idx))
                                    .ghost()
                                    .compact()
                                    .icon(IconName::X)
                                    .tooltip("关闭提示")
                                    .on_click(move |_, window, cx| cb(k.clone(), window, cx)),
                            )
                        }),
                )
                .into_any_element()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_deduplicates_by_key_and_preserves_recovery_toasts() {
        let mut q = WindowToastQueue::default();
        q.push_recovery(
            "send-fail-1",
            ToastKind::Error,
            "未能保存邮件",
            None,
            Some(ToastAction {
                label: "恢复编辑".into(),
                primary: true,
                kind: ToastActionKind::RecoverCompose,
            }),
        );
        q.push_transient("copy", ToastKind::Success, "已复制", None);
        q.push_transient("copy", ToastKind::Success, "已再次复制", None);
        q.push_transient("save", ToastKind::Success, "已保存附件", None);

        let vis = q.visible();
        assert_eq!(vis.len(), 2);
        assert_eq!(vis[0].key, "send-fail-1");
        assert_eq!(vis[1].key, "save");
    }

    #[test]
    fn countdown_state_clamps_and_tracks_wall_clock() {
        let (secs, prog) = compute_countdown_state(1000, 0, 1005, 5);
        assert_eq!(secs, 5);
        assert!((prog - 0.0).abs() < 0.001);

        let (secs, prog) = compute_countdown_state(1002, 500_000_000, 1005, 5);
        assert_eq!(secs, 3);
        assert!((prog - 0.5).abs() < 0.01);

        // 回执晚到时直接反映真实剩余秒数，超过截止则钳位为 0 / 1.0。
        let (secs, prog) = compute_countdown_state(1006, 0, 1005, 5);
        assert_eq!(secs, 0);
        assert!((prog - 1.0).abs() < 0.001);
    }

    #[test]
    fn webview_reserved_strip_and_rect_intersection_behave_safely() {
        assert!(feedback_strip_height(100) >= 70.0);
        assert!(feedback_strip_height(150) > feedback_strip_height(100));
        assert!(!main_window_uses_reserved_strip(
            false, false, false, false, 340.0
        ));
        assert!(main_window_uses_reserved_strip(
            false, false, false, false, 280.0
        ));
        assert!(main_window_uses_reserved_strip(
            false, true, false, false, 400.0
        ));
        assert!(main_window_uses_reserved_strip(
            true, false, false, false, 400.0
        ));
        assert!(!main_window_uses_reserved_strip(
            true, false, true, false, 200.0
        ));
        assert!(rects_intersect([0, 0, 100, 100], [90, 90, 150, 150]));
        assert!(!rects_intersect([0, 0, 100, 100], [100, 0, 200, 100]));
    }
}

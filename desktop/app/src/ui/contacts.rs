//! 联系人页（方案文档第三节页面 7）：
//! - 左栏：搜索框（变更发 [`ContactsEvent::QueryChanged`]）+ 联系人列表
//!   （姓名 + 邮箱 + 来源徽标 / 往来邮件数胶囊，自定义联系人优先展示）。
//! - 右栏：选中联系人卡片（头像、姓名、邮箱、往来邮件数、备注，
//!   「给 TA 写信」按钮发 [`ContactsEvent::ComposeTo`]；自定义联系人另带
//!   「编辑」「删除」按钮）。
//! - F-5：顶栏「+ 新建联系人」「导入 ▾」「导出 ▾」；新建 / 编辑为页内
//!   居中模态表单（email 必填、重复邮箱校验），删除带内联二次确认。
//!
//! 自包含组件：数据由集成者通过 [`ContactsState::set_contacts`] 注入，
//! 组件内部按搜索词做客户端过滤以便即时反馈。

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Icon;
use gpui_kit::component::{h_flex, v_flex, Disableable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    black, div, px, AppContext, Context, Div, EventEmitter, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, SharedString, Stateful, StatefulInteractiveElement, Styled,
};
use hylki_mail_core::cache::KnownContact;

use crate::ui::avatar;

/// 左栏宽度。
const LEFT_WIDTH: f32 = 300.0;
/// 联系人行高度。
const ROW_HEIGHT: f32 = 56.0;

/// F-5 导出格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactExportKind {
    /// vCard 3.0（.vcf）。
    Vcard,
    /// CSV（带 `Name,Email,Notes` 表头）。
    Csv,
}

/// 联系人页事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContactsEvent {
    /// 搜索框文本变化（完整当前值）。
    QueryChanged(String),
    /// 请求给指定邮箱写信。
    ComposeTo(String),
    /// F-5 保存联系人（`id == None` 表示新建；校验已在组件内完成）。
    SaveContact {
        id: Option<u32>,
        email: String,
        display_name: String,
        notes: String,
    },
    /// F-5 删除自定义联系人（参数为 user_contacts 行 ID）。
    DeleteContact(u32),
    /// F-5 请求导入 vCard / CSV 文件（对话框由集成者打开）。
    ImportContacts,
    /// F-5 请求把当前列表导出为 vCard / CSV。
    ExportContacts {
        kind: ContactExportKind,
        items: Vec<KnownContact>,
    },
}

/// F-5 新建 / 编辑联系人的模态表单状态。
struct ContactEditor {
    /// 编辑的自定义联系人行 ID；`None` 表示新建。
    id: Option<u32>,
    /// 原邮箱（去重校验时排除自身）。
    name_input: gpui_kit::Entity<InputState>,
    email_input: gpui_kit::Entity<InputState>,
    notes_input: gpui_kit::Entity<InputState>,
    /// 校验错误提示（邮箱为空 / 缺少 @ / 重复）。
    error: Option<SharedString>,
}

/// 联系人视图状态（独立 Entity，由 HylkiApp 挂载）。
pub struct ContactsState {
    search_input: gpui_kit::Entity<InputState>,
    /// 当前搜索词。
    query: String,
    /// 集成者注入的联系人（set_contacts 时按自定义优先 + 邮件数降序排列）。
    contacts: Vec<KnownContact>,
    /// 当前选中的联系人下标（指向过滤后的列表）。
    selected: usize,
    /// F-5 演示模式（CRUD 与导入按钮禁用）。
    demo_mode: bool,
    /// F-5 当前打开的新建 / 编辑表单（`None` 表示未打开）。
    editor: Option<ContactEditor>,
    /// F-5 待确认删除的自定义联系人（行 ID + 显示名）。
    confirm_delete: Option<(u32, String)>,
}

impl EventEmitter<ContactsEvent> for ContactsState {}

impl ContactsState {
    pub fn new(window: &mut gpui_kit::Window, cx: &mut Context<Self>) -> Self {
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("搜索姓名或邮箱..."));
        cx.subscribe(&search_input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let query = input.read(cx).value().to_string();
                this.query = query.clone();
                this.selected = 0;
                cx.emit(ContactsEvent::QueryChanged(query));
                cx.notify();
            }
        })
        .detach();
        Self {
            search_input,
            query: String::new(),
            contacts: Vec::new(),
            selected: 0,
            demo_mode: false,
            editor: None,
            confirm_delete: None,
        }
    }

    /// 注入联系人列表；内部按自定义优先 + 往来邮件数降序排序。
    pub fn set_contacts(&mut self, items: Vec<KnownContact>) {
        self.contacts = sort_contacts_desc(items);
        self.selected = 0;
    }

    /// 注入演示模式标记（CRUD / 导入按钮禁用，与全局 demo 惯例一致）。
    pub fn set_demo_mode(&mut self, demo: bool) {
        self.demo_mode = demo;
    }

    /// 打开新建联系人表单。
    fn open_new_editor(&mut self, window: &mut gpui_kit::Window, cx: &mut Context<Self>) {
        self.confirm_delete = None;
        self.editor = Some(ContactEditor {
            id: None,
            name_input: cx.new(|cx| InputState::new(window, cx).placeholder("显示名称（可选）")),
            email_input: cx.new(|cx| {
                InputState::new(window, cx).placeholder("邮箱地址（必填，如 user@example.com）")
            }),
            notes_input: cx.new(|cx| InputState::new(window, cx).placeholder("备注（可选）")),
            error: None,
        });
        cx.notify();
    }

    /// 打开编辑联系人表单（预填当前值）。
    fn open_edit_editor(
        &mut self,
        contact: &KnownContact,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_delete = None;
        self.editor = Some(ContactEditor {
            id: Some(contact.id),
            name_input: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("显示名称（可选）")
                    .default_value(contact.name.clone())
            }),
            email_input: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("邮箱地址（必填）")
                    .default_value(contact.email.clone())
            }),
            notes_input: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("备注（可选）")
                    .default_value(contact.notes.clone())
            }),
            error: None,
        });
        cx.notify();
    }

    /// 校验并提交表单：邮箱必填、需含 @、不得与其它联系人重复；失败就地提示。
    fn submit_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        let email = editor.email_input.read(cx).value().trim().to_string();
        let name = editor.name_input.read(cx).value().trim().to_string();
        let notes = editor.notes_input.read(cx).value().trim().to_string();
        let error = if email.is_empty() || !email.contains('@') {
            Some(SharedString::from("请填写有效的邮箱地址（需包含 @）"))
        } else if self
            .contacts
            .iter()
            .any(|c| c.email.eq_ignore_ascii_case(&email) && c.id != editor.id.unwrap_or(0))
        {
            Some(SharedString::from(format!("已存在邮箱为 {email} 的联系人")))
        } else {
            None
        };
        if let Some(err) = error {
            if let Some(editor) = self.editor.as_mut() {
                editor.error = Some(err);
            }
            cx.notify();
            return;
        }
        let id = self.editor.as_ref().and_then(|e| e.id);
        self.editor = None;
        cx.emit(ContactsEvent::SaveContact {
            id,
            email,
            display_name: name,
            notes,
        });
        cx.notify();
    }

    /// 过滤后的联系人（搜索词匹配姓名或邮箱，忽略大小写）。
    fn visible_contacts(&self) -> Vec<&KnownContact> {
        self.contacts
            .iter()
            .filter(|c| contact_matches(c, &self.query))
            .collect()
    }
}

/// 自定义联系人优先，其余按往来邮件数降序（同数按姓名升序稳定次序）。
fn sort_contacts_desc(mut items: Vec<KnownContact>) -> Vec<KnownContact> {
    items.sort_by(|a, b| {
        b.custom
            .cmp(&a.custom)
            .then_with(|| b.message_count.cmp(&a.message_count))
            .then_with(|| a.name.cmp(&b.name))
    });
    items
}

/// 搜索词匹配：空串匹配全部；否则忽略大小写的子串匹配姓名或邮箱。
fn contact_matches(contact: &KnownContact, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty()
        || contact.name.to_lowercase().contains(&q)
        || contact.email.to_lowercase().contains(&q)
}

/// 联系人行（头像 + 姓名/邮箱 + 邮件数胶囊）。
fn contact_row(
    contact: &KnownContact,
    is_selected: bool,
    p: &crate::theme::HylkiPalette,
) -> Stateful<Div> {
    let (text, muted_fg, accent, accent_fg, accent_soft) =
        (p.text, p.text_muted, p.accent, p.accent_fg, p.accent_soft());
    let count_text = contact.message_count.to_string();
    h_flex()
        .id(SharedString::from(format!("contact-{}", contact.email)))
        .h(px(ROW_HEIGHT))
        .mx_2()
        .px_2()
        .gap_2()
        .items_center()
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(move |s| s.bg(accent_soft))
        // 选中联系人：实心 accent 底 + accent_fg 字，无常驻边框（复刻原版 row:selected）。
        .when(is_selected, |row| row.bg(accent))
        .child(avatar::avatar(&contact.name, &contact.email, 32.0))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_weight(if is_selected {
                            gpui_kit::FontWeight::SEMIBOLD
                        } else {
                            gpui_kit::FontWeight::MEDIUM
                        })
                        // 选中行整体使用 accent_fg 字（原版选中行全部前景取 accent 前景）。
                        .text_color(if is_selected { accent_fg } else { text })
                        .truncate()
                        .child(SharedString::from(contact.name.clone())),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(if is_selected { accent_fg } else { muted_fg })
                        .truncate()
                        .child(SharedString::from(contact.email.clone())),
                ),
        )
        .child(
            div()
                .rounded_full()
                .px_1p5()
                .h(px(18.0))
                .min_w(px(20.0))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .when(is_selected, |pill| pill.bg(accent).text_color(accent_fg))
                .when(!is_selected, |pill| {
                    pill.bg(accent_soft).text_color(muted_fg)
                })
                .when(contact.custom, |pill| {
                    // F-5：自定义联系人以「自定义」徽标标注来源（替代邮件数胶囊）。
                    pill.child(SharedString::from("自定义"))
                })
                .when(!contact.custom, |pill| {
                    pill.child(SharedString::from(count_text))
                }),
        )
}

impl Render for ContactsState {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 避坑清单第 4 条：render 开头先把语义色 Copy 成局部变量。
        let p = crate::theme::current(cx);
        let (canvas, surface, border, text, muted_fg, accent_soft) = (
            p.canvas,
            p.surface,
            p.border,
            p.text,
            p.text_muted,
            p.accent_soft(),
        );

        let visible = self.visible_contacts();
        let selected = if visible.is_empty() {
            0
        } else {
            self.selected.min(visible.len() - 1)
        };
        let selected_contact = visible.get(selected).cloned();
        let visible_len = visible.len();

        let rows: Vec<Stateful<Div>> = visible
            .iter()
            .enumerate()
            .map(|(ix, contact)| {
                contact_row(contact, ix == selected, &p).on_click(cx.listener(
                    move |this, _, _, cx| {
                        // 选中该联系人（下标指向当前过滤后的列表）。
                        this.selected = ix;
                        cx.notify();
                    },
                ))
            })
            .collect();

        // F-5：「导入 ▾」下拉（vCard / CSV 由集成者按文件特征识别）。
        let entity = cx.entity();
        let import_button = Button::new("contact-import")
            .ghost()
            .compact()
            .icon(IconName::Upload)
            .tooltip("导入联系人（vCard / CSV）")
            .disabled(self.demo_mode)
            .dropdown_menu(move |menu, _, _| {
                let entity = entity.clone();
                menu.item(PopupMenuItem::new("导入 vCard / CSV 文件...").on_click(
                    move |_, _, cx| {
                        entity.update(cx, |_this, cx| {
                            cx.emit(ContactsEvent::ImportContacts);
                            cx.notify();
                        });
                    },
                ))
            });

        // F-5：「导出 ▾」下拉，导出当前过滤后的联系人列表。
        let export_items: Vec<KnownContact> = visible.iter().map(|c| (*c).clone()).collect();
        let entity = cx.entity();
        let export_button = Button::new("contact-export")
            .ghost()
            .compact()
            .icon(IconName::Download)
            .tooltip("导出联系人")
            .dropdown_menu(move |menu, _, _| {
                let items = export_items.clone();
                let entity = entity.clone();
                let vcf_items = items.clone();
                let entity_vcf = entity.clone();
                menu.item(
                    PopupMenuItem::new("导出为 vCard (.vcf)").on_click(move |_, _, cx| {
                        let items = vcf_items.clone();
                        entity_vcf.update(cx, |_this, cx| {
                            cx.emit(ContactsEvent::ExportContacts {
                                kind: ContactExportKind::Vcard,
                                items,
                            });
                            cx.notify();
                        });
                    }),
                )
                .item(
                    PopupMenuItem::new("导出为 CSV (.csv)").on_click(move |_, _, cx| {
                        let items = items.clone();
                        entity.update(cx, |_this, cx| {
                            cx.emit(ContactsEvent::ExportContacts {
                                kind: ContactExportKind::Csv,
                                items,
                            });
                            cx.notify();
                        });
                    }),
                )
            });

        let detail_card =
            match selected_contact {
                None => v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(
                        Icon::new(IconName::CircleUser)
                            .with_size(px(28.0))
                            .text_color(muted_fg),
                    )
                    .child(div().text_sm().text_color(muted_fg).child(
                        if self.contacts.is_empty() {
                            "暂无联系人（同步邮件后自动聚合）"
                        } else {
                            "无匹配的联系人"
                        },
                    )),
                Some(contact) => {
                    let name = contact.name.clone();
                    let email = contact.email.clone();
                    let count = contact.message_count;
                    let notes = contact.notes.clone();
                    let is_custom = contact.custom && contact.id > 0;
                    let compose_email = email.clone();
                    let edit_contact = contact.clone();
                    let delete_id = contact.id;
                    let delete_name = contact.name.clone();
                    v_flex()
                        .w(px(420.0))
                        .px_4()
                        .py_5()
                        .gap_3()
                        .bg(surface)
                        .rounded(px(12.0))
                        .border_1()
                        .border_color(border)
                        .items_center()
                        .child(avatar::avatar(&name, &email, 56.0))
                        .child(
                            div()
                                .text_base()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(text)
                                .child(SharedString::from(name)),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(muted_fg)
                                .child(SharedString::from(email)),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .items_center()
                                .child(
                                    Icon::new(IconName::Mail)
                                        .with_size(px(14.0))
                                        .text_color(muted_fg),
                                )
                                .when(is_custom, |el| {
                                    // F-5：自定义联系人展示来源徽标。
                                    el.child(
                                        div()
                                            .rounded_full()
                                            .px_1p5()
                                            .bg(accent_soft)
                                            .text_xs()
                                            .text_color(muted_fg)
                                            .child("自定义"),
                                    )
                                })
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(muted_fg)
                                        .child(SharedString::from(format!("往来邮件 {count} 封"))),
                                ),
                        )
                        .when(!notes.is_empty(), |el| {
                            el.child(
                                div()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .child(SharedString::from(notes)),
                            )
                        })
                        .child(
                            Button::new("contact-compose")
                                .primary()
                                .label("给 TA 写信")
                                .icon(IconName::Mail)
                                .rounded(px(8.0))
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(ContactsEvent::ComposeTo(compose_email.clone()));
                                    cx.notify();
                                })),
                        )
                        // F-5：自定义联系人的「编辑 / 删除」；删除带内联二次确认；
                        // demo 模式禁用。
                        .when(is_custom, |el| {
                            let confirming_delete = self
                                .confirm_delete
                                .as_ref()
                                .is_some_and(|(id, _)| *id == delete_id);
                            let confirm_bar = h_flex()
                                .gap_1p5()
                                .items_center()
                                .child(div().text_xs().text_color(text).child(SharedString::from(
                                    format!("确认删除「{delete_name}」？"),
                                )))
                                .child(
                                    Button::new("contact-delete-yes")
                                        .primary()
                                        .compact()
                                        .label("确认删除")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.confirm_delete = None;
                                            cx.emit(ContactsEvent::DeleteContact(delete_id));
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("contact-delete-no")
                                        .outline()
                                        .compact()
                                        .label("取消")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.confirm_delete = None;
                                            cx.notify();
                                        })),
                                );
                            let action_bar = h_flex()
                                .gap_1p5()
                                .items_center()
                                .child(
                                    Button::new("contact-edit")
                                        .outline()
                                        .compact()
                                        .label("编辑")
                                        .icon(IconName::Pencil)
                                        .disabled(self.demo_mode)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.open_edit_editor(&edit_contact, window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("contact-delete")
                                        .outline()
                                        .compact()
                                        .label("删除")
                                        .icon(IconName::Trash)
                                        .disabled(self.demo_mode)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.confirm_delete =
                                                Some((delete_id, delete_name.clone()));
                                            cx.notify();
                                        })),
                                );
                            el.child(if confirming_delete {
                                confirm_bar
                            } else {
                                action_bar
                            })
                        })
                }
            };

        // F-5：新建 / 编辑联系人表单浮层（遮罩 + 居中卡片，Esc 语义由全局
        // CloseOverlay 处理；这里通过遮罩点击关闭）。
        let editor_overlay = if let Some(editor) = self.editor.as_ref() {
            let p = &p;
            let is_new = editor.id.is_none();
            let error_text = editor.error.clone();
            div()
                .absolute()
                .top(px(0.0))
                .left(px(0.0))
                .size_full()
                .bg(black().opacity(0.4))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.editor = None;
                        cx.notify();
                    }),
                )
                .child(
                    h_flex().size_full().items_center().justify_center().child(
                        v_flex()
                            .w(px(400.0))
                            .bg(p.surface)
                            .rounded(px(12.0))
                            .border_1()
                            .border_color(p.border)
                            .shadow_xl()
                            .p_4()
                            .gap_3()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation();
                            })
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(gpui_kit::FontWeight::BOLD)
                                    .text_color(p.text)
                                    .child(if is_new {
                                        "新建联系人"
                                    } else {
                                        "编辑联系人"
                                    }),
                            )
                            .child(
                                v_flex()
                                    .gap_2()
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(p.text_muted)
                                                    .child("邮箱（必填）"),
                                            )
                                            .child(Input::new(&editor.email_input)),
                                    )
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(p.text_muted)
                                                    .child("显示名称"),
                                            )
                                            .child(Input::new(&editor.name_input)),
                                    )
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(p.text_muted)
                                                    .child("备注"),
                                            )
                                            .child(Input::new(&editor.notes_input)),
                                    ),
                            )
                            .when_some(error_text, |el, err| {
                                el.child(
                                    div()
                                        .text_xs()
                                        .text_color(p.danger)
                                        .child(SharedString::from(err)),
                                )
                            })
                            .child(
                                h_flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        Button::new("contact-editor-cancel")
                                            .outline()
                                            .compact()
                                            .label("取消")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.editor = None;
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new("contact-editor-save")
                                            .primary()
                                            .compact()
                                            .label("保存")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.submit_editor(cx);
                                            })),
                                    ),
                            ),
                    ),
                )
        } else {
            div()
        };

        v_flex()
            .relative()
            .size_full()
            .bg(canvas)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    // 左栏：搜索框 + 联系人列表。
                    .child(
                        v_flex()
                            .w(px(LEFT_WIDTH))
                            .h_full()
                            .flex_shrink_0()
                            .bg(surface)
                            .border_r_1()
                            .border_color(border)
                            .child(
                                h_flex()
                                    .h(px(48.0))
                                    .flex_shrink_0()
                                    .px_2()
                                    .gap_2()
                                    .items_center()
                                    .border_b_1()
                                    .border_color(border)
                                    .child(
                                        Icon::new(IconName::Users)
                                            .with_size(px(16.0))
                                            .text_color(muted_fg),
                                    )
                                    .child(
                                        div()
                                            .text_base()
                                            .font_weight(gpui_kit::FontWeight::BOLD)
                                            .text_color(text)
                                            .child("联系人"),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_xs()
                                            .text_color(muted_fg)
                                            .truncate()
                                            .child(SharedString::from(format!("{visible_len} 人"))),
                                    )
                                    // F-5：新建 / 导入 / 导出入口；demo 模式下新建与导入禁用。
                                    .child(
                                        Button::new("contact-new")
                                            .outline()
                                            .compact()
                                            .icon(IconName::UserPlus)
                                            .tooltip("新建联系人")
                                            .disabled(self.demo_mode)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.open_new_editor(window, cx);
                                            })),
                                    )
                                    .child(import_button)
                                    .child(export_button),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .py_2()
                                    .flex_shrink_0()
                                    .child(Input::new(&self.search_input)),
                            )
                            .child(
                                v_flex()
                                    .id("contacts-list")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scrollbar()
                                    .py_1()
                                    .when(visible.is_empty(), |el| {
                                        el.child(
                                            div()
                                                .px_3()
                                                .py_2()
                                                .text_xs()
                                                .text_color(muted_fg)
                                                .child(if self.contacts.is_empty() {
                                                    "暂无联系人"
                                                } else {
                                                    "无匹配的联系人"
                                                }),
                                        )
                                    })
                                    .children(rows),
                            ),
                    )
                    // 右栏：选中联系人卡片。
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .justify_center()
                            .child(detail_card),
                    ),
            )
            // F-5：新建 / 编辑联系人模态（页内居中卡片 + 遮罩）。
            .when(self.editor.is_some(), |root| root.child(editor_overlay))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(name: &str, email: &str, count: usize) -> KnownContact {
        KnownContact {
            name: name.to_string(),
            email: email.to_string(),
            message_count: count,
            ..KnownContact::default()
        }
    }

    #[test]
    fn contact_matches_is_case_insensitive_substring() {
        let c = contact("Marcus Chen", "Marcus@Studio.dev", 3);
        assert!(contact_matches(&c, ""));
        assert!(contact_matches(&c, "marcus"));
        assert!(contact_matches(&c, "  CHEN "));
        assert!(contact_matches(&c, "studio.dev"));
        assert!(!contact_matches(&c, "zhang"));
    }

    #[test]
    fn sort_contacts_desc_orders_by_message_count() {
        let sorted = sort_contacts_desc(vec![
            contact("低", "low@example.com", 1),
            contact("高", "high@example.com", 9),
            contact("中", "mid@example.com", 4),
            contact("同数", "tie@example.com", 4),
        ]);
        let names: Vec<&str> = sorted.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["高", "中", "同数", "低"]);
    }

    #[test]
    fn sort_contacts_desc_puts_custom_contacts_first() {
        let mut custom = contact("自定义", "custom@example.com", 0);
        custom.custom = true;
        let sorted = sort_contacts_desc(vec![
            contact("高", "high@example.com", 9),
            custom.clone(),
            contact("低", "low@example.com", 1),
        ]);
        assert!(sorted[0].custom);
        assert_eq!(sorted[0].email, "custom@example.com");
        assert!(!sorted[1].custom);
        assert_eq!(sorted[1].name, "高");
    }
}

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
use gpui_kit::component::{h_flex, v_flex, Disableable, Selectable, Sizable};
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
        phone: String,
        organization: String,
        job_title: String,
        birthday: String,
        groups: Vec<String>,
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
    phone_input: gpui_kit::Entity<InputState>,
    org_input: gpui_kit::Entity<InputState>,
    title_input: gpui_kit::Entity<InputState>,
    birthday_input: gpui_kit::Entity<InputState>,
    groups_input: gpui_kit::Entity<InputState>,
    notes_input: gpui_kit::Entity<InputState>,
    /// 校验错误提示（邮箱为空 / 缺少 @ / 重复）。
    error: Option<SharedString>,
}

#[derive(Clone, Debug)]
struct ContactsFeedback {
    ok: bool,
    message: SharedString,
}

/// 联系人视图状态（独立 Entity，由 HylkiApp 挂载）。
pub struct ContactsState {
    search_input: gpui_kit::Entity<InputState>,
    /// 当前搜索词。
    query: String,
    /// WP-10 当前选中的分组筛选（`None` = 全部）。
    selected_group: Option<String>,
    /// 集成者注入的联系人（set_contacts 时按自定义优先 + 邮件数降序排列）。
    contacts: Vec<KnownContact>,
    /// 当前选中的联系人下标（指向过滤后的列表）。
    selected: usize,
    /// F-5 演示模式（CRUD 与导入按钮禁用）。
    demo_mode: bool,
    /// F-5 当前打开的新建 / 编辑表单（`None` 表示未打开）。
    editor: Option<ContactEditor>,
    /// F-5 待确认删除的自定义联系人（行 ID + 显示名 + 邮箱）。
    confirm_delete: Option<(u32, String, String)>,
    /// 导入/导出及局部操作反馈。
    feedback: Option<ContactsFeedback>,
}

impl EventEmitter<ContactsEvent> for ContactsState {}

impl ContactsState {
    pub fn new(window: &mut gpui_kit::Window, cx: &mut Context<Self>) -> Self {
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("搜索姓名、邮箱、组织或分组..."));
        cx.subscribe(&search_input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let query = input.read(cx).value().to_string();
                this.query = query.clone();
                this.selected = 0;
                this.confirm_delete = None;
                cx.emit(ContactsEvent::QueryChanged(query));
                cx.notify();
            }
        })
        .detach();
        Self {
            search_input,
            query: String::new(),
            selected_group: None,
            contacts: Vec::new(),
            selected: 0,
            demo_mode: false,
            editor: None,
            confirm_delete: None,
            feedback: None,
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

    pub fn set_feedback(&mut self, ok: bool, message: impl Into<SharedString>) {
        self.feedback = Some(ContactsFeedback {
            ok,
            message: message.into(),
        });
    }

    pub fn dismiss_overlay(&mut self) -> bool {
        if self.editor.take().is_some() || self.confirm_delete.take().is_some() {
            return true;
        }
        false
    }

    fn init_editor(
        &mut self,
        contact: Option<&KnownContact>,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_delete = None;
        let make = |placeholder: &'static str,
                    initial: Option<String>,
                    window: &mut gpui_kit::Window,
                    cx: &mut Context<Self>| {
            cx.new(|cx| {
                let state = InputState::new(window, cx).placeholder(placeholder);
                if let Some(val) = initial {
                    state.default_value(val)
                } else {
                    state
                }
            })
        };
        let groups_str = contact.map(|c| c.groups.join(", "));
        let email_input = make(
            "邮箱地址（必填，如 user@example.com）",
            contact.map(|c| c.email.clone()),
            window,
            cx,
        );
        cx.subscribe(&email_input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let should_clear = this.editor.as_ref().is_some_and(|e| e.error.is_some());
                if should_clear {
                    let err = this.validate_editor_email(cx);
                    if let Some(ref mut ed) = this.editor {
                        ed.error = err;
                    }
                    cx.notify();
                }
            }
        })
        .detach();
        self.editor = Some(ContactEditor {
            id: contact.map(|c| c.id),
            name_input: make(
                "显示名称（可选）",
                contact.map(|c| c.name.clone()),
                window,
                cx,
            ),
            email_input,
            phone_input: make(
                "电话号码（可选）",
                contact.map(|c| c.phone.clone()),
                window,
                cx,
            ),
            org_input: make(
                "公司 / 组织（可选）",
                contact.map(|c| c.organization.clone()),
                window,
                cx,
            ),
            title_input: make(
                "职位（可选）",
                contact.map(|c| c.job_title.clone()),
                window,
                cx,
            ),
            birthday_input: make(
                "生日，如 1995-01-02",
                contact.map(|c| c.birthday.clone()),
                window,
                cx,
            ),
            groups_input: make(
                "分组标签，逗号分隔（如 工作, 客户）",
                groups_str,
                window,
                cx,
            ),
            notes_input: make("备注（可选）", contact.map(|c| c.notes.clone()), window, cx),
            error: None,
        });
        cx.notify();
    }

    fn validate_editor_email(&self, cx: &Context<Self>) -> Option<SharedString> {
        let editor = self.editor.as_ref()?;
        let email = editor.email_input.read(cx).value().trim().to_string();
        if email.is_empty() || !email.contains('@') {
            Some(SharedString::from("请填写有效的邮箱地址（需包含 @）"))
        } else if self
            .contacts
            .iter()
            .any(|c| c.email.eq_ignore_ascii_case(&email) && c.id != editor.id.unwrap_or(0))
        {
            Some(SharedString::from(format!("已存在邮箱为 {email} 的联系人")))
        } else {
            None
        }
    }

    /// 打开新建联系人表单。
    fn open_new_editor(&mut self, window: &mut gpui_kit::Window, cx: &mut Context<Self>) {
        self.init_editor(None, window, cx);
    }

    /// 打开编辑联系人表单（预填当前值）。
    fn open_edit_editor(
        &mut self,
        contact: &KnownContact,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        self.init_editor(Some(contact), window, cx);
    }

    /// 校验并提交表单：邮箱必填、需含 @、不得与其它联系人重复；失败就地提示并聚焦异常字段。
    fn submit_editor(&mut self, window: &mut gpui_kit::Window, cx: &mut Context<Self>) {
        if let Some(err) = self.validate_editor_email(cx) {
            if let Some(editor) = self.editor.as_mut() {
                editor.error = Some(err);
                let email_input = editor.email_input.clone();
                email_input.update(cx, |s, cx| s.focus(window, cx));
            }
            cx.notify();
            return;
        }
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        let email = editor.email_input.read(cx).value().trim().to_string();
        let name = editor.name_input.read(cx).value().trim().to_string();
        let phone = editor.phone_input.read(cx).value().trim().to_string();
        let organization = editor.org_input.read(cx).value().trim().to_string();
        let job_title = editor.title_input.read(cx).value().trim().to_string();
        let birthday = editor.birthday_input.read(cx).value().trim().to_string();
        let groups: Vec<String> = editor
            .groups_input
            .read(cx)
            .value()
            .split([',', '，'])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let notes = editor.notes_input.read(cx).value().trim().to_string();
        let id = editor.id;
        self.editor = None;
        cx.emit(ContactsEvent::SaveContact {
            id,
            email,
            display_name: name,
            notes,
            phone,
            organization,
            job_title,
            birthday,
            groups,
        });
        cx.notify();
    }

    /// 收集所有去重后的分组标签。
    fn all_groups(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for c in &self.contacts {
            for g in &c.groups {
                if !g.is_empty() && !out.iter().any(|existing| existing == g) {
                    out.push(g.clone());
                }
            }
        }
        out.sort();
        out
    }

    /// 过滤后的联系人（搜索词 + 分组筛选）。
    fn visible_contacts(&self) -> Vec<&KnownContact> {
        self.contacts
            .iter()
            .filter(|c| {
                self.selected_group
                    .as_ref()
                    .is_none_or(|g| c.groups.iter().any(|cg| cg == g))
                    && contact_matches(c, &self.query)
            })
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

/// 搜索词匹配：空串匹配全部；否则忽略大小写的子串匹配姓名、邮箱、组织、电话或分组。
fn contact_matches(contact: &KnownContact, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty()
        || contact.name.to_lowercase().contains(&q)
        || contact.email.to_lowercase().contains(&q)
        || contact.organization.to_lowercase().contains(&q)
        || contact.phone.to_lowercase().contains(&q)
        || contact.groups.iter().any(|g| g.to_lowercase().contains(&q))
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
            .disabled(export_items.is_empty())
            .tooltip(if export_items.is_empty() {
                "暂无可导出的联系人"
            } else {
                "导出联系人"
            })
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
                    let sent_count = contact.sent_count;
                    let notes = contact.notes.clone();
                    let phone = contact.phone.clone();
                    let organization = contact.organization.clone();
                    let job_title = contact.job_title.clone();
                    let birthday = contact.birthday.clone();
                    let groups = contact.groups.clone();
                    let is_custom = contact.custom && contact.id > 0;
                    let compose_email = email.clone();
                    let edit_contact = contact.clone();
                    let delete_id = contact.id;
                    let delete_name = contact.name.clone();
                    let delete_email = contact.email.clone();
                    let org_line = match (organization.is_empty(), job_title.is_empty()) {
                        (false, false) => format!("{organization} · {job_title}"),
                        (false, true) => organization,
                        (true, false) => job_title,
                        (true, true) => String::new(),
                    };
                    v_flex()
                        .w(px(440.0))
                        .px_5()
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
                                .gap_1p5()
                                .items_center()
                                .flex_wrap()
                                .justify_center()
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
                                .child(div().text_xs().text_color(muted_fg).child(
                                    SharedString::from(format!(
                                        "往来 {count} 封 · 发信 {sent_count} 封"
                                    )),
                                )),
                        )
                        .when(!groups.is_empty(), |el| {
                            el.child(h_flex().gap_1().flex_wrap().justify_center().children(
                                groups.into_iter().map(|g| {
                                    div()
                                        .rounded_full()
                                        .px_2()
                                        .py_0p5()
                                        .bg(accent_soft)
                                        .text_xs()
                                        .text_color(text)
                                        .child(SharedString::from(g))
                                }),
                            ))
                        })
                        .when(
                            !phone.is_empty()
                                || !org_line.is_empty()
                                || !birthday.is_empty()
                                || !notes.is_empty(),
                            |el| {
                                el.child(
                                    v_flex()
                                        .w_full()
                                        .p_3()
                                        .gap_1p5()
                                        .rounded(px(8.0))
                                        .border_1()
                                        .border_color(border)
                                        .when(!org_line.is_empty(), |card| {
                                            card.child(
                                                h_flex()
                                                    .justify_between()
                                                    .gap_2()
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(muted_fg)
                                                            .child("组织 / 职位"),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(text)
                                                            .child(SharedString::from(org_line)),
                                                    ),
                                            )
                                        })
                                        .when(!phone.is_empty(), |card| {
                                            card.child(
                                                h_flex()
                                                    .justify_between()
                                                    .gap_2()
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(muted_fg)
                                                            .child("电话"),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(text)
                                                            .child(SharedString::from(phone)),
                                                    ),
                                            )
                                        })
                                        .when(!birthday.is_empty(), |card| {
                                            card.child(
                                                h_flex()
                                                    .justify_between()
                                                    .gap_2()
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(muted_fg)
                                                            .child("生日"),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(text)
                                                            .child(SharedString::from(birthday)),
                                                    ),
                                            )
                                        })
                                        .when(!notes.is_empty(), |card| {
                                            card.child(
                                                h_flex()
                                                    .justify_between()
                                                    .gap_2()
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(muted_fg)
                                                            .child("备注"),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(text)
                                                            .child(SharedString::from(notes)),
                                                    ),
                                            )
                                        }),
                                )
                            },
                        )
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
                                .is_some_and(|(id, _, _)| *id == delete_id);
                            let delete_target = if delete_name.trim().is_empty() {
                                delete_email.clone()
                            } else {
                                format!("{delete_name} ({delete_email})")
                            };
                            let confirm_bar = h_flex()
                                .gap_1p5()
                                .items_center()
                                .child(div().text_xs().text_color(text).child(SharedString::from(
                                    format!("确认删除「{delete_target}」？"),
                                )))
                                .child(
                                    Button::new("contact-delete-no")
                                        .outline()
                                        .compact()
                                        .label("取消")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.confirm_delete = None;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("contact-delete-yes")
                                        .danger()
                                        .compact()
                                        .label("确认删除")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.confirm_delete = None;
                                            cx.emit(ContactsEvent::DeleteContact(delete_id));
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
                                            this.confirm_delete = Some((
                                                delete_id,
                                                delete_name.clone(),
                                                delete_email.clone(),
                                            ));
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
                                            .child(Input::new(&editor.email_input))
                                            .when_some(error_text, |col, err| {
                                                col.child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(p.danger)
                                                        .child(SharedString::from(err)),
                                                )
                                            }),
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
                                        h_flex()
                                            .gap_2()
                                            .child(
                                                v_flex()
                                                    .flex_1()
                                                    .gap_1()
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(p.text_muted)
                                                            .child("电话"),
                                                    )
                                                    .child(Input::new(&editor.phone_input)),
                                            )
                                            .child(
                                                v_flex()
                                                    .flex_1()
                                                    .gap_1()
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(p.text_muted)
                                                            .child("生日"),
                                                    )
                                                    .child(Input::new(&editor.birthday_input)),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(
                                                v_flex()
                                                    .flex_1()
                                                    .gap_1()
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(p.text_muted)
                                                            .child("组织 / 公司"),
                                                    )
                                                    .child(Input::new(&editor.org_input)),
                                            )
                                            .child(
                                                v_flex()
                                                    .flex_1()
                                                    .gap_1()
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(p.text_muted)
                                                            .child("职位"),
                                                    )
                                                    .child(Input::new(&editor.title_input)),
                                            ),
                                    )
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(p.text_muted)
                                                    .child("分组标签（逗号分隔）"),
                                            )
                                            .child(Input::new(&editor.groups_input)),
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
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.submit_editor(window, cx);
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
            .when_some(self.feedback.clone(), |root, fb| {
                let color = if fb.ok { p.text } else { p.danger };
                let bg = if fb.ok {
                    p.accent_soft()
                } else {
                    p.danger.opacity(0.10)
                };
                root.child(
                    h_flex()
                        .px_4()
                        .py_1p5()
                        .justify_between()
                        .items_center()
                        .bg(bg)
                        .border_b_1()
                        .border_color(if fb.ok {
                            p.border
                        } else {
                            p.danger.opacity(0.4)
                        })
                        .child(div().text_xs().text_color(color).child(fb.message))
                        .child(
                            Button::new("contacts-feedback-close")
                                .ghost()
                                .compact()
                                .icon(IconName::X)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.feedback = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
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
                            .when(!self.all_groups().is_empty(), |col| {
                                let groups = self.all_groups();
                                let selected_group = self.selected_group.clone();
                                col.child(
                                    h_flex()
                                        .px_2()
                                        .pb_2()
                                        .gap_1()
                                        .flex_wrap()
                                        .child(
                                            Button::new("contact-group-all")
                                                .ghost()
                                                .compact()
                                                .selected(selected_group.is_none())
                                                .label("全部")
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.selected_group = None;
                                                    this.selected = 0;
                                                    cx.notify();
                                                })),
                                        )
                                        .children(groups.into_iter().enumerate().map(|(i, g)| {
                                            let is_sel =
                                                selected_group.as_deref() == Some(g.as_str());
                                            let g_click = g.clone();
                                            Button::new(("contact-group", i))
                                                .ghost()
                                                .compact()
                                                .selected(is_sel)
                                                .label(g)
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.selected_group = Some(g_click.clone());
                                                    this.selected = 0;
                                                    cx.notify();
                                                }))
                                        })),
                                )
                            })
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

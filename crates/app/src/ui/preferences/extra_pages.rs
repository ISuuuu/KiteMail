//! WP-06 新增的 8 个偏好设置一级分页渲染：
//! `Tags`、`Filters`、`SmartFolders`、`OpenPgp`、`CloudStorage`、`Translation`、`ContactsSources`、`Mcp`。

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex, Disableable, Selectable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, AnyElement, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled,
};
use kitemail_mail_core::cache::{CacheStore, TagItem};
use kitemail_mail_core::config::{
    self, AppPreferences, CloudStorageAccount, ContactSourceEntry, FilterField, FilterMatch,
    FilterRule, PgpKeyEntry, SmartFolderConfig,
};

use crate::app::HylkiApp;
use crate::theme::HylkiPalette;
use crate::ui::avatar::{self, ACCOUNT_ACCENT_COLORS};
use crate::ui::preferences::widgets::{
    bool_switch, combo_button, combo_button_dynamic, setting_group, setting_row,
    CARDS_WIDE_MAX_WIDTH, CARD_MAX_WIDTH,
};
use crate::ui::preferences::SettingsWindow;

const CLOUD_PROVIDERS: &[(&str, &str)] = &[
    ("Nextcloud", "Nextcloud"),
    ("WebDAV", "WebDAV"),
    ("Amazon S3 / 兼容对象存储", "S3"),
    ("Dropbox", "Dropbox"),
];

const CLOUD_THRESHOLD_OPTIONS: &[(&str, u32)] = &[
    ("关闭自动上传", 0),
    ("超过 10 MB", 10),
    ("超过 25 MB（推荐）", 25),
    ("超过 50 MB", 50),
    ("超过 100 MB", 100),
];

const CONTACT_PROVIDERS: &[(&str, &str)] = &[
    ("通用 CardDAV", "CardDAV"),
    ("iCloud 通讯录", "iCloud"),
    ("Nextcloud 通讯录", "Nextcloud"),
    ("Fastmail 通讯录", "Fastmail"),
];

const FILTER_FIELD_OPTIONS: &[(&str, FilterField)] = &[
    ("发件人", FilterField::From),
    ("收件人", FilterField::To),
    ("主题", FilterField::Subject),
    ("正文", FilterField::Body),
];

const FILTER_MATCH_OPTIONS: &[(&str, FilterMatch)] = &[
    ("包含", FilterMatch::Contains),
    ("完全等于", FilterMatch::Equals),
    ("开头为", FilterMatch::StartsWith),
    ("结尾为", FilterMatch::EndsWith),
];

const SMART_ICON_OPTIONS: &[(&str, &str)] = &[
    ("智能搜索", "folder-search"),
    ("星标重要", "star"),
    ("收件箱", "inbox"),
    ("标签", "tag"),
    ("筛选", "filter"),
];

const TRANSLATION_ENGINES: &[(&str, &str)] = &[
    ("关闭", "off"),
    ("LibreTranslate（自建/开源）", "libretranslate"),
    ("DeepL API", "deepl"),
    ("Google Translate", "google"),
    ("本地 LLM（复用本机推理服务）", "local_llm"),
];

const TRANSLATION_TARGETS: &[(&str, &str)] = &[
    ("跟随应用界面语言", "system"),
    ("简体中文 (zh-CN)", "zh-CN"),
    ("English (en)", "en"),
    ("日本語 (ja)", "ja"),
    ("한국어 (ko)", "ko"),
    ("Français (fr)", "fr"),
    ("Deutsch (de)", "de"),
    ("Español (es)", "es"),
];

fn feedback_banner(fb: Option<&(bool, SharedString)>, p: &HylkiPalette) -> Option<AnyElement> {
    let (ok, msg) = fb?;
    Some(
        div()
            .w_full()
            .max_w(px(CARDS_WIDE_MAX_WIDTH))
            .px_3()
            .py_2()
            .rounded(px(8.0))
            .border_1()
            .border_color(if *ok {
                p.accent.opacity(0.45)
            } else {
                p.danger.opacity(0.45)
            })
            .bg(if *ok {
                p.accent_soft()
            } else {
                p.danger.opacity(0.10)
            })
            .text_xs()
            .text_color(if *ok { p.accent } else { p.danger })
            .child(msg.clone())
            .into_any_element(),
    )
}

fn empty_row(text: &str, p: &HylkiPalette) -> AnyElement {
    div()
        .w_full()
        .px_4()
        .py_3()
        .text_xs()
        .text_color(p.text_muted)
        .child(SharedString::from(text.to_string()))
        .into_any_element()
}

fn pref_switch(
    id: &'static str,
    checked: bool,
    disabled: bool,
    app_entity: Entity<HylkiApp>,
    mutate: impl Fn(&mut AppPreferences, bool) + 'static,
) -> AnyElement {
    bool_switch(id, checked, disabled, move |val, _, cx| {
        app_entity.update(cx, |app, cx| {
            mutate(&mut app.prefs, val);
            let _ = config::save_preferences_in(&config::data_dir(), &app.prefs);
            cx.notify();
        });
    })
}

// ===========================================================================
// 1. OpenPGP 密钥
// ===========================================================================

pub(crate) fn render_openpgp(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let (prefs, demo_mode) = {
        let app = app_entity.read(cx);
        (app.prefs.clone(), app.demo_mode)
    };
    let hl = win.highlighted_row_id;

    let policy_group = setting_group(
        "密钥查询策略",
        Some("发信时自动查找收件人公钥（本地元信息与策略持久化；Windows 端尚未内置 GnuPG/Sequoia 加解密引擎）"),
        CARD_MAX_WIDTH,
        vec![
            setting_row(
                "pref-openpgp-wkd",
                "通过 WKD 查询收件人公钥",
                Some("向收件人域名发起 Web Key Directory 查询"),
                pref_switch(
                    "sw-openpgp-wkd",
                    prefs.openpgp_wkd_lookup,
                    demo_mode,
                    app_entity.clone(),
                    |p, v| p.openpgp_wkd_lookup = v,
                ),
                demo_mode,
                hl == Some("pref-openpgp-wkd"),
                p,
            ),
            setting_row(
                "pref-openpgp-keyserver",
                "同时查询 keys.openpgp.org",
                Some("从公共 OpenPGP 密钥服务器拉取公钥"),
                pref_switch(
                    "sw-openpgp-keyserver",
                    prefs.openpgp_keyserver_lookup,
                    demo_mode,
                    app_entity.clone(),
                    |p, v| p.openpgp_keyserver_lookup = v,
                ),
                demo_mode,
                hl == Some("pref-openpgp-keyserver"),
                p,
            ),
        ],
        p,
    );

    let mut key_rows: Vec<AnyElement> = Vec::new();
    if prefs.pgp_keys.is_empty() {
        key_rows.push(empty_row("尚未登记 OpenPGP 密钥。", p));
    } else {
        for (idx, k) in prefs.pgp_keys.iter().enumerate() {
            let app_trust = app_entity.clone();
            let app_sign = app_entity.clone();
            let app_del = app_entity.clone();
            let key_id = k.id;
            let title = if k.name.trim().is_empty() {
                k.email.clone()
            } else {
                format!("{} <{}>", k.name.trim(), k.email.trim())
            };
            let sub = format!(
                "指纹: {} · {}",
                k.fingerprint,
                if k.has_secret {
                    "含私钥"
                } else {
                    "仅公钥"
                }
            );
            key_rows.push(
                h_flex()
                    .w_full()
                    .min_h(px(56.0))
                    .px_4()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.text)
                                    .truncate()
                                    .child(SharedString::from(title)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.text_muted)
                                    .truncate()
                                    .child(SharedString::from(sub)),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1p5()
                            .flex_shrink_0()
                            .child(
                                Button::new(("pgp-trust", idx))
                                    .outline()
                                    .compact()
                                    .disabled(demo_mode)
                                    .label(if k.trusted { "已信任" } else { "信任" })
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        app_trust.update(cx, |app, cx| {
                                            if let Some(item) = app
                                                .prefs
                                                .pgp_keys
                                                .iter_mut()
                                                .find(|x| x.id == key_id)
                                            {
                                                item.trusted = !item.trusted;
                                            }
                                            let _ = config::save_preferences_in(
                                                &config::data_dir(),
                                                &app.prefs,
                                            );
                                            cx.notify();
                                        });
                                    })),
                            )
                            .child(
                                Button::new(("pgp-sign", idx))
                                    .outline()
                                    .compact()
                                    .disabled(demo_mode)
                                    .label(if k.sign_by_default {
                                        "默认签名: 开"
                                    } else {
                                        "默认签名: 关"
                                    })
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        app_sign.update(cx, |app, cx| {
                                            if let Some(item) = app
                                                .prefs
                                                .pgp_keys
                                                .iter_mut()
                                                .find(|x| x.id == key_id)
                                            {
                                                item.sign_by_default = !item.sign_by_default;
                                            }
                                            let _ = config::save_preferences_in(
                                                &config::data_dir(),
                                                &app.prefs,
                                            );
                                            cx.notify();
                                        });
                                    })),
                            )
                            .child(
                                Button::new(("pgp-del", idx))
                                    .ghost()
                                    .compact()
                                    .disabled(demo_mode)
                                    .icon(IconName::Trash)
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        app_del.update(cx, |app, cx| {
                                            app.prefs.pgp_keys.retain(|x| x.id != key_id);
                                            let _ = config::save_preferences_in(
                                                &config::data_dir(),
                                                &app.prefs,
                                            );
                                            cx.notify();
                                        });
                                    })),
                            ),
                    )
                    .into_any_element(),
            );
        }
    }

    let app_add = app_entity.clone();
    key_rows.push(
        v_flex()
            .w_full()
            .px_4()
            .py_3()
            .gap_2()
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&win.pgp_name_input).cleanable(true)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&win.pgp_email_input).cleanable(true)),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&win.pgp_fp_input).cleanable(true)),
                    )
                    .child(
                        Button::new("pgp-add-btn")
                            .primary()
                            .compact()
                            .disabled(demo_mode)
                            .icon(IconName::Plus)
                            .label("登记密钥")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let email =
                                    this.pgp_email_input.read(cx).value().trim().to_string();
                                if email.is_empty() || !email.contains('@') {
                                    this.pgp_feedback =
                                        Some((false, "请输入有效的绑定邮箱地址".into()));
                                    cx.notify();
                                    return;
                                }
                                let name = this.pgp_name_input.read(cx).value().trim().to_string();
                                let mut fp = this.pgp_fp_input.read(cx).value().trim().to_string();
                                if fp.is_empty() {
                                    fp = format!(
                                        "LOCAL-{:08X}",
                                        chrono::Utc::now().timestamp() as u32
                                    );
                                }
                                app_add.update(cx, |app, cx| {
                                    let next_id =
                                        app.prefs.pgp_keys.iter().map(|k| k.id).max().unwrap_or(0)
                                            + 1;
                                    app.prefs.pgp_keys.push(PgpKeyEntry {
                                        id: next_id,
                                        name,
                                        email,
                                        fingerprint: fp,
                                        has_secret: true,
                                        trusted: true,
                                        sign_by_default: false,
                                        encrypt_by_default: false,
                                        created_at: chrono::Utc::now()
                                            .format("%Y-%m-%d")
                                            .to_string(),
                                    });
                                    let _ = config::save_preferences_in(
                                        &config::data_dir(),
                                        &app.prefs,
                                    );
                                    cx.notify();
                                });
                                this.pgp_name_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.pgp_email_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.pgp_fp_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.pgp_feedback = Some((
                                    true,
                                    "已保存密钥元信息（实际加解密待 Windows PGP 引擎接入）".into(),
                                ));
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element(),
    );

    v_flex()
        .w_full()
        .gap_5()
        .when_some(feedback_banner(win.pgp_feedback.as_ref(), p), |el, b| {
            el.child(b)
        })
        .child(policy_group)
        .child(setting_group(
            "已登记密钥列表",
            Some("按邮箱绑定默认签名/加密策略；删除仅移除本地登记"),
            CARD_MAX_WIDTH,
            key_rows,
            p,
        ))
        .into_any_element()
}

// ===========================================================================
// 2. 云存储
// ===========================================================================

pub(crate) fn render_cloud_storage(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let (prefs, demo_mode) = {
        let app = app_entity.read(cx);
        (app.prefs.clone(), app.demo_mode)
    };
    let hl = win.highlighted_row_id;

    let threshold_label = CLOUD_THRESHOLD_OPTIONS
        .iter()
        .find(|(_, v)| *v == prefs.cloud_auto_upload_threshold_mb)
        .map(|(l, _)| *l)
        .unwrap_or("超过 25 MB（推荐）");
    let app_th = app_entity.clone();
    let policy_group = setting_group(
        "超大附件自动转存",
        Some("写信附件体积超过阈值时提示上传到云盘并插入分享链接"),
        CARD_MAX_WIDTH,
        vec![setting_row(
            "pref-cloud-threshold",
            "自动上传阈值",
            Some("支持 Nextcloud / WebDAV / S3 / Dropbox"),
            combo_button(
                "cb-cloud-threshold",
                threshold_label,
                CLOUD_THRESHOLD_OPTIONS,
                prefs.cloud_auto_upload_threshold_mb,
                demo_mode,
                move |val, _, cx| {
                    app_th.update(cx, |app, cx| {
                        app.prefs.cloud_auto_upload_threshold_mb = val;
                        let _ = config::save_preferences_in(&config::data_dir(), &app.prefs);
                        cx.notify();
                    });
                },
            ),
            demo_mode,
            hl == Some("pref-cloud-threshold"),
            p,
        )],
        p,
    );

    let mut rows: Vec<AnyElement> = Vec::new();
    if prefs.cloud_accounts.is_empty() {
        rows.push(empty_row("尚未添加云存储账号。", p));
    } else {
        for (idx, acc) in prefs.cloud_accounts.iter().enumerate() {
            let app_tog = app_entity.clone();
            let app_del = app_entity.clone();
            let id = acc.id;
            let sub = format!("{} · {} ({})", acc.provider, acc.server_url, acc.username);
            rows.push(
                h_flex()
                    .w_full()
                    .min_h(px(54.0))
                    .px_4()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.text)
                                    .truncate()
                                    .child(SharedString::from(acc.name.clone())),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.text_muted)
                                    .truncate()
                                    .child(SharedString::from(sub)),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_shrink_0()
                            .items_center()
                            .child(
                                Switch::new(("cloud-sw", idx))
                                    .checked(acc.enabled)
                                    .disabled(demo_mode)
                                    .on_click(cx.listener(move |_, checked, _, cx| {
                                        app_tog.update(cx, |app, cx| {
                                            if let Some(a) = app
                                                .prefs
                                                .cloud_accounts
                                                .iter_mut()
                                                .find(|x| x.id == id)
                                            {
                                                a.enabled = *checked;
                                            }
                                            let _ = config::save_preferences_in(
                                                &config::data_dir(),
                                                &app.prefs,
                                            );
                                            cx.notify();
                                        });
                                    })),
                            )
                            .child(
                                Button::new(("cloud-del", idx))
                                    .ghost()
                                    .compact()
                                    .disabled(demo_mode)
                                    .icon(IconName::Trash)
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        app_del.update(cx, |app, cx| {
                                            app.prefs.cloud_accounts.retain(|x| x.id != id);
                                            let _ = config::save_preferences_in(
                                                &config::data_dir(),
                                                &app.prefs,
                                            );
                                            cx.notify();
                                        });
                                    })),
                            ),
                    )
                    .into_any_element(),
            );
        }
    }

    let (provider_lbl, provider_cur) = CLOUD_PROVIDERS
        .iter()
        .find(|(_, v)| *v == win.cloud_provider)
        .copied()
        .unwrap_or(("Nextcloud", "Nextcloud"));
    let win_cp = cx.entity();
    let app_add = app_entity.clone();
    rows.push(
        v_flex()
            .w_full()
            .px_4()
            .py_3()
            .gap_2()
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(combo_button(
                        "cb-cloud-provider",
                        provider_lbl,
                        CLOUD_PROVIDERS,
                        provider_cur,
                        demo_mode,
                        move |val, _, cx| {
                            let _ = win_cp.update(cx, |w, cx| {
                                w.cloud_provider = val.to_string();
                                cx.notify();
                            });
                        },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&win.cloud_name_input).cleanable(true)),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&win.cloud_url_input).cleanable(true)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&win.cloud_user_input).cleanable(true)),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&win.cloud_bucket_input).cleanable(true)),
                    )
                    .child(
                        Button::new("cloud-add-btn")
                            .primary()
                            .compact()
                            .disabled(demo_mode)
                            .icon(IconName::Plus)
                            .label("添加云盘")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let name =
                                    this.cloud_name_input.read(cx).value().trim().to_string();
                                let url = this.cloud_url_input.read(cx).value().trim().to_string();
                                if name.is_empty() || url.is_empty() {
                                    this.cloud_feedback =
                                        Some((false, "请填写账号名称与服务器地址".into()));
                                    cx.notify();
                                    return;
                                }
                                let user =
                                    this.cloud_user_input.read(cx).value().trim().to_string();
                                let bucket =
                                    this.cloud_bucket_input.read(cx).value().trim().to_string();
                                let provider = this.cloud_provider.clone();
                                app_add.update(cx, |app, cx| {
                                    let next_id = app
                                        .prefs
                                        .cloud_accounts
                                        .iter()
                                        .map(|a| a.id)
                                        .max()
                                        .unwrap_or(0)
                                        + 1;
                                    app.prefs.cloud_accounts.push(CloudStorageAccount {
                                        id: next_id,
                                        provider,
                                        name,
                                        server_url: url,
                                        username: user,
                                        bucket_or_path: bucket,
                                        enabled: true,
                                    });
                                    let _ = config::save_preferences_in(
                                        &config::data_dir(),
                                        &app.prefs,
                                    );
                                    cx.notify();
                                });
                                this.cloud_name_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.cloud_url_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.cloud_user_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.cloud_bucket_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.cloud_feedback = Some((true, "已添加云存储账号配置".into()));
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element(),
    );

    v_flex()
        .w_full()
        .gap_5()
        .when_some(feedback_banner(win.cloud_feedback.as_ref(), p), |el, b| {
            el.child(b)
        })
        .child(policy_group)
        .child(setting_group("云存储账号", None, CARD_MAX_WIDTH, rows, p))
        .into_any_element()
}

// ===========================================================================
// 3. 通讯录源
// ===========================================================================

pub(crate) fn render_contacts_sources(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let (prefs, demo_mode) = {
        let app = app_entity.read(cx);
        (app.prefs.clone(), app.demo_mode)
    };
    let hl = win.highlighted_row_id;

    let collect_group = setting_group(
        "自动收集",
        None,
        CARD_MAX_WIDTH,
        vec![setting_row(
            "pref-contacts-auto-collect",
            "自动收集发件人与收件人",
            Some("从已同步与已发送邮件中自动聚合联系人到自动补全列表"),
            pref_switch(
                "sw-contacts-auto-collect",
                prefs.contacts_auto_collect_senders,
                demo_mode,
                app_entity.clone(),
                |p, v| p.contacts_auto_collect_senders = v,
            ),
            demo_mode,
            hl == Some("pref-contacts-auto-collect"),
            p,
        )],
        p,
    );

    let mut rows: Vec<AnyElement> = Vec::new();
    if prefs.contact_sources.is_empty() {
        rows.push(empty_row("尚未添加外部 CardDAV 通讯录源。", p));
    } else {
        for (idx, src) in prefs.contact_sources.iter().enumerate() {
            let app_tog = app_entity.clone();
            let app_del = app_entity.clone();
            let id = src.id;
            let sub = format!("{} · {} ({})", src.provider, src.server_url, src.username);
            rows.push(
                h_flex()
                    .w_full()
                    .min_h(px(54.0))
                    .px_4()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.text)
                                    .truncate()
                                    .child(SharedString::from(src.name.clone())),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.text_muted)
                                    .truncate()
                                    .child(SharedString::from(sub)),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_shrink_0()
                            .items_center()
                            .child(
                                Switch::new(("cs-sw", idx))
                                    .checked(src.enabled)
                                    .disabled(demo_mode)
                                    .on_click(cx.listener(move |_, checked, _, cx| {
                                        app_tog.update(cx, |app, cx| {
                                            if let Some(s) = app
                                                .prefs
                                                .contact_sources
                                                .iter_mut()
                                                .find(|x| x.id == id)
                                            {
                                                s.enabled = *checked;
                                            }
                                            let _ = config::save_preferences_in(
                                                &config::data_dir(),
                                                &app.prefs,
                                            );
                                            cx.notify();
                                        });
                                    })),
                            )
                            .child(
                                Button::new(("cs-del", idx))
                                    .ghost()
                                    .compact()
                                    .disabled(demo_mode)
                                    .icon(IconName::Trash)
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        app_del.update(cx, |app, cx| {
                                            app.prefs.contact_sources.retain(|x| x.id != id);
                                            let _ = config::save_preferences_in(
                                                &config::data_dir(),
                                                &app.prefs,
                                            );
                                            cx.notify();
                                        });
                                    })),
                            ),
                    )
                    .into_any_element(),
            );
        }
    }

    let (provider_lbl, provider_cur) = CONTACT_PROVIDERS
        .iter()
        .find(|(_, v)| *v == win.cs_provider)
        .copied()
        .unwrap_or(("通用 CardDAV", "CardDAV"));
    let win_cs = cx.entity();
    let app_add = app_entity.clone();
    rows.push(
        v_flex()
            .w_full()
            .px_4()
            .py_3()
            .gap_2()
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(combo_button(
                        "cb-cs-provider",
                        provider_lbl,
                        CONTACT_PROVIDERS,
                        provider_cur,
                        demo_mode,
                        move |val, _, cx| {
                            let _ = win_cs.update(cx, |w, cx| {
                                w.cs_provider = val.to_string();
                                cx.notify();
                            });
                        },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&win.cs_name_input).cleanable(true)),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&win.cs_url_input).cleanable(true)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&win.cs_user_input).cleanable(true)),
                    )
                    .child(
                        Button::new("cs-add-btn")
                            .primary()
                            .compact()
                            .disabled(demo_mode)
                            .icon(IconName::Plus)
                            .label("添加源")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let name = this.cs_name_input.read(cx).value().trim().to_string();
                                let url = this.cs_url_input.read(cx).value().trim().to_string();
                                if name.is_empty() || url.is_empty() {
                                    this.cs_feedback =
                                        Some((false, "请填写通讯录名称与 CardDAV 地址".into()));
                                    cx.notify();
                                    return;
                                }
                                let user = this.cs_user_input.read(cx).value().trim().to_string();
                                let provider = this.cs_provider.clone();
                                app_add.update(cx, |app, cx| {
                                    let next_id = app
                                        .prefs
                                        .contact_sources
                                        .iter()
                                        .map(|s| s.id)
                                        .max()
                                        .unwrap_or(0)
                                        + 1;
                                    app.prefs.contact_sources.push(ContactSourceEntry {
                                        id: next_id,
                                        provider,
                                        name,
                                        server_url: url,
                                        username: user,
                                        enabled: true,
                                    });
                                    let _ = config::save_preferences_in(
                                        &config::data_dir(),
                                        &app.prefs,
                                    );
                                    cx.notify();
                                });
                                this.cs_name_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.cs_url_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.cs_user_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.cs_feedback = Some((true, "已保存 CardDAV 通讯录源".into()));
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element(),
    );

    v_flex()
        .w_full()
        .gap_5()
        .when_some(feedback_banner(win.cs_feedback.as_ref(), p), |el, b| {
            el.child(b)
        })
        .child(collect_group)
        .child(setting_group(
            "远程通讯录源（CardDAV）",
            None,
            CARD_MAX_WIDTH,
            rows,
            p,
        ))
        .into_any_element()
}

// ===========================================================================
// 4. 标签（SQLite `tags` 表）
// ===========================================================================

fn load_tags_from_db() -> Result<Vec<TagItem>, String> {
    let store = CacheStore::open_for_data_dir(&config::data_dir())?;
    store.list_tags()
}

pub(crate) fn render_tags(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let demo_mode = app_entity.read(cx).demo_mode;
    if !win.tags_loaded {
        let win_entity = cx.entity();
        if let Ok(tags) = load_tags_from_db() {
            cx.defer(move |cx| {
                let _ = win_entity.update(cx, |w, cx| {
                    w.cached_tags = tags;
                    w.tags_loaded = true;
                    cx.notify();
                });
            });
        }
    }

    let mut rows: Vec<AnyElement> = Vec::new();
    if win.cached_tags.is_empty() {
        rows.push(empty_row("尚未创建自定义标签，可在下方添加。", p));
    } else {
        for (idx, tag) in win.cached_tags.iter().enumerate() {
            let dot_color = avatar::parse_hex_color(&tag.color).unwrap_or(p.accent);
            let tag_id = tag.id;
            let tag_clone = tag.clone();
            let sc_text = tag
                .shortcut
                .map(|s| format!("快捷键 {s}"))
                .unwrap_or_else(|| "无快捷键".to_string());
            rows.push(
                h_flex()
                    .id(("tag-row", idx))
                    .w_full()
                    .min_h(px(50.0))
                    .px_4()
                    .py_2()
                    .gap_3()
                    .items_center()
                    .justify_between()
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2p5()
                            .items_center()
                            .child(div().size(px(12.0)).rounded(px(999.0)).bg(dot_color))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.text)
                                    .truncate()
                                    .child(SharedString::from(tag.name.clone())),
                            )
                            .child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(px(4.0))
                                    .bg(p.surface)
                                    .border_1()
                                    .border_color(p.border)
                                    .text_xs()
                                    .text_color(p.text_muted)
                                    .child(SharedString::from(sc_text)),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_shrink_0()
                            .child(
                                Button::new(("tag-edit", idx))
                                    .ghost()
                                    .compact()
                                    .disabled(demo_mode)
                                    .icon(IconName::Pencil)
                                    .tooltip("编辑标签")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.editing_tag_id = Some(tag_clone.id);
                                        this.tag_color = tag_clone.color.clone();
                                        this.tag_shortcut = tag_clone.shortcut;
                                        this.tag_name_input.update(cx, |s, cx| {
                                            s.set_value(tag_clone.name.clone(), window, cx)
                                        });
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new(("tag-del", idx))
                                    .ghost()
                                    .compact()
                                    .disabled(demo_mode)
                                    .icon(IconName::Trash)
                                    .tooltip("删除标签")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        match CacheStore::open_for_data_dir(&config::data_dir())
                                            .and_then(|mut s| {
                                                s.delete_tag(tag_id)?;
                                                s.list_tags()
                                            }) {
                                            Ok(list) => {
                                                this.cached_tags = list;
                                                if this.editing_tag_id == Some(tag_id) {
                                                    this.editing_tag_id = None;
                                                }
                                                this.tag_feedback =
                                                    Some((true, "已删除标签".into()));
                                            }
                                            Err(e) => {
                                                this.tag_feedback =
                                                    Some((false, SharedString::from(e)));
                                            }
                                        }
                                        cx.notify();
                                    })),
                            ),
                    )
                    .into_any_element(),
            );
        }
    }

    // 调色板圆点
    let mut palette_row = h_flex().gap_1p5().flex_wrap().items_center();
    for (idx, &(_label, hex)) in ACCOUNT_ACCENT_COLORS.iter().enumerate() {
        let col = avatar::parse_hex_color(hex).unwrap_or(p.accent);
        let selected = win.tag_color.eq_ignore_ascii_case(hex);
        palette_row = palette_row.child(
            div()
                .id(("tag-palette", idx))
                .size(px(22.0))
                .rounded(px(999.0))
                .bg(col)
                .border_2()
                .border_color(if selected { p.text } else { p.surface_raised })
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tag_color = hex.to_string();
                    cx.notify();
                }))
                .into_any_element(),
        );
    }

    let shortcut_options: &[(&str, Option<u8>)] = &[
        ("无快捷键", None),
        ("1", Some(1)),
        ("2", Some(2)),
        ("3", Some(3)),
        ("4", Some(4)),
        ("5", Some(5)),
        ("6", Some(6)),
        ("7", Some(7)),
        ("8", Some(8)),
        ("9", Some(9)),
    ];
    let sc_label = shortcut_options
        .iter()
        .find(|(_, v)| *v == win.tag_shortcut)
        .map(|(l, _)| *l)
        .unwrap_or("无快捷键");
    let win_sc = cx.entity();

    let is_editing = win.editing_tag_id.is_some();
    rows.push(
        v_flex()
            .w_full()
            .px_4()
            .py_3()
            .gap_2p5()
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&win.tag_name_input).cleanable(true)),
                    )
                    .child(combo_button(
                        "cb-tag-shortcut",
                        sc_label,
                        shortcut_options,
                        win.tag_shortcut,
                        demo_mode,
                        move |val, _, cx| {
                            let _ = win_sc.update(cx, |w, cx| {
                                w.tag_shortcut = val;
                                cx.notify();
                            });
                        },
                    )),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(palette_row)
                    .child(
                        h_flex()
                            .gap_1p5()
                            .when(is_editing, |el| {
                                el.child(
                                    Button::new("tag-cancel-edit")
                                        .outline()
                                        .compact()
                                        .label("取消")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.editing_tag_id = None;
                                            this.tag_name_input
                                                .update(cx, |s, cx| s.set_value("", window, cx));
                                            cx.notify();
                                        })),
                                )
                            })
                            .child(
                                Button::new("tag-save-btn")
                                    .primary()
                                    .compact()
                                    .disabled(demo_mode)
                                    .icon(if is_editing {
                                        IconName::Check
                                    } else {
                                        IconName::Plus
                                    })
                                    .label(if is_editing {
                                        "保存修改"
                                    } else {
                                        "添加标签"
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        let name =
                                            this.tag_name_input.read(cx).value().trim().to_string();
                                        match CacheStore::open_for_data_dir(&config::data_dir())
                                            .and_then(|mut store| {
                                                store.upsert_tag(
                                                    this.editing_tag_id,
                                                    &name,
                                                    &this.tag_color,
                                                    this.tag_shortcut,
                                                )?;
                                                store.list_tags()
                                            }) {
                                            Ok(list) => {
                                                this.cached_tags = list;
                                                this.tags_loaded = true;
                                                this.editing_tag_id = None;
                                                this.tag_name_input.update(cx, |s, cx| {
                                                    s.set_value("", window, cx)
                                                });
                                                this.tag_shortcut = None;
                                                this.tag_feedback =
                                                    Some((true, "已保存到 SQLite tags 表".into()));
                                            }
                                            Err(e) => {
                                                this.tag_feedback =
                                                    Some((false, SharedString::from(e)));
                                            }
                                        }
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .into_any_element(),
    );

    v_flex()
        .w_full()
        .gap_5()
        .when_some(feedback_banner(win.tag_feedback.as_ref(), p), |el, b| {
            el.child(b)
        })
        .child(setting_group(
            "自定义标签（SQLite tags 表）",
            Some("支持调色板颜色圆点与 1..9 数字快捷键绑定"),
            CARD_MAX_WIDTH,
            rows,
            p,
        ))
        .into_any_element()
}

// ===========================================================================
// 5. 过滤器规则
// ===========================================================================

pub(crate) fn render_filters(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let (prefs, accounts, demo_mode) = {
        let app = app_entity.read(cx);
        (app.prefs.clone(), app.accounts.clone(), app.demo_mode)
    };

    let mut rows: Vec<AnyElement> = Vec::new();
    if prefs.filter_rules.is_empty() {
        rows.push(empty_row("尚未创建自动化过滤规则。", p));
    } else {
        for (idx, rule) in prefs.filter_rules.iter().enumerate() {
            let app_tog = app_entity.clone();
            let app_del = app_entity.clone();
            let id = rule.id;
            let field_str = FILTER_FIELD_OPTIONS
                .iter()
                .find(|(_, f)| *f == rule.field)
                .map(|(l, _)| *l)
                .unwrap_or("发件人");
            let match_str = FILTER_MATCH_OPTIONS
                .iter()
                .find(|(_, m)| *m == rule.matcher)
                .map(|(l, _)| *l)
                .unwrap_or("包含");
            let mut actions = Vec::new();
            if !rule.move_to_folder.is_empty() {
                actions.push(format!("移动到 {}", rule.move_to_folder));
            }
            if rule.archive {
                actions.push("归档".to_string());
            }
            if rule.star {
                actions.push("标星".to_string());
            }
            if rule.mark_read {
                actions.push("标为已读".to_string());
            }
            if !rule.apply_tag.is_empty() {
                actions.push(format!("打标签 {}", rule.apply_tag));
            }
            if !rule.forward_to.is_empty() {
                actions.push(format!("转发 {}", rule.forward_to));
            }
            let sub = format!(
                "{field_str} {match_str} “{}” → {}",
                rule.value,
                if actions.is_empty() {
                    "无动作".to_string()
                } else {
                    actions.join("、")
                }
            );
            rows.push(
                h_flex()
                    .w_full()
                    .min_h(px(54.0))
                    .px_4()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.text)
                                    .truncate()
                                    .child(SharedString::from(if rule.name.trim().is_empty() {
                                        format!("规则 #{}", rule.id)
                                    } else {
                                        rule.name.clone()
                                    })),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.text_muted)
                                    .truncate()
                                    .child(SharedString::from(sub)),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_shrink_0()
                            .items_center()
                            .child(
                                Switch::new(("flt-sw", idx))
                                    .checked(rule.enabled)
                                    .disabled(demo_mode)
                                    .on_click(cx.listener(move |_, checked, _, cx| {
                                        app_tog.update(cx, |app, cx| {
                                            if let Some(r) = app
                                                .prefs
                                                .filter_rules
                                                .iter_mut()
                                                .find(|x| x.id == id)
                                            {
                                                r.enabled = *checked;
                                            }
                                            let _ = config::save_preferences_in(
                                                &config::data_dir(),
                                                &app.prefs,
                                            );
                                            cx.notify();
                                        });
                                    })),
                            )
                            .child(
                                Button::new(("flt-del", idx))
                                    .ghost()
                                    .compact()
                                    .disabled(demo_mode)
                                    .icon(IconName::Trash)
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        app_del.update(cx, |app, cx| {
                                            app.prefs.filter_rules.retain(|x| x.id != id);
                                            let _ = config::save_preferences_in(
                                                &config::data_dir(),
                                                &app.prefs,
                                            );
                                            cx.notify();
                                        });
                                    })),
                            ),
                    )
                    .into_any_element(),
            );
        }
    }

    let field_lbl = FILTER_FIELD_OPTIONS
        .iter()
        .find(|(_, f)| *f == win.filter_field)
        .map(|(l, _)| *l)
        .unwrap_or("发件人");
    let match_lbl = FILTER_MATCH_OPTIONS
        .iter()
        .find(|(_, m)| *m == win.filter_matcher)
        .map(|(l, _)| *l)
        .unwrap_or("包含");
    let mut acc_opts: Vec<(SharedString, String)> =
        vec![(SharedString::from("全部账号"), String::new())];
    for a in &accounts {
        acc_opts.push((SharedString::from(a.email.clone()), a.email.clone()));
    }
    let acc_lbl = acc_opts
        .iter()
        .find(|(_, e)| e == &win.filter_account_email)
        .map(|(l, _)| l.clone())
        .unwrap_or_else(|| SharedString::from("全部账号"));

    let win_f = cx.entity();
    let win_m = cx.entity();
    let win_a = cx.entity();
    let app_add = app_entity.clone();
    let app_run = app_entity.clone();

    rows.push(
        v_flex()
            .w_full()
            .px_4()
            .py_3()
            .gap_2()
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&win.filter_name_input).cleanable(true)),
                    )
                    .child(combo_button_dynamic(
                        "cb-flt-account",
                        acc_lbl,
                        acc_opts,
                        win.filter_account_email.clone(),
                        demo_mode,
                        move |val, _, cx| {
                            let _ = win_a.update(cx, |w, cx| {
                                w.filter_account_email = val;
                                cx.notify();
                            });
                        },
                    )),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(combo_button(
                        "cb-flt-field",
                        field_lbl,
                        FILTER_FIELD_OPTIONS,
                        win.filter_field,
                        demo_mode,
                        move |val, _, cx| {
                            let _ = win_f.update(cx, |w, cx| {
                                w.filter_field = val;
                                cx.notify();
                            });
                        },
                    ))
                    .child(combo_button(
                        "cb-flt-match",
                        match_lbl,
                        FILTER_MATCH_OPTIONS,
                        win.filter_matcher,
                        demo_mode,
                        move |val, _, cx| {
                            let _ = win_m.update(cx, |w, cx| {
                                w.filter_matcher = val;
                                cx.notify();
                            });
                        },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&win.filter_value_input).cleanable(true)),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&win.filter_folder_input).cleanable(true)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&win.filter_tag_input).cleanable(true)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&win.filter_forward_input).cleanable(true)),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_1p5()
                            .child(
                                Button::new("flt-opt-archive")
                                    .outline()
                                    .compact()
                                    .selected(win.filter_archive)
                                    .label("归档")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.filter_archive = !this.filter_archive;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("flt-opt-star")
                                    .outline()
                                    .compact()
                                    .selected(win.filter_star)
                                    .label("标星")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.filter_star = !this.filter_star;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("flt-opt-read")
                                    .outline()
                                    .compact()
                                    .selected(win.filter_mark_read)
                                    .label("标为已读")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.filter_mark_read = !this.filter_mark_read;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        Button::new("flt-add-btn")
                            .primary()
                            .compact()
                            .disabled(demo_mode)
                            .icon(IconName::Plus)
                            .label("添加规则")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let value =
                                    this.filter_value_input.read(cx).value().trim().to_string();
                                if value.is_empty() {
                                    this.filter_feedback =
                                        Some((false, "请填写过滤匹配关键词".into()));
                                    cx.notify();
                                    return;
                                }
                                let name =
                                    this.filter_name_input.read(cx).value().trim().to_string();
                                let move_to_folder =
                                    this.filter_folder_input.read(cx).value().trim().to_string();
                                let apply_tag =
                                    this.filter_tag_input.read(cx).value().trim().to_string();
                                let forward_to = this
                                    .filter_forward_input
                                    .read(cx)
                                    .value()
                                    .trim()
                                    .to_string();
                                let rule = FilterRule {
                                    id: 0,
                                    name,
                                    account_email: this.filter_account_email.clone(),
                                    enabled: true,
                                    match_all: true,
                                    field: this.filter_field,
                                    matcher: this.filter_matcher,
                                    value,
                                    move_to_folder,
                                    archive: this.filter_archive,
                                    star: this.filter_star,
                                    mark_read: this.filter_mark_read,
                                    apply_tag,
                                    forward_to,
                                };
                                app_add.update(cx, |app, cx| {
                                    let next_id = app
                                        .prefs
                                        .filter_rules
                                        .iter()
                                        .map(|r| r.id)
                                        .max()
                                        .unwrap_or(0)
                                        + 1;
                                    let mut r = rule;
                                    r.id = next_id;
                                    app.prefs.filter_rules.push(r);
                                    let _ = config::save_preferences_in(
                                        &config::data_dir(),
                                        &app.prefs,
                                    );
                                    cx.notify();
                                });
                                this.filter_name_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.filter_value_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.filter_folder_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.filter_tag_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.filter_forward_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.filter_archive = false;
                                this.filter_star = false;
                                this.filter_mark_read = false;
                                this.filter_feedback = Some((true, "已添加过滤器规则".into()));
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element(),
    );

    let run_group = setting_group(
        "立即运行",
        Some("对本地 SQLite 缓存的现有邮件执行已启用的过滤规则（标为已读 / 标星 / 移动或归档）"),
        CARD_MAX_WIDTH,
        vec![setting_row(
            "pref-filters-manage",
            "在现有邮件上运行过滤器",
            Some("仅处理已开启的规则"),
            Button::new("flt-run-now")
                .outline()
                .compact()
                .disabled(demo_mode || prefs.filter_rules.is_empty())
                .icon(IconName::Play)
                .label("立即在现有邮件上运行")
                .on_click(cx.listener(move |this, _, _, cx| {
                    let (rules, accs) = {
                        let app = app_run.read(cx);
                        (app.prefs.filter_rules.clone(), app.accounts.clone())
                    };
                    match CacheStore::open_for_data_dir(&config::data_dir())
                        .and_then(|mut store| store.apply_filter_rules(&rules, &accs))
                    {
                        Ok(count) => {
                            this.filter_feedback = Some((
                                true,
                                SharedString::from(format!("已扫描本地缓存，更新 {count} 封邮件")),
                            ));
                        }
                        Err(e) => {
                            this.filter_feedback = Some((false, SharedString::from(e)));
                        }
                    }
                    cx.notify();
                }))
                .into_any_element(),
            demo_mode,
            win.highlighted_row_id == Some("pref-filters-manage"),
            p,
        )],
        p,
    );

    v_flex()
        .w_full()
        .gap_5()
        .when_some(feedback_banner(win.filter_feedback.as_ref(), p), |el, b| {
            el.child(b)
        })
        .child(run_group)
        .child(setting_group(
            "自动化规则列表",
            None,
            CARD_MAX_WIDTH,
            rows,
            p,
        ))
        .into_any_element()
}

// ===========================================================================
// 6. 智能文件夹
// ===========================================================================

pub(crate) fn render_smart_folders(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let (prefs, accounts, demo_mode) = {
        let app = app_entity.read(cx);
        (app.prefs.clone(), app.accounts.clone(), app.demo_mode)
    };

    let mut rows: Vec<AnyElement> = Vec::new();
    if prefs.smart_folders.is_empty() {
        rows.push(empty_row("尚未创建智能文件夹。", p));
    } else {
        for (idx, sf) in prefs.smart_folders.iter().enumerate() {
            let app_del = app_entity.clone();
            let id = sf.id;
            let scope = if sf.account_email.is_empty() {
                "全部账号".to_string()
            } else {
                sf.account_email.clone()
            };
            let sub = format!("{scope} · 查询: {}", sf.query);
            rows.push(
                h_flex()
                    .w_full()
                    .min_h(px(52.0))
                    .px_4()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.text)
                                    .truncate()
                                    .child(SharedString::from(sf.name.clone())),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.text_muted)
                                    .truncate()
                                    .child(SharedString::from(sub)),
                            ),
                    )
                    .child(
                        Button::new(("sf-del", idx))
                            .ghost()
                            .compact()
                            .disabled(demo_mode)
                            .icon(IconName::Trash)
                            .on_click(cx.listener(move |_, _, _, cx| {
                                app_del.update(cx, |app, cx| {
                                    app.prefs.smart_folders.retain(|x| x.id != id);
                                    let _ = config::save_preferences_in(
                                        &config::data_dir(),
                                        &app.prefs,
                                    );
                                    cx.notify();
                                });
                            })),
                    )
                    .into_any_element(),
            );
        }
    }

    let (icon_lbl, icon_cur) = SMART_ICON_OPTIONS
        .iter()
        .find(|(_, v)| *v == win.smart_icon)
        .copied()
        .unwrap_or(("智能搜索", "folder-search"));
    let mut acc_opts: Vec<(SharedString, String)> =
        vec![(SharedString::from("全部账号"), String::new())];
    for a in &accounts {
        acc_opts.push((SharedString::from(a.email.clone()), a.email.clone()));
    }
    let acc_lbl = acc_opts
        .iter()
        .find(|(_, e)| e == &win.smart_account_email)
        .map(|(l, _)| l.clone())
        .unwrap_or_else(|| SharedString::from("全部账号"));

    let win_icon = cx.entity();
    let win_acc = cx.entity();
    let app_add = app_entity.clone();
    rows.push(
        v_flex()
            .w_full()
            .px_4()
            .py_3()
            .gap_2()
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&win.smart_name_input).cleanable(true)),
                    )
                    .child(combo_button(
                        "cb-sf-icon",
                        icon_lbl,
                        SMART_ICON_OPTIONS,
                        icon_cur,
                        demo_mode,
                        move |val, _, cx| {
                            let _ = win_icon.update(cx, |w, cx| {
                                w.smart_icon = val.to_string();
                                cx.notify();
                            });
                        },
                    ))
                    .child(combo_button_dynamic(
                        "cb-sf-acc",
                        acc_lbl,
                        acc_opts,
                        win.smart_account_email.clone(),
                        demo_mode,
                        move |val, _, cx| {
                            let _ = win_acc.update(cx, |w, cx| {
                                w.smart_account_email = val;
                                cx.notify();
                            });
                        },
                    )),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&win.smart_query_input).cleanable(true)),
                    )
                    .child(
                        Button::new("sf-add-btn")
                            .primary()
                            .compact()
                            .disabled(demo_mode)
                            .icon(IconName::Plus)
                            .label("创建智能文件夹")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let name =
                                    this.smart_name_input.read(cx).value().trim().to_string();
                                let query =
                                    this.smart_query_input.read(cx).value().trim().to_string();
                                if name.is_empty() || query.is_empty() {
                                    this.smart_feedback =
                                        Some((false, "请填写名称与搜索表达式".into()));
                                    cx.notify();
                                    return;
                                }
                                let icon = this.smart_icon.clone();
                                let account_email = this.smart_account_email.clone();
                                app_add.update(cx, |app, cx| {
                                    let next_id = app
                                        .prefs
                                        .smart_folders
                                        .iter()
                                        .map(|s| s.id)
                                        .max()
                                        .unwrap_or(0)
                                        + 1;
                                    app.prefs.smart_folders.push(SmartFolderConfig {
                                        id: next_id,
                                        name,
                                        icon,
                                        query,
                                        account_email,
                                    });
                                    let _ = config::save_preferences_in(
                                        &config::data_dir(),
                                        &app.prefs,
                                    );
                                    cx.notify();
                                });
                                this.smart_name_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.smart_query_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.smart_feedback = Some((true, "已创建智能文件夹".into()));
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element(),
    );

    v_flex()
        .w_full()
        .gap_5()
        .when_some(feedback_banner(win.smart_feedback.as_ref(), p), |el, b| {
            el.child(b)
        })
        .child(setting_group(
            "智能文件夹（动态虚拟视图）",
            Some("支持搜索语法如 from:alice is:unread has:attachment"),
            CARD_MAX_WIDTH,
            rows,
            p,
        ))
        .into_any_element()
}

// ===========================================================================
// 7. 翻译
// ===========================================================================

pub(crate) fn render_translation(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let (prefs, demo_mode) = {
        let app = app_entity.read(cx);
        (app.prefs.clone(), app.demo_mode)
    };
    let hl = win.highlighted_row_id;

    let (engine_lbl, engine_cur) = TRANSLATION_ENGINES
        .iter()
        .find(|(_, v)| *v == prefs.translation_engine)
        .copied()
        .unwrap_or(("关闭", "off"));
    let (target_lbl, target_cur) = TRANSLATION_TARGETS
        .iter()
        .find(|(_, v)| *v == prefs.translation_target_language)
        .copied()
        .unwrap_or(("跟随应用界面语言", "system"));

    let app_eng = app_entity.clone();
    let app_tgt = app_entity.clone();
    let app_save = app_entity.clone();

    let engine_group = setting_group(
        "翻译引擎",
        Some("仅在您点击翻译时发送邮件文本到所选引擎"),
        CARD_MAX_WIDTH,
        vec![
            setting_row(
                "pref-translation-engine",
                "翻译服务",
                Some("支持 LibreTranslate / DeepL / Google / 本地 LLM"),
                combo_button(
                    "cb-trans-engine",
                    engine_lbl,
                    TRANSLATION_ENGINES,
                    engine_cur,
                    demo_mode,
                    move |val, _, cx| {
                        app_eng.update(cx, |app, cx| {
                            app.prefs.translation_engine = val.to_string();
                            let _ = config::save_preferences_in(&config::data_dir(), &app.prefs);
                            cx.notify();
                        });
                    },
                ),
                demo_mode,
                hl == Some("pref-translation-engine"),
                p,
            ),
            v_flex()
                .w_full()
                .px_4()
                .py_3()
                .gap_2()
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .child(Input::new(&win.trans_url_input).cleanable(true)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .child(Input::new(&win.trans_key_input).cleanable(true)),
                        ),
                )
                .child(
                    h_flex().w_full().justify_end().child(
                        Button::new("trans-save-btn")
                            .outline()
                            .compact()
                            .disabled(demo_mode)
                            .label("保存服务地址")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let url = this.trans_url_input.read(cx).value().trim().to_string();
                                app_save.update(cx, |app, cx| {
                                    app.prefs.translation_api_url = url;
                                    let _ = config::save_preferences_in(
                                        &config::data_dir(),
                                        &app.prefs,
                                    );
                                    cx.notify();
                                });
                                this.trans_feedback = Some((true, "已保存翻译服务地址配置".into()));
                                cx.notify();
                            })),
                    ),
                )
                .into_any_element(),
        ],
        p,
    );

    let behavior_group = setting_group(
        "目标语言与入口",
        None,
        CARD_MAX_WIDTH,
        vec![
            setting_row(
                "pref-translation-target",
                "翻译目标语言",
                None,
                combo_button(
                    "cb-trans-target",
                    target_lbl,
                    TRANSLATION_TARGETS,
                    target_cur,
                    demo_mode,
                    move |val, _, cx| {
                        app_tgt.update(cx, |app, cx| {
                            app.prefs.translation_target_language = val.to_string();
                            let _ = config::save_preferences_in(&config::data_dir(), &app.prefs);
                            cx.notify();
                        });
                    },
                ),
                demo_mode,
                false,
                p,
            ),
            setting_row(
                "pref-translation-reader",
                "阅读器提供一键翻译",
                Some("检测到非目标语言邮件时展示翻译按钮"),
                pref_switch(
                    "sw-trans-reader",
                    prefs.translation_offer_reader,
                    demo_mode,
                    app_entity.clone(),
                    |p, v| p.translation_offer_reader = v,
                ),
                demo_mode,
                false,
                p,
            ),
            setting_row(
                "pref-translation-composer",
                "写信器提供一键翻译",
                Some("在写信工具栏中提供正文翻译入口"),
                pref_switch(
                    "sw-trans-composer",
                    prefs.translation_offer_composer,
                    demo_mode,
                    app_entity,
                    |p, v| p.translation_offer_composer = v,
                ),
                demo_mode,
                false,
                p,
            ),
        ],
        p,
    );

    v_flex()
        .w_full()
        .gap_5()
        .when_some(feedback_banner(win.trans_feedback.as_ref(), p), |el, b| {
            el.child(b)
        })
        .child(engine_group)
        .child(behavior_group)
        .into_any_element()
}

// ===========================================================================
// 8. MCP 服务
// ===========================================================================

pub(crate) fn render_mcp(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let (prefs, demo_mode) = {
        let app = app_entity.read(cx);
        (app.prefs.clone(), app.demo_mode)
    };
    let hl = win.highlighted_row_id;
    let app_save = app_entity.clone();

    let server_group = setting_group(
        "MCP 服务",
        Some("允许外部 AI 助手通过 MCP (Model Context Protocol) 按权限访问本地邮件（Windows 命名管道监听待后台服务端接入）"),
        CARD_MAX_WIDTH,
        vec![
            setting_row(
                "pref-mcp-enabled",
                "启用本地 MCP Server",
                Some("开启后将按下方工具权限暴露只读/草稿能力"),
                pref_switch(
                    "sw-mcp-enabled",
                    prefs.mcp_enabled,
                    demo_mode,
                    app_entity.clone(),
                    |p, v| p.mcp_enabled = v,
                ),
                demo_mode,
                hl == Some("pref-mcp-enabled"),
                p,
            ),
            h_flex()
                .w_full()
                .px_4()
                .py_3()
                .gap_2()
                .items_center()
                .child(div().flex_1().child(Input::new(&win.mcp_socket_input).cleanable(true)))
                .child(
                    Button::new("mcp-save-socket")
                        .outline()
                        .compact()
                        .disabled(demo_mode)
                        .label("保存路径")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let path = this
                                .mcp_socket_input
                                .read(cx)
                                .value()
                                .trim()
                                .to_string();
                            app_save.update(cx, |app, cx| {
                                app.prefs.mcp_socket_path = if path.is_empty() {
                                    r"\\.\pipe\hylki-mcp".to_string()
                                } else {
                                    path
                                };
                                let _ = config::save_preferences_in(
                                    &config::data_dir(),
                                    &app.prefs,
                                );
                                cx.notify();
                            });
                            this.mcp_feedback = Some((true, "已保存 MCP 命名管道路径".into()));
                            cx.notify();
                        })),
                )
                .into_any_element(),
        ],
        p,
    );

    let perms_group = setting_group(
        "工具权限",
        Some("外部 AI 助手仅可调用已授权的工具；不允许直接发信或永久删信"),
        CARD_MAX_WIDTH,
        vec![
            setting_row(
                "pref-mcp-read",
                "允许读取邮件内容",
                Some("读取本地缓存的邮件正文与发件人信息"),
                pref_switch(
                    "sw-mcp-read",
                    prefs.mcp_allow_read,
                    demo_mode,
                    app_entity.clone(),
                    |p, v| p.mcp_allow_read = v,
                ),
                demo_mode,
                false,
                p,
            ),
            setting_row(
                "pref-mcp-search",
                "允许搜索邮件",
                Some("执行全文检索与条件筛选"),
                pref_switch(
                    "sw-mcp-search",
                    prefs.mcp_allow_search,
                    demo_mode,
                    app_entity.clone(),
                    |p, v| p.mcp_allow_search = v,
                ),
                demo_mode,
                false,
                p,
            ),
            setting_row(
                "pref-mcp-draft",
                "允许创建草稿",
                Some("生成待您审核发送的回复草稿"),
                pref_switch(
                    "sw-mcp-draft",
                    prefs.mcp_allow_draft,
                    demo_mode,
                    app_entity.clone(),
                    |p, v| p.mcp_allow_draft = v,
                ),
                demo_mode,
                false,
                p,
            ),
            setting_row(
                "pref-mcp-flags",
                "允许标记已读 / 星标 / 标签",
                Some("整理收件箱状态与分类标签"),
                pref_switch(
                    "sw-mcp-flags",
                    prefs.mcp_allow_modify_flags,
                    demo_mode,
                    app_entity,
                    |p, v| p.mcp_allow_modify_flags = v,
                ),
                demo_mode,
                false,
                p,
            ),
        ],
        p,
    );

    v_flex()
        .w_full()
        .gap_5()
        .when_some(feedback_banner(win.mcp_feedback.as_ref(), p), |el, b| {
            el.child(b)
        })
        .child(server_group)
        .child(perms_group)
        .into_any_element()
}

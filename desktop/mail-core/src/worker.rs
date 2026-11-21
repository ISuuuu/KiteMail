//! Dedicated Tokio worker thread orchestrating cache, credentials, IMAP, SMTP, and Graph OAuth2.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::cache::CacheStore;
use crate::config::{self, AccountConfig, AuthBackend};
use crate::secrets::{self, AccountSecrets};
use crate::smtp_client::SendOutcome;
use crate::{
    graph_client, imap_client, oauth, smtp_client, MailRequest, OutboxStatus, UnifiedView,
    WorkerEvent,
};

type TokenCache = HashMap<u32, (String, Instant)>;

/// Handle owned by the UI to dispatch [`MailRequest`]s and poll [`WorkerEvent`]s.
pub struct MailWorkerHandle {
    req_tx: mpsc::Sender<MailRequest>,
    evt_rx: mpsc::Receiver<WorkerEvent>,
}

impl MailWorkerHandle {
    /// Spawn the background Tokio runtime and worker loop using `data_dir`.
    pub fn spawn(data_dir: PathBuf) -> Result<Self, String> {
        let (req_tx, req_rx) = mpsc::channel::<MailRequest>();
        let (evt_tx, evt_rx) = mpsc::channel::<WorkerEvent>();

        std::thread::Builder::new()
            .name("hylki-mail-worker".to_string())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .worker_threads(2)
                    .build()
                    .expect("failed to start Tokio runtime");

                rt.block_on(worker_loop(data_dir, req_rx, evt_tx));
            })
            .map_err(|e| format!("启动邮件后台线程失败: {e}"))?;

        Ok(Self { req_tx, evt_rx })
    }

    /// Send a request to the background mail worker.
    pub fn send(&self, req: MailRequest) {
        let _ = self.req_tx.send(req);
    }

    /// Non-blocking drain of all pending [`WorkerEvent`]s.
    pub fn drain_events(&self) -> Vec<WorkerEvent> {
        let mut out = Vec::new();
        while let Ok(evt) = self.evt_rx.try_recv() {
            out.push(evt);
        }
        out
    }
}

async fn worker_loop(
    data_dir: PathBuf,
    req_rx: mpsc::Receiver<MailRequest>,
    evt_tx: mpsc::Sender<WorkerEvent>,
) {
    let db_path = config::cache_db_in(&data_dir);
    let mut cache = match CacheStore::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            let _ = evt_tx.send(WorkerEvent::Error {
                account_id: None,
                message: e,
            });
            return;
        }
    };
    let mut tokens: TokenCache = HashMap::new();

    while let Ok(req) = req_rx.recv() {
        handle_request(&data_dir, &mut cache, &mut tokens, req, &evt_tx).await;
    }
}

async fn handle_request(
    data_dir: &PathBuf,
    cache: &mut CacheStore,
    tokens: &mut TokenCache,
    req: MailRequest,
    evt_tx: &mpsc::Sender<WorkerEvent>,
) {
    match req {
        MailRequest::VerifyAccount {
            config,
            imap_password,
            smtp_password,
        } => {
            let smtp_pwd = if config.smtp_separate_credentials {
                smtp_password.as_deref().unwrap_or(&imap_password)
            } else {
                imap_password.as_str()
            };
            let imap_res = imap_client::verify_login(&config, &imap_password).await;
            let smtp_res = smtp_client::verify_login(&config, smtp_pwd).await;
            match (imap_res, smtp_res) {
                (Ok(()), Ok(())) => {
                    let _ = evt_tx.send(WorkerEvent::AccountVerified {
                        ok: true,
                        message: "IMAP 与 SMTP 验证均通过".to_string(),
                    });
                }
                (Err(e), _) | (_, Err(e)) => {
                    let _ = evt_tx.send(WorkerEvent::AccountVerified {
                        ok: false,
                        message: e,
                    });
                }
            }
        }

        MailRequest::SaveAndSyncAccount {
            config,
            imap_password,
            smtp_password,
        } => {
            if let Err(e) = secrets::save_account_secrets(
                config.id,
                AccountSecrets {
                    imap_password,
                    smtp_password,
                },
            ) {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(config.id),
                    message: e,
                });
                return;
            }
            if let Err(e) = config::upsert_account_in(data_dir, config.clone()) {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(config.id),
                    message: e,
                });
                return;
            }
            let account_id = config.id;
            let _ = evt_tx.send(WorkerEvent::AccountSaved { account: config });
            sync_account_inner(data_dir, cache, tokens, account_id, evt_tx).await;
        }

        MailRequest::StartMicrosoftOAuth {
            custom_client_id,
            editing_account_id,
        } => {
            let _ = evt_tx.send(WorkerEvent::Status {
                account_id: None,
                text: "已打开浏览器，请在微软页面完成 Outlook 授权...".to_string(),
            });

            let sign_in = tokio::task::spawn_blocking(move || {
                oauth::run_microsoft_oauth_login(&custom_client_id)
            })
            .await;

            let result = match sign_in {
                Ok(Ok(res)) => res,
                Ok(Err(e)) => {
                    let _ = evt_tx.send(WorkerEvent::AccountVerified {
                        ok: false,
                        message: e.clone(),
                    });
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: None,
                        message: e,
                    });
                    return;
                }
                Err(e) => {
                    let msg = format!("OAuth2 登录任务异常: {e}");
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: None,
                        message: msg,
                    });
                    return;
                }
            };

            let existing = config::load_accounts_in(data_dir).unwrap_or_default();
            let account_id = editing_account_id
                .or_else(|| {
                    existing
                        .iter()
                        .find(|a| a.email.eq_ignore_ascii_case(&result.email))
                        .map(|a| a.id)
                })
                .unwrap_or_else(|| config::next_account_id(&existing));

            let account = AccountConfig {
                id: account_id,
                auth_backend: AuthBackend::MicrosoftGraphOAuth2,
                oauth_client_id: result.client_id,
                display_name: result.display_name,
                email: result.email.clone(),
                imap_host: "outlook.office365.com".to_string(),
                imap_port: 993,
                imap_security: config::SecurityMode::Tls,
                imap_username: result.email.clone(),
                smtp_host: "smtp.office365.com".to_string(),
                smtp_port: 587,
                smtp_security: config::SecurityMode::StartTls,
                smtp_username: result.email,
                smtp_separate_credentials: false,
            };

            if let Err(e) = secrets::save_oauth_refresh_token(account_id, result.refresh_token) {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: e,
                });
                return;
            }
            if let Err(e) = config::upsert_account_in(data_dir, account.clone()) {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: e,
                });
                return;
            }

            let expires_at =
                Instant::now() + Duration::from_secs(result.expires_in_secs.saturating_sub(120));
            tokens.insert(account_id, (result.access_token, expires_at));

            let _ = evt_tx.send(WorkerEvent::AccountVerified {
                ok: true,
                message: "Outlook OAuth2 授权成功，正在同步邮件...".to_string(),
            });
            let _ = evt_tx.send(WorkerEvent::AccountSaved { account });
            sync_account_inner(data_dir, cache, tokens, account_id, evt_tx).await;
        }

        MailRequest::DeleteAccount { account_id } => {
            tokens.remove(&account_id);
            let _ = secrets::delete_account_secrets(account_id);
            let _ = config::delete_account_in(data_dir, account_id);
            let _ = cache.delete_account_data(account_id);
            let _ = evt_tx.send(WorkerEvent::AccountDeleted { account_id });
            if let Ok(drafts) = cache.list_drafts() {
                let _ = evt_tx.send(WorkerEvent::DraftsUpdated { drafts });
            }
            if let Ok(page) = cache.list_unified_inbox("", 0, 100) {
                let _ = evt_tx.send(WorkerEvent::MessagesPageLoaded(page));
            }
        }

        MailRequest::LoadAllCached => {
            let accounts = config::load_accounts_in(data_dir).unwrap_or_default();
            for acc in &accounts {
                if let Ok(folders) = cache.list_folders(acc.id) {
                    let _ = evt_tx.send(WorkerEvent::FoldersLoaded {
                        account_id: acc.id,
                        folders,
                    });
                }
                if let Ok(items) = cache.list_outbox(acc.id) {
                    let _ = evt_tx.send(WorkerEvent::OutboxUpdated {
                        account_id: acc.id,
                        items,
                    });
                }
            }
            if let Ok(drafts) = cache.list_drafts() {
                let _ = evt_tx.send(WorkerEvent::DraftsUpdated { drafts });
            }
            // 初始页大小跟随用户偏好设置（读取失败回退 100）。
            let page_size = config::load_preferences_in(data_dir)
                .map(|p| p.page_size)
                .unwrap_or(100);
            if let Ok(page) = cache.list_unified_inbox("", 0, page_size) {
                let _ = evt_tx.send(WorkerEvent::MessagesPageLoaded(page));
            }
        }

        MailRequest::SyncAllAccounts => {
            let accounts = config::load_accounts_in(data_dir).unwrap_or_default();
            for acc in accounts {
                sync_account_inner(data_dir, cache, tokens, acc.id, evt_tx).await;
            }
            if let Ok(page) = cache.list_unified_inbox("", 0, 100) {
                let _ = evt_tx.send(WorkerEvent::MessagesPageLoaded(page));
            }
        }

        MailRequest::SyncAccount { account_id } => {
            sync_account_inner(data_dir, cache, tokens, account_id, evt_tx).await;
        }

        MailRequest::SyncFolder {
            account_id,
            folder_id,
            folder_path,
        } => {
            let Ok(account) = load_account_config(data_dir, account_id) else {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: format!("账号 #{account_id} 未找到配置"),
                });
                return;
            };
            sync_folder_inner(cache, tokens, &account, folder_id, &folder_path, evt_tx).await;
        }

        MailRequest::LoadUnifiedInbox {
            query,
            filter,
            offset,
            limit,
        } => match cache.list_unified_inbox_filtered(&query, filter, offset, limit) {
            Ok(page) => {
                let _ = evt_tx.send(WorkerEvent::MessagesPageLoaded(page));
            }
            Err(e) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: None,
                    message: e,
                });
            }
        },

        MailRequest::LoadMessagesPage {
            folder_id,
            query,
            filter,
            offset,
            limit,
        } => match cache.list_messages_page_filtered(folder_id, &query, filter, offset, limit) {
            Ok(page) => {
                let _ = evt_tx.send(WorkerEvent::MessagesPageLoaded(page));
            }
            Err(e) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: None,
                    message: e,
                });
            }
        },

        MailRequest::LoadMessageBody {
            account_id,
            message_id,
        } => {
            let Ok(Some((detail, uid, body_loaded))) = cache.get_message_detail(message_id) else {
                return;
            };
            if body_loaded {
                let _ = evt_tx.send(WorkerEvent::MessageBodyLoaded(detail));
                return;
            }
            let Ok(account) = load_account_config(data_dir, account_id) else {
                let _ = evt_tx.send(WorkerEvent::MessageBodyLoaded(detail));
                return;
            };

            let _ = evt_tx.send(WorkerEvent::Status {
                account_id: Some(account_id),
                text: format!("[{}] 正在获取邮件正文...", account.email),
            });

            let fetch_res = match account.auth_backend {
                AuthBackend::MicrosoftGraphOAuth2 => {
                    let Some(remote_id) = cache.get_message_remote_id(message_id).ok().flatten()
                    else {
                        let _ = evt_tx.send(WorkerEvent::MessageBodyLoaded(detail));
                        return;
                    };
                    match ensure_graph_token(tokens, &account).await {
                        Ok(token) => tokio::task::spawn_blocking(move || {
                            graph_client::fetch_message_body(&token, &remote_id)
                        })
                        .await
                        .unwrap_or_else(|e| Err(format!("Graph 任务异常: {e}"))),
                        Err(e) => Err(e),
                    }
                }
                AuthBackend::Password => {
                    let Ok(Some((folder_path, _))) = cache.folder_meta(detail.folder_id) else {
                        return;
                    };
                    match secrets::load_account_secrets(account_id) {
                        Ok(sec) => {
                            imap_client::fetch_message_body(
                                &account,
                                &sec.imap_password,
                                &folder_path,
                                uid,
                            )
                            .await
                        }
                        Err(e) => Err(e),
                    }
                }
            };

            match fetch_res {
                Ok((body_text, attachments)) => {
                    if let Err(e) = cache.store_message_body(message_id, &body_text, &attachments) {
                        let _ = evt_tx.send(WorkerEvent::Error {
                            account_id: Some(account_id),
                            message: e,
                        });
                        return;
                    }
                    if let Ok(Some((updated, _, _))) = cache.get_message_detail(message_id) {
                        let _ = evt_tx.send(WorkerEvent::MessageBodyLoaded(updated));
                    }
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: format!("[{}] {e}", account.email),
                    });
                }
            }
        }

        MailRequest::SetSeen {
            account_id,
            folder_path,
            message_id,
            uid,
            seen,
        } => {
            let _ = cache.set_message_seen(message_id, seen);
            if let Ok(folders) = cache.list_folders(account_id) {
                let _ = evt_tx.send(WorkerEvent::FoldersLoaded {
                    account_id,
                    folders,
                });
            }
            if let Ok(account) = load_account_config(data_dir, account_id) {
                let res = match account.auth_backend {
                    AuthBackend::MicrosoftGraphOAuth2 => {
                        if let Ok(Some(remote_id)) = cache.get_message_remote_id(message_id) {
                            match ensure_graph_token(tokens, &account).await {
                                Ok(token) => tokio::task::spawn_blocking(move || {
                                    graph_client::set_seen(&token, &remote_id, seen)
                                })
                                .await
                                .unwrap_or_else(|e| Err(format!("Graph 任务异常: {e}"))),
                                Err(e) => Err(e),
                            }
                        } else {
                            Ok(())
                        }
                    }
                    AuthBackend::Password => match secrets::load_account_secrets(account_id) {
                        Ok(sec) => {
                            imap_client::set_flag(
                                &account,
                                &sec.imap_password,
                                &folder_path,
                                uid,
                                "\\Seen",
                                seen,
                            )
                            .await
                        }
                        Err(e) => Err(e),
                    },
                };
                if let Err(e) = res {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: format!("[{}] {e}", account.email),
                    });
                }
            }
        }

        MailRequest::SetFlagged {
            account_id,
            folder_path,
            message_id,
            uid,
            flagged,
        } => {
            let _ = cache.set_message_flagged(message_id, flagged);
            if let Ok(Some((updated, _, _))) = cache.get_message_detail(message_id) {
                let _ = evt_tx.send(WorkerEvent::MessageBodyLoaded(updated));
            }
            if let Ok(account) = load_account_config(data_dir, account_id) {
                let res = match account.auth_backend {
                    AuthBackend::MicrosoftGraphOAuth2 => {
                        if let Ok(Some(remote_id)) = cache.get_message_remote_id(message_id) {
                            match ensure_graph_token(tokens, &account).await {
                                Ok(token) => tokio::task::spawn_blocking(move || {
                                    graph_client::set_flagged(&token, &remote_id, flagged)
                                })
                                .await
                                .unwrap_or_else(|e| Err(format!("Graph 任务异常: {e}"))),
                                Err(e) => Err(e),
                            }
                        } else {
                            Ok(())
                        }
                    }
                    AuthBackend::Password => match secrets::load_account_secrets(account_id) {
                        Ok(sec) => {
                            imap_client::set_flag(
                                &account,
                                &sec.imap_password,
                                &folder_path,
                                uid,
                                "\\Flagged",
                                flagged,
                            )
                            .await
                        }
                        Err(e) => Err(e),
                    },
                };
                if let Err(e) = res {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: format!("[{}] {e}", account.email),
                    });
                }
            }
        }

        MailRequest::ArchiveMessage {
            account_id,
            message_id,
        } => {
            move_message_inner(
                data_dir,
                cache,
                tokens,
                account_id,
                message_id,
                MoveTarget::Archive,
                evt_tx,
            )
            .await;
        }

        MailRequest::DeleteMessage {
            account_id,
            message_id,
        } => {
            move_message_inner(
                data_dir,
                cache,
                tokens,
                account_id,
                message_id,
                MoveTarget::Trash,
                evt_tx,
            )
            .await;
        }

        MailRequest::SaveAttachment {
            attachment_id,
            dest_path,
        } => match cache.get_attachment_bytes(attachment_id) {
            Ok(Some((_name, bytes))) => match std::fs::write(&dest_path, bytes) {
                Ok(()) => {
                    let _ = evt_tx.send(WorkerEvent::AttachmentSaved { path: dest_path });
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: None,
                        message: format!("保存附件失败: {e}"),
                    });
                }
            },
            Ok(None) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: None,
                    message: "本地缓存中未找到该附件".to_string(),
                });
            }
            Err(e) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: None,
                    message: e,
                });
            }
        },

        MailRequest::SaveLocalDraft { mut draft } => {
            draft.updated_at = chrono::Utc::now().timestamp();
            match cache.upsert_draft(&draft) {
                Ok(_) => {
                    if let Ok(drafts) = cache.list_drafts() {
                        let _ = evt_tx.send(WorkerEvent::DraftsUpdated { drafts });
                    }
                    let _ = evt_tx.send(WorkerEvent::Status {
                        account_id: Some(draft.account_id),
                        text: "草稿已保存至本地".to_string(),
                    });
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(draft.account_id),
                        message: e,
                    });
                }
            }
        }

        MailRequest::DeleteLocalDraft { draft_id } => {
            let _ = cache.delete_draft(draft_id);
            if let Ok(drafts) = cache.list_drafts() {
                let _ = evt_tx.send(WorkerEvent::DraftsUpdated { drafts });
            }
        }

        MailRequest::SendDraft {
            account_id,
            draft,
            local_draft_id,
        } => {
            let now = chrono::Utc::now().timestamp();
            let outbox_id = match cache.enqueue_outbox(
                account_id,
                &draft.to,
                &draft.cc,
                &draft.bcc,
                &draft.subject,
                &draft.body,
                &draft.in_reply_to,
                &draft.attachment_paths,
                now,
            ) {
                Ok(id) => id,
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: e,
                    });
                    return;
                }
            };
            if let Some(draft_id) = local_draft_id {
                let _ = cache.delete_draft(draft_id);
                if let Ok(drafts) = cache.list_drafts() {
                    let _ = evt_tx.send(WorkerEvent::DraftsUpdated { drafts });
                }
            }
            send_outbox_item(data_dir, cache, tokens, account_id, outbox_id, evt_tx).await;
        }

        MailRequest::RetryOutboxItem {
            account_id,
            outbox_id,
        } => {
            send_outbox_item(data_dir, cache, tokens, account_id, outbox_id, evt_tx).await;
        }

        MailRequest::LoadUnifiedView {
            view,
            query,
            filter,
            offset,
            limit,
        } => {
            let result = match view {
                UnifiedView::Starred => {
                    cache.list_starred_messages_filtered(&query, filter, offset, limit)
                }
                UnifiedView::Sent => {
                    cache.list_sent_messages_filtered(&query, filter, offset, limit)
                }
                UnifiedView::Archive => {
                    cache.list_archived_messages_filtered(&query, filter, offset, limit)
                }
            };
            match result {
                Ok(page) => {
                    let _ = evt_tx.send(WorkerEvent::UnifiedViewLoaded { view, page });
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: None,
                        message: e,
                    });
                }
            }
        }

        MailRequest::ListCachedAttachments {
            query,
            offset,
            limit,
        } => match cache.list_cached_attachments(&query, offset, limit) {
            Ok(items) => {
                let _ = evt_tx.send(WorkerEvent::CachedAttachmentsLoaded { items });
            }
            Err(e) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: None,
                    message: e,
                });
            }
        },

        MailRequest::ListKnownContacts {
            query,
            offset,
            limit,
        } => match cache.list_known_contacts(&query, offset, limit) {
            Ok(items) => {
                let _ = evt_tx.send(WorkerEvent::KnownContactsLoaded { items });
            }
            Err(e) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: None,
                    message: e,
                });
            }
        },

        MailRequest::ClearBodyCaches => match cache.clear_message_bodies() {
            Ok(_) => {
                let _ = evt_tx.send(WorkerEvent::Status {
                    account_id: None,
                    text: "正文缓存已清理".to_string(),
                });
            }
            Err(e) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: None,
                    message: e,
                });
            }
        },
    }
}

/// Destination folder role for server-side message moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MoveTarget {
    Archive,
    Trash,
}

/// Move a message to the account's archive or trash folder on the server,
/// then drop it from the local cache and emit a completion event. Failures
/// (missing account config, missing remote ID, no target folder, remote
/// error) are reported through the regular error event channel.
async fn move_message_inner(
    data_dir: &PathBuf,
    cache: &mut CacheStore,
    tokens: &mut TokenCache,
    account_id: u32,
    message_id: u32,
    target: MoveTarget,
    evt_tx: &mpsc::Sender<WorkerEvent>,
) {
    let Some((detail, uid, _)) = cache.get_message_detail(message_id).ok().flatten() else {
        let _ = evt_tx.send(WorkerEvent::Error {
            account_id: Some(account_id),
            message: "本地缓存中未找到该邮件，无法执行归档/删除".to_string(),
        });
        return;
    };
    let Ok(account) = load_account_config(data_dir, account_id) else {
        let _ = evt_tx.send(WorkerEvent::Error {
            account_id: Some(account_id),
            message: format!("账号 #{account_id} 未找到配置"),
        });
        return;
    };

    let op_name = match target {
        MoveTarget::Archive => "归档",
        MoveTarget::Trash => "删除",
    };

    let res = match account.auth_backend {
        AuthBackend::MicrosoftGraphOAuth2 => {
            let Ok(Some(remote_id)) = cache.get_message_remote_id(message_id) else {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: format!("[{}] 缺少远端 ID，无法{}该邮件", account.email, op_name),
                });
                return;
            };
            let dest_folder = match target {
                // Graph accepts well-known folder names directly.
                MoveTarget::Trash => Ok("deleteditems".to_string()),
                MoveTarget::Archive => match cache.find_archive_folder(account_id) {
                    Ok(Some((_, path))) => Ok(path),
                    Ok(None) => Err("该账号没有归档文件夹，无法归档邮件".to_string()),
                    Err(e) => Err(e),
                },
            };
            match dest_folder {
                Ok(dest) => match ensure_graph_token(tokens, &account).await {
                    Ok(token) => tokio::task::spawn_blocking(move || {
                        graph_client::move_message(&token, &remote_id, &dest)
                    })
                    .await
                    .unwrap_or_else(|e| Err(format!("Graph 任务异常: {e}"))),
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            }
        }
        AuthBackend::Password => {
            let dest_folder = match target {
                MoveTarget::Archive => match cache.find_archive_folder(account_id) {
                    Ok(Some((_, path))) => Ok(Some(path)),
                    Ok(None) => Err("该账号没有归档文件夹，无法归档邮件".to_string()),
                    Err(e) => Err(e),
                },
                // No trash folder: fall back to \Deleted + EXPUNGE.
                MoveTarget::Trash => Ok(cache
                    .find_trash_folder(account_id)
                    .ok()
                    .flatten()
                    .map(|(_, path)| path)),
            };
            match dest_folder {
                Ok(Some(dest)) => match secrets::load_account_secrets(account_id) {
                    Ok(sec) => {
                        imap_client::move_message(
                            &account,
                            &sec.imap_password,
                            &detail.folder_path,
                            uid,
                            &dest,
                        )
                        .await
                    }
                    Err(e) => Err(e),
                },
                Ok(None) => match secrets::load_account_secrets(account_id) {
                    Ok(sec) => {
                        imap_client::delete_message(
                            &account,
                            &sec.imap_password,
                            &detail.folder_path,
                            uid,
                        )
                        .await
                    }
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            }
        }
    };

    match res {
        Ok(()) => {
            // Drop the message from the old folder's cache; the UI refreshes
            // its lists when it receives the completion event.
            let _ = cache.remove_message(message_id);
            if let Ok(folders) = cache.list_folders(account_id) {
                let _ = evt_tx.send(WorkerEvent::FoldersLoaded {
                    account_id,
                    folders,
                });
            }
            let _ = evt_tx.send(match target {
                MoveTarget::Archive => WorkerEvent::MessageArchived {
                    account_id,
                    message_id,
                },
                MoveTarget::Trash => WorkerEvent::MessageDeleted {
                    account_id,
                    message_id,
                },
            });
            let _ = evt_tx.send(WorkerEvent::Status {
                account_id: Some(account_id),
                text: format!("[{}] 邮件{}完成", account.email, op_name),
            });
        }
        Err(e) => {
            let _ = evt_tx.send(WorkerEvent::Error {
                account_id: Some(account_id),
                message: format!("[{}] {e}", account.email),
            });
        }
    }
}

async fn ensure_graph_token(
    tokens: &mut TokenCache,
    account: &AccountConfig,
) -> Result<String, String> {
    if let Some((tok, exp)) = tokens.get(&account.id) {
        if Instant::now() < *exp {
            return Ok(tok.clone());
        }
    }
    let refresh_token = secrets::load_oauth_refresh_token(account.id)?;
    let client_id = account.oauth_client_id.clone();
    let refreshed = tokio::task::spawn_blocking(move || {
        oauth::refresh_microsoft_token(&client_id, &refresh_token)
    })
    .await
    .map_err(|e| format!("刷新令牌任务异常: {e}"))??;

    if let Some(new_rt) = refreshed.refresh_token {
        let _ = secrets::save_oauth_refresh_token(account.id, new_rt);
    }
    let expires_at =
        Instant::now() + Duration::from_secs(refreshed.expires_in_secs.saturating_sub(120));
    tokens.insert(account.id, (refreshed.access_token.clone(), expires_at));
    Ok(refreshed.access_token)
}

async fn sync_account_inner(
    data_dir: &PathBuf,
    cache: &mut CacheStore,
    tokens: &mut TokenCache,
    account_id: u32,
    evt_tx: &mpsc::Sender<WorkerEvent>,
) {
    let Ok(account) = load_account_config(data_dir, account_id) else {
        let _ = evt_tx.send(WorkerEvent::Error {
            account_id: Some(account_id),
            message: format!("账号 #{account_id} 未找到配置"),
        });
        return;
    };

    let _ = evt_tx.send(WorkerEvent::Status {
        account_id: Some(account_id),
        text: format!("[{}] 正在同步文件夹与邮件头...", account.email),
    });

    let folders = match account.auth_backend {
        AuthBackend::MicrosoftGraphOAuth2 => {
            let token = match ensure_graph_token(tokens, &account).await {
                Ok(t) => t,
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: format!("[{}] {e}", account.email),
                    });
                    return;
                }
            };
            let remote = tokio::task::spawn_blocking(move || graph_client::fetch_folders(&token))
                .await
                .unwrap_or_else(|e| Err(format!("Graph 文件夹任务异常: {e}")));
            match remote {
                Ok(list) => match cache.sync_folders_with_flags(account_id, &list) {
                    Ok(f) => f,
                    Err(e) => {
                        let _ = evt_tx.send(WorkerEvent::Error {
                            account_id: Some(account_id),
                            message: format!("[{}] {e}", account.email),
                        });
                        return;
                    }
                },
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: format!("[{}] {e}", account.email),
                    });
                    return;
                }
            }
        }
        AuthBackend::Password => {
            let Ok(sec) = secrets::load_account_secrets(account_id) else {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: format!("[{}] 缺少系统凭据", account.email),
                });
                return;
            };
            let remote_folders =
                match imap_client::fetch_folders(&account, &sec.imap_password).await {
                    Ok(f) => f,
                    Err(e) => {
                        let _ = evt_tx.send(WorkerEvent::Error {
                            account_id: Some(account_id),
                            message: format!("[{}] {e}", account.email),
                        });
                        return;
                    }
                };
            match cache.sync_folders(account_id, &remote_folders) {
                Ok(f) => f,
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: format!("[{}] {e}", account.email),
                    });
                    return;
                }
            }
        }
    };

    if let Some(inbox) = folders
        .iter()
        .find(|f| f.path.eq_ignore_ascii_case("INBOX") || f.name == "收件箱")
        .or_else(|| folders.first())
    {
        sync_folder_inner(cache, tokens, &account, inbox.id, &inbox.path, evt_tx).await;
    }

    if let Ok(updated_folders) = cache.list_folders(account_id) {
        let _ = evt_tx.send(WorkerEvent::FoldersLoaded {
            account_id,
            folders: updated_folders,
        });
    }
}

async fn sync_folder_inner(
    cache: &mut CacheStore,
    tokens: &mut TokenCache,
    account: &AccountConfig,
    folder_id: u32,
    folder_path: &str,
    evt_tx: &mpsc::Sender<WorkerEvent>,
) {
    let sync_res: Result<(), String> = match account.auth_backend {
        AuthBackend::MicrosoftGraphOAuth2 => {
            let token = match ensure_graph_token(tokens, account).await {
                Ok(t) => t,
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account.id),
                        message: format!("[{}] {e}", account.email),
                    });
                    return;
                }
            };
            let fp = folder_path.to_string();
            let fetched = tokio::task::spawn_blocking(move || {
                graph_client::fetch_folder_headers(&token, &fp)
            })
            .await
            .unwrap_or_else(|e| Err(format!("Graph 拉取邮件头任务异常: {e}")));
            match fetched {
                Ok(items) => {
                    let live_uids: Vec<u32> = items.iter().map(|(h, _)| h.uid).collect();
                    cache.commit_folder_headers_with_remote(
                        account.id,
                        folder_id,
                        1,
                        &items,
                        Some(&live_uids),
                    )
                }
                Err(e) => Err(e),
            }
        }
        AuthBackend::Password => match secrets::load_account_secrets(account.id) {
            Ok(sec) => {
                match imap_client::fetch_folder_headers(account, &sec.imap_password, folder_path)
                    .await
                {
                    Ok((uid_validity, headers, live_uids)) => cache.commit_folder_headers(
                        account.id,
                        folder_id,
                        uid_validity,
                        &headers,
                        Some(&live_uids),
                    ),
                    Err(e) => Err(e),
                }
            }
            Err(e) => Err(e),
        },
    };

    match sync_res {
        Ok(()) => {
            if let Ok(page) = cache.list_messages_page(folder_id, "", 0, 100) {
                let _ = evt_tx.send(WorkerEvent::MessagesPageLoaded(page));
            }
            if let Ok(folders) = cache.list_folders(account.id) {
                let _ = evt_tx.send(WorkerEvent::FoldersLoaded {
                    account_id: account.id,
                    folders,
                });
            }
            let _ = evt_tx.send(WorkerEvent::Status {
                account_id: Some(account.id),
                text: format!("[{}] 同步完成", account.email),
            });
        }
        Err(e) => {
            let _ = evt_tx.send(WorkerEvent::Error {
                account_id: Some(account.id),
                message: format!("[{}] {e}", account.email),
            });
        }
    }
}

async fn send_outbox_item(
    data_dir: &PathBuf,
    cache: &mut CacheStore,
    tokens: &mut TokenCache,
    account_id: u32,
    outbox_id: u32,
    evt_tx: &mpsc::Sender<WorkerEvent>,
) {
    let items = cache.list_outbox(account_id).unwrap_or_default();
    let Some(item) = items.into_iter().find(|i| i.id == outbox_id) else {
        return;
    };

    let Ok(account) = load_account_config(data_dir, account_id) else {
        let _ = cache.update_outbox_status(outbox_id, OutboxStatus::Failed, "缺少账号配置");
        return;
    };

    let _ = cache.update_outbox_status(outbox_id, OutboxStatus::Sending, "");
    if let Ok(outbox) = cache.list_outbox(account_id) {
        let _ = evt_tx.send(WorkerEvent::OutboxUpdated {
            account_id,
            items: outbox,
        });
    }

    let (outcome, sent_raw_rfc822, imap_password_for_sent) = match account.auth_backend {
        AuthBackend::MicrosoftGraphOAuth2 => match ensure_graph_token(tokens, &account).await {
            Ok(token) => {
                let to_addr = item.to_addr.clone();
                let cc_addrs = item.cc_addr.clone();
                let bcc_addrs = item.bcc_addr.clone();
                let subject = item.subject.clone();
                let body = item.body_text.clone();
                let atts = item.attachment_paths.clone();
                let out = tokio::task::spawn_blocking(move || {
                    graph_client::send_message(
                        &token, &to_addr, &cc_addrs, &bcc_addrs, &subject, &body, &atts,
                    )
                })
                .await
                .unwrap_or_else(|e| SendOutcome::Uncertain(format!("Graph 发信任务异常: {e}")));
                (out, None, None)
            }
            Err(e) => (SendOutcome::Failed(e), None, None),
        },
        AuthBackend::Password => match secrets::load_account_secrets(account_id) {
            Ok(secrets) => {
                let smtp_pwd = if account.smtp_separate_credentials {
                    secrets
                        .smtp_password
                        .as_deref()
                        .unwrap_or(&secrets.imap_password)
                } else {
                    secrets.imap_password.as_str()
                };
                let (out, raw) = smtp_client::send_message_with_raw(
                    &account,
                    smtp_pwd,
                    &item.to_addr,
                    &item.cc_addr,
                    &item.bcc_addr,
                    &item.subject,
                    &item.body_text,
                    &item.in_reply_to,
                    &item.attachment_paths,
                )
                .await;
                (out, raw, Some(secrets.imap_password))
            }
            Err(e) => (SendOutcome::Failed(e), None, None),
        },
    };

    match outcome {
        SendOutcome::Accepted => {
            let _ = cache.delete_outbox(outbox_id);
            let _ = evt_tx.send(WorkerEvent::DraftSent {
                account_id,
                subject: item.subject,
            });
            // 移植方案阶段 2：服务器确认接收后，保存已发副本属于独立操作；
            // 若 APPEND 失败仅发温和状态提示，绝不将已发邮件标记为失败或重发。
            if let (Some(raw), Some(imap_pwd)) = (sent_raw_rfc822, imap_password_for_sent) {
                if let Ok(Some((_, sent_path))) = cache.find_sent_folder(account_id) {
                    if let Err(e) =
                        imap_client::append_sent_message(&account, &imap_pwd, &sent_path, &raw)
                            .await
                    {
                        let _ = evt_tx.send(WorkerEvent::Status {
                            account_id: Some(account_id),
                            text: format!(
                                "[{}] 邮件已发出，但保存已发副本失败：{e}",
                                account.email
                            ),
                        });
                    }
                }
            }
        }
        SendOutcome::Failed(err) => {
            let _ = cache.update_outbox_status(outbox_id, OutboxStatus::Failed, &err);
            let _ = evt_tx.send(WorkerEvent::Error {
                account_id: Some(account_id),
                message: format!("[{}] {err}", account.email),
            });
        }
        SendOutcome::Uncertain(err) => {
            let _ = cache.update_outbox_status(outbox_id, OutboxStatus::Uncertain, &err);
            let _ = evt_tx.send(WorkerEvent::Error {
                account_id: Some(account_id),
                message: format!("[{}] {err}", account.email),
            });
        }
    }

    if let Ok(outbox) = cache.list_outbox(account_id) {
        let _ = evt_tx.send(WorkerEvent::OutboxUpdated {
            account_id,
            items: outbox,
        });
    }
}

fn load_account_config(data_dir: &PathBuf, account_id: u32) -> Result<AccountConfig, String> {
    let accounts = config::load_accounts_in(data_dir)?;
    accounts
        .into_iter()
        .find(|a| a.id == account_id)
        .ok_or_else(|| format!("找不到账号 ID {account_id}"))
}

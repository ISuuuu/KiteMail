//! Compose, local drafts, and outbox/undo-send IPC handlers and session logic.

use std::sync::mpsc;
use std::time::Duration;

use kitemail_mail_core::cache::CacheStore;
use kitemail_mail_core::config;
use kitemail_mail_core::{
    ComposeDraft, LocalDraft, MailRequest, MessageSummary, OutboxItem, OutboxStatus, SendTiming,
    UndoOutboxOutcome,
};
use tauri::State;

use crate::commands::AppState;
use crate::dto::{
    AppMode, DeleteDraftInput, DeleteDraftOutput, DraftSavedEvent, IpcError, ListDraftsOutput,
    ListOutboxOutput, LocalDraftDto, MailChangedEvent, OutboxEventKindDto, OutboxItemActionInput,
    OutboxItemActionOutput, OutboxItemDto, OutboxUpdatedEvent, SaveDraftInput, SaveDraftOutput,
    SendMailInput, SendMailOutput, UndoSendMailInput, UndoSendMailOutput,
};
use crate::mail_session::{DemoOutboxEntry, MailSession};

const WORKER_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
const DEMO_SENT_FOLDER_ID: u32 = 2;

pub(crate) fn parse_positive_u32(raw: &str, field: &str) -> Result<u32, IpcError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(IpcError::invalid_input(format!("{field} 不能为空"), None));
    }
    let val = trimmed
        .parse::<u32>()
        .map_err(|_| IpcError::invalid_input(format!("{field} 必须为正整数"), None))?;
    if val == 0 {
        return Err(IpcError::invalid_input(
            format!("{field} 必须为正整数"),
            None,
        ));
    }
    Ok(val)
}

pub(crate) fn parse_optional_positive_u32(
    raw: Option<&str>,
    field: &str,
) -> Result<Option<u32>, IpcError> {
    let Some(text) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    parse_positive_u32(text, field).map(Some)
}

fn parse_positive_u64(raw: &str, field: &str) -> Result<u64, IpcError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(IpcError::invalid_input(format!("{field} 不能为空"), None));
    }
    let val = trimmed
        .parse::<u64>()
        .map_err(|_| IpcError::invalid_input(format!("{field} 格式非法"), None))?;
    if val == 0 {
        return Err(IpcError::invalid_input(
            format!("{field} 必须为正整数"),
            None,
        ));
    }
    Ok(val)
}

fn normalize_opt(opt: Option<&str>) -> String {
    opt.map(str::trim).unwrap_or("").to_string()
}

impl MailSession {
    pub fn list_drafts(&self) -> Result<ListDraftsOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let drafts = match self.mode {
            AppMode::Demo => {
                let demo = self.demo.lock().expect("demo mutex poisoned");
                let mut list = demo.local_drafts.clone();
                list.sort_by(|a, b| {
                    b.updated_at
                        .cmp(&a.updated_at)
                        .then_with(|| b.id.cmp(&a.id))
                });
                list.iter().map(LocalDraftDto::from).collect()
            }
            AppMode::Live => {
                let mut live = self.live.lock().expect("live mutex poisoned");
                live.refresh_drafts_from_db();
                let mut list = live.drafts_cache.clone();
                list.sort_by(|a, b| {
                    b.updated_at
                        .cmp(&a.updated_at)
                        .then_with(|| b.id.cmp(&a.id))
                });
                list.iter().map(LocalDraftDto::from).collect()
            }
        };
        Ok(ListDraftsOutput {
            drafts,
            state_version: self.state_version(),
        })
    }

    pub fn save_draft(&self, input: &SaveDraftInput) -> Result<SaveDraftOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let account_id = parse_positive_u32(&input.account_id, "accountId")?;
        let draft_id = parse_optional_positive_u32(input.id.as_deref(), "id")?.unwrap_or(0);
        let body_format = input
            .body_format
            .unwrap_or_default()
            .as_core_str()
            .to_string();
        let now = chrono::Utc::now().timestamp();

        let mut draft = LocalDraft {
            id: draft_id,
            account_id,
            to_addr: input.to_addr.clone(),
            cc_addr: input.cc_addr.clone(),
            bcc_addr: input.bcc_addr.clone(),
            subject: input.subject.clone(),
            body_text: input.body_text.clone(),
            in_reply_to: normalize_opt(input.in_reply_to.as_deref()),
            from_addr: normalize_opt(input.from_addr.as_deref()),
            reply_to: normalize_opt(input.reply_to.as_deref()),
            body_format,
            attachment_paths: input.attachment_paths.clone(),
            updated_at: now,
        };

        match self.mode {
            AppMode::Demo => {
                let saved = {
                    let mut demo = self.demo.lock().expect("demo mutex poisoned");
                    if !demo.account_configs.iter().any(|a| a.id == account_id) {
                        return Err(IpcError::invalid_input(
                            format!("未找到账号 #{account_id}"),
                            None,
                        ));
                    }
                    if draft.id == 0 {
                        let next_id = demo.next_local_draft_id;
                        demo.next_local_draft_id = demo.next_local_draft_id.saturating_add(1);
                        draft.id = next_id;
                        demo.local_drafts.push(draft.clone());
                    } else if let Some(existing) =
                        demo.local_drafts.iter_mut().find(|d| d.id == draft.id)
                    {
                        *existing = draft.clone();
                    } else {
                        demo.next_local_draft_id = demo.next_local_draft_id.max(draft.id + 1);
                        demo.local_drafts.push(draft.clone());
                    }
                    draft
                };
                let ver = self.bump_state_version();
                self.emitter.emit_draft_saved(DraftSavedEvent {
                    state_version: ver,
                    draft_id: saved.id.to_string(),
                });
                if !input.silent {
                    self.emitter.emit_mail_changed(MailChangedEvent {
                        state_version: ver,
                        reason: "draftsUpdated".to_string(),
                        account_id: Some(account_id),
                        folder_id: None,
                        message_id: None,
                    });
                }
                Ok(SaveDraftOutput {
                    draft: LocalDraftDto::from(&saved),
                    state_version: ver,
                })
            }
            AppMode::Live => {
                let (worker, async_worker, rx_opt) = {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    if !live.account_configs.is_empty()
                        && !live.account_configs.iter().any(|a| a.id == account_id)
                    {
                        return Err(IpcError::invalid_input(
                            format!("未找到账号 #{account_id}"),
                            None,
                        ));
                    }
                    let worker = live.worker.clone();
                    let is_async = worker.as_ref().is_some_and(|w| w.is_async_worker());
                    let rx_opt = if is_async {
                        let (tx, rx) = mpsc::channel();
                        live.waiters.draft_saved.push_back(tx);
                        Some(rx)
                    } else {
                        None
                    };
                    (worker, is_async, rx_opt)
                };

                let Some(w) = worker else {
                    return Err(IpcError::unavailable("邮件后台服务尚未就绪", None));
                };
                let req = if input.silent {
                    MailRequest::AutosaveLocalDraft {
                        draft: draft.clone(),
                    }
                } else {
                    MailRequest::SaveLocalDraft {
                        draft: draft.clone(),
                    }
                };
                w.send(req);

                if async_worker {
                    let rx = rx_opt.expect("async waiter missing");
                    let assigned_id = rx
                        .recv_timeout(WORKER_WAIT_TIMEOUT)
                        .map_err(|_| IpcError::unavailable("保存草稿超时，请稍后重试", None))?
                        .map_err(|msg| IpcError::internal(msg, None))?;
                    let final_draft = {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        live.refresh_drafts_from_db();
                        live.drafts_cache
                            .iter()
                            .find(|d| d.id == assigned_id)
                            .cloned()
                            .unwrap_or_else(|| {
                                let mut d = draft;
                                d.id = assigned_id;
                                d
                            })
                    };
                    return Ok(SaveDraftOutput {
                        draft: LocalDraftDto::from(&final_draft),
                        state_version: self.state_version(),
                    });
                }

                let saved = {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    if draft.id == 0 {
                        let next_id = live
                            .drafts_cache
                            .iter()
                            .map(|d| d.id)
                            .max()
                            .unwrap_or(0)
                            .saturating_add(1);
                        draft.id = next_id;
                    }
                    if let Some(existing) = live.drafts_cache.iter_mut().find(|d| d.id == draft.id)
                    {
                        *existing = draft.clone();
                    } else {
                        live.drafts_cache.push(draft.clone());
                    }
                    draft
                };
                let ver = self.bump_state_version();
                self.emitter.emit_draft_saved(DraftSavedEvent {
                    state_version: ver,
                    draft_id: saved.id.to_string(),
                });
                Ok(SaveDraftOutput {
                    draft: LocalDraftDto::from(&saved),
                    state_version: ver,
                })
            }
        }
    }

    pub fn delete_draft(&self, input: &DeleteDraftInput) -> Result<DeleteDraftOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let draft_id = parse_positive_u32(&input.draft_id, "draftId")?;

        match self.mode {
            AppMode::Demo => {
                let deleted = {
                    let mut demo = self.demo.lock().expect("demo mutex poisoned");
                    let before = demo.local_drafts.len();
                    demo.local_drafts.retain(|d| d.id != draft_id);
                    demo.local_drafts.len() != before
                };
                let ver = if deleted {
                    let v = self.bump_state_version();
                    self.emitter.emit_mail_changed(MailChangedEvent {
                        state_version: v,
                        reason: "draftsUpdated".to_string(),
                        account_id: None,
                        folder_id: None,
                        message_id: None,
                    });
                    v
                } else {
                    self.state_version()
                };
                Ok(DeleteDraftOutput {
                    deleted,
                    state_version: ver,
                })
            }
            AppMode::Live => {
                let (worker, existed) = {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    live.refresh_drafts_from_db();
                    let before = live.drafts_cache.len();
                    live.drafts_cache.retain(|d| d.id != draft_id);
                    (live.worker.clone(), live.drafts_cache.len() != before)
                };
                if let Some(w) = worker {
                    w.send(MailRequest::DeleteLocalDraft { draft_id });
                }
                let ver = if existed {
                    let v = self.bump_state_version();
                    self.emitter.emit_mail_changed(MailChangedEvent {
                        state_version: v,
                        reason: "draftsUpdated".to_string(),
                        account_id: None,
                        folder_id: None,
                        message_id: None,
                    });
                    v
                } else {
                    self.state_version()
                };
                Ok(DeleteDraftOutput {
                    deleted: existed,
                    state_version: ver,
                })
            }
        }
    }

    pub fn send_mail(&self, input: &SendMailInput) -> Result<SendMailOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let account_id = parse_positive_u32(&input.account_id, "accountId")?;
        if input.to.trim().is_empty() {
            return Err(IpcError::invalid_input("收件人不能为空", None));
        }
        let local_draft_id =
            parse_optional_positive_u32(input.local_draft_id.as_deref(), "localDraftId")?;
        let now = chrono::Utc::now().timestamp();

        let (timing, undo_window_secs, scheduled_at, demo_send_at) =
            if let Some(ts) = input.scheduled_at {
                if ts <= now {
                    return Err(IpcError::invalid_input(
                        "定时发送时间必须晚于当前时间",
                        None,
                    ));
                }
                (SendTiming::ScheduledAt(ts), 0, Some(ts), Some(ts))
            } else {
                let default_delay = self.prefs.snapshot_raw().undo_send_secs;
                let window = input.undo_delay_secs.unwrap_or(default_delay).min(60);
                let timing = if window == 0 {
                    SendTiming::Immediate
                } else {
                    SendTiming::DelaySecs(window as u8)
                };
                let send_at = (window > 0).then(|| now + i64::from(window));
                (timing, window, None, send_at)
            };

        let body_format = input
            .body_format
            .unwrap_or_default()
            .as_core_str()
            .to_string();
        let idempotency_key = input
            .client_request_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let compose_draft = ComposeDraft {
            to: input.to.trim().to_string(),
            cc: input.cc.trim().to_string(),
            bcc: input.bcc.trim().to_string(),
            subject: input.subject.clone(),
            body: input.body.clone(),
            in_reply_to: normalize_opt(input.in_reply_to.as_deref()),
            from_addr: normalize_opt(input.from_addr.as_deref()),
            reply_to: normalize_opt(input.reply_to.as_deref()),
            body_format: body_format.clone(),
            body_html: input.body_html.clone(),
            attachment_paths: input.attachment_paths.clone(),
        };

        match self.mode {
            AppMode::Demo => {
                let existing_receipt = {
                    let demo = self.demo.lock().expect("demo mutex poisoned");
                    idempotency_key.as_ref().and_then(|key| {
                        demo.send_idempotency
                            .get(&(account_id, key.clone()))
                            .copied()
                    })
                };
                if let Some((outbox_id, undo_token, saved_send_at)) = existing_receipt {
                    return Ok(SendMailOutput {
                        accepted: true,
                        undo_token: undo_token.to_string(),
                        outbox_id: outbox_id.to_string(),
                        undo_window_secs,
                        scheduled_at: saved_send_at.filter(|_| scheduled_at.is_some()),
                        state_version: self.state_version(),
                    });
                }
                let (outbox_id, undo_token, items, removed_draft) = {
                    let mut demo = self.demo.lock().expect("demo mutex poisoned");
                    let Some(acc) = demo.account_configs.iter().find(|a| a.id == account_id) else {
                        return Err(IpcError::invalid_input(
                            format!("未找到发件账号 #{account_id}"),
                            None,
                        ));
                    };
                    if !acc.enabled {
                        return Err(IpcError::invalid_input("发件账号已停用，无法发送", None));
                    }
                    let removed_draft = if let Some(did) = local_draft_id {
                        let before = demo.local_drafts.len();
                        demo.local_drafts.retain(|d| d.id != did);
                        demo.local_drafts.len() != before
                    } else {
                        false
                    };
                    let outbox_id = demo.next_outbox_id;
                    demo.next_outbox_id = demo.next_outbox_id.saturating_add(1);
                    let undo_token = demo.next_undo_token;
                    demo.next_undo_token = demo.next_undo_token.saturating_add(1);
                    let entry = DemoOutboxEntry {
                        item: OutboxItem {
                            id: outbox_id,
                            account_id,
                            to_addr: compose_draft.to.clone(),
                            cc_addr: compose_draft.cc.clone(),
                            bcc_addr: compose_draft.bcc.clone(),
                            subject: compose_draft.subject.clone(),
                            body_text: compose_draft.body.clone(),
                            in_reply_to: compose_draft.in_reply_to.clone(),
                            from_addr: compose_draft.from_addr.clone(),
                            reply_to: compose_draft.reply_to.clone(),
                            body_format,
                            body_html: compose_draft.body_html.clone(),
                            attachment_paths: compose_draft.attachment_paths.clone(),
                            status: OutboxStatus::Queued,
                            last_error: String::new(),
                            send_at: demo_send_at,
                        },
                        undo_token,
                    };
                    demo.outbox.push(entry);
                    if let Some(ref key) = idempotency_key {
                        demo.send_idempotency.insert(
                            (account_id, key.clone()),
                            (outbox_id, undo_token, demo_send_at),
                        );
                    }
                    (outbox_id, undo_token, demo.outbox_dtos(), removed_draft)
                };

                let ver = self.bump_state_version();
                if removed_draft {
                    self.emitter.emit_mail_changed(MailChangedEvent {
                        state_version: ver,
                        reason: "draftsUpdated".to_string(),
                        account_id: Some(account_id),
                        folder_id: None,
                        message_id: None,
                    });
                }
                self.emitter.emit_outbox_updated(OutboxUpdatedEvent {
                    state_version: ver,
                    items,
                    event_kind: OutboxEventKindDto::Enqueued,
                    undo_token: Some(undo_token.to_string()),
                    outbox_id: Some(outbox_id.to_string()),
                    message: None,
                });

                if scheduled_at.is_none() {
                    self.spawn_demo_delivery(account_id, outbox_id, undo_token, undo_window_secs);
                }

                Ok(SendMailOutput {
                    accepted: true,
                    undo_token: undo_token.to_string(),
                    outbox_id: outbox_id.to_string(),
                    undo_window_secs,
                    scheduled_at,
                    state_version: ver,
                })
            }
            AppMode::Live => {
                let (worker, async_worker, undo_token, rx_opt, mock_dedup) = {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    if let Some(acc) = live.account_configs.iter().find(|a| a.id == account_id) {
                        if !acc.enabled {
                            return Err(IpcError::invalid_input("发件账号已停用，无法发送", None));
                        }
                    } else if !live.account_configs.is_empty() {
                        return Err(IpcError::invalid_input(
                            format!("未找到发件账号 #{account_id}"),
                            None,
                        ));
                    }
                    let worker = live.worker.clone();
                    let is_async = worker.as_ref().is_some_and(|w| w.is_async_worker());
                    let mock_dedup = if !is_async {
                        idempotency_key.as_ref().and_then(|key| {
                            live.mock_send_idempotency
                                .get(&(account_id, key.clone()))
                                .copied()
                        })
                    } else {
                        None
                    };
                    let undo_token = live.next_undo_token;
                    if mock_dedup.is_none() {
                        live.next_undo_token = live.next_undo_token.saturating_add(1);
                    }
                    let rx_opt = if is_async {
                        let (tx, rx) = mpsc::channel();
                        live.waiters.send_enqueued.insert(undo_token, tx);
                        Some(rx)
                    } else {
                        None
                    };
                    (worker, is_async, undo_token, rx_opt, mock_dedup)
                };

                if let Some((outbox_id, saved_token, saved_send_at)) = mock_dedup {
                    return Ok(SendMailOutput {
                        accepted: true,
                        undo_token: saved_token.to_string(),
                        outbox_id: outbox_id.to_string(),
                        undo_window_secs,
                        scheduled_at: scheduled_at.or(saved_send_at),
                        state_version: self.state_version(),
                    });
                }

                let Some(w) = worker else {
                    return Err(IpcError::unavailable("邮件后台服务尚未就绪", None));
                };

                w.send(MailRequest::SendDraft {
                    account_id,
                    draft: compose_draft.clone(),
                    local_draft_id,
                    undo_token,
                    timing,
                    idempotency_key: idempotency_key.clone(),
                });

                if async_worker {
                    let rx = rx_opt.expect("async send waiter missing");
                    let res = rx.recv_timeout(WORKER_WAIT_TIMEOUT).map_err(|_| {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        live.waiters.send_enqueued.remove(&undo_token);
                        IpcError::unavailable("发件入队超时，请检查发件箱状态", None)
                    })?;
                    let (outbox_id, actual_send_at, _scheduled) =
                        res.map_err(|msg| IpcError::internal(msg, None))?;
                    return Ok(SendMailOutput {
                        accepted: true,
                        undo_token: undo_token.to_string(),
                        outbox_id: outbox_id.to_string(),
                        undo_window_secs,
                        scheduled_at: scheduled_at.or(if undo_window_secs == 0 {
                            actual_send_at
                        } else {
                            None
                        }),
                        state_version: self.state_version(),
                    });
                }

                // Mock/recording worker mode: update optimistic outbox state deterministically.
                let (outbox_id, items) = {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    if let Some(did) = local_draft_id {
                        live.drafts_cache.retain(|d| d.id != did);
                    }
                    let outbox_id = live
                        .outbox_cache
                        .values()
                        .flatten()
                        .map(|i| i.id)
                        .max()
                        .unwrap_or(0)
                        .saturating_add(1);
                    let item = OutboxItem {
                        id: outbox_id,
                        account_id,
                        to_addr: compose_draft.to,
                        cc_addr: compose_draft.cc,
                        bcc_addr: compose_draft.bcc,
                        subject: compose_draft.subject,
                        body_text: compose_draft.body,
                        in_reply_to: compose_draft.in_reply_to,
                        from_addr: compose_draft.from_addr,
                        reply_to: compose_draft.reply_to,
                        body_format,
                        body_html: compose_draft.body_html,
                        attachment_paths: compose_draft.attachment_paths,
                        status: OutboxStatus::Queued,
                        last_error: String::new(),
                        send_at: demo_send_at,
                    };
                    live.outbox_cache.entry(account_id).or_default().push(item);
                    if let Some(ref key) = idempotency_key {
                        live.mock_send_idempotency.insert(
                            (account_id, key.clone()),
                            (outbox_id, undo_token, demo_send_at),
                        );
                    }
                    (outbox_id, live.combined_outbox_dtos())
                };
                let ver = self.bump_state_version();
                self.emitter.emit_outbox_updated(OutboxUpdatedEvent {
                    state_version: ver,
                    items,
                    event_kind: OutboxEventKindDto::Enqueued,
                    undo_token: Some(undo_token.to_string()),
                    outbox_id: Some(outbox_id.to_string()),
                    message: None,
                });
                Ok(SendMailOutput {
                    accepted: true,
                    undo_token: undo_token.to_string(),
                    outbox_id: outbox_id.to_string(),
                    undo_window_secs,
                    scheduled_at,
                    state_version: ver,
                })
            }
        }
    }

    fn spawn_demo_delivery(
        &self,
        account_id: u32,
        outbox_id: u32,
        undo_token: u64,
        delay_secs: u32,
    ) {
        let weak = self
            .self_weak
            .lock()
            .expect("self_weak mutex poisoned")
            .clone();
        if weak.strong_count() == 0 {
            return;
        }
        let delay = Duration::from_secs(u64::from(delay_secs));
        let _ = std::thread::Builder::new()
            .name("kitemail-demo-send".to_string())
            .spawn(move || {
                let step = Duration::from_millis(25);
                let mut elapsed = Duration::ZERO;
                while elapsed < delay {
                    let Some(session) = weak.upgrade() else {
                        return;
                    };
                    if session.quitting.load(std::sync::atomic::Ordering::SeqCst) {
                        return;
                    }
                    {
                        let demo = session.demo.lock().expect("demo mutex poisoned");
                        let still_queued = demo.outbox.iter().any(|e| {
                            e.item.id == outbox_id
                                && e.item.account_id == account_id
                                && e.undo_token == undo_token
                                && e.item.status == OutboxStatus::Queued
                        });
                        if !still_queued {
                            return;
                        }
                    }
                    let sleep_for = (delay - elapsed).min(step);
                    std::thread::sleep(sleep_for);
                    elapsed += sleep_for;
                }
                if let Some(session) = weak.upgrade() {
                    session.deliver_demo_outbox_item(account_id, outbox_id, undo_token);
                }
            });
    }

    pub(crate) fn deliver_demo_outbox_item(
        &self,
        account_id: u32,
        outbox_id: u32,
        undo_token: u64,
    ) -> bool {
        let outcome = {
            let mut demo = self.demo.lock().expect("demo mutex poisoned");
            let Some(pos) = demo.outbox.iter().position(|e| {
                e.item.id == outbox_id
                    && e.item.account_id == account_id
                    && e.undo_token == undo_token
                    && e.item.status == OutboxStatus::Queued
            }) else {
                return false;
            };
            let entry = demo.outbox.remove(pos);
            let sent_id = demo.next_sent_msg_id;
            demo.next_sent_msg_id = demo.next_sent_msg_id.saturating_add(1);
            let sender = demo
                .account_configs
                .iter()
                .find(|a| a.id == account_id)
                .map(|a| {
                    if a.display_name.trim().is_empty() {
                        a.email.clone()
                    } else {
                        format!("{} <{}>", a.display_name, a.email)
                    }
                })
                .unwrap_or_else(|| "demo@hylki.example".to_string());
            let subject = entry.item.subject.clone();
            let now_ts = chrono::Utc::now().timestamp();
            demo.sent.insert(
                0,
                MessageSummary {
                    id: sent_id,
                    account_id,
                    folder_id: DEMO_SENT_FOLDER_ID,
                    folder_path: "Sent".to_string(),
                    uid: sent_id,
                    sender,
                    subject: subject.clone(),
                    preview: entry.item.body_text.chars().take(120).collect(),
                    date_ts: now_ts,
                    date_display: "刚刚".to_string(),
                    unread: false,
                    flagged: false,
                    has_attachment: !entry.item.attachment_paths.is_empty(),
                    recipients: entry.item.to_addr,
                    thread_count: 1,
                    importance: 0,
                    tags: Vec::new(),
                    answered: false,
                    forwarded: false,
                    snoozed_until_ts: 0,
                },
            );
            (subject, demo.outbox_dtos())
        };

        let ver = self.bump_state_version();
        self.emitter.emit_outbox_updated(OutboxUpdatedEvent {
            state_version: ver,
            items: outcome.1,
            event_kind: OutboxEventKindDto::Sent,
            undo_token: None,
            outbox_id: Some(outbox_id.to_string()),
            message: Some(format!("邮件「{}」已发送", outcome.0)),
        });
        self.emitter.emit_mail_changed(MailChangedEvent {
            state_version: ver,
            reason: "draftSent".to_string(),
            account_id: Some(account_id),
            folder_id: Some(DEMO_SENT_FOLDER_ID),
            message_id: None,
        });
        true
    }

    pub fn undo_send_mail(
        &self,
        input: &UndoSendMailInput,
    ) -> Result<UndoSendMailOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let account_id = parse_positive_u32(&input.account_id, "accountId")?;
        let outbox_id = parse_positive_u32(&input.outbox_id, "outboxId")?;
        let token = parse_positive_u64(&input.undo_token, "undoToken")?;

        match self.mode {
            AppMode::Demo => {
                enum DemoUndoOutcome {
                    Restored {
                        draft_id: u32,
                        items: Vec<OutboxItemDto>,
                    },
                    NotRestored(String),
                }
                let result =
                    {
                        let mut demo = self.demo.lock().expect("demo mutex poisoned");
                        let Some(pos) = demo.outbox.iter().position(|e| {
                            e.item.id == outbox_id && e.item.account_id == account_id
                        }) else {
                            return Ok(UndoSendMailOutput {
                                undone: false,
                                message: "邮件已发送或不存在，无法撤销".to_string(),
                                state_version: self.state_version(),
                            });
                        };
                        if demo.outbox[pos].undo_token != token {
                            DemoUndoOutcome::NotRestored("撤销令牌不匹配".to_string())
                        } else if demo.outbox[pos].item.status == OutboxStatus::Sending {
                            DemoUndoOutcome::NotRestored("邮件正在投递中，无法撤销".to_string())
                        } else if demo.outbox[pos].item.status != OutboxStatus::Queued {
                            DemoUndoOutcome::NotRestored("当前状态不可撤销".to_string())
                        } else {
                            let entry = demo.outbox.remove(pos);
                            let draft_id = demo.next_local_draft_id;
                            demo.next_local_draft_id = demo.next_local_draft_id.saturating_add(1);
                            demo.local_drafts.push(LocalDraft {
                                id: draft_id,
                                account_id,
                                to_addr: entry.item.to_addr,
                                cc_addr: entry.item.cc_addr,
                                bcc_addr: entry.item.bcc_addr,
                                subject: entry.item.subject,
                                body_text: entry.item.body_text,
                                in_reply_to: entry.item.in_reply_to,
                                from_addr: entry.item.from_addr,
                                reply_to: entry.item.reply_to,
                                body_format: entry.item.body_format,
                                attachment_paths: entry.item.attachment_paths,
                                updated_at: chrono::Utc::now().timestamp(),
                            });
                            DemoUndoOutcome::Restored {
                                draft_id,
                                items: demo.outbox_dtos(),
                            }
                        }
                    };

                match result {
                    DemoUndoOutcome::Restored { draft_id, items } => {
                        let ver = self.bump_state_version();
                        let msg = "已撤销发送，邮件已恢复为草稿".to_string();
                        self.emitter.emit_outbox_updated(OutboxUpdatedEvent {
                            state_version: ver,
                            items,
                            event_kind: OutboxEventKindDto::Undone,
                            undo_token: Some(token.to_string()),
                            outbox_id: Some(outbox_id.to_string()),
                            message: Some(msg.clone()),
                        });
                        self.emitter.emit_draft_saved(DraftSavedEvent {
                            state_version: ver,
                            draft_id: draft_id.to_string(),
                        });
                        self.emitter.emit_mail_changed(MailChangedEvent {
                            state_version: ver,
                            reason: "outboxUndone".to_string(),
                            account_id: Some(account_id),
                            folder_id: None,
                            message_id: None,
                        });
                        Ok(UndoSendMailOutput {
                            undone: true,
                            message: msg,
                            state_version: ver,
                        })
                    }
                    DemoUndoOutcome::NotRestored(message) => Ok(UndoSendMailOutput {
                        undone: false,
                        message,
                        state_version: self.state_version(),
                    }),
                }
            }
            AppMode::Live => {
                let (worker, async_worker, rx_opt) = {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    let worker = live.worker.clone();
                    let is_async = worker.as_ref().is_some_and(|w| w.is_async_worker());
                    let rx_opt = if is_async {
                        let (tx, rx) = mpsc::channel();
                        live.waiters.undo_result.insert(token, tx);
                        Some(rx)
                    } else {
                        None
                    };
                    (worker, is_async, rx_opt)
                };

                let Some(w) = worker else {
                    return Err(IpcError::unavailable("邮件后台服务尚未就绪", None));
                };
                w.send(MailRequest::UndoOutboxItem {
                    account_id,
                    outbox_id,
                    token,
                });

                if async_worker {
                    let rx = rx_opt.expect("async undo waiter missing");
                    let res = rx.recv_timeout(WORKER_WAIT_TIMEOUT).map_err(|_| {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        live.waiters.undo_result.remove(&token);
                        IpcError::unavailable("撤销发送超时，请查看发件箱状态", None)
                    })?;
                    let outcome = res.map_err(|msg| IpcError::internal(msg, None))?;
                    let (undone, message) = match outcome {
                        UndoOutboxOutcome::Restored { .. } => {
                            (true, "已撤销发送，邮件已恢复为草稿".to_string())
                        }
                        UndoOutboxOutcome::AlreadySending => {
                            (false, "邮件正在投递中，无法撤销".to_string())
                        }
                        UndoOutboxOutcome::NotFound => {
                            (false, "邮件已发送或不存在，无法撤销".to_string())
                        }
                    };
                    return Ok(UndoSendMailOutput {
                        undone,
                        message,
                        state_version: self.state_version(),
                    });
                }

                // Mock/recording worker mode: handle optimistically on live cache.
                let restored = {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    if let Some(list) = live.outbox_cache.get_mut(&account_id) {
                        if let Some(pos) = list
                            .iter()
                            .position(|i| i.id == outbox_id && i.status == OutboxStatus::Queued)
                        {
                            let entry = list.remove(pos);
                            let draft_id = live
                                .drafts_cache
                                .iter()
                                .map(|d| d.id)
                                .max()
                                .unwrap_or(0)
                                .saturating_add(1);
                            live.drafts_cache.push(LocalDraft {
                                id: draft_id,
                                account_id,
                                to_addr: entry.to_addr,
                                cc_addr: entry.cc_addr,
                                bcc_addr: entry.bcc_addr,
                                subject: entry.subject,
                                body_text: entry.body_text,
                                in_reply_to: entry.in_reply_to,
                                from_addr: entry.from_addr,
                                reply_to: entry.reply_to,
                                body_format: entry.body_format,
                                attachment_paths: entry.attachment_paths,
                                updated_at: chrono::Utc::now().timestamp(),
                            });
                            Some((draft_id, live.combined_outbox_dtos()))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                if let Some((draft_id, items)) = restored {
                    let ver = self.bump_state_version();
                    let msg = "已撤销发送，邮件已恢复为草稿".to_string();
                    self.emitter.emit_outbox_updated(OutboxUpdatedEvent {
                        state_version: ver,
                        items,
                        event_kind: OutboxEventKindDto::Undone,
                        undo_token: Some(token.to_string()),
                        outbox_id: Some(outbox_id.to_string()),
                        message: Some(msg.clone()),
                    });
                    self.emitter.emit_draft_saved(DraftSavedEvent {
                        state_version: ver,
                        draft_id: draft_id.to_string(),
                    });
                    Ok(UndoSendMailOutput {
                        undone: true,
                        message: msg,
                        state_version: ver,
                    })
                } else {
                    Ok(UndoSendMailOutput {
                        undone: false,
                        message: "已提交撤销请求".to_string(),
                        state_version: self.state_version(),
                    })
                }
            }
        }
    }

    pub fn list_outbox(&self) -> Result<ListOutboxOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let items = match self.mode {
            AppMode::Demo => self.demo.lock().expect("demo mutex poisoned").outbox_dtos(),
            AppMode::Live => {
                let mut live = self.live.lock().expect("live mutex poisoned");
                if config::global_db_in(&live.data_dir).exists() {
                    if let Ok(cache) = CacheStore::open_for_data_dir_read_only(&live.data_dir) {
                        let ids: Vec<u32> = live.account_configs.iter().map(|a| a.id).collect();
                        for id in ids {
                            if let Ok(list) = cache.list_outbox(id) {
                                live.outbox_cache.insert(id, list);
                            }
                        }
                    }
                }
                live.combined_outbox_dtos()
            }
        };
        Ok(ListOutboxOutput {
            items,
            state_version: self.state_version(),
        })
    }

    pub fn retry_outbox_item(
        &self,
        input: &OutboxItemActionInput,
    ) -> Result<OutboxItemActionOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let account_id = parse_positive_u32(&input.account_id, "accountId")?;
        let outbox_id = parse_positive_u32(&input.outbox_id, "outboxId")?;

        match self.mode {
            AppMode::Demo => {
                let retry_info = {
                    let mut demo = self.demo.lock().expect("demo mutex poisoned");
                    if let Some(pos) = demo.outbox.iter().position(|e| {
                        e.item.id == outbox_id
                            && e.item.account_id == account_id
                            && matches!(
                                e.item.status,
                                OutboxStatus::Failed | OutboxStatus::Uncertain
                            )
                    }) {
                        let token = demo.next_undo_token;
                        demo.next_undo_token = demo.next_undo_token.saturating_add(1);
                        let entry = &mut demo.outbox[pos];
                        entry.item.status = OutboxStatus::Queued;
                        entry.item.last_error.clear();
                        entry.item.send_at = None;
                        entry.undo_token = token;
                        Some((token, demo.outbox_dtos()))
                    } else {
                        None
                    }
                };
                if let Some((token, items)) = retry_info {
                    let ver = self.bump_state_version();
                    self.emitter.emit_outbox_updated(OutboxUpdatedEvent {
                        state_version: ver,
                        items,
                        event_kind: OutboxEventKindDto::Updated,
                        undo_token: None,
                        outbox_id: Some(outbox_id.to_string()),
                        message: None,
                    });
                    self.spawn_demo_delivery(account_id, outbox_id, token, 0);
                    Ok(OutboxItemActionOutput {
                        accepted: true,
                        state_version: ver,
                    })
                } else {
                    Ok(OutboxItemActionOutput {
                        accepted: false,
                        state_version: self.state_version(),
                    })
                }
            }
            AppMode::Live => {
                let (worker, can_retry) = {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    live.refresh_account_outbox_from_db(account_id);
                    let item_status = live
                        .outbox_cache
                        .get(&account_id)
                        .and_then(|list| list.iter().find(|i| i.id == outbox_id))
                        .map(|i| i.status);
                    let can_retry = match item_status {
                        Some(OutboxStatus::Failed | OutboxStatus::Uncertain) => true,
                        Some(_) => false,
                        None => {
                            !config::global_db_in(&live.data_dir).exists() && live.worker.is_some()
                        }
                    };
                    (live.worker.clone(), can_retry)
                };
                if !can_retry {
                    return Ok(OutboxItemActionOutput {
                        accepted: false,
                        state_version: self.state_version(),
                    });
                }
                if let Some(w) = worker {
                    w.send(MailRequest::RetryOutboxItem {
                        account_id,
                        outbox_id,
                    });
                    let ver = self.bump_state_version();
                    Ok(OutboxItemActionOutput {
                        accepted: true,
                        state_version: ver,
                    })
                } else {
                    Ok(OutboxItemActionOutput {
                        accepted: false,
                        state_version: self.state_version(),
                    })
                }
            }
        }
    }

    pub fn delete_outbox_item(
        &self,
        input: &OutboxItemActionInput,
    ) -> Result<OutboxItemActionOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let account_id = parse_positive_u32(&input.account_id, "accountId")?;
        let outbox_id = parse_positive_u32(&input.outbox_id, "outboxId")?;

        match self.mode {
            AppMode::Demo => {
                let items_opt = {
                    let mut demo = self.demo.lock().expect("demo mutex poisoned");
                    if let Some(pos) = demo.outbox.iter().position(|e| {
                        e.item.id == outbox_id
                            && e.item.account_id == account_id
                            && e.item.status != OutboxStatus::Sending
                    }) {
                        demo.outbox.remove(pos);
                        Some(demo.outbox_dtos())
                    } else {
                        None
                    }
                };
                if let Some(items) = items_opt {
                    let ver = self.bump_state_version();
                    self.emitter.emit_outbox_updated(OutboxUpdatedEvent {
                        state_version: ver,
                        items,
                        event_kind: OutboxEventKindDto::Updated,
                        undo_token: None,
                        outbox_id: Some(outbox_id.to_string()),
                        message: None,
                    });
                    Ok(OutboxItemActionOutput {
                        accepted: true,
                        state_version: ver,
                    })
                } else {
                    Ok(OutboxItemActionOutput {
                        accepted: false,
                        state_version: self.state_version(),
                    })
                }
            }
            AppMode::Live => {
                let (worker, async_worker, can_delete, rx_opt, items_mock) = {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    live.refresh_account_outbox_from_db(account_id);
                    let worker = live.worker.clone();
                    let is_async = worker.as_ref().is_some_and(|w| w.is_async_worker());
                    let has_db = config::global_db_in(&live.data_dir).exists();
                    let matched_status = live
                        .outbox_cache
                        .get(&account_id)
                        .and_then(|list| list.iter().find(|i| i.id == outbox_id))
                        .map(|i| i.status);
                    let can_delete = match matched_status {
                        Some(OutboxStatus::Sending) => false,
                        Some(
                            OutboxStatus::Queued | OutboxStatus::Failed | OutboxStatus::Uncertain,
                        ) => true,
                        None => !has_db && worker.is_some(),
                    };
                    if !can_delete {
                        (worker, is_async, false, None, Vec::new())
                    } else if is_async {
                        let (tx, rx) = mpsc::channel();
                        live.waiters.outbox_deleted.insert(outbox_id, tx);
                        (worker, true, true, Some(rx), Vec::new())
                    } else {
                        if let Some(list) = live.outbox_cache.get_mut(&account_id) {
                            list.retain(|i| i.id != outbox_id);
                        }
                        (worker, false, true, None, live.combined_outbox_dtos())
                    }
                };
                let Some(w) = worker else {
                    return Ok(OutboxItemActionOutput {
                        accepted: false,
                        state_version: self.state_version(),
                    });
                };
                if !can_delete {
                    return Ok(OutboxItemActionOutput {
                        accepted: false,
                        state_version: self.state_version(),
                    });
                }
                w.send(MailRequest::DeleteOutboxItem {
                    account_id,
                    outbox_id,
                });
                if async_worker {
                    let rx = rx_opt.expect("async delete outbox waiter missing");
                    let deleted = rx
                        .recv_timeout(WORKER_WAIT_TIMEOUT)
                        .map_err(|_| {
                            let mut live = self.live.lock().expect("live mutex poisoned");
                            live.waiters.outbox_deleted.remove(&outbox_id);
                            IpcError::unavailable("删除发件箱记录超时，请刷新后重试", None)
                        })?
                        .map_err(|msg| IpcError::internal(msg, None))?;
                    return Ok(OutboxItemActionOutput {
                        accepted: deleted,
                        state_version: self.state_version(),
                    });
                }
                let ver = self.bump_state_version();
                self.emitter.emit_outbox_updated(OutboxUpdatedEvent {
                    state_version: ver,
                    items: items_mock,
                    event_kind: OutboxEventKindDto::Updated,
                    undo_token: None,
                    outbox_id: Some(outbox_id.to_string()),
                    message: None,
                });
                Ok(OutboxItemActionOutput {
                    accepted: true,
                    state_version: ver,
                })
            }
        }
    }
}

#[tauri::command]
pub fn list_drafts(state: State<'_, AppState>) -> Result<ListDraftsOutput, IpcError> {
    state.session.list_drafts()
}

#[tauri::command]
pub fn save_draft(
    state: State<'_, AppState>,
    input: SaveDraftInput,
) -> Result<SaveDraftOutput, IpcError> {
    state.session.save_draft(&input)
}

#[tauri::command]
pub fn delete_draft(
    state: State<'_, AppState>,
    input: DeleteDraftInput,
) -> Result<DeleteDraftOutput, IpcError> {
    state.session.delete_draft(&input)
}

#[tauri::command]
pub fn send_mail(
    state: State<'_, AppState>,
    input: SendMailInput,
) -> Result<SendMailOutput, IpcError> {
    state.session.send_mail(&input)
}

#[tauri::command]
pub fn undo_send_mail(
    state: State<'_, AppState>,
    input: UndoSendMailInput,
) -> Result<UndoSendMailOutput, IpcError> {
    state.session.undo_send_mail(&input)
}

#[tauri::command]
pub fn list_outbox(state: State<'_, AppState>) -> Result<ListOutboxOutput, IpcError> {
    state.session.list_outbox()
}

#[tauri::command]
pub fn retry_outbox_item(
    state: State<'_, AppState>,
    input: OutboxItemActionInput,
) -> Result<OutboxItemActionOutput, IpcError> {
    state.session.retry_outbox_item(&input)
}

#[tauri::command]
pub fn delete_outbox_item(
    state: State<'_, AppState>,
    input: OutboxItemActionInput,
) -> Result<OutboxItemActionOutput, IpcError> {
    state.session.delete_outbox_item(&input)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::dto::BodyFormatDto;
    use crate::mail_session::tests::{make_live_test_session, make_test_session, TestEmitter};
    use crate::preferences::PreferencesStore;
    use crate::reader::{NullReaderSink, ReaderManager};

    fn make_demo_arc_session() -> (Arc<MailSession>, Arc<TestEmitter>, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("kitemail-compose-demo-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let prefs = Arc::new(PreferencesStore::open_in(dir.clone()));
        let reader = Arc::new(ReaderManager::new(Arc::new(NullReaderSink)));
        let emitter = Arc::new(TestEmitter::default());
        let session = Arc::new(MailSession::new(
            AppMode::Demo,
            prefs,
            reader,
            emitter.clone(),
        ));
        session.bind_self(&session);
        (session, emitter, dir)
    }

    #[test]
    fn demo_draft_crud_and_silent_autosave_work() {
        let (session, emitter, dir) = make_test_session();

        let saved1 = session
            .save_draft(&SaveDraftInput {
                id: None,
                account_id: "1".to_string(),
                to_addr: "alice@example.com".to_string(),
                cc_addr: String::new(),
                bcc_addr: String::new(),
                subject: "草稿主题".to_string(),
                body_text: "第一版正文".to_string(),
                in_reply_to: None,
                from_addr: None,
                reply_to: None,
                body_format: Some(BodyFormatDto::Markdown),
                attachment_paths: vec!["/tmp/a.txt".to_string()],
                silent: false,
            })
            .expect("create draft");
        assert_eq!(saved1.draft.id, "1");
        assert_eq!(saved1.draft.body_format, BodyFormatDto::Markdown);
        assert_eq!(emitter.draft_saved.lock().unwrap().len(), 1);
        assert_eq!(emitter.mail_changed.lock().unwrap().len(), 1);

        // Silent autosave updates in-place and emits draft-saved without mail-changed noise.
        let saved2 = session
            .save_draft(&SaveDraftInput {
                id: Some(saved1.draft.id.clone()),
                account_id: "1".to_string(),
                to_addr: "alice@example.com".to_string(),
                cc_addr: "bob@example.com".to_string(),
                bcc_addr: String::new(),
                subject: "草稿主题(更新)".to_string(),
                body_text: "第二版正文".to_string(),
                in_reply_to: Some("<orig@example.com>".to_string()),
                from_addr: None,
                reply_to: Some("reply@example.com".to_string()),
                body_format: Some(BodyFormatDto::Html),
                attachment_paths: Vec::new(),
                silent: true,
            })
            .expect("autosave update");
        assert_eq!(saved2.draft.id, "1");
        assert_eq!(saved2.draft.subject, "草稿主题(更新)");
        assert_eq!(
            saved2.draft.in_reply_to.as_deref(),
            Some("<orig@example.com>")
        );
        assert_eq!(emitter.draft_saved.lock().unwrap().len(), 2);
        assert_eq!(emitter.mail_changed.lock().unwrap().len(), 1);

        let listed = session.list_drafts().expect("list drafts");
        assert_eq!(listed.drafts.len(), 1);
        assert_eq!(listed.drafts[0].body_text, "第二版正文");

        let deleted = session
            .delete_draft(&DeleteDraftInput {
                draft_id: "1".to_string(),
            })
            .expect("delete draft");
        assert!(deleted.deleted);
        assert!(session.list_drafts().unwrap().drafts.is_empty());

        let deleted_again = session
            .delete_draft(&DeleteDraftInput {
                draft_id: "1".to_string(),
            })
            .expect("delete missing draft");
        assert!(!deleted_again.deleted);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn send_mail_validates_recipients_account_and_schedule() {
        let (session, _, dir) = make_test_session();

        let empty_to = session
            .send_mail(&SendMailInput {
                account_id: "1".to_string(),
                to: "   ".to_string(),
                cc: String::new(),
                bcc: String::new(),
                subject: "hi".to_string(),
                body: "body".to_string(),
                in_reply_to: None,
                from_addr: None,
                reply_to: None,
                body_format: None,
                body_html: None,
                attachment_paths: Vec::new(),
                local_draft_id: None,
                undo_delay_secs: None,
                scheduled_at: None,
                client_request_id: None,
            })
            .unwrap_err();
        assert_eq!(empty_to.code, "INVALID_INPUT");

        let bad_acc = session
            .send_mail(&SendMailInput {
                account_id: "99".to_string(),
                to: "a@b.com".to_string(),
                cc: String::new(),
                bcc: String::new(),
                subject: "hi".to_string(),
                body: "body".to_string(),
                in_reply_to: None,
                from_addr: None,
                reply_to: None,
                body_format: None,
                body_html: None,
                attachment_paths: Vec::new(),
                local_draft_id: None,
                undo_delay_secs: None,
                scheduled_at: None,
                client_request_id: None,
            })
            .unwrap_err();
        assert_eq!(bad_acc.code, "INVALID_INPUT");

        let past_sched = session
            .send_mail(&SendMailInput {
                account_id: "1".to_string(),
                to: "a@b.com".to_string(),
                cc: String::new(),
                bcc: String::new(),
                subject: "hi".to_string(),
                body: "body".to_string(),
                in_reply_to: None,
                from_addr: None,
                reply_to: None,
                body_format: None,
                body_html: None,
                attachment_paths: Vec::new(),
                local_draft_id: None,
                undo_delay_secs: None,
                scheduled_at: Some(1_000_000),
                client_request_id: None,
            })
            .unwrap_err();
        assert_eq!(past_sched.code, "INVALID_INPUT");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn demo_send_and_undo_prevent_double_delivery_and_restore_draft() {
        let (session, emitter, dir) = make_demo_arc_session();

        let draft = session
            .save_draft(&SaveDraftInput {
                id: None,
                account_id: "1".to_string(),
                to_addr: "target@example.com".to_string(),
                cc_addr: String::new(),
                bcc_addr: String::new(),
                subject: "可撤销邮件".to_string(),
                body_text: "请勿发出".to_string(),
                in_reply_to: None,
                from_addr: None,
                reply_to: None,
                body_format: Some(BodyFormatDto::Plain),
                attachment_paths: Vec::new(),
                silent: false,
            })
            .unwrap();

        let sent = session
            .send_mail(&SendMailInput {
                account_id: "1".to_string(),
                to: "target@example.com".to_string(),
                cc: String::new(),
                bcc: String::new(),
                subject: "可撤销邮件".to_string(),
                body: "请勿发出".to_string(),
                in_reply_to: None,
                from_addr: None,
                reply_to: None,
                body_format: Some(BodyFormatDto::Plain),
                body_html: None,
                attachment_paths: Vec::new(),
                local_draft_id: Some(draft.draft.id),
                undo_delay_secs: Some(1),
                scheduled_at: None,
                client_request_id: None,
            })
            .expect("enqueue send");
        assert!(sent.accepted);
        assert_eq!(sent.undo_window_secs, 1);
        assert!(session.list_drafts().unwrap().drafts.is_empty());
        assert_eq!(session.list_outbox().unwrap().items.len(), 1);

        // Undo immediately before 1s background timer finishes.
        let undone = session
            .undo_send_mail(&UndoSendMailInput {
                account_id: "1".to_string(),
                outbox_id: sent.outbox_id.clone(),
                undo_token: sent.undo_token.clone(),
            })
            .expect("undo send");
        assert!(undone.undone);
        assert!(session.list_outbox().unwrap().items.is_empty());
        assert_eq!(session.list_drafts().unwrap().drafts.len(), 1);

        // Wait past 1s to verify background delivery thread sees undo and does NOT deliver.
        std::thread::sleep(Duration::from_millis(1150));
        let outbox_events = emitter.outbox_updated.lock().unwrap().clone();
        assert!(!outbox_events
            .iter()
            .any(|e| e.event_kind == OutboxEventKindDto::Sent));

        // Second undo returns undone: false cleanly (race/idempotency).
        let second_undo = session
            .undo_send_mail(&UndoSendMailInput {
                account_id: "1".to_string(),
                outbox_id: sent.outbox_id,
                undo_token: sent.undo_token,
            })
            .expect("second undo");
        assert!(!second_undo.undone);

        // Immediate send (undoDelaySecs = 0) completes delivery in background.
        let immediate = session
            .send_mail(&SendMailInput {
                account_id: "1".to_string(),
                to: "target2@example.com".to_string(),
                cc: String::new(),
                bcc: String::new(),
                subject: "立即发送邮件".to_string(),
                body: "立即投递".to_string(),
                in_reply_to: None,
                from_addr: None,
                reply_to: None,
                body_format: None,
                body_html: None,
                attachment_paths: Vec::new(),
                local_draft_id: None,
                undo_delay_secs: Some(0),
                scheduled_at: None,
                client_request_id: None,
            })
            .expect("immediate send");
        assert_eq!(immediate.undo_window_secs, 0);
        let delivery_deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < delivery_deadline {
            if emitter
                .outbox_updated
                .lock()
                .unwrap()
                .iter()
                .any(|e| e.event_kind == OutboxEventKindDto::Sent)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(emitter
            .outbox_updated
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.event_kind == OutboxEventKindDto::Sent));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_compose_dispatches_worker_requests_and_rejects_when_locked() {
        let (session, emitter, _, worker, dir) = make_live_test_session(true, Some("1234"));

        session.lock_app(Some("manual"));
        assert_eq!(session.list_drafts().unwrap_err().code, "LOCKED");
        assert_eq!(session.list_outbox().unwrap_err().code, "LOCKED");
        assert_eq!(
            session
                .save_draft(&SaveDraftInput {
                    id: None,
                    account_id: "1".to_string(),
                    to_addr: "a@b.com".to_string(),
                    cc_addr: String::new(),
                    bcc_addr: String::new(),
                    subject: "s".to_string(),
                    body_text: "b".to_string(),
                    in_reply_to: None,
                    from_addr: None,
                    reply_to: None,
                    body_format: None,
                    attachment_paths: Vec::new(),
                    silent: true,
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .delete_draft(&DeleteDraftInput {
                    draft_id: "1".to_string()
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .send_mail(&SendMailInput {
                    account_id: "1".to_string(),
                    to: "a@b.com".to_string(),
                    cc: String::new(),
                    bcc: String::new(),
                    subject: "s".to_string(),
                    body: "b".to_string(),
                    in_reply_to: None,
                    from_addr: None,
                    reply_to: None,
                    body_format: None,
                    body_html: None,
                    attachment_paths: Vec::new(),
                    local_draft_id: None,
                    undo_delay_secs: None,
                    scheduled_at: None,
                    client_request_id: None,
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .undo_send_mail(&UndoSendMailInput {
                    account_id: "1".to_string(),
                    outbox_id: "1".to_string(),
                    undo_token: "1".to_string()
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .retry_outbox_item(&OutboxItemActionInput {
                    account_id: "1".to_string(),
                    outbox_id: "1".to_string()
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .delete_outbox_item(&OutboxItemActionInput {
                    account_id: "1".to_string(),
                    outbox_id: "1".to_string()
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );

        session.unlock_app(&crate::dto::UnlockAppInput {
            passcode: "1234".to_string(),
        });

        // Silent save -> AutosaveLocalDraft; non-silent -> SaveLocalDraft
        session
            .save_draft(&SaveDraftInput {
                id: None,
                account_id: "1".to_string(),
                to_addr: "x@y.com".to_string(),
                cc_addr: String::new(),
                bcc_addr: String::new(),
                subject: "自动保存".to_string(),
                body_text: "内容".to_string(),
                in_reply_to: None,
                from_addr: None,
                reply_to: None,
                body_format: Some(BodyFormatDto::Plain),
                attachment_paths: Vec::new(),
                silent: true,
            })
            .unwrap();
        let send_out = session
            .send_mail(&SendMailInput {
                account_id: "1".to_string(),
                to: "x@y.com".to_string(),
                cc: String::new(),
                bcc: String::new(),
                subject: "发送测试".to_string(),
                body: "正文".to_string(),
                in_reply_to: None,
                from_addr: None,
                reply_to: None,
                body_format: None,
                body_html: None,
                attachment_paths: Vec::new(),
                local_draft_id: Some("1".to_string()),
                undo_delay_secs: Some(5),
                scheduled_at: None,
                client_request_id: None,
            })
            .unwrap();
        assert!(send_out.accepted);

        session
            .undo_send_mail(&UndoSendMailInput {
                account_id: "1".to_string(),
                outbox_id: send_out.outbox_id,
                undo_token: send_out.undo_token,
            })
            .unwrap();

        let reqs = worker.requests.lock().unwrap();
        assert!(reqs
            .iter()
            .any(|r| matches!(r, MailRequest::AutosaveLocalDraft { .. })));
        assert!(reqs
            .iter()
            .any(|r| matches!(r, MailRequest::SendDraft { .. })));
        assert!(reqs
            .iter()
            .any(|r| matches!(r, MailRequest::UndoOutboxItem { .. })));
        assert!(!emitter.outbox_updated.lock().unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_input_accepts_both_string_and_number_ids() {
        let draft_del_str: DeleteDraftInput =
            serde_json::from_value(serde_json::json!({ "draftId": "12" })).unwrap();
        let draft_del_num: DeleteDraftInput =
            serde_json::from_value(serde_json::json!({ "draftId": 12 })).unwrap();
        assert_eq!(draft_del_str.draft_id, "12");
        assert_eq!(draft_del_num.draft_id, "12");

        let undo_in: UndoSendMailInput = serde_json::from_value(serde_json::json!({
            "accountId": 1,
            "outboxId": "7",
            "undoToken": 99
        }))
        .unwrap();
        assert_eq!(undo_in.account_id, "1");
        assert_eq!(undo_in.outbox_id, "7");
        assert_eq!(undo_in.undo_token, "99");
    }

    #[test]
    fn send_mail_idempotency_and_outbox_delete_status_contract_work() {
        let (demo_session, _, demo_dir) = make_demo_arc_session();
        let first = demo_session
            .send_mail(&SendMailInput {
                account_id: "1".to_string(),
                to: "dup@example.com".to_string(),
                cc: String::new(),
                bcc: String::new(),
                subject: "幂等主题".to_string(),
                body: "内容".to_string(),
                in_reply_to: None,
                from_addr: None,
                reply_to: None,
                body_format: None,
                body_html: None,
                attachment_paths: Vec::new(),
                local_draft_id: None,
                undo_delay_secs: Some(10),
                scheduled_at: None,
                client_request_id: Some("req-idem-1".to_string()),
            })
            .expect("first demo send");
        let retry = demo_session
            .send_mail(&SendMailInput {
                account_id: "1".to_string(),
                to: "dup@example.com".to_string(),
                cc: String::new(),
                bcc: String::new(),
                subject: "幂等主题".to_string(),
                body: "内容".to_string(),
                in_reply_to: None,
                from_addr: None,
                reply_to: None,
                body_format: None,
                body_html: None,
                attachment_paths: Vec::new(),
                local_draft_id: None,
                undo_delay_secs: Some(10),
                scheduled_at: None,
                client_request_id: Some("req-idem-1".to_string()),
            })
            .expect("retry demo send");
        assert_eq!(first.outbox_id, retry.outbox_id);
        assert_eq!(demo_session.list_outbox().unwrap().items.len(), 1);
        let _ = std::fs::remove_dir_all(&demo_dir);

        let (live_session, _, _, worker, live_dir) = make_live_test_session(false, None);
        {
            let mut live = live_session.live.lock().unwrap();
            live.outbox_cache.insert(
                1,
                vec![
                    OutboxItem {
                        id: 1,
                        account_id: 1,
                        to_addr: "a@b.com".to_string(),
                        cc_addr: String::new(),
                        bcc_addr: String::new(),
                        subject: "queued".to_string(),
                        body_text: String::new(),
                        in_reply_to: String::new(),
                        from_addr: String::new(),
                        reply_to: String::new(),
                        body_format: "plain".to_string(),
                        body_html: None,
                        attachment_paths: Vec::new(),
                        status: OutboxStatus::Queued,
                        last_error: String::new(),
                        send_at: None,
                    },
                    OutboxItem {
                        id: 2,
                        account_id: 1,
                        to_addr: "a@b.com".to_string(),
                        cc_addr: String::new(),
                        bcc_addr: String::new(),
                        subject: "sending".to_string(),
                        body_text: String::new(),
                        in_reply_to: String::new(),
                        from_addr: String::new(),
                        reply_to: String::new(),
                        body_format: "plain".to_string(),
                        body_html: None,
                        attachment_paths: Vec::new(),
                        status: OutboxStatus::Sending,
                        last_error: String::new(),
                        send_at: None,
                    },
                    OutboxItem {
                        id: 3,
                        account_id: 1,
                        to_addr: "a@b.com".to_string(),
                        cc_addr: String::new(),
                        bcc_addr: String::new(),
                        subject: "failed".to_string(),
                        body_text: String::new(),
                        in_reply_to: String::new(),
                        from_addr: String::new(),
                        reply_to: String::new(),
                        body_format: "plain".to_string(),
                        body_html: None,
                        attachment_paths: Vec::new(),
                        status: OutboxStatus::Failed,
                        last_error: "err".to_string(),
                        send_at: None,
                    },
                    OutboxItem {
                        id: 4,
                        account_id: 1,
                        to_addr: "a@b.com".to_string(),
                        cc_addr: String::new(),
                        bcc_addr: String::new(),
                        subject: "uncertain".to_string(),
                        body_text: String::new(),
                        in_reply_to: String::new(),
                        from_addr: String::new(),
                        reply_to: String::new(),
                        body_format: "plain".to_string(),
                        body_html: None,
                        attachment_paths: Vec::new(),
                        status: OutboxStatus::Uncertain,
                        last_error: "uncertain".to_string(),
                        send_at: None,
                    },
                ],
            );
        }

        assert!(
            live_session
                .delete_outbox_item(&OutboxItemActionInput {
                    account_id: "1".to_string(),
                    outbox_id: "1".to_string(),
                })
                .unwrap()
                .accepted
        );
        assert!(
            !live_session
                .delete_outbox_item(&OutboxItemActionInput {
                    account_id: "1".to_string(),
                    outbox_id: "2".to_string(),
                })
                .unwrap()
                .accepted
        );
        assert!(
            live_session
                .delete_outbox_item(&OutboxItemActionInput {
                    account_id: "1".to_string(),
                    outbox_id: "3".to_string(),
                })
                .unwrap()
                .accepted
        );
        assert!(
            live_session
                .delete_outbox_item(&OutboxItemActionInput {
                    account_id: "1".to_string(),
                    outbox_id: "4".to_string(),
                })
                .unwrap()
                .accepted
        );
        assert_eq!(worker.requests.lock().unwrap().len(), 3);
        let _ = std::fs::remove_dir_all(&live_dir);
    }
}

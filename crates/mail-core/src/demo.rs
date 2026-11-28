//! Deterministic demo dataset for `--demo` mode, aligned with `99b408b:src/backend/demo_mail.rs`.

use std::collections::BTreeMap;

use crate::cache::{CachedAttachment, KnownContact};
use crate::search_query::SearchQuery;
use crate::{
    start_of_today_ts, AccountSummary, AttachmentMeta, FolderSummary, MailPage, MessageDetail,
    MessageFilter, MessageSort, MessageSummary, QuickFilterSet, ThreadMessage,
};

pub const DEMO_INBOX_ID: u32 = 1;
pub const DEMO_SENT_ID: u32 = 2;
pub const DEMO_DRAFTS_ID: u32 = 3;
pub const DEMO_ARCHIVE_ID: u32 = 4;
pub const DEMO_JUNK_ID: u32 = 5;
pub const DEMO_TRASH_ID: u32 = 6;

/// Legacy constant retained for external callers; default `--demo` mailbox uses the curated 3-account showcase.
pub const DEMO_INBOX_TOTAL: usize = 10_000;

const ME: &str = "jason@hylki.hyprlab.co";
const LAB: &str = "hello@hyprlab.dev";
const PERSONAL: &str = "jason.m@fastmail.com";

/// Deterministic Oct 2025 base timestamp so demo mail groups under `Older` (`更早`).
const DEMO_BASE_TS: i64 = 1_760_025_180;

/// In-memory demo store serving 3 accounts, folders, paginated headers, threads, and bodies.
#[derive(Clone, Debug)]
pub struct DemoStore {
    pub accounts: Vec<AccountSummary>,
    pub folders: Vec<FolderSummary>,
    inbox_messages: Vec<MessageSummary>,
    sent_messages: Vec<MessageSummary>,
    draft_messages: Vec<MessageSummary>,
    archive_messages: Vec<MessageSummary>,
    junk_messages: Vec<MessageSummary>,
}

impl Default for DemoStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DemoStore {
    /// Build the 3-account demo dataset aligned with `99b408b:src/backend/demo_mail.rs`.
    pub fn new() -> Self {
        let accounts = vec![
            AccountSummary {
                id: 1,
                display_name: "Jason M.".to_string(),
                email: ME.to_string(),
            },
            AccountSummary {
                id: 2,
                display_name: "Hyprlab".to_string(),
                email: LAB.to_string(),
            },
            AccountSummary {
                id: 3,
                display_name: "Jason (Personal)".to_string(),
                email: PERSONAL.to_string(),
            },
        ];

        let mut inbox_messages = vec![
            // ---- Account 1 · Inbox ----
            MessageSummary {
                id: 1,
                account_id: 1,
                folder_id: 1,
                folder_path: "INBOX".to_string(),
                uid: 1,
                sender: "Marcus Chen <marcus@studio.dev>".to_string(),
                subject: "Reader redesign: final review".to_string(),
                preview: "Hover-reveal gets my vote — it keeps the cards calm and the fade is lovely. Ship it 🚀".to_string(),
                date_ts: DEMO_BASE_TS,
                date_display: "Oct 9, 2025, 3:53 PM".to_string(),
                unread: true,
                flagged: false,
                has_attachment: true,
                recipients: ME.to_string(),
                thread_count: 6,
                importance: 1,
                tags: vec!["Work".to_string()],
                answered: true,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            MessageSummary {
                id: 7,
                account_id: 1,
                folder_id: 1,
                folder_path: "INBOX".to_string(),
                uid: 7,
                sender: "GNOME Foundation <news@gnome.org>".to_string(),
                subject: "GNOME 49 release candidate is here".to_string(),
                preview: "The release candidate for GNOME 49 is now available for testing. This cycle brings major performance work…".to_string(),
                date_ts: DEMO_BASE_TS - 21_600,
                date_display: "Oct 8, 2025, 9:53 PM".to_string(),
                unread: true,
                flagged: true,
                has_attachment: false,
                recipients: ME.to_string(),
                thread_count: 1,
                importance: 0,
                tags: vec!["Personal".to_string()],
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            MessageSummary {
                id: 8,
                account_id: 1,
                folder_id: 1,
                folder_path: "INBOX".to_string(),
                uid: 8,
                sender: "Sophie Turner <sophie@studio.dev>".to_string(),
                subject: "Q3 roadmap review".to_string(),
                preview: "Perfect, updated the doc with the parallel track and Priya's staging step. Thursday's session is now a…".to_string(),
                date_ts: DEMO_BASE_TS - 25_200,
                date_display: "Oct 8, 2025, 8:53 PM".to_string(),
                unread: true,
                flagged: false,
                has_attachment: true,
                recipients: ME.to_string(),
                thread_count: 5,
                importance: 1,
                tags: vec!["Work".to_string()],
                answered: true,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            MessageSummary {
                id: 10,
                account_id: 1,
                folder_id: 1,
                folder_path: "INBOX".to_string(),
                uid: 10,
                sender: "Rust Weekly <digest@this-week-in-rust.org>".to_string(),
                subject: "This Week in Rust #612".to_string(),
                preview: "Crate of the week, RFCs, and community updates. This issue: async closures stabilize, and a deep dive into…".to_string(),
                date_ts: DEMO_BASE_TS - 32_400,
                date_display: "Oct 8, 2025, 6:53 PM".to_string(),
                unread: true,
                flagged: false,
                has_attachment: false,
                recipients: ME.to_string(),
                thread_count: 1,
                importance: 0,
                tags: Vec::new(),
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            MessageSummary {
                id: 11,
                account_id: 1,
                folder_id: 1,
                folder_path: "INBOX".to_string(),
                uid: 11,
                sender: "Apple <no-reply@apple.com>".to_string(),
                subject: "Your receipt from Apple".to_string(),
                preview: "Thank you for your purchase. Your receipt is attached. Order ID W1234567890…".to_string(),
                date_ts: DEMO_BASE_TS - 36_000,
                date_display: "Oct 8, 2025, 5:53 PM".to_string(),
                unread: false,
                flagged: false,
                has_attachment: true,
                recipients: ME.to_string(),
                thread_count: 1,
                importance: 0,
                tags: Vec::new(),
                answered: false,
                forwarded: true,
                snoozed_until_ts: 0,
            },
            MessageSummary {
                id: 12,
                account_id: 1,
                folder_id: 1,
                folder_path: "INBOX".to_string(),
                uid: 12,
                sender: "Marcus Chen <marcus@studio.dev>".to_string(),
                subject: "Design tokens are merged 🎨".to_string(),
                preview: "The design token pipeline is finally merged into main. Dark mode now derives entirely from the token set…".to_string(),
                date_ts: DEMO_BASE_TS - 39_600,
                date_display: "Oct 8, 2025, 4:53 PM".to_string(),
                unread: true,
                flagged: false,
                has_attachment: false,
                recipients: ME.to_string(),
                thread_count: 1,
                importance: 0,
                tags: Vec::new(),
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            MessageSummary {
                id: 13,
                account_id: 1,
                folder_id: 1,
                folder_path: "INBOX".to_string(),
                uid: 13,
                sender: "Calendar <calendar@hylki.hyprlab.co>".to_string(),
                subject: "Invitation: Architecture sync @ Thu 2:00 PM".to_string(),
                preview: "You have been invited to Architecture sync. Thursday 2:00 PM – 3:00 PM. Conference Room B / video link…".to_string(),
                date_ts: DEMO_BASE_TS - 43_200,
                date_display: "Oct 8, 2025, 3:53 PM".to_string(),
                unread: false,
                flagged: false,
                has_attachment: true,
                recipients: ME.to_string(),
                thread_count: 1,
                importance: 0,
                tags: Vec::new(),
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            MessageSummary {
                id: 14,
                account_id: 1,
                folder_id: 1,
                folder_path: "INBOX".to_string(),
                uid: 14,
                sender: "Emma Wright <emma@example.com>".to_string(),
                subject: "Lunch this weekend?".to_string(),
                preview: "It's been ages! Are you free Saturday for lunch at that new place downtown? Let me know what works…".to_string(),
                date_ts: DEMO_BASE_TS - 46_800,
                date_display: "Oct 8, 2025, 2:53 PM".to_string(),
                unread: false,
                flagged: true,
                has_attachment: false,
                recipients: ME.to_string(),
                thread_count: 1,
                importance: 0,
                tags: Vec::new(),
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            MessageSummary {
                id: 15,
                account_id: 1,
                folder_id: 1,
                folder_path: "INBOX".to_string(),
                uid: 15,
                sender: "Linear <notifications@linear.app>".to_string(),
                subject: "3 issues assigned to you this sprint".to_string(),
                preview: "VIREO-142 Reader dark mode, VIREO-148 Infinite scroll spinner, VIREO-151 OAuth for Microsoft…".to_string(),
                date_ts: DEMO_BASE_TS - 50_400,
                date_display: "Oct 8, 2025, 1:53 PM".to_string(),
                unread: true,
                flagged: false,
                has_attachment: false,
                recipients: ME.to_string(),
                thread_count: 1,
                importance: 0,
                tags: Vec::new(),
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            MessageSummary {
                id: 16,
                account_id: 1,
                folder_id: 1,
                folder_path: "INBOX".to_string(),
                uid: 16,
                sender: "Framer <team@framer.com>".to_string(),
                subject: "Your weekly site analytics".to_string(),
                preview: "hylki.hyprlab.co had 4,218 visitors this week, up 32%. Top page: /download. See the full breakdown…".to_string(),
                date_ts: DEMO_BASE_TS - 54_000,
                date_display: "Oct 8, 2025, 12:53 PM".to_string(),
                unread: false,
                flagged: false,
                has_attachment: false,
                recipients: ME.to_string(),
                thread_count: 1,
                importance: 0,
                tags: Vec::new(),
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            // ---- Account 2 · Inbox (Hyprlab) ----
            MessageSummary {
                id: 30,
                account_id: 2,
                folder_id: 11,
                folder_path: "INBOX".to_string(),
                uid: 30,
                sender: "Proton <security@proton.me>".to_string(),
                subject: "New sign-in to your account".to_string(),
                preview: "We noticed a new sign-in from Fedora Linux · Firefox. If this was you, no action is needed…".to_string(),
                date_ts: DEMO_BASE_TS - 104_400,
                date_display: "Oct 8, 2025, 10:11 AM".to_string(),
                unread: true,
                flagged: false,
                has_attachment: false,
                recipients: LAB.to_string(),
                thread_count: 1,
                importance: 0,
                tags: Vec::new(),
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            MessageSummary {
                id: 31,
                account_id: 2,
                folder_id: 11,
                folder_path: "INBOX".to_string(),
                uid: 31,
                sender: "Buy Me a Coffee <no-reply@buymeacoffee.com>".to_string(),
                subject: "You have a new supporter ☕".to_string(),
                preview: "Alex bought you a coffee and left a note: “Love Hylki — the GNOME-native mail client I've wanted for years!”".to_string(),
                date_ts: DEMO_BASE_TS - 108_000,
                date_display: "Oct 8, 2025, 9:11 AM".to_string(),
                unread: true,
                flagged: true,
                has_attachment: false,
                recipients: LAB.to_string(),
                thread_count: 1,
                importance: 0,
                tags: Vec::new(),
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            MessageSummary {
                id: 32,
                account_id: 2,
                folder_id: 11,
                folder_path: "INBOX".to_string(),
                uid: 32,
                sender: "GitHub <notifications@github.com>".to_string(),
                subject: "[hyprlab/hylki] Star milestone: 1,000 ⭐".to_string(),
                preview: "Your repository hyprlab/hylki just reached 1,000 stars. Nice work! See who starred recently…".to_string(),
                date_ts: DEMO_BASE_TS - 111_600,
                date_display: "Oct 8, 2025, 8:11 AM".to_string(),
                unread: true,
                flagged: false,
                has_attachment: false,
                recipients: LAB.to_string(),
                thread_count: 1,
                importance: 0,
                tags: Vec::new(),
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            MessageSummary {
                id: 33,
                account_id: 2,
                folder_id: 11,
                folder_path: "INBOX".to_string(),
                uid: 33,
                sender: "This Week in GNOME <hello@thisweek.gnome.org>".to_string(),
                subject: "This Week in GNOME #180".to_string(),
                preview: "Highlights from across the GNOME project: new adaptive widgets, shell performance work, and a wave of app releases…".to_string(),
                date_ts: DEMO_BASE_TS - 115_200,
                date_display: "Oct 7, 2025".to_string(),
                unread: false,
                flagged: false,
                has_attachment: false,
                recipients: LAB.to_string(),
                thread_count: 1,
                importance: 0,
                tags: Vec::new(),
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            // ---- Account 3 · Inbox (Jason Personal) ----
            MessageSummary {
                id: 40,
                account_id: 3,
                folder_id: 21,
                folder_path: "INBOX".to_string(),
                uid: 40,
                sender: "Elena Ruiz <elena.ruiz@gmail.com>".to_string(),
                subject: "Cabin weekend — final headcount?".to_string(),
                preview: "Locking the booking tonight! Are you and Sam still in for the 19th? We're at nine people if so, which…".to_string(),
                date_ts: DEMO_BASE_TS - 140_400,
                date_display: "Oct 7, 2025, 11:02 AM".to_string(),
                unread: true,
                flagged: true,
                has_attachment: false,
                recipients: PERSONAL.to_string(),
                thread_count: 1,
                importance: 0,
                tags: vec!["Personal".to_string()],
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            MessageSummary {
                id: 41,
                account_id: 3,
                folder_id: 21,
                folder_path: "INBOX".to_string(),
                uid: 41,
                sender: "Fastmail <support@fastmail.com>".to_string(),
                subject: "Your storage is 80% full".to_string(),
                preview: "Your mailbox is approaching its storage limit. Consider archiving old attachments or upgrading your plan…".to_string(),
                date_ts: DEMO_BASE_TS - 144_000,
                date_display: "Oct 7, 2025, 9:47 AM".to_string(),
                unread: true,
                flagged: false,
                has_attachment: false,
                recipients: PERSONAL.to_string(),
                thread_count: 1,
                importance: 0,
                tags: Vec::new(),
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            MessageSummary {
                id: 42,
                account_id: 3,
                folder_id: 21,
                folder_path: "INBOX".to_string(),
                uid: 42,
                sender: "City Library <noreply@citylibrary.org>".to_string(),
                subject: "Hold ready for pickup: \"The Pragmatic Programmer\"".to_string(),
                preview: "Good news — an item you placed on hold is ready for pickup at the Central branch. Please collect it within…".to_string(),
                date_ts: DEMO_BASE_TS - 147_600,
                date_display: "Oct 7, 2025".to_string(),
                unread: true,
                flagged: false,
                has_attachment: false,
                recipients: PERSONAL.to_string(),
                thread_count: 1,
                importance: 0,
                tags: Vec::new(),
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            MessageSummary {
                id: 43,
                account_id: 3,
                folder_id: 21,
                folder_path: "INBOX".to_string(),
                uid: 43,
                sender: "Ridgeline Cycles <service@ridgelinecycles.com>".to_string(),
                subject: "Your tune-up is complete".to_string(),
                preview: "Your bike is ready! We replaced the chain, trued the rear wheel, and bled the rear brake. Total comes to…".to_string(),
                date_ts: DEMO_BASE_TS - 151_200,
                date_display: "Oct 6, 2025".to_string(),
                unread: false,
                flagged: false,
                has_attachment: true,
                recipients: PERSONAL.to_string(),
                thread_count: 1,
                importance: 0,
                tags: Vec::new(),
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
        ];

        // Optional bulk padding when HYLKI_DEMO_BULK=N is explicitly set (matches 99b408b).
        if let Some(n) = std::env::var("HYLKI_DEMO_BULK")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
        {
            let mut bulk_id = 100_000u32;
            for (account_id, folder_id, to_addr) in [
                (1u32, 1u32, ME),
                (2u32, 11u32, LAB),
                (3u32, 21u32, PERSONAL),
            ] {
                for i in 0..n {
                    bulk_id += 1;
                    inbox_messages.push(MessageSummary {
                        id: bulk_id,
                        account_id,
                        folder_id,
                        folder_path: "INBOX".to_string(),
                        uid: bulk_id,
                        sender: format!("Sender {} <sender{}@example.com>", i % 97, i % 97),
                        subject: format!("Bulk message {i} in folder {folder_id}"),
                        preview: "A generated message that stands in for real mail, so a list of thousands can be measured.".to_string(),
                        date_ts: DEMO_BASE_TS - 200_000 - 3600 * (i as i64 + 1) - account_id as i64,
                        date_display: "Oct 1, 2025".to_string(),
                        unread: i % 7 == 0,
                        flagged: i % 41 == 0,
                        has_attachment: false,
                        recipients: to_addr.to_string(),
                        thread_count: 1,
                        importance: 0,
                        tags: Vec::new(),
                        answered: false,
                        forwarded: false,
                        snoozed_until_ts: 0,
                    });
                }
            }
        }

        let sent_messages = vec![
            MessageSummary {
                id: 20_001,
                account_id: 1,
                folder_id: 2,
                folder_path: "Sent".to_string(),
                uid: 20_001,
                sender: format!("Jason M. <{ME}>"),
                subject: "Re: Reader redesign: final review".to_string(),
                preview: "Agreed on the continuous surface for plain mail — that's in. I also merged the thread chip work.".to_string(),
                date_ts: DEMO_BASE_TS - 10_800,
                date_display: "Oct 9, 2025, 12:53 PM".to_string(),
                unread: false,
                flagged: false,
                has_attachment: false,
                recipients: "Sophie Turner <sophie@studio.dev>".to_string(),
                thread_count: 1,
                importance: 0,
                tags: vec!["Work".to_string()],
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            MessageSummary {
                id: 20_002,
                account_id: 1,
                folder_id: 2,
                folder_path: "Sent".to_string(),
                uid: 20_002,
                sender: format!("Jason M. <{ME}>"),
                subject: "Re: Q3 roadmap review".to_string(),
                preview: "Looks great overall. I left comments on the migration phase — I think we can parallelize the first two steps.".to_string(),
                date_ts: DEMO_BASE_TS - 64_800,
                date_display: "Oct 8, 2025, 6:53 AM".to_string(),
                unread: false,
                flagged: false,
                has_attachment: false,
                recipients: "Sophie Turner <sophie@studio.dev>".to_string(),
                thread_count: 1,
                importance: 0,
                tags: vec!["Work".to_string()],
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            MessageSummary {
                id: 20_003,
                account_id: 2,
                folder_id: 12,
                folder_path: "Sent".to_string(),
                uid: 20_003,
                sender: format!("Hyprlab <{LAB}>"),
                subject: "Re: Hetzner billing follow-up".to_string(),
                preview: "Confirmed invoice R0034215 for September.".to_string(),
                date_ts: DEMO_BASE_TS - 180_000,
                date_display: "Oct 6, 2025".to_string(),
                unread: false,
                flagged: false,
                has_attachment: false,
                recipients: "billing@hetzner.com".to_string(),
                thread_count: 1,
                importance: 0,
                tags: Vec::new(),
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
        ];

        let draft_messages = vec![MessageSummary {
            id: 20,
            account_id: 1,
            folder_id: 3,
            folder_path: "Drafts".to_string(),
            uid: 20,
            sender: format!("Jason M. <{ME}>"),
            subject: "Release notes for 0.2".to_string(),
            preview: "Draft — Highlights for the next build: actions palette, message-content theme, infinite scroll…".to_string(),
            date_ts: DEMO_BASE_TS - 72_000,
            date_display: "Oct 8, 2025".to_string(),
            unread: false,
            flagged: false,
            has_attachment: false,
            recipients: "team@hylki.hyprlab.co".to_string(),
            thread_count: 1,
            importance: 0,
            tags: Vec::new(),
            answered: false,
            forwarded: false,
            snoozed_until_ts: 0,
        }];

        let archive_messages = vec![
            MessageSummary {
                id: 40_001,
                account_id: 1,
                folder_id: 4,
                folder_path: "Archive".to_string(),
                uid: 40_001,
                sender: "Design Systems Weekly <hello@designsystems.substack.com>".to_string(),
                subject: "Issue 84: tokens that survive a rebrand".to_string(),
                preview: "This week: how three teams versioned their color tokens through a rebrand without touching a component, plus…".to_string(),
                date_ts: DEMO_BASE_TS - 250_000,
                date_display: "Oct 5, 2025".to_string(),
                unread: false,
                flagged: false,
                has_attachment: false,
                recipients: ME.to_string(),
                thread_count: 1,
                importance: 0,
                tags: Vec::new(),
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            MessageSummary {
                id: 40_002,
                account_id: 2,
                folder_id: 14,
                folder_path: "Archive".to_string(),
                uid: 40_002,
                sender: "Fastly <billing@fastly.com>".to_string(),
                subject: "Your Fastly invoice is ready".to_string(),
                preview: "The invoice for last month's usage is now available in your account. Total: $12.04.".to_string(),
                date_ts: DEMO_BASE_TS - 280_000,
                date_display: "Oct 4, 2025".to_string(),
                unread: false,
                flagged: false,
                has_attachment: true,
                recipients: LAB.to_string(),
                thread_count: 1,
                importance: 0,
                tags: vec!["Work".to_string()],
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            MessageSummary {
                id: 40_003,
                account_id: 3,
                folder_id: 24,
                folder_path: "Archive".to_string(),
                uid: 40_003,
                sender: "Bookshop.org <orders@bookshop.org>".to_string(),
                subject: "Your order is confirmed".to_string(),
                preview: "Thanks for supporting Central Books! Your order of 2 items is confirmed and will ship within 2 business days…".to_string(),
                date_ts: DEMO_BASE_TS - 320_000,
                date_display: "Oct 3, 2025".to_string(),
                unread: false,
                flagged: false,
                has_attachment: false,
                recipients: PERSONAL.to_string(),
                thread_count: 1,
                importance: 0,
                tags: vec!["Personal".to_string()],
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
        ];

        let junk_messages = vec![
            MessageSummary {
                id: 46,
                account_id: 1,
                folder_id: 5,
                folder_path: "Junk".to_string(),
                uid: 46,
                sender: "Prize Department <winner@lucky-draw-notify.biz>".to_string(),
                subject: "You have been selected!!!".to_string(),
                preview: "Congratulations, your address was chosen in our monthly draw. Confirm your details within 24 hours to claim…".to_string(),
                date_ts: DEMO_BASE_TS - 190_000,
                date_display: "Oct 6, 2025".to_string(),
                unread: true,
                flagged: false,
                has_attachment: false,
                recipients: ME.to_string(),
                thread_count: 1,
                importance: 0,
                tags: Vec::new(),
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
            MessageSummary {
                id: 47,
                account_id: 1,
                folder_id: 5,
                folder_path: "Junk".to_string(),
                uid: 47,
                sender: "Nordic Sauna Club <news@nordicsaunaclub.example>".to_string(),
                subject: "Your membership offer expires tonight".to_string(),
                preview: "Last chance: 40% off a year of unlimited sessions. Offer ends at midnight…".to_string(),
                date_ts: DEMO_BASE_TS - 210_000,
                date_display: "Oct 5, 2025".to_string(),
                unread: true,
                flagged: false,
                has_attachment: false,
                recipients: ME.to_string(),
                thread_count: 1,
                importance: 0,
                tags: Vec::new(),
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            },
        ];

        let count_slice =
            |slice: &[MessageSummary], account_id: u32, folder_id: u32| -> (u32, usize) {
                let mut unread = 0u32;
                let mut total = 0usize;
                for m in slice {
                    if m.account_id == account_id && m.folder_id == folder_id {
                        total += 1;
                        if m.unread {
                            unread += 1;
                        }
                    }
                }
                (unread, total)
            };

        let make_folder = |id: u32,
                           account_id: u32,
                           name: &str,
                           path: &str,
                           slice: &[MessageSummary]|
         -> FolderSummary {
            let (unread_count, total_count) = count_slice(slice, account_id, id);
            FolderSummary {
                id,
                account_id,
                name: name.to_string(),
                path: path.to_string(),
                unread_count,
                total_count,
                has_more_remote: false,
            }
        };

        let folders = vec![
            // Account 1
            make_folder(DEMO_INBOX_ID, 1, "收件箱", "INBOX", &inbox_messages),
            make_folder(DEMO_SENT_ID, 1, "已发送", "Sent", &sent_messages),
            make_folder(DEMO_DRAFTS_ID, 1, "草稿箱", "Drafts", &draft_messages),
            make_folder(DEMO_ARCHIVE_ID, 1, "归档", "Archive", &archive_messages),
            make_folder(DEMO_JUNK_ID, 1, "垃圾邮件", "Junk", &junk_messages),
            make_folder(DEMO_TRASH_ID, 1, "已删除", "Trash", &[]),
            // Account 2
            make_folder(11, 2, "收件箱", "INBOX", &inbox_messages),
            make_folder(12, 2, "已发送", "Sent", &sent_messages),
            make_folder(13, 2, "草稿箱", "Drafts", &[]),
            make_folder(14, 2, "归档", "Archive", &archive_messages),
            make_folder(15, 2, "垃圾邮件", "Junk", &[]),
            make_folder(16, 2, "已删除", "Trash", &[]),
            // Account 3
            make_folder(21, 3, "收件箱", "INBOX", &inbox_messages),
            make_folder(22, 3, "已发送", "Sent", &sent_messages),
            make_folder(23, 3, "草稿箱", "Drafts", &[]),
            make_folder(24, 3, "归档", "Archive", &archive_messages),
            make_folder(25, 3, "垃圾邮件", "Junk", &[]),
            make_folder(26, 3, "已删除", "Trash", &[]),
        ];

        Self {
            accounts,
            folders,
            inbox_messages,
            sent_messages,
            draft_messages,
            archive_messages,
            junk_messages,
        }
    }

    pub fn all_inbox_messages(&self) -> &[MessageSummary] {
        &self.inbox_messages
    }

    pub fn all_sent_messages(&self) -> &[MessageSummary] {
        &self.sent_messages
    }

    pub fn all_draft_messages(&self) -> &[MessageSummary] {
        &self.draft_messages
    }

    pub fn all_archive_messages(&self) -> &[MessageSummary] {
        &self.archive_messages
    }

    pub fn all_junk_messages(&self) -> &[MessageSummary] {
        &self.junk_messages
    }

    fn messages_for_folder(&self, folder_id: u32) -> Vec<MessageSummary> {
        self.inbox_messages
            .iter()
            .chain(self.sent_messages.iter())
            .chain(self.draft_messages.iter())
            .chain(self.archive_messages.iter())
            .chain(self.junk_messages.iter())
            .filter(|m| m.folder_id == folder_id)
            .cloned()
            .collect()
    }

    /// Return a slice of messages in `folder_id` starting at `offset` with at most `limit` rows.
    pub fn list_messages_page(
        &self,
        folder_id: u32,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> MailPage {
        self.list_messages_page_filtered(folder_id, query, MessageFilter::All, offset, limit)
    }

    /// Return a slice of messages in `folder_id` applying both `query` and `filter`.
    pub fn list_messages_page_filtered(
        &self,
        folder_id: u32,
        query: &str,
        filter: MessageFilter,
        offset: usize,
        limit: usize,
    ) -> MailPage {
        self.list_messages_page_with_options(
            folder_id,
            query,
            QuickFilterSet::from_message_filter(filter),
            MessageSort::DateNewest,
            offset,
            limit,
        )
    }

    /// Return a slice of messages in `folder_id` with structured query, quick filters, and sort.
    pub fn list_messages_page_with_options(
        &self,
        folder_id: u32,
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> MailPage {
        let all = self.messages_for_folder(folder_id);
        let parsed = SearchQuery::parse(query);
        let folder_name = self.folder_name_for(folder_id);
        let today_start = start_of_today_ts();
        let mut filtered: Vec<MessageSummary> = all
            .into_iter()
            .filter(|m| Self::matches_quick_filters(m, quick_filters, today_start))
            .filter(|m| parsed.is_empty() || parsed.matches_summary(m, folder_name, ME, None))
            .collect();

        Self::sort_summaries(&mut filtered, sort);

        let total = filtered.len();
        let start = offset.min(total);
        let end = start.saturating_add(limit).min(total);
        MailPage {
            folder_id,
            offset: start,
            total,
            items: filtered[start..end].to_vec(),
        }
    }

    /// Return a page of unread demo messages with structured query, quick filters, and sort.
    pub fn list_unread_page_with_options(
        &self,
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> MailPage {
        let parsed = SearchQuery::parse(query);
        let today_start = start_of_today_ts();
        let mut filtered: Vec<MessageSummary> = self
            .inbox_messages
            .iter()
            .chain(self.sent_messages.iter())
            .chain(self.archive_messages.iter())
            .filter(|m| m.unread && Self::matches_quick_filters(m, quick_filters, today_start))
            .filter(|m| {
                parsed.is_empty()
                    || parsed.matches_summary(m, self.folder_name_for(m.folder_id), ME, None)
            })
            .cloned()
            .collect();

        Self::sort_summaries(&mut filtered, sort);

        let total = filtered.len();
        let start = offset.min(total);
        let end = start.saturating_add(limit).min(total);
        MailPage {
            folder_id: 0,
            offset: start,
            total,
            items: filtered[start..end].to_vec(),
        }
    }

    /// Return a page of starred demo messages across all demo folders.
    pub fn list_starred_page(
        &self,
        query: &str,
        filter: MessageFilter,
        offset: usize,
        limit: usize,
    ) -> MailPage {
        self.list_starred_page_with_options(
            query,
            QuickFilterSet::from_message_filter(filter),
            MessageSort::DateNewest,
            offset,
            limit,
        )
    }

    /// Return a page of starred demo messages with structured query, quick filters, and sort.
    pub fn list_starred_page_with_options(
        &self,
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> MailPage {
        let parsed = SearchQuery::parse(query);
        let today_start = start_of_today_ts();
        let mut filtered: Vec<MessageSummary> = self
            .inbox_messages
            .iter()
            .chain(self.sent_messages.iter())
            .chain(self.archive_messages.iter())
            .filter(|m| m.flagged && Self::matches_quick_filters(m, quick_filters, today_start))
            .filter(|m| {
                parsed.is_empty()
                    || parsed.matches_summary(m, self.folder_name_for(m.folder_id), ME, None)
            })
            .cloned()
            .collect();

        Self::sort_summaries(&mut filtered, sort);

        let total = filtered.len();
        let start = offset.min(total);
        let end = start.saturating_add(limit).min(total);
        MailPage {
            folder_id: 0,
            offset: start,
            total,
            items: filtered[start..end].to_vec(),
        }
    }

    /// Cross-folder search across all demo folders.
    pub fn search_all_folders(
        &self,
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> MailPage {
        let parsed = SearchQuery::parse(query);
        let today_start = start_of_today_ts();
        let mut filtered: Vec<MessageSummary> = self
            .inbox_messages
            .iter()
            .chain(self.sent_messages.iter())
            .chain(self.archive_messages.iter())
            .filter(|m| Self::matches_quick_filters(m, quick_filters, today_start))
            .filter(|m| {
                parsed.is_empty()
                    || parsed.matches_summary(m, self.folder_name_for(m.folder_id), ME, None)
            })
            .cloned()
            .collect();

        Self::sort_summaries(&mut filtered, sort);

        let total = filtered.len();
        let start = offset.min(total);
        let end = start.saturating_add(limit).min(total);
        MailPage {
            folder_id: 0,
            offset: start,
            total,
            items: filtered[start..end].to_vec(),
        }
    }

    fn folder_name_for(&self, folder_id: u32) -> &str {
        self.folders
            .iter()
            .find(|f| f.id == folder_id)
            .map(|f| f.name.as_str())
            .unwrap_or("")
    }

    fn matches_quick_filters(
        m: &MessageSummary,
        filters: QuickFilterSet,
        today_start_ts: i64,
    ) -> bool {
        let sender_lower = m.sender.to_lowercase();
        let recip_lower = m.recipients.to_lowercase();
        (!filters.unread || m.unread)
            && (!filters.starred || m.flagged)
            && (!filters.has_attachment || m.has_attachment)
            && (!filters.today || m.date_ts >= today_start_ts)
            && (!filters.to_me
                || recip_lower.contains(ME)
                || recip_lower.contains(LAB)
                || recip_lower.contains(PERSONAL))
            && (!filters.from_contacts
                || sender_lower.contains("sophie")
                || sender_lower.contains("marcus")
                || sender_lower.contains("priya"))
            && (!filters.high_importance || m.importance > 0)
            && (!filters.tagged || !m.tags.is_empty())
            && filters.date_from_ts.is_none_or(|from| m.date_ts >= from)
            && filters.date_to_ts.is_none_or(|to| m.date_ts < to)
    }

    fn sort_summaries(items: &mut [MessageSummary], sort: MessageSort) {
        match sort {
            MessageSort::DateNewest => {
                items.sort_by(|a, b| b.date_ts.cmp(&a.date_ts).then_with(|| b.uid.cmp(&a.uid)));
            }
            MessageSort::DateOldest => {
                items.sort_by(|a, b| a.date_ts.cmp(&b.date_ts).then_with(|| a.uid.cmp(&b.uid)));
            }
            MessageSort::SenderAsc => {
                items.sort_by(|a, b| {
                    a.sender
                        .to_lowercase()
                        .cmp(&b.sender.to_lowercase())
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
            MessageSort::SenderDesc => {
                items.sort_by(|a, b| {
                    b.sender
                        .to_lowercase()
                        .cmp(&a.sender.to_lowercase())
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
            MessageSort::SubjectAsc => {
                items.sort_by(|a, b| {
                    a.subject
                        .to_lowercase()
                        .cmp(&b.subject.to_lowercase())
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
            MessageSort::SubjectDesc => {
                items.sort_by(|a, b| {
                    b.subject
                        .to_lowercase()
                        .cmp(&a.subject.to_lowercase())
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
            MessageSort::UnreadFirst => {
                items.sort_by(|a, b| {
                    b.unread
                        .cmp(&a.unread)
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
            MessageSort::StarredFirst => {
                items.sort_by(|a, b| {
                    b.flagged
                        .cmp(&a.flagged)
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
            MessageSort::AccountAsc => {
                items.sort_by(|a, b| {
                    a.account_id
                        .cmp(&b.account_id)
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
            MessageSort::AccountDesc => {
                items.sort_by(|a, b| {
                    b.account_id
                        .cmp(&a.account_id)
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
            MessageSort::AttachmentFirst => {
                items.sort_by(|a, b| {
                    b.has_attachment
                        .cmp(&a.has_attachment)
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
            MessageSort::ImportanceFirst => {
                items.sort_by(|a, b| {
                    b.importance
                        .cmp(&a.importance)
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
            MessageSort::TagsAsc => {
                items.sort_by(|a, b| {
                    a.tags
                        .join(",")
                        .to_lowercase()
                        .cmp(&b.tags.join(",").to_lowercase())
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
        }
    }

    /// Return thread child summaries for inline expansion in demo mode.
    pub fn list_thread_summaries(&self, head_message_id: u32) -> Vec<MessageSummary> {
        let Some(detail) = self.get_message_detail(head_message_id) else {
            return Vec::new();
        };
        if detail.thread_messages.len() <= 1 {
            return Vec::new();
        }
        detail
            .thread_messages
            .iter()
            .filter(|tm| tm.id != head_message_id)
            .map(|tm| MessageSummary {
                id: tm.id,
                account_id: detail.account_id,
                folder_id: detail.folder_id,
                folder_path: if tm.folder_path.is_empty() {
                    detail.folder_path.clone()
                } else {
                    tm.folder_path.clone()
                },
                uid: tm.uid,
                sender: tm.sender.clone(),
                subject: if tm.subject.is_empty() {
                    detail.subject.clone()
                } else {
                    tm.subject.clone()
                },
                preview: tm.body_text.lines().next().unwrap_or("").trim().to_string(),
                date_ts: tm.date_ts,
                date_display: tm.date_display.clone(),
                unread: tm.unread,
                flagged: tm.flagged,
                has_attachment: !tm.attachments.is_empty(),
                recipients: tm.recipients.clone(),
                thread_count: 1,
                importance: 0,
                tags: Vec::new(),
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            })
            .collect()
    }

    /// Update tags on a demo message.
    pub fn set_message_tags(&mut self, message_id: u32, tags: Vec<String>) {
        for list in [
            &mut self.inbox_messages,
            &mut self.sent_messages,
            &mut self.draft_messages,
            &mut self.archive_messages,
            &mut self.junk_messages,
        ] {
            if let Some(m) = list.iter_mut().find(|m| m.id == message_id) {
                m.tags = tags;
                return;
            }
        }
    }

    /// Snooze or unsnooze a demo message.
    pub fn snooze_message(&mut self, message_id: u32, until_ts: i64) {
        for list in [
            &mut self.inbox_messages,
            &mut self.sent_messages,
            &mut self.draft_messages,
            &mut self.archive_messages,
            &mut self.junk_messages,
        ] {
            if let Some(m) = list.iter_mut().find(|m| m.id == message_id) {
                m.snoozed_until_ts = until_ts;
                return;
            }
        }
    }

    fn attachments_for_message(summary: &MessageSummary) -> Vec<AttachmentMeta> {
        match summary.id {
            8 => vec![
                AttachmentMeta {
                    id: 801,
                    filename: "Q3-roadmap-draft.pdf".to_string(),
                    mime_type: "application/pdf".to_string(),
                    size_bytes: 24_576,
                    downloaded: true,
                },
                AttachmentMeta {
                    id: 802,
                    filename: "migration-timeline.pdf".to_string(),
                    mime_type: "application/pdf".to_string(),
                    size_bytes: 9_830,
                    downloaded: true,
                },
                AttachmentMeta {
                    id: 803,
                    filename: "notes.txt".to_string(),
                    mime_type: "text/plain".to_string(),
                    size_bytes: 53,
                    downloaded: true,
                },
            ],
            1 => vec![
                AttachmentMeta {
                    id: 101,
                    filename: "三栏布局截图.png".to_string(),
                    mime_type: "image/png".to_string(),
                    size_bytes: 188_416,
                    downloaded: true,
                },
                AttachmentMeta {
                    id: 102,
                    filename: "windows-baseline-notes.txt".to_string(),
                    mime_type: "text/plain".to_string(),
                    size_bytes: 12_288,
                    downloaded: true,
                },
            ],
            11 => vec![AttachmentMeta {
                id: 1101,
                filename: "apple-receipt-W1234567890.pdf".to_string(),
                mime_type: "application/pdf".to_string(),
                size_bytes: 42_110,
                downloaded: true,
            }],
            13 => vec![AttachmentMeta {
                id: 1301,
                filename: "invite.ics".to_string(),
                mime_type: "text/calendar".to_string(),
                size_bytes: 780,
                downloaded: true,
            }],
            43 => vec![AttachmentMeta {
                id: 4301,
                filename: "tune-up-invoice.pdf".to_string(),
                mime_type: "application/pdf".to_string(),
                size_bytes: 18_432,
                downloaded: true,
            }],
            _ if summary.has_attachment => vec![AttachmentMeta {
                id: summary.id * 10 + 1,
                filename: "attachment.txt".to_string(),
                mime_type: "text/plain".to_string(),
                size_bytes: 4_096,
                downloaded: true,
            }],
            _ => Vec::new(),
        }
    }

    /// Return cached attachments for the gallery view in `--demo` mode.
    pub fn list_cached_attachments(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Vec<CachedAttachment> {
        let q = query.trim().to_lowercase();
        self.inbox_messages
            .iter()
            .chain(self.archive_messages.iter())
            .filter(|m| m.has_attachment)
            .flat_map(|m| {
                Self::attachments_for_message(m)
                    .into_iter()
                    .map(move |att| CachedAttachment {
                        id: att.id,
                        message_id: m.id,
                        account_id: m.account_id,
                        filename: att.filename,
                        mime_type: att.mime_type,
                        size_bytes: att.size_bytes,
                        sender: m.sender.clone(),
                        subject: m.subject.clone(),
                        date_display: m.date_display.clone(),
                        date_ts: m.date_ts,
                        downloaded: att.downloaded,
                    })
            })
            .filter(|att| {
                q.is_empty()
                    || att.filename.to_lowercase().contains(&q)
                    || att.sender.to_lowercase().contains(&q)
                    || att.subject.to_lowercase().contains(&q)
            })
            .skip(offset)
            .take(limit)
            .collect()
    }

    /// Return aggregated contacts for the contacts view in `--demo` mode.
    pub fn list_known_contacts(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Vec<KnownContact> {
        let q = query.trim().to_lowercase();
        let mut counts: BTreeMap<String, (String, usize)> = BTreeMap::new();
        for msg in self
            .inbox_messages
            .iter()
            .chain(self.sent_messages.iter())
            .take(400)
        {
            let (name, email) =
                if let (Some(o), Some(c)) = (msg.sender.find('<'), msg.sender.rfind('>')) {
                    (
                        msg.sender[..o].trim().to_string(),
                        msg.sender[o + 1..c].trim().to_lowercase(),
                    )
                } else {
                    (msg.sender.clone(), msg.sender.to_lowercase())
                };
            if email.is_empty() {
                continue;
            }
            let entry = counts.entry(email).or_insert_with(|| (name, 0));
            entry.1 += 1;
        }
        let mut items: Vec<KnownContact> = counts
            .into_iter()
            .map(|(email, (name, message_count))| {
                let (id, notes, custom) = if email == "sophie@studio.dev" {
                    (1u32, "产品设计与路线图负责人（演示）".to_string(), true)
                } else {
                    (0u32, String::new(), false)
                };
                KnownContact {
                    name,
                    email,
                    message_count,
                    id,
                    notes,
                    custom,
                    ..KnownContact::default()
                }
            })
            .filter(|c| {
                q.is_empty()
                    || c.name.to_lowercase().contains(&q)
                    || c.email.to_lowercase().contains(&q)
            })
            .collect();
        items.sort_by(|a, b| {
            b.custom
                .cmp(&a.custom)
                .then_with(|| b.message_count.cmp(&a.message_count))
                .then_with(|| a.name.cmp(&b.name))
        });
        items.into_iter().skip(offset).take(limit).collect()
    }

    /// Return full message details for `message_id`.
    pub fn get_message_detail(&self, message_id: u32) -> Option<MessageDetail> {
        let Some(summary) = self
            .inbox_messages
            .iter()
            .chain(self.sent_messages.iter())
            .chain(self.draft_messages.iter())
            .chain(self.archive_messages.iter())
            .chain(self.junk_messages.iter())
            .find(|m| m.id == message_id)
        else {
            // Also resolve thread child messages when opened in a standalone window.
            for head_id in [1u32, 8u32] {
                if let Some(head) = self.get_message_detail(head_id) {
                    if let Some(child) = head.thread_messages.iter().find(|t| t.id == message_id) {
                        return Some(MessageDetail {
                            id: child.id,
                            account_id: head.account_id,
                            folder_id: head.folder_id,
                            folder_path: head.folder_path.clone(),
                            uid: child.uid,
                            rfc_message_id: child.rfc_message_id.clone(),
                            sender: child.sender.clone(),
                            recipients: child.recipients.clone(),
                            cc: child.cc.clone(),
                            subject: child.subject.clone(),
                            date_ts: child.date_ts,
                            date_display: child.date_display.clone(),
                            body_text: child.body_text.clone(),
                            body_html: child.body_html.clone(),
                            flagged: child.flagged,
                            attachments: child.attachments.clone(),
                            thread_messages: Vec::new(),
                            unsubscribe_url: None,
                            unsubscribe_one_click: false,
                            auth_results: None,
                        });
                    }
                }
            }
            return None;
        };

        let attachments = Self::attachments_for_message(summary);
        let folder_path = summary.folder_path.clone();

        let (body_text, body_html, thread_messages, unsubscribe_url, unsubscribe_one_click, auth_results) =
            match summary.id {
                // Q3 roadmap review: 5 conversation cards + 3 attachments (Q3-roadmap-draft.pdf, migration-timeline.pdf, notes.txt)
                8 => {
                    let sophie_draft_att = vec![attachments[0].clone()];
                    let marcus_atts = vec![attachments[1].clone(), attachments[2].clone()];
                    let cards = vec![
                        ThreadMessage {
                            id: 23,
                            rfc_message_id: "<demo-23@hylki.local>".to_string(),
                            subject: "Q3 roadmap review".to_string(),
                            sender: "Sophie Turner <sophie@studio.dev>".to_string(),
                            recipients: ME.to_string(),
                            cc: String::new(),
                            date_ts: DEMO_BASE_TS - 82_800,
                            date_display: "Oct 8, 2025 at 5:53 AM".to_string(),
                            body_text: "Hi Jason,\n\nSharing the draft roadmap ahead of Thursday. The migration phase is the big open question — see the timeline in the doc and let me know if the sequencing works.\n\nThanks,\nSophie".to_string(),
                            body_html: None,
                            body_loaded: true,
                            unread: false,
                            flagged: false,
                            attachments: sophie_draft_att,
                            uid: 23,
                            folder_path: folder_path.clone(),
                        },
                        ThreadMessage {
                            id: 22,
                            rfc_message_id: "<demo-22@hylki.local>".to_string(),
                            subject: "Re: Q3 roadmap review".to_string(),
                            sender: format!("Jason M. <{ME}>"),
                            recipients: "Sophie Turner <sophie@studio.dev>".to_string(),
                            cc: String::new(),
                            date_ts: DEMO_BASE_TS - 79_200,
                            date_display: "Oct 8, 2025 at 6:53 AM".to_string(),
                            body_text: "Looks great overall. I left comments on the migration phase — I think we can parallelize the first two steps and pull the whole thing in by a week. Happy to walk through it tomorrow at 10.\n\nJason".to_string(),
                            body_html: None,
                            body_loaded: true,
                            unread: false,
                            flagged: false,
                            attachments: Vec::new(),
                            uid: 22,
                            folder_path: folder_path.clone(),
                        },
                        ThreadMessage {
                            id: 21,
                            rfc_message_id: "<demo-21@hylki.local>".to_string(),
                            subject: "Re: Q3 roadmap review".to_string(),
                            sender: "Marcus Chen <marcus@studio.dev>".to_string(),
                            recipients: ME.to_string(),
                            cc: String::new(),
                            date_ts: DEMO_BASE_TS - 75_600,
                            date_display: "Oct 8, 2025 at 7:53 AM".to_string(),
                            body_text: "+1 on parallelising. I mocked up the timeline both ways (attached). The compressed version lands the migration on the 14th with a week of slack before the release freeze.\n\nMarcus".to_string(),
                            body_html: None,
                            body_loaded: true,
                            unread: false,
                            flagged: false,
                            attachments: marcus_atts,
                            uid: 21,
                            folder_path: folder_path.clone(),
                        },
                        ThreadMessage {
                            id: 9,
                            rfc_message_id: "<demo-9@hylki.local>".to_string(),
                            subject: "Re: Q3 roadmap review".to_string(),
                            sender: "Priya Sharma <priya@studio.dev>".to_string(),
                            recipients: ME.to_string(),
                            cc: String::new(),
                            date_ts: DEMO_BASE_TS - 28_800,
                            date_display: "Oct 8, 2025 at 7:53 PM".to_string(),
                            body_text: "One thing from QA: if steps one and two run in parallel we need the staging environment a week earlier than the doc says. I've asked Marcus whether ops can do it.\n\nOtherwise the sequencing looks right to me.\n\nPriya".to_string(),
                            body_html: None,
                            body_loaded: true,
                            unread: false,
                            flagged: false,
                            attachments: Vec::new(),
                            uid: 9,
                            folder_path: folder_path.clone(),
                        },
                        ThreadMessage {
                            id: 8,
                            rfc_message_id: "<demo-8@hylki.local>".to_string(),
                            subject: "Re: Q3 roadmap review".to_string(),
                            sender: "Sophie Turner <sophie@studio.dev>".to_string(),
                            recipients: ME.to_string(),
                            cc: String::new(),
                            date_ts: summary.date_ts,
                            date_display: "Oct 8, 2025 at 8:53 PM".to_string(),
                            body_text: "Perfect, updated the doc with the parallel track and Priya's staging step. Thursday's session is now a sign-off rather than a review, so I've cut it to 30 minutes.\n\nLast call for comments before then!\n\nSophie".to_string(),
                            body_html: None,
                            body_loaded: true,
                            unread: summary.unread,
                            flagged: summary.flagged,
                            attachments: Vec::new(),
                            uid: 8,
                            folder_path: folder_path.clone(),
                        },
                    ];
                    (
                        cards.last().map(|c| c.body_text.clone()).unwrap_or_default(),
                        None,
                        cards,
                        None,
                        false,
                        None,
                    )
                }
                // Reader redesign: final review (6 conversation cards)
                1 => {
                    let latest_html = "<p>Hover-reveal gets my vote — it keeps the cards calm and the fade is lovely. Ship it 🚀</p><p>Marcus <img src=\"https://tracker.hylki.example/pixel.png\" width=\"1\" height=\"1\" alt=\"\"></p>".to_string();
                    let cards = vec![
                        ThreadMessage {
                            id: 6,
                            rfc_message_id: "<demo-6@hylki.local>".to_string(),
                            subject: "Reader redesign: final review".to_string(),
                            sender: "Sophie Turner <sophie@studio.dev>".to_string(),
                            recipients: ME.to_string(),
                            cc: String::new(),
                            date_ts: DEMO_BASE_TS - 18_000,
                            date_display: "Oct 9, 2025 at 8:12 AM".to_string(),
                            body_text: "Hi all,\n\nBefore we tag the release I'd like one last pass over the reading pane. I've collected everything into the review board:\n• Full-width conversation layout\n• Card action palettes\n• The new accent selection in the list\n\nComments by Thursday, please!\n\nSophie".to_string(),
                            body_html: Some("<p>Hi all,</p><p>Before we tag the release I'd like one last pass over the reading pane. I've collected everything into <a href=\"https://example.com/review-board\">the review board</a>:</p><ul><li>Full-width conversation layout</li><li>Card action palettes</li><li>The new accent selection in the list</li></ul><p>Comments by Thursday, please!</p><p>Sophie</p>".to_string()),
                            body_loaded: true,
                            unread: false,
                            flagged: false,
                            attachments: Vec::new(),
                            uid: 6,
                            folder_path: folder_path.clone(),
                        },
                        ThreadMessage {
                            id: 5,
                            rfc_message_id: "<demo-5@hylki.local>".to_string(),
                            subject: "Re: Reader redesign: final review".to_string(),
                            sender: "Marcus Chen <marcus@studio.dev>".to_string(),
                            recipients: ME.to_string(),
                            cc: String::new(),
                            date_ts: DEMO_BASE_TS - 14_400,
                            date_display: "Oct 9, 2025 at 8:31 AM".to_string(),
                            body_text: "Went through the board this morning. The full-width layout is a clear win — one note on plain-text mail: the header should sit on the same ground as the body so it reads as one surface.\n\nMarcus".to_string(),
                            body_html: None,
                            body_loaded: true,
                            unread: false,
                            flagged: false,
                            attachments: Vec::new(),
                            uid: 5,
                            folder_path: folder_path.clone(),
                        },
                        ThreadMessage {
                            id: 4,
                            rfc_message_id: "<demo-4@hylki.local>".to_string(),
                            subject: "Re: Reader redesign: final review".to_string(),
                            sender: format!("Jason M. <{ME}>"),
                            recipients: "Sophie Turner <sophie@studio.dev>".to_string(),
                            cc: String::new(),
                            date_ts: DEMO_BASE_TS - 10_800,
                            date_display: "Oct 9, 2025 at 8:56 AM".to_string(),
                            body_text: "Agreed on the continuous surface for plain mail — that's in.\n\nI also merged the thread chip work so the count and caret are one quiet pill now, and selection uses the system accent with white text in both schemes.\n\nJason".to_string(),
                            body_html: None,
                            body_loaded: true,
                            unread: false,
                            flagged: false,
                            attachments: Vec::new(),
                            uid: 4,
                            folder_path: folder_path.clone(),
                        },
                        ThreadMessage {
                            id: 3,
                            rfc_message_id: "<demo-3@hylki.local>".to_string(),
                            subject: "Re: Reader redesign: final review".to_string(),
                            sender: "Priya Sharma <priya@studio.dev>".to_string(),
                            recipients: ME.to_string(),
                            cc: String::new(),
                            date_ts: DEMO_BASE_TS - 7_200,
                            date_display: "Oct 9, 2025 at 9:20 AM".to_string(),
                            body_text: "Tested on a 13\" laptop — the narrower message list is great and nothing overflows.\n\nAttaching two screens of the hover palette so we can compare the reserved-space fade against the old layout shift.\n\nPriya".to_string(),
                            body_html: None,
                            body_loaded: true,
                            unread: false,
                            flagged: false,
                            attachments: attachments.clone(),
                            uid: 3,
                            folder_path: folder_path.clone(),
                        },
                        ThreadMessage {
                            id: 2,
                            rfc_message_id: "<demo-2@hylki.local>".to_string(),
                            subject: "Re: Reader redesign: final review".to_string(),
                            sender: "Sophie Turner <sophie@studio.dev>".to_string(),
                            recipients: ME.to_string(),
                            cc: String::new(),
                            date_ts: DEMO_BASE_TS - 3_600,
                            date_display: "Oct 9, 2025 at 9:41 AM".to_string(),
                            body_text: "Love it. Last open question from me: should the card actions default to hover-reveal or always visible? Preferences has all three modes, we just need a default for new installs.\n\nSophie".to_string(),
                            body_html: None,
                            body_loaded: true,
                            unread: false,
                            flagged: false,
                            attachments: Vec::new(),
                            uid: 2,
                            folder_path: folder_path.clone(),
                        },
                        ThreadMessage {
                            id: 1,
                            rfc_message_id: "<demo-1@hylki.example>".to_string(),
                            subject: "Re: Reader redesign: final review".to_string(),
                            sender: summary.sender.clone(),
                            recipients: ME.to_string(),
                            cc: String::new(),
                            date_ts: summary.date_ts,
                            date_display: summary.date_display.clone(),
                            body_text: "Hover-reveal gets my vote — it keeps the cards calm and the fade is lovely. Ship it 🚀\n\nMarcus".to_string(),
                            body_html: Some(latest_html.clone()),
                            body_loaded: true,
                            unread: summary.unread,
                            flagged: summary.flagged,
                            attachments: Vec::new(),
                            uid: 1,
                            folder_path: folder_path.clone(),
                        },
                    ];
                    (
                        cards.last().map(|c| c.body_text.clone()).unwrap_or_default(),
                        Some(latest_html),
                        cards,
                        None,
                        false,
                        None,
                    )
                }
                7 => (
                    "Hi Jason,\n\nThe release candidate for GNOME 49 is now available for testing. This cycle brings major performance work across the shell and a refreshed libadwaita with new adaptive widgets.\n\nHighlights:\n  • Faster startup and lower memory use\n  • New AdwMultiLayoutView for responsive layouts\n  • Improved Wayland fractional scaling\n\nPlease help us test and file issues before the final release.\n\n— The GNOME Release Team\n\n---\nYou are receiving this because you signed up for GNOME news.\nTo unsubscribe, visit https://mail.gnome.org/list/news/unsubscribe/j7Kq2".to_string(),
                    None,
                    Vec::new(),
                    None,
                    false,
                    Some("mx.hylki.local; dkim=pass header.d=gnome.org; spf=pass smtp.mailfrom=gnome.org; dmarc=pass header.from=gnome.org".to_string()),
                ),
                10 => {
                    let html = "<html><head><style>body{margin:0;background:#f4f1ea;font-family:Georgia,serif}.btn{background:#c0392b;color:#fff!important;padding:10px 18px;border-radius:4px;text-decoration:none;display:inline-block}</style></head>\
                   <body><div style=\"display:none;max-height:0;overflow:hidden;font-size:1px;color:#f4f1ea\">Async closures land on stable, zero-copy parsing, 14 crates \u{200b}\u{200b}\u{200b}</div>\
                   <table width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" bgcolor=\"#f4f1ea\"><tr><td align=\"center\" style=\"padding:24px 12px\">\
                   <table width=\"560\" cellpadding=\"0\" cellspacing=\"0\" bgcolor=\"#ffffff\" style=\"border:1px solid #e0d9c8\">\
                   <tr><td bgcolor=\"#2c3e50\" style=\"padding:22px 28px;color:#ecf0f1;font-size:26px;font-weight:bold;letter-spacing:1px\">This Week in Rust <span style=\"color:#f39c12\">#612</span></td></tr>\
                   <tr><td style=\"padding:24px 28px 8px;font-size:15px;line-height:1.5;color:#333\"><p style=\"margin:0 0 14px\">Welcome to another issue of <b>This Week in Rust</b>!</p>\
                   <p style=\"margin:0 0 14px\">This week: async closures stabilize on stable, a deep dive into zero-copy parsing, and 14 new crates worth your attention.</p></td></tr>\
                   <tr><td style=\"padding:0 28px\"><table cellpadding=\"0\" cellspacing=\"0\" width=\"100%\"><tr>\
                   <td width=\"50%\" valign=\"top\" style=\"padding:12px;background:#fdf6e3;font-size:14px;color:#333\"><h3 style=\"margin:0 0 6px;color:#c0392b;font-size:16px\">Crate of the week</h3>zerocopy-parse: parse without copying a byte. <a href=\"https://example.com/crate\" style=\"color:#c0392b\">Read more</a></td>\
                   <td width=\"50%\" valign=\"top\" style=\"padding:12px;background:#eef5fb;font-size:14px;color:#333\"><h3 style=\"margin:0 0 6px;color:#2980b9;font-size:16px\">RFCs</h3>Three RFCs entered final comment period this week, including the one on <i>async closures</i>.</td>\
                   </tr></table></td></tr>\
                   <tr><td style=\"padding:20px 28px;font-size:15px;color:#333\"><table cellpadding=\"0\" cellspacing=\"0\"><tr><th align=\"left\" style=\"padding:4px 12px 4px 0;border-bottom:2px solid #2c3e50\">Crate</th><th align=\"left\" style=\"padding:4px 0;border-bottom:2px solid #2c3e50\">Downloads</th></tr>\
                   <tr><td style=\"padding:4px 12px 4px 0\">zerocopy-parse</td><td style=\"padding:4px 0\">41,200</td></tr><tr><td style=\"padding:4px 12px 4px 0\">tokio-tap</td><td style=\"padding:4px 0\">12,844</td></tr></table></td></tr>\
                   <tr><td align=\"center\" style=\"padding:8px 28px 28px\"><a class=\"btn\" href=\"https://example.com/digest/612\">Read the full digest</a></td></tr>\
                   <tr><td bgcolor=\"#ecf0f1\" style=\"padding:14px 28px;font-size:12px;color:#7f8c8d\">You are receiving this because you subscribed. <a href=\"https://example.com/unsubscribe\" style=\"color:#7f8c8d\">Unsubscribe</a></td></tr>\
                   </table></td></tr></table><img src=\"https://example.com/open.gif\" width=\"1\" height=\"1\" alt=\"\"></body></html>".to_string();
                    (
                        "Welcome to another issue of This Week in Rust!\n\nThis week: async closures stabilize on stable, a deep dive into zero-copy parsing, and 14 new crates worth your attention.".to_string(),
                        Some(html),
                        Vec::new(),
                        Some("https://this-week-in-rust.org/unsubscribe/612/abc".to_string()),
                        true,
                        Some("mx.hylki.local; dkim=pass header.d=this-week-in-rust.org; spf=pass smtp.mailfrom=this-week-in-rust.org; dmarc=pass header.from=this-week-in-rust.org".to_string()),
                    )
                }
                11 => (
                    "Thank you for your purchase.\n\nOrder ID: W1234567890\niCloud+ 2TB — $9.99\n\nYour receipt is attached.".to_string(),
                    None,
                    Vec::new(),
                    None,
                    false,
                    None,
                ),
                12 => (
                    "Hey,\n\nThe design token pipeline is finally merged into main. Dark mode now derives entirely from the token set, so we no longer maintain two stylesheets. Pull main when you get a chance.\n\nMarcus".to_string(),
                    None,
                    Vec::new(),
                    None,
                    false,
                    None,
                ),
                13 => (
                    "You have been invited to: Architecture sync\n\nWhen: Thursday 2:00 PM – 3:00 PM\nWhere: Conference Room B / video link\nOrganiser: Priya Sharma\n\nAccept · Decline · Maybe".to_string(),
                    None,
                    Vec::new(),
                    None,
                    false,
                    None,
                ),
                14 => (
                    "Hey stranger!\n\nIt's been ages! Are you free Saturday for lunch at that new place downtown? Let me know what works.\n\nxo Emma".to_string(),
                    None,
                    Vec::new(),
                    None,
                    false,
                    None,
                ),
                15 => (
                    "You have 3 issues in the current sprint:\n\n  • VIREO-142  Reader dark mode\n  • VIREO-148  Infinite scroll spinner\n  • VIREO-151  OAuth for Microsoft\n\nOpen in Linear to update status.".to_string(),
                    None,
                    Vec::new(),
                    None,
                    false,
                    None,
                ),
                16 => (
                    "hylki.hyprlab.co — weekly summary\n\nVisitors: 4,218 (+32%)\nTop page: /download\nAvg. time on page: 1m 47s\n\nView the full report online.".to_string(),
                    None,
                    Vec::new(),
                    None,
                    false,
                    None,
                ),
                30 => (
                    "We noticed a new sign-in to your Proton account.\n\nDevice: Fedora Linux · Firefox\nLocation: —\n\nIf this was you, no action is needed.".to_string(),
                    None,
                    Vec::new(),
                    None,
                    false,
                    None,
                ),
                31 => (
                    "Good news!\n\nAlex bought you a coffee and left a note:\n\n  “Love Hylki — the GNOME-native mail client I've wanted for years!”\n\nSay thanks from your dashboard.".to_string(),
                    None,
                    Vec::new(),
                    None,
                    false,
                    None,
                ),
                32 => (
                    "hyprlab/hylki just reached 1,000 stars 🎉\n\nRecent stargazers: @ada, @torvalds-fan, @rustacean…\n\nView on GitHub.".to_string(),
                    None,
                    Vec::new(),
                    None,
                    false,
                    None,
                ),
                40 => (
                    "Hey!\n\nLocking the booking tonight! Are you and Sam still in for the 19th? We're at nine people if so, which means the bigger cabin.\n\nBring the good coffee ☕\n\nElena".to_string(),
                    None,
                    Vec::new(),
                    None,
                    false,
                    None,
                ),
                _ => (
                    format!("{}\n\n{}\n", summary.subject, summary.preview),
                    None,
                    Vec::new(),
                    None,
                    false,
                    None,
                ),
            };

        Some(MessageDetail {
            id: summary.id,
            account_id: summary.account_id,
            folder_id: summary.folder_id,
            folder_path,
            uid: summary.uid,
            rfc_message_id: format!("<demo-{}@hylki.example>", summary.id),
            sender: summary.sender.clone(),
            recipients: if summary.recipients.is_empty() {
                ME.to_string()
            } else {
                summary.recipients.clone()
            },
            cc: String::new(),
            subject: summary.subject.clone(),
            date_ts: summary.date_ts,
            date_display: summary.date_display.clone(),
            body_text,
            body_html,
            flagged: summary.flagged,
            attachments,
            thread_messages,
            unsubscribe_url,
            unsubscribe_one_click,
            auth_results,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_store_has_three_accounts_tags_older_dates_and_q3_thread() {
        let store = DemoStore::new();
        assert_eq!(store.accounts.len(), 3);
        assert_eq!(store.accounts[0].display_name, "Jason M.");
        assert_eq!(store.accounts[1].display_name, "Hyprlab");
        assert_eq!(store.accounts[2].display_name, "Jason (Personal)");

        let page0 = store.list_messages_page(DEMO_INBOX_ID, "", 0, 50);
        assert_eq!(page0.total, 10);
        assert_eq!(page0.items[0].id, 1);
        assert!(page0.items[0].tags.contains(&"Work".to_string()));
        assert!(page0
            .items
            .iter()
            .any(|m| m.tags.contains(&"Personal".to_string())));

        // All curated timestamps sit in Oct 2025 so date grouping yields Older (`更早`).
        let now_ts = chrono::Utc::now().timestamp();
        assert!(page0
            .items
            .iter()
            .all(|m| crate::date_group_label(m.date_ts, now_ts) == "更早"));

        let q3 = store
            .get_message_detail(8)
            .expect("Q3 roadmap review thread should exist");
        assert_eq!(q3.subject, "Q3 roadmap review");
        assert_eq!(q3.thread_messages.len(), 5);
        assert_eq!(q3.attachments.len(), 3);
        assert_eq!(q3.attachments[0].filename, "Q3-roadmap-draft.pdf");
        assert_eq!(q3.attachments[1].filename, "migration-timeline.pdf");
        assert_eq!(q3.attachments[2].filename, "notes.txt");
    }

    #[test]
    fn demo_store_supports_filters_starred_attachments_and_contacts() {
        let store = DemoStore::new();
        let unread =
            store.list_messages_page_filtered(DEMO_INBOX_ID, "", MessageFilter::Unread, 0, 50);
        assert_eq!(unread.total, 6);
        assert!(unread.items.iter().all(|m| m.unread));

        let starred = store.list_starred_page("", MessageFilter::All, 0, 50);
        assert!(starred.total >= 4);
        assert!(starred.items.iter().all(|m| m.flagged));

        let attachments = store.list_cached_attachments("", 0, 50);
        assert!(attachments.len() >= 7);

        let contacts = store.list_known_contacts("", 0, 50);
        assert!(!contacts.is_empty());
        let aggregated: Vec<_> = contacts.iter().filter(|c| !c.custom).collect();
        assert!(!aggregated.is_empty());
        assert!(aggregated
            .windows(2)
            .all(|w| w[0].message_count >= w[1].message_count));
        if contacts[0].custom {
            assert!(contacts.iter().skip_while(|c| c.custom).all(|c| !c.custom));
        }
    }
}

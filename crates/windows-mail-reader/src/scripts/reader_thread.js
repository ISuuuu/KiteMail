(() => {
    const boundDocs = new WeakSet();
    const foldedDocs = new WeakSet();
    const observedDocs = new WeakSet();
    const attachedFrames = new WeakSet();
    const lastHeights = new WeakMap();

    let openMenu = null;
    let openMenuBtn = null;
    let linkPreviewEl = null;
    let initialAnchorPending = Boolean(document.body && document.body.dataset.initialAnchorId);
    let programmaticScroll = false;

    function sendAction(msg) {
        try {
            window.chrome.webview.postMessage(msg);
        } catch (_) {}
    }

    function showLinkPreview(href) {
        if (!href || !/^(https?:|mailto:)/i.test(href)) {
            hideLinkPreview();
            return;
        }
        if (!linkPreviewEl) {
            linkPreviewEl = document.createElement("div");
            linkPreviewEl.className = "km-link-preview";
            document.body.appendChild(linkPreviewEl);
        }
        linkPreviewEl.textContent = href;
        linkPreviewEl.classList.add("on");
    }

    function hideLinkPreview() {
        if (linkPreviewEl) {
            linkPreviewEl.classList.remove("on");
        }
    }

    function onMailtoClick(event) {
        if (!event.isTrusted || event.defaultPrevented) return;
        if (event.type === "click" && event.button !== 0) return;
        if (event.type === "auxclick" && event.button !== 1) return;

        let target = event.target;
        while (target && target.nodeType !== 1) {
            target = target.parentNode;
        }
        const anchor = target && typeof target.closest === "function" ? target.closest("a[href]") : null;
        if (!anchor) return;

        const raw = (anchor.getAttribute("href") || "").trim();
        if (!/^mailto:/i.test(raw)) return;

        event.preventDefault();
        event.stopPropagation();
        sendAction("open-link:" + (anchor.href || raw));
    }

    function onFrameHover(event) {
        let target = event.target;
        while (target && target.nodeType !== 1) {
            target = target.parentNode;
        }
        const anchor = target && typeof target.closest === "function" ? target.closest("a[href]") : null;
        if (!anchor) {
            hideLinkPreview();
            return;
        }
        const raw = (anchor.getAttribute("href") || "").trim();
        showLinkPreview(anchor.href || raw);
    }

    const EXPLICIT_QUOTE_SELECTOR = '.gmail_quote, #divRplyFwdMsg, #appendonsend, blockquote[type="cite"], .yahoo_quoted';
    const GENERIC_BLOCKQUOTE_SELECTOR = 'blockquote:not([style*="border: none"]):not([style*="border:none"])';
    const SIG_SELECTOR = '.moz-signature, #Signature, .gmail_signature, [class*="signature"]';
    const ATTR_RE = /(wrote|writes|schrieb|a écrit|escribió|escreveu|scrisse|schreef|skrev|kirjoitti|napisał|napsal|írta|написал|написала|έγραψε|写道)\s*[:：]?$/i;
    const ORIG_MSG_RE = /^\s*-{3,}\s*(Original Message|原始邮件)\s*-{3,}\s*$/i;

    function isBlankNode(node) {
        if (!node) return true;
        if (node.nodeType === 3) return !node.textContent.trim();
        if (node.nodeType !== 1) return true;
        if (/^(STYLE|SCRIPT|META|LINK|TITLE)$/.test(node.tagName)) return true;
        if (node.tagName === "IMG" || node.querySelector("img")) return false;
        return !node.textContent.trim();
    }

    function nonBlankKids(el) {
        const out = [];
        for (let n = el.firstChild; n; n = n.nextSibling) {
            if (!isBlankNode(n)) out.push(n);
        }
        return out;
    }

    function isQuoteLike(node) {
        if (!node || node.nodeType !== 1) return false;
        return node.matches(EXPLICIT_QUOTE_SELECTOR)
            || node.matches(GENERIC_BLOCKQUOTE_SELECTOR)
            || Boolean(node.querySelector(EXPLICIT_QUOTE_SELECTOR))
            || Boolean(node.querySelector(GENERIC_BLOCKQUOTE_SELECTOR));
    }

    function hasTrailingBody(top) {
        let n = top.nextSibling;
        while (n && (isQuoteLike(n) || isBlankNode(n))) {
            n = n.nextSibling;
        }
        let text = "";
        for (; n; n = n.nextSibling) {
            if (n.nodeType === 3) {
                text += n.textContent;
            } else if (n.nodeType === 1 && !n.matches(SIG_SELECTOR)) {
                const clone = n.cloneNode(true);
                const sigs = clone.querySelectorAll(SIG_SELECTOR);
                for (const sig of sigs) {
                    sig.parentNode.removeChild(sig);
                }
                const brs = clone.querySelectorAll("br");
                for (const br of brs) {
                    br.parentNode.replaceChild(clone.ownerDocument.createTextNode("\n"), br);
                }
                text += "\n" + clone.textContent;
            }
        }
        text = text.replace(/\u00a0/g, " ").trim();
        return Boolean(text) && !/^[-_]{2,}[ \t]*(\n|$)/.test(text);
    }

    function foldReplyQuote(doc, onToggle) {
        if (!doc.body || doc.querySelector("[data-kitemail-quote]")) return;

        let isExplicit = true;
        let quoteNode = doc.querySelector(EXPLICIT_QUOTE_SELECTOR);
        if (!quoteNode) {
            isExplicit = false;
            quoteNode = doc.querySelector(GENERIC_BLOCKQUOTE_SELECTOR);
        }
        if (!quoteNode) return;

        let root = doc.body;
        for (;;) {
            const kids = nonBlankKids(root);
            if (kids.length === 1 && kids[0].nodeType === 1 && kids[0] !== quoteNode && kids[0].contains(quoteNode)) {
                root = kids[0];
            } else {
                break;
            }
        }
        if (/^(TABLE|TBODY|THEAD|TFOOT|TR)$/.test(root.tagName)) return;

        let top = quoteNode;
        while (top.parentNode && top.parentNode !== root) {
            top = top.parentNode;
        }
        if (!top.parentNode) return;

        let start = top;
        let prev = top.previousSibling;
        while (prev && (isBlankNode(prev) || prev.nodeName === "BR")) {
            prev = prev.previousSibling;
        }
        if (prev) {
            const prevText = (prev.textContent || "").trim();
            if (prevText && prevText.length < 400 && (ATTR_RE.test(prevText) || ORIG_MSG_RE.test(prevText))) {
                start = prev;
            }
        }

        let hasBefore = false;
        for (let n = root.firstChild; n && n !== start; n = n.nextSibling) {
            if (!isBlankNode(n)) {
                hasBefore = true;
                break;
            }
        }
        if (!hasBefore) return;

        let hasQuoteContent = false;
        for (let n = start; n; n = n.nextSibling) {
            if (!isBlankNode(n)) {
                hasQuoteContent = true;
                break;
            }
        }
        if (!hasQuoteContent) return;

        if (!isExplicit && hasTrailingBody(top)) return;

        const box = doc.createElement("div");
        box.setAttribute("data-kitemail-quote", "1");
        box.style.display = "none";
        root.insertBefore(box, start);
        while (box.nextSibling) {
            box.appendChild(box.nextSibling);
        }

        const btn = doc.createElement("button");
        btn.type = "button";
        btn.setAttribute("data-kitemail-quote-toggle", "1");
        btn.textContent = "⋯ 显示引用内容";
        btn.style.cssText = "display:inline-flex;align-items:center;margin:10px 0 6px;padding:3px 10px;border-radius:999px;border:1px solid rgba(128,128,128,0.35);background:rgba(128,128,128,0.12);color:inherit;font:12px/1.4 system-ui,sans-serif;cursor:pointer;";
        root.insertBefore(btn, box);

        btn.addEventListener("click", (event) => {
            if (!event.isTrusted) return;
            event.preventDefault();
            event.stopPropagation();
            const expanded = box.style.display !== "none";
            box.style.display = expanded ? "none" : "";
            btn.textContent = expanded ? "⋯ 显示引用内容" : "▾ 隐藏引用内容";
            if (onToggle) onToggle();
        });
    }

    function sizeFrame(frame, doc) {
        if (!doc || !doc.documentElement || doc.URL === "about:blank") return;
        const card = frame.closest(".km-card");
        if (card && card.classList.contains("km-folded")) return;
        const rectH = doc.documentElement.getBoundingClientRect().height || 0;
        const bodyRectH = doc.body ? doc.body.getBoundingClientRect().height || 0 : 0;
        const zoom = parseFloat(doc.defaultView.getComputedStyle(doc.documentElement).zoom) || 1;
        const scrollH = doc.body ? doc.body.scrollHeight * zoom : 0;
        const nextH = Math.max(24, Math.ceil(Math.max(rectH, bodyRectH, scrollH)));
        const prevH = lastHeights.get(frame) || 0;
        if (Math.abs(nextH - prevH) >= 2) {
            lastHeights.set(frame, nextH);
            frame.style.height = nextH + "px";
        }
        frame.classList.add("km-live");
        if (initialAnchorPending) {
            applyInitialAnchor();
        }
    }

    function applyInitialAnchor() {
        if (!initialAnchorPending || !document.body) return;
        const id = document.body.dataset.initialAnchorId;
        if (!id) {
            initialAnchorPending = false;
            return;
        }
        const offset = Math.max(0, parseInt(document.body.dataset.initialAnchorOffset || "0", 10) || 0);
        const card = document.querySelector('.km-card[data-msg-id="' + id + '"]');
        if (!card) return;
        const rect = card.getBoundingClientRect();
        const top = Math.max(0, Math.round(window.scrollY + rect.top + offset - 12));
        programmaticScroll = true;
        window.scrollTo({ top, behavior: "instant" });
        setTimeout(() => { programmaticScroll = false; }, 0);
    }

    function attachToFrame(frame) {
        if (!frame.hasAttribute("srcdoc")) return false;
        let doc;
        try {
            doc = frame.contentDocument;
        } catch (_) {
            return false;
        }
        if (!doc || doc.URL === "about:blank") return false;

        if (!boundDocs.has(doc)) {
            boundDocs.add(doc);
            doc.addEventListener("load", () => sizeFrame(frame, doc), true);
            doc.addEventListener("click", (event) => {
                closeMenu();
                onMailtoClick(event);
            }, true);
            doc.addEventListener("auxclick", onMailtoClick, true);
            doc.addEventListener("mousemove", onFrameHover, { passive: true });
            doc.addEventListener("mouseleave", hideLinkPreview, { passive: true });
        }

        if (doc.readyState !== "loading" && doc.body && !foldedDocs.has(doc)) {
            foldedDocs.add(doc);
            try {
                foldReplyQuote(doc, () => sizeFrame(frame, doc));
            } catch (_) {}
        }

        if (doc.body && !observedDocs.has(doc)) {
            observedDocs.add(doc);
            sizeFrame(frame, doc);
            if (typeof ResizeObserver === "function") {
                const observer = new ResizeObserver(() => sizeFrame(frame, doc));
                observer.observe(doc.documentElement);
                observer.observe(doc.body);
            }
            requestAnimationFrame(() => sizeFrame(frame, doc));
        } else if (doc.body) {
            sizeFrame(frame, doc);
        }

        return boundDocs.has(doc) && foldedDocs.has(doc);
    }

    function watchFrame(frame) {
        if (!frame.hasAttribute("srcdoc")) return;
        if (!attachedFrames.has(frame)) {
            attachedFrames.add(frame);
            frame.addEventListener("load", () => attachToFrame(frame));
        }
        if (!attachToFrame(frame)) {
            let polls = 0;
            const timer = setInterval(() => {
                if (!frame.isConnected || attachToFrame(frame) || ++polls >= 100) {
                    clearInterval(timer);
                }
            }, 30);
        }
    }

    function hydrateCard(card) {
        card.classList.remove("km-folded");
        for (const template of card.querySelectorAll("template.km-frame-template")) {
            const frame = template.content.querySelector("iframe.km-frame");
            if (!frame) continue;
            const pending = frame.getAttribute("data-km-srcdoc");
            if (pending !== null) {
                frame.removeAttribute("data-km-srcdoc");
                frame.setAttribute("srcdoc", pending);
            }
            template.replaceWith(frame);
        }
        for (const frame of card.querySelectorAll("iframe.km-frame")) {
            watchFrame(frame);
        }
    }

    function closeMenu() {
        if (openMenu) {
            openMenu.remove();
            openMenu = null;
            openMenuBtn = null;
        }
    }

    function showCardMenu(btn, card) {
        if (openMenu && openMenuBtn === btn) {
            closeMenu();
            return;
        }
        closeMenu();

        const msgId = card.dataset.msgId;
        const flagged = card.dataset.flagged === "1";
        const exporting = card.dataset.exporting === "1";
        const isRaw = card.dataset.raw === "1";
        const isDemo = card.dataset.demo === "1";
        const hasUnsub = card.dataset.unsub === "1";

        const items = [
            { label: "回复", msg: "card-reply:" + msgId },
            { label: "回复全部", msg: "card-reply-all:" + msgId },
            { label: "转发", msg: "card-forward:" + msgId },
            { sep: true },
            { label: "标为未读", msg: "card-mark-unread:" + msgId },
            { label: flagged ? "取消星标" : "加星标", msg: "card-toggle-flag:" + msgId },
        ];
        if (!isDemo) {
            items.push(
                { sep: true },
                { label: "归档", msg: "card-archive:" + msgId },
                { label: "删除", msg: "card-delete:" + msgId, danger: true },
            );
        }
        items.push(
            { sep: true },
            { label: exporting ? "导出中..." : "导出为 .eml", msg: "card-export-eml:" + msgId, disabled: exporting },
            { label: "复制邮件正文", msg: "card-copy-body:" + msgId },
            { label: isRaw ? "返回正文" : "查看邮件原文", msg: "card-toggle-raw:" + msgId },
        );
        if (hasUnsub) {
            items.push(
                { sep: true },
                { label: "退订", msg: "card-unsub:" + msgId },
            );
        }

        const menu = document.createElement("div");
        menu.className = "km-menu";
        for (const item of items) {
            if (item.sep) {
                const sep = document.createElement("div");
                sep.className = "km-menu-sep";
                menu.appendChild(sep);
                continue;
            }
            const el = document.createElement("button");
            el.type = "button";
            el.className = "km-menu-item" + (item.danger ? " danger" : "");
            el.textContent = item.label;
            if (item.disabled) {
                el.disabled = true;
            } else {
                el.addEventListener("click", (event) => {
                    if (!event.isTrusted) return;
                    event.preventDefault();
                    event.stopPropagation();
                    closeMenu();
                    sendAction(item.msg);
                });
            }
            menu.appendChild(el);
        }

        document.body.appendChild(menu);
        const btnRect = btn.getBoundingClientRect();
        const menuRect = menu.getBoundingClientRect();
        let left = Math.min(btnRect.right - menuRect.width, window.innerWidth - menuRect.width - 8);
        left = Math.max(8, left);
        let top = btnRect.bottom + 4;
        if (top + menuRect.height > window.innerHeight - 8) {
            top = Math.max(8, btnRect.top - menuRect.height - 4);
        }
        menu.style.left = Math.round(left) + "px";
        menu.style.top = Math.round(top) + "px";
        openMenu = menu;
        openMenuBtn = btn;
    }

    document.addEventListener("click", (event) => {
        if (!event.isTrusted) return;
        const target = event.target instanceof Element ? event.target : null;
        if (!target) {
            closeMenu();
            return;
        }
        if (openMenu && openMenu.contains(target)) {
            return;
        }

        const moreBtn = target.closest(".km-more-btn");
        if (moreBtn) {
            event.preventDefault();
            event.stopPropagation();
            const card = moreBtn.closest(".km-card");
            if (card) showCardMenu(moreBtn, card);
            return;
        }

        closeMenu();

        const actionBtn = target.closest("[data-action]");
        if (actionBtn) {
            event.preventDefault();
            event.stopPropagation();
            const action = actionBtn.dataset.action;
            const msgId = actionBtn.dataset.msgId;
            if (action === "allow-remote" && msgId) {
                sendAction("card-allow-remote:" + msgId);
            } else if (action === "load-body" && msgId) {
                sendAction("card-load-body:" + msgId);
            } else if (action === "open-att" && actionBtn.dataset.attId) {
                sendAction("card-open-att:" + actionBtn.dataset.attId);
            } else if (action === "save-att" && actionBtn.dataset.attId) {
                sendAction("card-save-att:" + actionBtn.dataset.attId);
            } else if (action === "save-all-atts" && msgId) {
                sendAction("card-save-all-atts:" + msgId);
            } else if (action === "ics" && msgId && actionBtn.dataset.status) {
                sendAction("card-ics:" + msgId + ":" + actionBtn.dataset.status);
            } else if (action === "mdn" && msgId && actionBtn.dataset.decision) {
                sendAction("card-mdn:" + msgId + ":" + actionBtn.dataset.decision);
            } else if (action === "toggle-raw" && msgId) {
                sendAction("card-toggle-raw:" + msgId);
            } else if (action === "toggle-rcpts") {
                const card = actionBtn.closest(".km-card");
                const rcpts = card ? card.querySelector(".km-rcpts") : null;
                if (rcpts) {
                    const isHidden = rcpts.hasAttribute("hidden");
                    if (isHidden) {
                        rcpts.removeAttribute("hidden");
                    } else {
                        rcpts.setAttribute("hidden", "");
                    }
                    actionBtn.classList.toggle("open", isHidden);
                    actionBtn.setAttribute("aria-expanded", isHidden ? "true" : "false");
                }
            }
            return;
        }

        const hdr = target.closest(".km-hdr");
        if (hdr && !target.closest("button, a, .km-rcpts")) {
            const card = hdr.closest(".km-card");
            if (!card || !card.dataset.msgId) return;
            const msgId = card.dataset.msgId;
            const willFold = !card.classList.contains("km-folded");
            if (willFold) {
                card.classList.add("km-folded");
            } else {
                hydrateCard(card);
            }
            sendAction("card-fold:" + msgId + ":" + (willFold ? "1" : "0"));
        }
    });

    window.addEventListener("keydown", (event) => {
        if (event.key === "Escape") {
            closeMenu();
            return;
        }
        if ((event.ctrlKey || event.metaKey) && !event.altKey && !event.shiftKey
            && (event.key === "c" || event.key === "C")) {
            const outerSel = window.getSelection();
            if (outerSel && !outerSel.isCollapsed && String(outerSel).length > 0) {
                return;
            }
            for (const frame of document.querySelectorAll("iframe.km-frame")) {
                let doc;
                try {
                    doc = frame.contentDocument;
                } catch (_) {
                    continue;
                }
                if (!doc) continue;
                const sel = doc.getSelection();
                if (sel && !sel.isCollapsed && String(sel).length > 0) {
                    try {
                        doc.defaultView.focus();
                        if (doc.execCommand("copy")) {
                            event.preventDefault();
                            event.stopPropagation();
                        }
                    } catch (_) {}
                    break;
                }
            }
        }
    }, true);

    let scrollTimer = null;
    function reportScrollAnchor() {
        const cards = document.querySelectorAll(".km-card");
        for (const card of cards) {
            const rect = card.getBoundingClientRect();
            if (rect.bottom > 12 && card.dataset.msgId) {
                const offset = Math.max(0, Math.round(12 - rect.top));
                sendAction("scroll-anchor:" + card.dataset.msgId + ":" + offset);
                break;
            }
        }
    }

    window.addEventListener("scroll", () => {
        closeMenu();
        if (!programmaticScroll) {
            initialAnchorPending = false;
        }
        clearTimeout(scrollTimer);
        scrollTimer = setTimeout(reportScrollAnchor, 120);
    }, { passive: true });

    ["wheel", "touchstart", "mousedown", "keydown"].forEach((ev) => {
        window.addEventListener(ev, () => {
            initialAnchorPending = false;
        }, { passive: true, capture: true });
    });

    window.__kitemailSyncFolded = function (collapsedIds) {
        const set = new Set((collapsedIds || []).map(String));
        const cards = document.querySelectorAll(".km-card");
        for (const card of cards) {
            const id = card.dataset.msgId;
            if (!id) continue;
            if (set.has(id)) {
                card.classList.add("km-folded");
            } else {
                hydrateCard(card);
            }
        }
    };

    window.__kitemailSyncCardMeta = function (items) {
        if (!Array.isArray(items)) return;
        for (const item of items) {
            if (!item || item.id === undefined) continue;
            const card = document.querySelector('.km-card[data-msg-id="' + item.id + '"]');
            if (!card) continue;
            card.dataset.flagged = item.flagged ? "1" : "0";
            card.dataset.exporting = item.exporting ? "1" : "0";
            const star = card.querySelector(".km-star-dot");
            if (star) {
                star.classList.toggle("on", Boolean(item.flagged));
            }
        }
    };

    const findState = { marks: [], cur: -1 };

    function ensureFindCss(doc) {
        if (!doc || doc.getElementById("km-find-css")) return;
        const st = doc.createElement("style");
        st.id = "km-find-css";
        st.textContent = ".km-find-hit{background:rgba(255,198,0,0.45);color:#000;border-radius:3px;box-shadow:0 0 0 1px rgba(255,198,0,0.55);}.km-find-hit.current{background:#ffc600;box-shadow:0 0 0 2px #f59e0b;}";
        (doc.head || doc.documentElement).appendChild(st);
    }

    function collectFindDocs() {
        const docs = [document];
        for (const frame of document.querySelectorAll("iframe.km-frame")) {
            const card = frame.closest(".km-card");
            if (card && card.classList.contains("km-folded")) continue;
            try {
                if (frame.contentDocument && frame.contentDocument.body) {
                    docs.push(frame.contentDocument);
                }
            } catch (_) {}
        }
        return docs;
    }

    function postFindCount() {
        const cur = findState.marks.length ? findState.cur + 1 : 0;
        sendAction("find-count:" + cur + ":" + findState.marks.length);
    }

    function showActiveFind() {
        for (let i = 0; i < findState.marks.length; i++) {
            findState.marks[i].className = "km-find-hit" + (i === findState.cur ? " current" : "");
        }
        postFindCount();
        const mark = findState.marks[findState.cur];
        if (!mark) return;
        const doc = mark.ownerDocument;
        if (doc === document) {
            mark.scrollIntoView({ block: "center" });
            return;
        }
        for (const frame of document.querySelectorAll("iframe.km-frame")) {
            try {
                if (frame.contentDocument === doc) {
                    const r = mark.getBoundingClientRect();
                    const fr = frame.getBoundingClientRect();
                    window.scrollTo({
                        top: Math.max(0, Math.round(window.scrollY + fr.top + r.top - window.innerHeight / 2)),
                    });
                    break;
                }
            } catch (_) {}
        }
    }

    function findMarkY(mark) {
        const doc = mark.ownerDocument;
        const r = mark.getBoundingClientRect();
        if (doc === document) return r.top + window.scrollY;
        for (const frame of document.querySelectorAll("iframe.km-frame")) {
            try {
                if (frame.contentDocument === doc) {
                    return frame.getBoundingClientRect().top + window.scrollY + r.top;
                }
            } catch (_) {}
        }
        return r.top;
    }

    window.kmFindClear = function () {
        for (const m of findState.marks) {
            const p = m.parentNode;
            if (!p) continue;
            while (m.firstChild) p.insertBefore(m.firstChild, m);
            p.removeChild(m);
            p.normalize();
        }
        findState.marks = [];
        findState.cur = -1;
    };

    window.kmFindStep = function (dir) {
        if (!findState.marks.length) return;
        const step = dir < 0 ? -1 : 1;
        findState.cur = (findState.cur + step + findState.marks.length) % findState.marks.length;
        showActiveFind();
    };

    window.kmFind = function (rawQuery) {
        window.kmFindClear();
        const q = (rawQuery || "").trim().toLowerCase();
        if (!q) {
            postFindCount();
            return;
        }
        for (const doc of collectFindDocs()) {
            ensureFindCss(doc);
            const root = doc.body || doc.documentElement;
            if (!root) continue;
            const walker = doc.createTreeWalker(root, NodeFilter.SHOW_TEXT, null);
            const nodes = [];
            let node;
            while ((node = walker.nextNode())) {
                const parent = node.parentNode;
                const pname = parent && parent.nodeName;
                if (pname === "SCRIPT" || pname === "STYLE" || pname === "NOSCRIPT") continue;
                if (doc === document && parent && typeof parent.closest === "function") {
                    if (parent.closest(".km-folded .km-details, .km-rcpts[hidden], .km-menu")) continue;
                }
                if (node.nodeValue && node.nodeValue.toLowerCase().indexOf(q) >= 0) {
                    nodes.push(node);
                }
            }
            for (const textNode of nodes) {
                const text = textNode.nodeValue;
                const lower = text.toLowerCase();
                let pos = 0;
                let idx;
                const frag = doc.createDocumentFragment();
                let had = false;
                while ((idx = lower.indexOf(q, pos)) >= 0) {
                    had = true;
                    frag.appendChild(doc.createTextNode(text.slice(pos, idx)));
                    const span = doc.createElement("span");
                    span.className = "km-find-hit";
                    span.textContent = text.slice(idx, idx + q.length);
                    frag.appendChild(span);
                    findState.marks.push(span);
                    pos = idx + q.length;
                }
                frag.appendChild(doc.createTextNode(text.slice(pos)));
                if (had && textNode.parentNode) {
                    textNode.parentNode.replaceChild(frag, textNode);
                }
            }
        }
        findState.marks = findState.marks.filter((m) => m.getClientRects().length > 0);
        if (findState.marks.length) {
            findState.marks.sort((a, b) => findMarkY(a) - findMarkY(b));
            findState.cur = 0;
            showActiveFind();
        } else {
            postFindCount();
        }
    };

    for (const frame of document.querySelectorAll("iframe.km-frame")) {
        watchFrame(frame);
    }
    if (initialAnchorPending) {
        applyInitialAnchor();
        setTimeout(() => { initialAnchorPending = false; }, 500);
    }
})();

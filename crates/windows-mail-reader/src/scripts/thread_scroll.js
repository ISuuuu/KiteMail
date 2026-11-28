(() => {
    const frame = document.querySelector("iframe");
    if (!frame) return;

    const boundDocs = new WeakSet();
    const foldedDocs = new WeakSet();

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
        window.chrome.webview.postMessage("open-link:" + (anchor.href || raw));
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

    function foldReplyQuote(doc) {
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
        });
    }

    function attachToFrame() {
        let doc;
        try {
            doc = frame.contentDocument;
        } catch (_) {
            return false;
        }
        if (!doc || doc.URL === "about:blank") return false;

        if (!boundDocs.has(doc)) {
            boundDocs.add(doc);
            doc.addEventListener("click", onMailtoClick, true);
            doc.addEventListener("auxclick", onMailtoClick, true);
        }

        if (doc.readyState !== "loading" && doc.body && !foldedDocs.has(doc)) {
            foldedDocs.add(doc);
            try {
                foldReplyQuote(doc);
            } catch (_) {}
        }

        return boundDocs.has(doc) && foldedDocs.has(doc);
    }

    frame.addEventListener("load", attachToFrame);
    if (!attachToFrame()) {
        let polls = 0;
        const timer = setInterval(() => {
            if (attachToFrame() || ++polls >= 100) {
                clearInterval(timer);
            }
        }, 30);
    }

    const findState = { marks: [], cur: -1 };

    function ensureFindCss(doc) {
        if (!doc || doc.getElementById("km-find-css")) return;
        const st = doc.createElement("style");
        st.id = "km-find-css";
        st.textContent = ".km-find-hit{background:rgba(255,198,0,0.45);color:#000;border-radius:3px;box-shadow:0 0 0 1px rgba(255,198,0,0.55);}.km-find-hit.current{background:#ffc600;box-shadow:0 0 0 2px #f59e0b;}";
        (doc.head || doc.documentElement).appendChild(st);
    }

    function postFindCount() {
        const cur = findState.marks.length ? findState.cur + 1 : 0;
        try {
            window.chrome.webview.postMessage("find-count:" + cur + ":" + findState.marks.length);
        } catch (_) {}
    }

    function showActiveFind() {
        for (let i = 0; i < findState.marks.length; i++) {
            findState.marks[i].className = "km-find-hit" + (i === findState.cur ? " current" : "");
        }
        postFindCount();
        const mark = findState.marks[findState.cur];
        if (mark) {
            mark.scrollIntoView({ block: "center" });
        }
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
        let doc = null;
        try {
            doc = frame.contentDocument;
        } catch (_) {}
        if (!doc || !doc.body) {
            postFindCount();
            return;
        }
        ensureFindCss(doc);
        const walker = doc.createTreeWalker(doc.body, NodeFilter.SHOW_TEXT, null);
        const nodes = [];
        let node;
        while ((node = walker.nextNode())) {
            const pname = node.parentNode && node.parentNode.nodeName;
            if (pname === "SCRIPT" || pname === "STYLE" || pname === "NOSCRIPT") continue;
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
        findState.marks = findState.marks.filter((m) => m.getClientRects().length > 0);
        if (findState.marks.length) {
            findState.cur = 0;
            showActiveFind();
        } else {
            postFindCount();
        }
    };
})();

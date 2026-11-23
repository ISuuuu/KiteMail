(() => {
    const frame = document.querySelector("iframe");
    if (!frame) return;

    function canScroll(element, delta) {
        const remaining = delta < 0
            ? element.scrollTop
            : element.scrollHeight - element.clientHeight - element.scrollTop;
        return remaining > 1;
    }

    function onWheel(event) {
        const mode = event.deltaMode;
        const delta = event.deltaY;
        if (!event.isTrusted || event.ctrlKey || event.shiftKey || event.altKey
            || event.metaKey || Math.abs(delta) <= Math.abs(event.deltaX)) {
            return;
        }

        const doc = event.currentTarget;
        const view = doc.defaultView;
        const root = doc.scrollingElement;
        for (const element of event.composedPath()) {
            if (!(element instanceof view.Element) || element === root) continue;
            if (canScroll(element, delta) && /^(auto|scroll)$/.test(view.getComputedStyle(element).overflowY)) {
                return;
            }
        }
        if (root && canScroll(root, delta)) {
            const rootOverflow = view.getComputedStyle(doc.documentElement).overflowY;
            const viewportOverflow = rootOverflow === "visible" && doc.body
                ? view.getComputedStyle(doc.body).overflowY
                : rootOverflow;
            if (viewportOverflow !== "hidden" && viewportOverflow !== "clip") return;
        }

        event.preventDefault();
        const pixels = delta * (mode === 1 ? 16 : mode === 2 ? window.innerHeight : 1);
        window.chrome.webview.postMessage("thread-wheel:" + pixels);
    }

    function listenToFrame() {
        frame.contentDocument?.addEventListener("wheel", onWheel, { passive: false });
    }

    document.addEventListener("wheel", onWheel, { passive: false });
    frame.addEventListener("load", listenToFrame);
    listenToFrame();
})();

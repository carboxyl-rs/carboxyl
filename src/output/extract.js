(function() {
    const vh = window.innerHeight;
    const vw = window.innerWidth;
    const seen = new WeakSet();
    const nodes = [];

    const SKIP_TAGS = new Set([
        'SCRIPT', 'STYLE', 'NOSCRIPT', 'HEAD', 'META',
        'LINK', 'TEMPLATE', 'CANVAS', 'IFRAME',
    ]);
    const CSS_LEAK_RE = /^\s*(?:[.#*\[]|[\w-]+\s*\{|@[\w-]+)/;

    function visible(r) {
        return r && r.width > 0 && r.height > 0
            && r.bottom > 0 && r.top < vh
            && r.right > 0 && r.left < vw;
    }

    // Reject elements occluded by a higher stacking context (modals, overlays,
    // z-index layering). elementFromPoint returns the topmost element at the
    // centre of the rect; we accept only if that element is el itself, a child
    // of el (inline child received the hit), or an ancestor of el (el is part
    // of the topmost stacking context).
    function isTopmost(el, r) {
        const cx = (r.left + r.right) / 2;
        const cy = (r.top + r.bottom) / 2;
        if (cx < 0 || cx >= vw || cy < 0 || cy >= vh) return false;
        const hit = document.elementFromPoint(cx, cy);
        if (!hit) return false;
        return el === hit || el.contains(hit) || hit.contains(el);
    }

    function push(el, text, r) {
        if (seen.has(el)) return;
        seen.add(el);
        const s = getComputedStyle(el);
        if (s.display === 'none' || s.visibility === 'hidden' || s.opacity === '0') return;
        nodes.push({ t: text, x: r.left, y: r.top, w: r.width, h: r.height, c: s.color });
    }

    // --- Regular text nodes ---
    const walker = document.createTreeWalker(
        document.body || document.documentElement,
        NodeFilter.SHOW_TEXT,
        null
    );
    let node;
    while ((node = walker.nextNode())) {
        const text = (node.textContent || '').trim();
        if (!text) continue;
        const el = node.parentElement;
        if (!el) continue;
        if (SKIP_TAGS.has(el.tagName)) continue;
        if (CSS_LEAK_RE.test(text)) continue;
        const r = el.getBoundingClientRect();
        if (!visible(r)) continue;
        if (!isTopmost(el, r)) continue;   // ← new
        push(el, text, r);
    }

    // --- Form controls + buttons ---
    const controls = document.querySelectorAll(
        'button, select, ' +
        'input[type="text"], input[type="search"], input[type="submit"], ' +
        'input[type="button"], input[type="reset"], input[type="email"], ' +
        'input[type="url"], input[type="tel"], input[type="number"], ' +
        'input:not([type]), textarea, [contenteditable]'
    );
    for (const el of controls) {
        const text = ((el.value !== undefined && el.value !== '')
            ? el.value
            : el.textContent || '').trim();
        if (!text) continue;
        if (CSS_LEAK_RE.test(text)) continue;
        const r = el.getBoundingClientRect();
        if (!visible(r)) continue;
        if (!isTopmost(el, r)) continue;   // ← new
        push(el, text, r);
    }

    return nodes;
})();

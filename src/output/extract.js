(function() {
    const vh = window.innerHeight;
    const vw = window.innerWidth;
    const seen = new WeakSet();
    const nodes = [];

    // Tags whose text content is never user-visible (or not renderable).
    const SKIP_TAGS = new Set([
        'SCRIPT', 'STYLE', 'NOSCRIPT', 'HEAD', 'META',
        'LINK', 'TEMPLATE', 'CANVAS', 'IFRAME',
    ]);

    // Detects raw CSS code that leaked into text nodes (stylesheet injection
    // artifacts occasionally visible in some site builds).
    const CSS_LEAK_RE = /^\s*(?:[.#*\[]|[\w-]+\s*\{|@[\w-]+)/;

    // Selector for form controls that expose text via .value rather than DOM
    // text nodes. After TreeWalker extracts a descendant text node we mark the
    // closest matching ancestor in `seen` so the controls loop below does not
    // emit a duplicate entry for the same visual text.
    const CTRL_SELECTOR =
        'button, select, textarea, input, [contenteditable]';

    // Is the bounding rect inside the viewport?
    function visible(r) {
        return r && r.width > 0 && r.height > 0
            && r.bottom > 0 && r.top < vh
            && r.right > 0 && r.left < vw;
    }

    // Is the element topmost at its center (not occluded by a modal/overlay)?
    function isTopmost(el, r) {
        const cx = (r.left + r.right) / 2;
        const cy = (r.top + r.bottom) / 2;
        if (cx < 0 || cx >= vw || cy < 0 || cy >= vh) return false;
        const hit = document.elementFromPoint(cx, cy);
        if (!hit) return false;
        return el === hit || el.contains(hit) || hit.contains(el);
    }

    // Emit one text entry. `s` is the pre-computed style for `el` (read while
    // the suppress stylesheet is disabled, so `s.color` is the page's authored
    // color, not transparent).
    //
    // x/y are offset by padding + border-width so the coordinate points at the
    // content box where glyphs actually begin, not the element's box edge.
    function push(el, text, r, s) {
        if (seen.has(el)) return;
        seen.add(el);
        if (s.display === 'none' || s.visibility === 'hidden' || s.opacity === '0') return;
        const padL = parseFloat(s.paddingLeft)     || 0;
        const padT = parseFloat(s.paddingTop)      || 0;
        const bdrL = parseFloat(s.borderLeftWidth) || 0;
        const bdrT = parseFloat(s.borderTopWidth)  || 0;
        nodes.push({
            t: text,
            x: r.left + padL + bdrL,
            y: r.top  + padT + bdrT,
            w: r.width,
            h: r.height,
            c: s.color,
        });
    }

    // Temporarily disable the carboxyl text-suppression stylesheet so that
    // getComputedStyle().color returns the page's authored colors instead of
    // the transparent override.  Layout is unaffected — `color: transparent`
    // has no effect on geometry or getBoundingClientRect() values.
    const suppressEl = document.getElementById('carboxyl-text-suppress');
    const suppressSheet = suppressEl && suppressEl.sheet;
    if (suppressSheet) suppressSheet.disabled = true;

    // --- Regular text nodes (TreeWalker) ---
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
        if (!isTopmost(el, r)) continue;
        const s = getComputedStyle(el);
        push(el, text, r, s);
        // Prevent the controls loop below from re-emitting this text through
        // the containing form control element (e.g. <button><span>X</span></button>
        // would otherwise produce two entries for the same visual glyph run).
        const ctrl = el.closest(CTRL_SELECTOR);
        if (ctrl) seen.add(ctrl);
    }

    // --- Form controls (value-based text not in the DOM text tree) ---
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
        if (!isTopmost(el, r)) continue;
        const s = getComputedStyle(el);
        push(el, text, r, s);
    }

    // Restore suppression so the Servo pixel render keeps text transparent.
    if (suppressSheet) suppressSheet.disabled = false;

    return nodes;
})();

(function() {
    const vh = window.innerHeight;
    const vw = window.innerWidth;
    const seen = new WeakSet();
    const nodes = [];

    function visible(r) {
        return r && r.width > 0 && r.height > 0
            && r.bottom > 0 && r.top < vh
            && r.right > 0 && r.left < vw;
    }

    function push(el, text, r) {
        if (seen.has(el)) return;
        seen.add(el);

        const s = getComputedStyle(el);
        if (s.display === 'none' || s.visibility === 'hidden' || s.opacity === '0') return;

        nodes.push({
            t: text,
            x: r.left,
            y: r.top,
            w: r.width,
            h: r.height,
            c: s.color
        });
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

        const r = el.getBoundingClientRect();
        if (!visible(r)) continue;

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

        const r = el.getBoundingClientRect();
        if (!visible(r)) continue;

        push(el, text, r);
    }

    return nodes;
})();

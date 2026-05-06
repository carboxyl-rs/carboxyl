(function() {
    const ATTR = 'data-carboxyl-suppress';
    if (document.documentElement.hasAttribute(ATTR)) return;
    document.documentElement.setAttribute(ATTR, '1');

    const style = document.createElement('style');
    style.id = 'carboxyl-text-suppress';
    style.textContent = `
        * {
            color: transparent !important;
            caret-color: transparent !important;
            text-shadow: none !important;
        }
    `;
    (document.head || document.documentElement).appendChild(style);
})();

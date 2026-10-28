export function providerPresetModels(catalog, id) {
    const preset = catalog.providers?.find((p) => p.id === id);
    if (!preset)
        return [];
    const prefix = preset.prefix ?? '';
    const models = (catalog.models ?? []).filter((model) => (model.providerPrefix ?? model.provider_prefix ?? '') === prefix);
    const defaults = models.filter((model) => model.default);
    return defaults.length ? defaults : models.slice(0, 1);
}
export function providerEditorValidation(name, url, models, zh) {
    if (!name.trim() ||
        !url.trim() ||
        !models.length ||
        models.some((m) => !m.name.trim() || !Number.isSafeInteger(m.context) || m.context < 1))
        return zh
            ? '请填写厂商名称、接口地址、模型 ID 和正整数上下文长度。'
            : 'Enter a provider name, endpoint, model ID and positive integer context length.';
    try {
        if (!['http:', 'https:'].includes(new URL(url).protocol))
            throw new Error();
    }
    catch {
        return zh
            ? '接口地址须以 http:// 或 https:// 开头。'
            : 'The endpoint must use http:// or https://.';
    }
    if (new Set(models.map((m) => m.name.trim())).size !== models.length)
        return zh ? '模型 ID 不能重复。' : 'Model IDs must be unique.';
    return null;
}
export function providerTabKey(event) {
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key))
        return;
    const button = event.currentTarget;
    const tabs = Array.from(button.parentElement.querySelectorAll('[role="tab"]'));
    const index = tabs.indexOf(button);
    const next = event.key === 'Home'
        ? 0
        : event.key === 'End'
            ? tabs.length - 1
            : (index + (event.key === 'ArrowRight' ? 1 : -1) + tabs.length) %
                tabs.length;
    if (tabs[next]?.disabled)
        return;
    event.preventDefault();
    tabs[next].focus();
    tabs[next].click();
}
/** Keep focus and background interaction inside a mounted modal, including Lit shadow roots. */
export function bindProviderModal(node, options) {
    const root = node.getRootNode();
    const previous = root.activeElement;
    const backdrop = node.parentElement;
    const siblings = Array.from(backdrop.parentElement?.children ?? []).filter((el) => el !== backdrop);
    const inert = siblings.map((el) => Boolean(el.inert));
    siblings.forEach((el) => (el.inert = true));
    const focusable = () => Array.from(node.querySelectorAll('button,input,select,textarea,[tabindex]')).filter((el) => !el.hasAttribute('disabled') &&
        !el.disabled &&
        !el.closest('[hidden],fieldset:disabled') &&
        el.getAttribute('type') !== 'hidden' &&
        el.tabIndex >= 0);
    const keydown = (event) => {
        if (event.key === 'Escape') {
            event.preventDefault();
            event.stopImmediatePropagation();
            if (!options.busy())
                options.close();
        }
        if (event.key === 'Tab') {
            const items = focusable();
            const active = root.activeElement;
            if (!items.length) {
                event.preventDefault();
                node.focus();
            }
            else if (event.shiftKey && (active === items[0] || active === node)) {
                event.preventDefault();
                items.at(-1).focus();
            }
            else if (!event.shiftKey &&
                (active === items.at(-1) || !node.contains(active))) {
                event.preventDefault();
                items[0].focus();
            }
        }
    };
    const outside = (event) => {
        if (event.target === backdrop && !options.busy())
            options.close();
    };
    node.addEventListener('keydown', keydown, true);
    backdrop.addEventListener('pointerdown', outside);
    node.focus();
    return () => {
        node.removeEventListener('keydown', keydown, true);
        backdrop.removeEventListener('pointerdown', outside);
        siblings.forEach((el, index) => (el.inert = inert[index]));
        if (previous?.isConnected)
            previous.focus();
    };
}
//# sourceMappingURL=provider-editor.js.map
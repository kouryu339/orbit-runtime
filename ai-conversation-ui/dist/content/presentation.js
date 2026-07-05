export function visiblePresentationItems(items, completedKeys) {
    let pendingProgressiveIncluded = false;
    return items.filter((item) => {
        const key = `presentation:${item.id}`;
        if (item.reveal !== 'progressive' || completedKeys.has(key))
            return true;
        if (pendingProgressiveIncluded)
            return false;
        pendingProgressiveIncluded = true;
        return true;
    });
}
//# sourceMappingURL=presentation.js.map
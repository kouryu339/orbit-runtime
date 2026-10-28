export const REFERENCE_COLORS = [
    '#8b3a52',
    '#215b82',
    '#27624f',
    '#6f4b91',
    '#8a4f1e',
];
// A reference's visual identity depends on its target, never on its position in a message.
export function referenceColor(objectKey) {
    let hash = 2166136261;
    for (const character of objectKey) {
        hash ^= character.codePointAt(0);
        hash = Math.imul(hash, 16777619);
    }
    return REFERENCE_COLORS[(hash >>> 0) % REFERENCE_COLORS.length];
}
/** Image IDs are opaque. Only host-resolved browser URLs may become image sources. */
export function isSafeImageSource(source) {
    try {
        const url = new URL(source);
        return (['http:', 'https:', 'blob:'].includes(url.protocol) ||
            (url.protocol === 'data:' &&
                /^data:image\/(?:png|jpeg|gif|webp|svg\+xml)(?:;|,)/i.test(source)));
    }
    catch {
        return false;
    }
}
//# sourceMappingURL=references.js.map
import type { BuiltinProviderCatalog } from '../host/types.js';
export declare function providerPresetModels(catalog: BuiltinProviderCatalog, id: string): import("../index.js").BuiltinProviderCatalogModel[];
export declare function providerEditorValidation(name: string, url: string, models: Array<{
    name: string;
    context: number;
}>, zh: boolean): string | null;
export declare function providerTabKey(event: KeyboardEvent): void;
/** Keep focus and background interaction inside a mounted modal, including Lit shadow roots. */
export declare function bindProviderModal(node: HTMLElement, options: {
    close: () => void;
    busy: () => boolean;
}): () => void;
//# sourceMappingURL=provider-editor.d.ts.map
import { LitElement } from 'lit';
import type { AgentRuntimeConversationPublicApi, CommandResult, ConversationDensity, ConversationExtension, ConversationHostCapabilities, PresetMarkdown, ConversationProviderControlsPolicy, ConversationPersistencePolicy, ConversationPresentationItem, ConversationThemeName, ConversationTransport, PersistedConversation, PresentationItemPatch, SendOptions, SendResult } from '../host/types.js';
import { type ConversationState } from '../protocol/index.js';
import './rich-content.js';
export declare class AgentRuntimeConversationElement extends LitElement implements AgentRuntimeConversationPublicApi {
    static properties: {
        transport: {
            attribute: boolean;
        };
        capabilities: {
            attribute: boolean;
        };
        presentationItems: {
            attribute: boolean;
        };
        extensions: {
            attribute: boolean;
        };
        conversationId: {
            type: StringConstructor;
            attribute: string;
        };
        locale: {
            type: StringConstructor;
        };
        colorScheme: {
            type: StringConstructor;
            attribute: string;
            reflect: boolean;
        };
        theme: {
            type: StringConstructor;
            reflect: boolean;
        };
        density: {
            type: StringConstructor;
            reflect: boolean;
        };
        persistence: {
            attribute: boolean;
        };
        providerControls: {
            attribute: boolean;
        };
        progressiveReveal: {
            type: BooleanConstructor;
            attribute: string;
        };
        hideToolCalls: {
            type: BooleanConstructor;
            attribute: string;
        };
        disclaimer: {
            type: StringConstructor;
        };
        state: {
            state: boolean;
        };
        draft: {
            state: boolean;
        };
        historyOpen: {
            state: boolean;
        };
        persistedConversations: {
            state: boolean;
        };
        persistenceOperation: {
            state: boolean;
        };
        providerPanelOpen: {
            state: boolean;
        };
        providerEditorOpen: {
            state: boolean;
        };
        providerDefinitions: {
            state: boolean;
        };
        builtinProviderCatalog: {
            state: boolean;
        };
        providerEditorProviderUid: {
            state: boolean;
        };
        providerEditorBuiltinProviderId: {
            state: boolean;
        };
        providerEditorProviderName: {
            state: boolean;
        };
        providerEditorBaseUrl: {
            state: boolean;
        };
        providerEditorApiParadigm: {
            state: boolean;
        };
        providerEditorApiKey: {
            state: boolean;
        };
        providerEditorModels: {
            state: boolean;
        };
        providerOperation: {
            state: boolean;
        };
        permissionOperation: {
            state: boolean;
        };
    };
    static styles: import("lit").CSSResult;
    transport: ConversationTransport | null;
    capabilities: ConversationHostCapabilities;
    presentationItems: readonly ConversationPresentationItem[];
    extensions: readonly ConversationExtension[];
    conversationId: string | null;
    locale: string;
    colorScheme: 'light' | 'dark' | 'system';
    theme: ConversationThemeName;
    density: ConversationDensity;
    persistence: ConversationPersistencePolicy;
    providerControls: ConversationProviderControlsPolicy;
    progressiveReveal: boolean;
    hideToolCalls: boolean;
    disclaimer: string;
    state: ConversationState;
    private draft;
    private historyOpen;
    private persistedConversations;
    private persistenceOperation;
    private providerPanelOpen;
    private providerEditorOpen;
    private providerDefinitions;
    private builtinProviderCatalog;
    private providerEditorProviderUid;
    private providerEditorBuiltinProviderId;
    private providerEditorProviderName;
    private providerEditorBaseUrl;
    private providerEditorApiParadigm;
    private providerEditorApiKey;
    private providerEditorModels;
    private providerOperation;
    private permissionOperation;
    private connection;
    private connectAbort;
    private localMessageSequence;
    private providerEditorModelSequence;
    private localPresetSequence;
    private stickToBottom;
    private readonly completedRevealKeys;
    private readonly revealContentLengths;
    private persistenceUnsubscribe;
    private persistenceController;
    constructor();
    disconnectedCallback(): void;
    connect(): Promise<void>;
    disconnect(): void;
    openConversation(conversationId: string): Promise<void>;
    createConversation(): Promise<void>;
    saveConversation(): Promise<PersistedConversation | null>;
    restoreConversation(archiveId: string): Promise<void>;
    refreshConversationHistory(): Promise<void>;
    send(content: string, options?: SendOptions): Promise<SendResult>;
    pause(): Promise<CommandResult>;
    resolveToolPermission(toolCallId: string, decision: 'allow' | 'deny'): Promise<CommandResult>;
    private onToolPermissionDecision;
    closeConversation(): Promise<void>;
    focusComposer(): void;
    insertPresetMarkdown(preset: PresetMarkdown): string;
    insertPresentationItem(item: ConversationPresentationItem): void;
    updatePresentationItem(id: string, patch: PresentationItemPatch): void;
    removePresentationItem(id: string): void;
    clearPresentationItems(scope?: string): void;
    protected willUpdate(changed: Map<PropertyKey, unknown>): void;
    protected updated(changed: Map<PropertyKey, unknown>): void;
    protected render(): import("lit-html").TemplateResult<1>;
    private renderPersistenceActions;
    private renderProviderActions;
    private renderProviderPanel;
    private renderProviderEditor;
    private openProviderEditor;
    private resetProviderEditorDraft;
    private loadProviderEditorDraft;
    private selectProviderPreset;
    private createProviderEditorModelRow;
    private addProviderEditorModel;
    private removeProviderEditorModel;
    private updateProviderEditorModel;
    private providerEditorBuiltinModels;
    private builtinProviderById;
    private builtinModelById;
    private providerApiParadigm;
    private builtinModelContext;
    private builtinModelLabel;
    private parseOptionalPositiveInt;
    private providerEditorModelContext;
    private refreshProviderDefinitions;
    private selectProviderModel;
    private submitProviderEditor;
    private existingProviderRegistrationsExcept;
    private providerRegistrationFromDefinition;
    private configureProviderInput;
    private providerModels;
    private providerModelsForProvider;
    private nextProviderUid;
    private nextModelUid;
    private currentProviderModelUid;
    private currentProviderModelLabel;
    private providerModelLabel;
    private isCurrentArchive;
    private formatArchiveTime;
    private emitProviderError;
    private renderPermissionDialog;
    private renderDialogPermissionItem;
    private permissionEffectLabel;
    private permissionArguments;
    private hasRenderedToolAnchor;
    private renderHistoryPanel;
    private renderDisplayItem;
    private createDisplayItems;
    private latestWidgetRecordKey;
    private latestAssistantKey;
    private handleTransportEvent;
    private dispatch;
    private emitConnectionChange;
    private emit;
    private onDraftInput;
    private resetComposerHeight;
    private onComposerKeydown;
    private onScroll;
    private followReveal;
    private completeReveal;
    private markRecordsRevealComplete;
    private refreshRevealTracking;
    private statusLabel;
    private canCompose;
    private canSaveConversation;
    private configurePersistenceController;
    private handlePersistenceEvent;
    private upsertPersistedConversation;
    private emitPersistenceError;
    private activityLabel;
    private resolvedColorScheme;
}
declare global {
    interface HTMLElementTagNameMap {
        'agent-runtime-conversation': AgentRuntimeConversationElement;
    }
}
//# sourceMappingURL=conversation-element.d.ts.map
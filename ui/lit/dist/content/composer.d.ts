import type { ConversationState } from '../protocol/types.js';
import type { CommandResult, ConversationTransport, ConversationComposerController, ConversationComposerSettings, ReasoningEffort } from '../host/types.js';
export declare const effortLabels: Record<ReasoningEffort, [string, string]>;
export declare function effortLabel(effort: ReasoningEffort | null | undefined, zh: boolean): string;
/** Shared UI state machine; Runtime snapshots remain execution truth. */
export declare class ComposerSession {
    private changed;
    private error;
    settings: ConversationComposerSettings;
    effortPreview: {
        value: ReasoningEffort | null;
    } | null;
    get displayedEffort(): ReasoningEffort | null | undefined;
    previewEffort(value: ReasoningEffort | null): void;
    clearEffortPreview(): void;
    loading: boolean;
    operation: 'pause' | 'resume' | 'effort' | 'permission' | null;
    awaiting: 'pause' | 'resume' | null;
    paused: boolean;
    private epoch;
    private state;
    private transport;
    private controller;
    private modelUid;
    private externalBusy;
    constructor(changed: () => void, error: (message: string) => void);
    sync(state: ConversationState, transport: ConversationTransport | null, controller: ConversationComposerController | null, modelUid: number | undefined, externalBusy?: boolean): void;
    get ready(): boolean;
    get canCompose(): boolean;
    get phase(): "thinking" | "executing" | "compacting" | "ready" | "initializing" | "offline" | "pausing" | "resuming" | "approval" | "paused";
    get canPause(): boolean;
    get canResume(): boolean;
    get canConfigure(): boolean;
    private request;
    load(): Promise<void>;
    configure(kind: 'effort' | 'permission', value: string | null): Promise<void>;
    command(kind: 'pause' | 'resume'): Promise<CommandResult>;
    sent(): void;
    dispose(): void;
}
//# sourceMappingURL=composer.d.ts.map
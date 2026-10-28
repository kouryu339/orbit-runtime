export const effortLabels = {
    none: ['关闭', 'None'],
    minimal: ['最低', 'Minimal'],
    low: ['低', 'Low'],
    medium: ['中', 'Medium'],
    high: ['高', 'High'],
    xhigh: ['更高', 'Extra high'],
    max: ['最高', 'Maximum'],
};
export function effortLabel(effort, zh) {
    return effort ? effortLabels[effort][zh ? 0 : 1] : zh ? '默认' : 'Default';
}
/** Shared UI state machine; Runtime snapshots remain execution truth. */
export class ComposerSession {
    changed;
    error;
    settings = {};
    effortPreview = null;
    get displayedEffort() {
        return this.effortPreview ? this.effortPreview.value : this.settings.effort;
    }
    previewEffort(value) {
        if (!this.canConfigure ||
            (value !== null && !this.settings.supportedEfforts?.includes(value)))
            return;
        this.effortPreview = { value };
        this.changed();
    }
    clearEffortPreview() {
        if (this.effortPreview) {
            this.effortPreview = null;
            this.changed();
        }
    }
    loading = false;
    operation = null;
    awaiting = null;
    paused = false;
    epoch = 0;
    state;
    transport = null;
    controller = null;
    modelUid;
    externalBusy = false;
    constructor(changed, error) {
        this.changed = changed;
        this.error = error;
    }
    sync(state, transport, controller, modelUid, externalBusy = false) {
        this.externalBusy = externalBusy;
        const wasReady = this.ready;
        const conversationChanged = this.state?.conversationId !== state.conversationId ||
            this.transport !== transport ||
            (this.state?.connection === 'connected' &&
                state.connection !== 'connected');
        const contextChanged = conversationChanged ||
            this.controller !== controller ||
            this.modelUid !== modelUid;
        const newSnapshot = this.state !== state && state.snapshotReceived;
        this.state = state;
        this.transport = transport;
        this.controller = controller;
        this.modelUid = modelUid;
        if (contextChanged) {
            this.epoch++;
            this.settings = {};
            this.effortPreview = null;
            this.loading = false;
            this.operation = null;
            if (conversationChanged) {
                this.awaiting = null;
                this.paused = false;
            }
        }
        if ((contextChanged || !wasReady) && this.ready && controller)
            void this.load();
        if (newSnapshot && this.awaiting) {
            if (this.awaiting === 'pause' &&
                state.runtimeState === 'waiting' &&
                !state.awaitingAssistantResponse) {
                this.awaiting = null;
                this.paused = true;
            }
            else if (this.awaiting === 'resume' &&
                (state.runtimeState !== 'waiting' || !state.awaitingAssistantResponse)) {
                this.awaiting = null;
                this.paused = false;
                this.settings = { ...this.settings, canResume: false };
            }
        }
        if (state.runtimeState !== 'waiting') {
            this.paused = false;
            this.settings = { ...this.settings, canResume: false };
        }
    }
    get ready() {
        const s = this.state;
        return Boolean(s?.conversationId &&
            s.connection === 'connected' &&
            s.initialized &&
            s.snapshotReceived);
    }
    get canCompose() {
        return (this.ready &&
            this.state.runtimeState === 'waiting' &&
            !this.state.awaitingAssistantResponse &&
            !this.operation &&
            !this.externalBusy &&
            !this.awaiting &&
            !this.state.pendingPermissions.length);
    }
    get phase() {
        if (!this.ready)
            return this.state?.connection === 'connected'
                ? 'initializing'
                : 'offline';
        if (this.operation === 'pause' ||
            this.awaiting === 'pause' ||
            this.state.runtimeState === 'stopping')
            return 'pausing';
        if (this.operation === 'resume' || this.awaiting === 'resume')
            return 'resuming';
        if (this.state.runtimeState === 'compacting')
            return 'compacting';
        if (this.state.pendingPermissions.length)
            return 'approval';
        if (this.state.runtimeState !== 'waiting' ||
            this.state.awaitingAssistantResponse)
            return this.state.runtimeState === 'executing' ? 'executing' : 'thinking';
        return this.paused || this.settings.canResume ? 'paused' : 'ready';
    }
    get canPause() {
        return (this.ready &&
            !this.operation &&
            !this.awaiting &&
            Boolean(this.transport?.pause) &&
            ['thinking', 'executing', 'approval'].includes(this.phase));
    }
    get canResume() {
        return (this.canCompose &&
            this.phase === 'paused' &&
            Boolean(this.transport?.resume));
    }
    get canConfigure() {
        return this.canCompose && !this.loading;
    }
    request() {
        return {
            conversationId: this.state.conversationId,
            modelUid: this.modelUid,
        };
    }
    async load() {
        const controller = this.controller;
        if (!controller || !this.ready)
            return;
        const epoch = this.epoch;
        this.loading = true;
        this.changed();
        try {
            const settings = await controller.getSettings(this.request());
            if (epoch === this.epoch)
                this.settings = settings;
        }
        catch (error) {
            if (epoch === this.epoch)
                this.error(error instanceof Error ? error.message : String(error));
        }
        finally {
            if (epoch === this.epoch) {
                this.loading = false;
                this.changed();
            }
        }
    }
    async configure(kind, value) {
        if (!this.canConfigure || !this.controller)
            return;
        const request = this.request(), controller = this.controller, epoch = this.epoch;
        const operation = kind === 'effort'
            ? controller.setEffort &&
                (value === null ||
                    this.settings.supportedEfforts?.includes(value))
                ? () => controller.setEffort({
                    ...request,
                    effort: value,
                })
                : null
            : controller.setPermissionMode &&
                this.settings.permissionModes?.some((mode) => mode.id === value)
                ? () => controller.setPermissionMode({ ...request, mode: value })
                : null;
        if (!operation)
            return;
        this.operation = kind;
        this.changed();
        try {
            const result = await operation();
            if (!result.accepted)
                throw new Error(result.rejectReason ?? 'Settings change rejected.');
            if (epoch === this.epoch)
                await this.load();
        }
        catch (error) {
            if (epoch === this.epoch)
                this.error(error instanceof Error ? error.message : String(error));
        }
        finally {
            if (epoch === this.epoch) {
                this.operation = null;
                this.effortPreview = null;
                this.changed();
            }
        }
    }
    async command(kind) {
        if (kind === 'pause' ? !this.canPause : !this.canResume)
            return {
                accepted: false,
                rejectReason: 'Action is unavailable in the current state.',
            };
        const transport = this.transport, epoch = this.epoch, request = this.request();
        this.operation = kind;
        this.awaiting = kind;
        this.changed();
        try {
            const result = await transport[kind](request);
            if (epoch === this.epoch && !result.accepted)
                this.awaiting = null;
            return result;
        }
        catch (error) {
            if (epoch === this.epoch) {
                this.awaiting = null;
                this.error(error instanceof Error ? error.message : String(error));
            }
            return {
                accepted: false,
                rejectReason: error instanceof Error ? error.message : String(error),
            };
        }
        finally {
            if (epoch === this.epoch) {
                this.operation = null;
                this.changed();
            }
        }
    }
    sent() {
        this.paused = false;
        this.settings = { ...this.settings, canResume: false };
        this.changed();
    }
    dispose() {
        this.effortPreview = null;
        this.epoch++;
        this.operation = null;
        this.awaiting = null;
        this.loading = false;
    }
}
//# sourceMappingURL=composer.js.map
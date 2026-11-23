import { LitElement, nothing } from 'lit';
import type { ConversationHostCapabilities } from '../host/types.js';
import type { ExecutionPlan } from '../protocol/types.js';
import './rich-content.js';
/** A single, live plan projection. Step content is always rendered as text. */
export declare class AgentRuntimePlanCard extends LitElement {
    static properties: {
        plan: {
            attribute: boolean;
        };
        locale: {
            type: StringConstructor;
        };
        capabilities: {
            attribute: boolean;
        };
    };
    plan: ExecutionPlan | undefined;
    locale: string;
    capabilities: ConversationHostCapabilities | undefined;
    constructor();
    private expanded;
    private identity;
    static styles: import("lit").CSSResult;
    protected render(): import("lit-html").TemplateResult<1> | typeof nothing;
}
//# sourceMappingURL=plan-card.d.ts.map
import { LitElement, nothing } from 'lit';
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
    };
    plan: ExecutionPlan | undefined;
    locale: string;
    constructor();
    private expanded;
    private identity;
    static styles: import("lit").CSSResult;
    protected render(): import("lit-html").TemplateResult<1> | typeof nothing;
}
//# sourceMappingURL=plan-card.d.ts.map
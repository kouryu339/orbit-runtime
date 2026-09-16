import { LitElement, css, html, nothing } from 'lit';
import './rich-content.js';
/** A single, live plan projection. Step content is always rendered as text. */
export class AgentRuntimePlanCard extends LitElement {
    static properties = { plan: { attribute: false }, locale: { type: String } };
    constructor() { super(); this.locale = 'en'; }
    expanded = true;
    identity = '';
    static styles = css `
    :host { display: block; margin: 12px 0; color: inherit; font: inherit; }
    details { border: 1px solid color-mix(in srgb, currentColor 16%, transparent); border-radius: 12px; overflow: hidden; }
    summary { cursor: pointer; padding: 13px 16px; display: flex; align-items: center; gap: 12px; list-style: none; }
    summary::-webkit-details-marker { display: none; }
    summary:focus-visible { outline: 2px solid currentColor; outline-offset: -3px; }
    .heading { flex: 1; min-width: 0; }
    .title { font-size: 13px; font-weight: 600; overflow-wrap: anywhere; }
    .meta { font-size: 11px; opacity: .65; margin-top: 4px; }
    .chevron { font-size: 12px; transition: transform 120ms ease; }
    details[open] .chevron { transform: rotate(90deg); }
    progress { width: 40px; height: 4px; accent-color: #409678; }
    ol { list-style: none; padding: 0 16px 14px; margin: 0; }
    li { display: flex; align-items: baseline; gap: 10px; padding: 6px 0; font-size: 12px; line-height: 1.6; overflow-wrap: anywhere; }
    .mark { width: 16px; flex-shrink: 0; text-align: center; }
    li[data-status="completed"] { opacity: .6; }
    li[data-status="completed"] .text { text-decoration: line-through; }
    li[data-status="in_progress"] { font-weight: 600; }
    li[data-status="in_progress"] .mark { color: #409678; }
    .step-status { margin-left: auto; font-size: 10px; opacity: .65; white-space: nowrap; }
    .body { padding: 0 16px 14px; font-size: 12px; }
    @media (prefers-reduced-motion: reduce) { .chevron { transition: none; } }
  `;
    render() {
        const plan = this.plan;
        if (!plan)
            return nothing;
        const identity = plan.plan_id || plan.created_at || plan.title;
        if (identity !== this.identity) {
            this.identity = identity;
            this.expanded = plan.status === 'active';
        }
        const zh = this.locale.startsWith('zh');
        const labels = zh
            ? { active: '进行中', finished: '已完成', canceled: '已取消', pending: '待执行', in_progress: '进行中', completed: '已完成', blocked: '受阻' }
            : { active: 'In progress', finished: 'Completed', canceled: 'Canceled', pending: 'Pending', in_progress: 'In progress', completed: 'Completed', blocked: 'Blocked' };
        const steps = plan.steps ?? [];
        const completed = steps.filter(step => step.status === 'completed').length;
        return html `<details .open=${this.expanded} @toggle=${(event) => {
            this.expanded = event.target.open;
        }}>
      <summary aria-label=${zh ? '执行计划' : 'Execution plan'}>
        <span class="chevron" aria-hidden="true">›</span>
        <span class="heading">
          <span class="title">${plan.title}</span>
          <div class="meta" role="status">${labels[plan.status]}${steps.length ? ` · ${completed}/${steps.length}` : ''}</div>
        </span>
        ${steps.length ? html `<progress aria-label=${zh ? '计划完成进度' : 'Plan progress'} max=${steps.length} value=${completed}></progress>` : nothing}
      </summary>
      ${steps.length ? html `<ol>${steps.map(step => html `<li data-status=${step.status}>
        <span class="mark" aria-hidden="true">${({ completed: '✓', in_progress: '●', pending: '○', blocked: '!', canceled: '−' })[step.status]}</span>
        <span class="text">${step.text}</span>
        <span class="step-status">${labels[step.status]}</span>
      </li>`)}</ol>` : html `<div class="body"><agent-conversation-rich-content .content=${plan.content ?? ''}></agent-conversation-rich-content></div>`}
    </details>`;
    }
}
if (!customElements.get('agent-runtime-plan-card'))
    customElements.define('agent-runtime-plan-card', AgentRuntimePlanCard);
//# sourceMappingURL=plan-card.js.map
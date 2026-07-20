import { LitElement, css, html, nothing } from 'lit';
import { repeat } from 'lit/directives/repeat.js';
import { visiblePresentationItems } from '../content/presentation.js';
import { conversationReducer, createConversationState, createPendingUserMessage, displayText, recordKey, } from '../protocol/index.js';
import './rich-content.js';
function isVisibleGatewayRecord(record) {
    if (record.role !== 'gateway_message')
        return false;
    const subtype = record.metadata?.subtype;
    return subtype === 'llm_error' || subtype === 'interrupted';
}
export class AgentRuntimeConversationElement extends LitElement {
    static properties = {
        transport: { attribute: false },
        capabilities: { attribute: false },
        presentationItems: { attribute: false },
        extensions: { attribute: false },
        conversationId: { type: String, attribute: 'conversation-id' },
        locale: { type: String },
        colorScheme: { type: String, attribute: 'color-scheme', reflect: true },
        theme: { type: String, reflect: true },
        density: { type: String, reflect: true },
        persistence: { attribute: false },
        providerControls: { attribute: false },
        progressiveReveal: { type: Boolean, attribute: 'progressive-reveal' },
        hideToolCalls: { type: Boolean, attribute: 'hide-tool-calls' },
        disclaimer: { type: String },
        state: { state: true },
        draft: { state: true },
        historyOpen: { state: true },
        persistedConversations: { state: true },
        persistenceOperation: { state: true },
        providerPanelOpen: { state: true },
        providerEditorOpen: { state: true },
        providerDefinitions: { state: true },
        builtinProviderCatalog: { state: true },
        providerEditorProviderUid: { state: true },
        providerEditorBuiltinProviderId: { state: true },
        providerEditorProviderName: { state: true },
        providerEditorBaseUrl: { state: true },
        providerEditorApiParadigm: { state: true },
        providerEditorApiKey: { state: true },
        providerEditorModels: { state: true },
        providerOperation: { state: true },
        permissionOperation: { state: true },
    };
    static styles = css `
    :host {
      --conversation-accent: #c65f3c;
      --conversation-accent-contrast: #fffaf6;
      --conversation-surface: #f7f5f1;
      --conversation-surface-raised: #fffdf9;
      --conversation-text: #292722;
      --conversation-text-strong: #151411;
      --conversation-text-muted: #747069;
      --conversation-user-background: #292722;
      --conversation-user-text: #fffdf9;
      --conversation-border: #ded9d0;
      --conversation-code-background: #ebe7df;
      --conversation-tool-running: #b8862f;
      --conversation-tool-success: #4e8865;
      --conversation-tool-error: #bd5858;
      --conversation-font-display: "Aptos Display", "Segoe UI Variable Display", sans-serif;
      --conversation-font-body: "Aptos", "Segoe UI Variable Text", sans-serif;
      --conversation-font-mono: "Cascadia Code", "SFMono-Regular", monospace;
      --conversation-shell-radius: 10px;
      --conversation-shell-shadow: none;
      --conversation-composer-radius: 16px;
      --conversation-composer-shadow: 0 10px 24px color-mix(in srgb, #24342f 8%, transparent);
      --conversation-message-radius: 14px 14px 5px 14px;
      --conversation-assistant-rule: 2px solid color-mix(in srgb, var(--conversation-accent) 44%, var(--conversation-border));
      display: block;
      min-width: 0;
      height: 100%;
      color: var(--conversation-text);
    }

    :host([theme="studio"]),
    :host([theme="green"]) {
      --conversation-accent: #167d71;
      --conversation-accent-contrast: #ffffff;
      --conversation-surface: #fbfaf5;
      --conversation-surface-raised: #fffefa;
      --conversation-text: #304944;
      --conversation-text-strong: #173d36;
      --conversation-text-muted: #718781;
      --conversation-user-background: #245c53;
      --conversation-border: #d6e0dd;
      --conversation-code-background: #f0f3ef;
      --conversation-shell-radius: 8px;
      --conversation-composer-radius: 12px;
      --conversation-composer-shadow: 0 6px 18px rgba(35, 78, 69, 0.08);
      --conversation-message-radius: 12px 12px 4px 12px;
    }

    :host([theme="paper"]),
    :host([theme="amber"]) {
      --conversation-accent: #a64b2a;
      --conversation-accent-contrast: #fffaf3;
      --conversation-surface: #f4f0e7;
      --conversation-surface-raised: #fffcf5;
      --conversation-text: #39342d;
      --conversation-text-strong: #1f1c18;
      --conversation-text-muted: #7c7469;
      --conversation-user-background: #3b352e;
      --conversation-border: #d8d0c3;
      --conversation-code-background: #ebe4d8;
      --conversation-shell-radius: 3px;
      --conversation-composer-radius: 3px;
      --conversation-composer-shadow: 3px 3px 0 rgba(66, 52, 39, 0.12);
      --conversation-message-radius: 3px;
      --conversation-assistant-rule: 3px solid var(--conversation-accent);
    }

    :host([theme="soft"]),
    :host([theme="blue"]) { --conversation-accent: #3d72c5; }
    :host([theme="soft"]) {
      --conversation-accent: #5d63d6;
      --conversation-accent-contrast: #ffffff;
      --conversation-surface: #f4f5fb;
      --conversation-surface-raised: #ffffff;
      --conversation-text: #33364d;
      --conversation-text-strong: #202238;
      --conversation-text-muted: #767992;
      --conversation-user-background: #5d63d6;
      --conversation-border: #dfe1ef;
      --conversation-code-background: #eceefa;
      --conversation-shell-radius: 22px;
      --conversation-shell-shadow: 0 20px 50px rgba(43, 48, 99, 0.14);
      --conversation-composer-radius: 22px;
      --conversation-composer-shadow: 0 12px 30px rgba(52, 57, 111, 0.12);
      --conversation-message-radius: 18px 18px 5px 18px;
      --conversation-assistant-rule: 0 solid transparent;
    }

    :host([theme="midnight"]) {
      --conversation-accent: #73d7c7;
      --conversation-accent-contrast: #09201d;
      --conversation-surface: #101518;
      --conversation-surface-raised: #171e22;
      --conversation-text: #dce7e5;
      --conversation-text-strong: #f4fbfa;
      --conversation-text-muted: #8da09d;
      --conversation-user-background: #244741;
      --conversation-user-text: #f4fbfa;
      --conversation-border: #2b373b;
      --conversation-code-background: #0b1012;
      --conversation-shell-radius: 12px;
      --conversation-shell-shadow: 0 18px 50px rgba(0, 0, 0, 0.28);
      --conversation-composer-radius: 10px;
      --conversation-composer-shadow: 0 0 0 1px rgba(115, 215, 199, 0.04), 0 12px 28px rgba(0, 0, 0, 0.26);
      --conversation-message-radius: 8px;
      --conversation-assistant-rule: 1px solid #38504f;
    }

    :host([color-scheme="dark"]) {
      --conversation-surface: #171715;
      --conversation-surface-raised: #20201d;
      --conversation-text: #e9e6df;
      --conversation-text-strong: #fffdf8;
      --conversation-text-muted: #aaa69d;
      --conversation-user-background: #34322e;
      --conversation-user-text: #fffdf8;
      --conversation-border: #393833;
      --conversation-code-background: #11110f;
    }

    @media (prefers-color-scheme: dark) {
      :host([color-scheme="system"]) {
        --conversation-surface: #171715;
        --conversation-surface-raised: #20201d;
        --conversation-text: #e9e6df;
        --conversation-text-strong: #fffdf8;
        --conversation-text-muted: #aaa69d;
        --conversation-user-background: #34322e;
        --conversation-user-text: #fffdf8;
        --conversation-border: #393833;
        --conversation-code-background: #11110f;
      }
    }

    * { box-sizing: border-box; }
    .shell {
      position: relative;
      display: grid;
      grid-template-rows: auto minmax(0, 1fr) auto auto;
      height: 100%;
      min-height: 320px;
      overflow: hidden;
      border: 1px solid var(--conversation-border);
      border-radius: var(--conversation-shell-radius);
      background:
        linear-gradient(180deg, color-mix(in srgb, var(--conversation-accent) 3%, transparent), transparent 120px),
        var(--conversation-surface);
      box-shadow: var(--conversation-shell-shadow);
    }
    .header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
      min-width: 0;
      min-height: 48px;
      padding: 10px 14px;
      border-bottom: 1px solid var(--conversation-border);
    }
    .status {
      flex: 0 1 auto;
      min-width: 0;
    }
    .header-control-strip {
      display: flex;
      align-items: center;
      justify-content: flex-end;
      gap: 7px;
      flex: 1 1 auto;
      min-width: 0;
    }
    .header-actions {
      display: flex;
      align-items: center;
      gap: 7px;
      min-width: 0;
    }
    .header-actions[part="provider-actions"] {
      flex: 1 1 auto;
      justify-content: flex-end;
    }
    .header-actions[part="persistence-actions"] {
      flex: 0 0 auto;
    }
    .header-action {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      flex: 0 0 auto;
      min-width: 0;
      width: auto;
      max-width: 190px;
      height: 32px;
      padding: 0 10px;
      border: 1px solid var(--conversation-border);
      border-radius: 8px;
      background: var(--conversation-surface-raised);
      color: var(--conversation-text);
      font: 600 11px var(--conversation-font-body);
    }
    .header-action[data-tone="model"] {
      flex: 1 1 auto;
      max-width: min(260px, 100%);
    }
    .header-action span {
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .header-action[data-active="true"] {
      border-color: color-mix(in srgb, var(--conversation-accent) 45%, var(--conversation-border));
      background: color-mix(in srgb, var(--conversation-accent) 9%, var(--conversation-surface-raised));
      color: var(--conversation-text-strong);
    }
    .header-action[data-tone="saved"]::before,
    .header-action[data-tone="model"]::before,
    .header-action[data-tone="history"]::before,
    .header-action[data-tone="add"]::before {
      display: block;
      width: 6px;
      height: 6px;
      flex: 0 0 auto;
      border-radius: 50%;
      background: var(--conversation-text-muted);
      content: "";
    }
    .header-action[data-tone="saved"]::before { background: var(--conversation-tool-success); }
    .header-action[data-tone="model"]::before { background: var(--conversation-accent); }
    .header-action[data-tone="history"]::before { background: var(--conversation-tool-running); }
    .header-action[data-tone="add"]::before { background: var(--conversation-text-muted); }
    .provider-panel {
      position: absolute;
      z-index: 6;
      top: 47px;
      right: 12px;
      width: min(390px, calc(100% - 24px));
      overflow: auto;
      padding: 10px;
      border: 1px solid var(--conversation-border);
      border-radius: 10px;
      background: var(--conversation-surface-raised);
      box-shadow: 0 18px 44px color-mix(in srgb, #000 16%, transparent);
    }
    .provider-field {
      display: grid;
      gap: 6px;
    }
    .provider-label {
      color: var(--conversation-text-muted);
      font: 650 11px var(--conversation-font-body);
    }
    .provider-select {
      width: 100%;
      min-height: 34px;
      padding: 0 9px;
      border: 1px solid var(--conversation-border);
      border-radius: 7px;
      background: var(--conversation-surface);
      color: var(--conversation-text);
      font: 12px var(--conversation-font-body);
    }
    .provider-panel-actions {
      display: flex;
      justify-content: flex-end;
      gap: 7px;
      margin-top: 10px;
    }
    .provider-panel-actions button {
      width: auto;
      height: 32px;
      padding: 0 10px;
      border-radius: 7px;
      font: 650 11px var(--conversation-font-body);
    }
    .provider-list {
      display: grid;
      gap: 7px;
      margin-top: 10px;
    }
    .provider-list-item {
      display: grid;
      grid-template-columns: minmax(0, 1fr) auto;
      gap: 8px;
      align-items: center;
      padding: 8px;
      border: 1px solid color-mix(in srgb, var(--conversation-border) 70%, transparent);
      border-radius: 8px;
      background: color-mix(in srgb, var(--conversation-surface) 84%, transparent);
    }
    .provider-list-main {
      display: grid;
      gap: 2px;
      min-width: 0;
    }
    .provider-list-main strong,
    .provider-list-main span {
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .provider-list-main strong {
      color: var(--conversation-text-strong);
      font: 650 12px var(--conversation-font-body);
    }
    .provider-list-main span {
      color: var(--conversation-text-muted);
      font: 11px var(--conversation-font-body);
    }
    .provider-list-item button {
      width: auto;
      height: 30px;
      padding: 0 10px;
      border-radius: 7px;
      font: 650 11px var(--conversation-font-body);
    }
    .provider-empty {
      padding: 10px 4px;
      color: var(--conversation-text-muted);
      font: 12px var(--conversation-font-body);
    }
    .provider-file-input { display: none; }
    .provider-editor-backdrop {
      position: absolute;
      z-index: 13;
      inset: 48px 0 0;
      display: grid;
      place-items: center;
      padding: 18px;
      background: color-mix(in srgb, var(--conversation-surface) 74%, transparent);
      backdrop-filter: blur(3px);
    }
    .provider-editor {
      width: min(100%, 520px);
      max-height: min(620px, calc(100% - 24px));
      overflow: auto;
      padding: 18px;
      border: 1px solid var(--conversation-border);
      border-radius: 8px;
      background: var(--conversation-surface-raised);
      box-shadow: 0 22px 60px color-mix(in srgb, #000 20%, transparent);
    }
    .provider-editor h2 {
      margin: 0;
      color: var(--conversation-text-strong);
      font: 650 16px/1.35 var(--conversation-font-display);
    }
    .provider-editor-grid {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 12px;
      margin-top: 16px;
    }
    .provider-editor-field { display: grid; gap: 6px; min-width: 0; }
    .provider-editor-field.wide { grid-column: 1 / -1; }
    .provider-editor-field span {
      color: var(--conversation-text-muted);
      font: 650 11px var(--conversation-font-body);
    }
    .provider-editor-field input,
    .provider-editor-field select {
      width: 100%;
      min-width: 0;
      height: 36px;
      padding: 0 10px;
      border: 1px solid var(--conversation-border);
      border-radius: 7px;
      background: var(--conversation-surface);
      color: var(--conversation-text);
      font: 12px var(--conversation-font-body);
    }
    .provider-editor-models {
      grid-column: 1 / -1;
      display: grid;
      gap: 8px;
      margin-top: 2px;
    }
    .provider-editor-models-title {
      color: var(--conversation-text-muted);
      font: 650 11px var(--conversation-font-body);
    }
    .provider-editor-model-row {
      display: grid;
      grid-template-columns: minmax(0, 1fr) 132px 34px;
      gap: 8px;
      align-items: end;
    }
    .provider-editor-icon-button {
      width: 34px;
      min-width: 34px;
      height: 36px;
      padding: 0;
      border-radius: 7px;
    }
    .provider-editor-add-model {
      justify-self: start;
      width: auto;
      height: 32px;
      padding: 0 10px;
      border-radius: 7px;
      font: 650 11px var(--conversation-font-body);
    }
    .provider-editor-actions {
      display: flex;
      justify-content: flex-end;
      gap: 8px;
      margin-top: 18px;
    }
    .provider-editor-actions button {
      width: auto;
      min-width: 82px;
      height: 36px;
      padding: 0 14px;
    }
    @media (max-width: 560px) {
      .provider-editor-grid { grid-template-columns: 1fr; }
      .provider-editor-field.wide { grid-column: auto; }
      .provider-editor-models { grid-column: auto; }
      .provider-editor-model-row { grid-template-columns: minmax(0, 1fr); }
      .provider-editor-icon-button { width: 100%; }
    }
    .history-panel {
      position: absolute;
      z-index: 5;
      top: 47px;
      right: 12px;
      width: min(360px, calc(100% - 24px));
      max-height: min(440px, calc(100% - 72px));
      overflow: auto;
      padding: 8px;
      border: 1px solid var(--conversation-border);
      border-radius: 10px;
      background: var(--conversation-surface-raised);
      box-shadow: 0 18px 44px color-mix(in srgb, #000 16%, transparent);
    }
    .history-empty {
      padding: 24px 14px;
      color: var(--conversation-text-muted);
      text-align: center;
      font: 12px var(--conversation-font-body);
    }
    .history-item {
      display: grid;
      grid-template-columns: minmax(0, 1fr) auto;
      gap: 10px;
      align-items: center;
      width: 100%;
      padding: 10px;
      border: 1px solid transparent;
      border-radius: 8px;
    }
    .history-item[data-current="true"] {
      border-color: color-mix(in srgb, var(--conversation-accent) 28%, var(--conversation-border));
      background: color-mix(in srgb, var(--conversation-accent) 7%, transparent);
    }
    .history-copy { min-width: 0; }
    .history-title {
      overflow: hidden;
      color: var(--conversation-text-strong);
      font: 600 12px var(--conversation-font-body);
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .history-preview {
      margin-top: 3px;
      overflow: hidden;
      color: var(--conversation-text-muted);
      font: 11px var(--conversation-font-body);
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .history-meta {
      margin-top: 4px;
      color: var(--conversation-text-muted);
      font: 10px var(--conversation-font-body);
    }
    .history-item button {
      width: auto;
      height: 30px;
      padding: 0 10px;
      font-size: 11px;
    }
    .status {
      display: flex;
      align-items: center;
      gap: 8px;
      color: var(--conversation-text-muted);
      font: 600 11px var(--conversation-font-body);
      letter-spacing: 0.04em;
      text-transform: uppercase;
    }
    .status-dot {
      width: 7px;
      height: 7px;
      border-radius: 50%;
      background: var(--conversation-text-muted);
    }
    .status-dot[data-connected="true"] {
      background: var(--conversation-tool-success);
      box-shadow: 0 0 0 3px color-mix(in srgb, var(--conversation-tool-success) 16%, transparent);
    }
    .messages {
      overflow-y: auto;
      overscroll-behavior: contain;
      padding: 22px clamp(14px, 4vw, 34px) 30px;
      scroll-behavior: smooth;
    }
    .empty {
      display: grid;
      min-height: 100%;
      place-items: center;
      color: var(--conversation-text-muted);
      font: 13px var(--conversation-font-body);
    }
    .message {
      width: min(100%, 760px);
      margin: 0 auto 20px;
    }
    .message.user {
      display: flex;
      justify-content: flex-end;
    }
    .user-bubble {
      max-width: 72%;
      padding: 9px 12px;
      border-radius: var(--conversation-message-radius);
      background: var(--conversation-user-background);
      color: var(--conversation-user-text);
      font: 13px/1.55 var(--conversation-font-body);
      white-space: pre-wrap;
      overflow-wrap: anywhere;
    }
    .pending { opacity: 0.62; }
    .assistant {
      padding-left: 15px;
      border-left: var(--conversation-assistant-rule);
    }
    .waiting {
      display: flex;
      align-items: center;
      gap: 8px;
      color: var(--conversation-text-muted);
      font: 12px var(--conversation-font-body);
    }
    .pulse {
      width: 7px;
      height: 7px;
      border-radius: 50%;
      background: var(--conversation-accent);
      animation: pulse 1.4s ease-in-out infinite;
    }
    .error {
      width: min(100%, 760px);
      margin: 0 auto 16px;
      padding: 10px 12px;
      border: 1px solid color-mix(in srgb, var(--conversation-tool-error) 48%, var(--conversation-border));
      border-radius: 10px;
      color: var(--conversation-tool-error);
      font: 12px/1.5 var(--conversation-font-body);
    }
    .permission-shelf-wrap {
      position: relative;
      z-index: 4;
      padding: 0 12px;
      background: var(--conversation-surface);
    }
    .permission-shelf {
      width: min(100%, 800px);
      max-height: min(320px, 40vh);
      margin: 0 auto;
      overflow: auto;
      border: 1px solid color-mix(in srgb, var(--conversation-accent) 38%, var(--conversation-border));
      border-radius: 8px 8px 0 0;
      background: var(--conversation-surface-raised);
      box-shadow: 0 -8px 24px color-mix(in srgb, #000 10%, transparent);
    }
    .permission-shelf-header {
      display: flex;
      align-items: center;
      gap: 8px;
      min-height: 34px;
      padding: 7px 12px;
      border-bottom: 1px solid var(--conversation-border);
      color: var(--conversation-text-muted);
      font: 650 11px/1.4 var(--conversation-font-body);
    }
    .permission-shelf-header::before {
      content: '';
      width: 7px;
      height: 7px;
      border-radius: 50%;
      background: var(--conversation-accent);
      box-shadow: 0 0 0 3px color-mix(in srgb, var(--conversation-accent) 15%, transparent);
    }
    .permission-tools {
      display: grid;
    }
    .permission-tool {
      display: grid;
      grid-template-columns: minmax(0, 1fr) auto;
      gap: 12px;
      align-items: center;
      padding: 11px 12px;
    }
    .permission-tool + .permission-tool {
      border-top: 1px solid var(--conversation-border);
    }
    .permission-copy {
      min-width: 0;
    }
    .permission-tool-name {
      color: var(--conversation-text-strong);
      font: 650 13px var(--conversation-font-body);
    }
    .permission-effect {
      margin-top: 4px;
      color: var(--conversation-text-muted);
      font: 11px var(--conversation-font-body);
    }
    .permission-arguments {
      max-height: 100px;
      margin: 8px 0 0;
      overflow: auto;
      padding: 7px 8px;
      border-radius: 5px;
      background: var(--conversation-code-background);
      color: var(--conversation-text);
      font: 11px/1.5 var(--conversation-font-mono);
      white-space: pre-wrap;
      overflow-wrap: anywhere;
    }
    .permission-actions {
      display: flex;
      justify-content: flex-end;
      gap: 8px;
    }
    .permission-actions button {
      width: auto;
      min-width: 72px;
      height: 32px;
      padding: 0 12px;
      border-radius: 6px;
      font: 650 12px var(--conversation-font-body);
    }
    .permission-actions .deny {
      border: 1px solid var(--conversation-border);
      background: transparent;
      color: var(--conversation-text);
    }
    .disclaimer {
      margin-top: 12px;
      color: var(--conversation-text-muted);
      font: 11px/1.45 var(--conversation-font-body);
    }
    .composer-wrap {
      padding: 8px 12px 12px;
      background:
        linear-gradient(180deg, transparent, color-mix(in srgb, var(--conversation-surface-raised) 74%, transparent) 24%);
    }
    .composer {
      display: grid;
      grid-template-columns: minmax(0, 1fr) auto;
      align-items: center;
      gap: 10px;
      max-width: 800px;
      margin: 0 auto;
      min-height: 54px;
      padding: 6px 7px 6px 16px;
      border: 1px solid var(--conversation-border);
      border-radius: var(--conversation-composer-radius);
      background: var(--conversation-surface-raised);
      box-shadow: var(--conversation-composer-shadow);
      transition: border-color 140ms ease, box-shadow 140ms ease;
    }
    .composer:focus-within {
      border-color: color-mix(in srgb, var(--conversation-accent) 74%, var(--conversation-border));
      box-shadow:
        0 0 0 3px color-mix(in srgb, var(--conversation-accent) 14%, transparent),
        var(--conversation-composer-shadow);
    }
    textarea {
      width: 100%;
      min-height: 40px;
      max-height: 180px;
      resize: none;
      padding: 9px 0;
      border: 0;
      outline: none;
      background: transparent;
      color: var(--conversation-text);
      font: 14px/1.5 var(--conversation-font-body);
    }
    textarea::placeholder { color: var(--conversation-text-muted); }
    .actions { display: flex; align-items: center; gap: 5px; }
    button {
      display: inline-grid;
      place-items: center;
      width: 42px;
      height: 42px;
      padding: 0;
      border: 0;
      border-radius: calc(var(--conversation-composer-radius) - 5px);
      background: var(--conversation-accent);
      color: var(--conversation-accent-contrast);
      cursor: pointer;
      transition: transform 120ms ease, filter 120ms ease;
    }
    button.secondary {
      border: 1px solid var(--conversation-border);
      background: transparent;
      color: var(--conversation-text);
    }
    button:hover:not(:disabled) { filter: brightness(1.04); transform: translateY(-1px); }
    button:active:not(:disabled) { transform: translateY(0); }
    button:focus-visible {
      outline: 2px solid var(--conversation-accent);
      outline-offset: 2px;
    }
    button:disabled { opacity: 0.42; cursor: not-allowed; }
    button svg {
      width: 18px;
      height: 18px;
      fill: none;
      stroke: currentColor;
      stroke-linecap: round;
      stroke-linejoin: round;
      stroke-width: 1.8;
    }
    :host([density="compact"]) .messages { padding-top: 14px; }
    :host([density="compact"]) .message { margin-bottom: 13px; }
    @media (max-width: 560px) {
      .permission-tool { grid-template-columns: minmax(0, 1fr); }
      .permission-actions { justify-content: stretch; }
      .permission-actions button { flex: 1 1 0; }
    }
    @keyframes pulse { 50% { opacity: 0.28; transform: scale(0.76); } }
    @media (prefers-reduced-motion: reduce) {
      .messages { scroll-behavior: auto; }
      .pulse { animation: none; }
    }
  `;
    connection = null;
    connectAbort = null;
    localMessageSequence = 0;
    providerEditorModelSequence = 0;
    localPresetSequence = 0;
    stickToBottom = true;
    completedRevealKeys = new Set();
    revealContentLengths = new Map();
    persistenceUnsubscribe = null;
    persistenceController = null;
    constructor() {
        super();
        this.transport = null;
        this.capabilities = {};
        this.presentationItems = [];
        this.extensions = [];
        this.conversationId = null;
        this.locale = 'en-US';
        this.colorScheme = 'system';
        this.theme = 'studio';
        this.density = 'comfortable';
        this.persistence = { enabled: false };
        this.providerControls = { enabled: false };
        this.progressiveReveal = true;
        this.hideToolCalls = false;
        this.disclaimer = '';
        this.state = createConversationState();
        this.draft = '';
        this.historyOpen = false;
        this.persistedConversations = [];
        this.persistenceOperation = null;
        this.providerPanelOpen = false;
        this.providerEditorOpen = false;
        this.providerDefinitions = null;
        this.builtinProviderCatalog = null;
        this.providerEditorProviderUid = null;
        this.providerEditorBuiltinProviderId = '';
        this.providerEditorProviderName = '';
        this.providerEditorBaseUrl = '';
        this.providerEditorApiParadigm = 'openai_chat_completions';
        this.providerEditorApiKey = '';
        this.providerEditorModels = [this.createProviderEditorModelRow()];
        this.providerOperation = null;
        this.permissionOperation = null;
    }
    disconnectedCallback() {
        super.disconnectedCallback();
        this.persistenceUnsubscribe?.();
        this.persistenceUnsubscribe = null;
        this.disconnect();
    }
    async connect() {
        if (!this.transport)
            throw new Error('Conversation transport is not configured.');
        this.disconnect();
        // A reconnect to the same conversation must not carry transient running
        // flags from the previous connection into the new authoritative snapshot.
        // openConversation() already resets, but the host also reconnects in place
        // when its Runtime becomes healthy.
        this.dispatch({ type: 'reset', conversationId: this.conversationId });
        this.connectAbort = new AbortController();
        this.dispatch({ type: 'connection', state: 'connecting' });
        this.emitConnectionChange('connecting');
        try {
            this.connection = await this.transport.connect({
                conversationId: this.conversationId,
                locale: this.locale,
                signal: this.connectAbort.signal,
            }, {
                event: (event) => this.handleTransportEvent(event),
                connection: (connection) => {
                    this.dispatch({ type: 'connection', state: connection });
                    this.emitConnectionChange(connection);
                },
                error: (error) => this.emit('agent-conversation-error', error),
            });
        }
        catch (error) {
            this.dispatch({ type: 'connection', state: 'disconnected' });
            this.emit('agent-conversation-error', {
                code: 'transport-connect-failed',
                message: error instanceof Error ? error.message : String(error),
                recoverable: true,
                cause: error,
            });
            throw error;
        }
    }
    disconnect() {
        this.connectAbort?.abort();
        this.connectAbort = null;
        const connection = this.connection;
        this.connection = null;
        if (connection)
            void connection.disconnect();
        if (this.state.connection !== 'disconnected') {
            this.dispatch({ type: 'connection', state: 'disconnected' });
            this.emitConnectionChange('disconnected');
        }
    }
    async openConversation(conversationId) {
        this.disconnect();
        this.completedRevealKeys.clear();
        this.conversationId = conversationId;
        this.dispatch({ type: 'reset', conversationId });
        await this.connect();
    }
    async createConversation() {
        if (!this.persistence.enabled)
            return;
        if (!this.persistence.controller.create) {
            this.emit('agent-conversation-diagnostic', {
                code: 'conversation-create-unsupported',
                message: 'The host persistence controller does not support creating conversations.',
            });
            return;
        }
        this.persistenceOperation = 'create';
        try {
            const binding = await this.persistence.controller.create();
            this.persistence = { ...this.persistence, binding };
            this.historyOpen = false;
            await this.openConversation(binding.runtimeConversationId);
        }
        catch (error) {
            this.emitPersistenceError('create', error);
        }
        finally {
            this.persistenceOperation = null;
        }
    }
    async saveConversation() {
        if (!this.persistence.enabled || !this.state.conversationId)
            return null;
        if (this.state.runtimeState !== 'waiting') {
            this.emit('agent-conversation-diagnostic', {
                code: 'conversation-not-waiting',
                message: 'Conversation can only be saved while waiting.',
            });
            return null;
        }
        this.persistenceOperation = 'save';
        try {
            const archive = await this.persistence.controller.save({
                archiveId: this.persistence.binding?.archiveId,
                runtimeConversationId: this.state.conversationId,
            });
            this.persistence = {
                ...this.persistence,
                binding: {
                    archiveId: archive.archiveId,
                    runtimeConversationId: archive.runtimeConversationId ?? this.state.conversationId,
                },
            };
            this.upsertPersistedConversation(archive);
            return archive;
        }
        catch (error) {
            this.emitPersistenceError('save', error);
            return null;
        }
        finally {
            this.persistenceOperation = null;
        }
    }
    async restoreConversation(archiveId) {
        if (!this.persistence.enabled)
            return;
        this.persistenceOperation = `restore:${archiveId}`;
        try {
            const binding = await this.persistence.controller.restore({ archiveId });
            this.persistence = { ...this.persistence, binding };
            this.historyOpen = false;
            await this.openConversation(binding.runtimeConversationId);
        }
        catch (error) {
            this.emitPersistenceError('restore', error, archiveId);
        }
        finally {
            this.persistenceOperation = null;
        }
    }
    async refreshConversationHistory() {
        if (!this.persistence.enabled)
            return;
        const controller = this.persistence.controller;
        this.persistenceOperation = 'list';
        try {
            const page = await controller.list();
            if (this.persistence.enabled && this.persistence.controller === controller) {
                this.persistedConversations = page.items;
            }
        }
        catch (error) {
            this.emitPersistenceError('list', error);
        }
        finally {
            this.persistenceOperation = null;
        }
    }
    async send(content, options = {}) {
        const value = content.trim();
        if (!value)
            return { accepted: false, rejectReason: 'Message is empty.' };
        if (!this.transport)
            return { accepted: false, rejectReason: 'Transport is not configured.' };
        if (!this.state.conversationId)
            return { accepted: false, rejectReason: 'Conversation is not ready.' };
        if (!this.canCompose()) {
            return { accepted: false, rejectReason: 'Conversation is still running.' };
        }
        const clientMessageId = `local-user-${Date.now()}-${++this.localMessageSequence}`;
        this.dispatch({
            type: 'local-message-added',
            message: createPendingUserMessage(clientMessageId, value),
        });
        this.draft = '';
        this.resetComposerHeight();
        this.stickToBottom = true;
        try {
            const result = await this.transport.send({
                conversationId: this.state.conversationId,
                content: value,
                clientMessageId,
                metadata: { ...options.metadata, source: options.source },
                signal: options.signal,
            });
            this.dispatch(result.accepted
                ? { type: 'local-message-accepted', id: clientMessageId }
                : {
                    type: 'local-message-failed',
                    id: clientMessageId,
                    error: result.rejectReason ?? 'Message rejected.',
                });
            this.emit('agent-conversation-send', {
                conversationId: this.state.conversationId,
                clientMessageId,
                content: value,
                source: options.source,
                result,
            });
            return result;
        }
        catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            this.dispatch({ type: 'local-message-failed', id: clientMessageId, error: message });
            const result = { accepted: false, rejectReason: message };
            this.emit('agent-conversation-send', {
                conversationId: this.state.conversationId,
                clientMessageId,
                content: value,
                source: options.source,
                result,
            });
            return result;
        }
    }
    async pause() {
        const conversationId = this.state.conversationId;
        if (!conversationId || !this.transport?.pause) {
            return { accepted: false, rejectReason: 'Pause is not supported.' };
        }
        const result = await this.transport.pause({ conversationId });
        this.emit('agent-conversation-pause', { conversationId, result });
        return result;
    }
    async resolveToolPermission(toolCallId, decision) {
        const conversationId = this.state.conversationId;
        if (!conversationId || !this.transport?.resolveToolPermission) {
            return { accepted: false, rejectReason: 'Tool permission response is not supported.' };
        }
        this.permissionOperation = toolCallId;
        try {
            const result = await this.transport.resolveToolPermission({
                conversationId,
                toolCallId,
                decision,
            });
            this.emit('agent-conversation-tool-permission', {
                conversationId,
                toolCallId,
                decision,
                result,
            });
            return result;
        }
        catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            this.emit('agent-conversation-error', {
                code: 'tool-permission-response-failed',
                message,
                recoverable: true,
                cause: error,
                metadata: { toolCallId, decision },
            });
            return { accepted: false, rejectReason: message };
        }
        finally {
            this.permissionOperation = null;
        }
    }
    async closeConversation() {
        const conversationId = this.state.conversationId;
        if (conversationId && this.transport?.close) {
            await this.transport.close({ conversationId });
        }
        this.dispatch({ type: 'conversation-closed', conversationId: conversationId ?? undefined });
    }
    focusComposer() {
        this.renderRoot.querySelector('textarea')?.focus();
    }
    insertPresetMarkdown(preset) {
        const id = preset.id ?? `preset-${Date.now()}-${++this.localPresetSequence}`;
        const lastRecord = [...this.state.records]
            .reverse()
            .find((record) => record.role === 'user' || record.role === 'assistant');
        this.insertPresentationItem({
            contract: 'agent-conversation-presentation/v1',
            id,
            scope: preset.scope ?? 'preset-markdown',
            kind: 'assistant-markdown',
            anchor: lastRecord
                ? { type: 'after-record', recordId: recordKey(lastRecord) }
                : { type: 'tail' },
            content: preset.markdown,
            reveal: 'progressive',
            createdAt: preset.createdAt,
            metadata: {
                presetName: preset.name,
                baseDir: preset.baseDir,
            },
        });
        return id;
    }
    insertPresentationItem(item) {
        this.presentationItems = [...this.presentationItems, item];
    }
    updatePresentationItem(id, patch) {
        this.presentationItems = this.presentationItems.map((item) => item.id === id ? { ...item, ...patch } : item);
    }
    removePresentationItem(id) {
        this.presentationItems = this.presentationItems.filter((item) => item.id !== id);
    }
    clearPresentationItems(scope) {
        this.presentationItems = scope
            ? this.presentationItems.filter((item) => item.scope !== scope)
            : [];
    }
    willUpdate(changed) {
        if (changed.has('conversationId') &&
            this.conversationId !== null &&
            this.conversationId !== this.state.conversationId) {
            this.dispatch({ type: 'reset', conversationId: this.conversationId });
        }
        if (changed.has('persistence'))
            this.configurePersistenceController();
        if (changed.has('providerControls')) {
            this.providerPanelOpen = false;
            this.providerEditorOpen = false;
            this.providerDefinitions = null;
            this.builtinProviderCatalog = null;
            this.resetProviderEditorDraft();
            if (this.providerControls.enabled)
                void this.refreshProviderDefinitions();
        }
    }
    updated(changed) {
        if (this.stickToBottom && (changed.has('state') || changed.has('presentationItems'))) {
            requestAnimationFrame(() => {
                const messages = this.renderRoot.querySelector('.messages');
                if (messages)
                    messages.scrollTop = messages.scrollHeight;
            });
        }
        if (changed.has('presentationItems')) {
            const ids = new Set(this.presentationItems.map((item) => `presentation:${item.id}`));
            for (const key of this.completedRevealKeys) {
                if (key.startsWith('presentation:') && !ids.has(key)) {
                    this.completedRevealKeys.delete(key);
                }
            }
        }
        if (changed.has('state') || changed.has('presentationItems')) {
            this.refreshRevealTracking();
        }
    }
    render() {
        const displayItems = this.createDisplayItems();
        const scheme = this.resolvedColorScheme();
        const activityLabel = this.activityLabel();
        return html `<section class="shell" part="shell">
      <header class="header" part="header">
        <slot name="header">
          <div class="status">
            <span
              class="status-dot"
              data-connected=${String(this.state.connection === 'connected')}
            ></span>
            <span>${this.statusLabel()}</span>
          </div>
        </slot>
        <div class="header-control-strip">
          ${this.renderProviderActions()}
          ${this.renderPersistenceActions()}
        </div>
      </header>
      ${this.renderProviderPanel()}
      ${this.renderProviderEditor()}
      ${this.renderHistoryPanel()}
      <main
        class="messages"
        part="message-list"
        aria-live="polite"
        @scroll=${this.onScroll}
        @wheel=${() => { this.stickToBottom = false; }}
      >
        ${displayItems.length === 0
            ? html `<div class="empty"><slot name="empty-state">Start a conversation</slot></div>`
            : repeat(displayItems, (item) => item.key, (item, index) => this.renderDisplayItem(item, index, displayItems, scheme))}
        ${activityLabel
            ? html `<div class="message assistant waiting">
              <span class="pulse"></span><span>${activityLabel}</span>
            </div>`
            : nothing}
        ${this.state.lastError
            ? html `<div class="error" role="alert">${this.state.lastError}</div>`
            : nothing}
      </main>

      ${this.renderPermissionShelf()}

      <footer class="composer-wrap" part="composer">
        <div class="composer">
          <slot name="composer-prefix"></slot>
          <textarea
            rows="1"
            aria-label="Message"
            placeholder=${this.state.initialized ? 'Message the agent' : 'Waiting for conversation'}
            .value=${this.draft}
            ?disabled=${!this.canCompose()}
            @input=${this.onDraftInput}
            @keydown=${this.onComposerKeydown}
          ></textarea>
          <div class="actions">
            <slot name="composer-suffix"></slot>
            ${this.state.runtimeState === 'thinking' ||
            this.state.runtimeState === 'executing'
            ? html `<button
                  class="secondary"
                  type="button"
                  aria-label="Pause response"
                  title="Pause response"
                  @click=${() => void this.pause()}
                ><svg viewBox="0 0 24 24" aria-hidden="true">
                  <path d="M8 6v12M16 6v12"></path>
                </svg></button>`
            : nothing}
            <button
              type="button"
              aria-label="Send message"
              title="Send message"
              ?disabled=${!this.canCompose() || !this.draft.trim()}
              @click=${() => void this.send(this.draft)}
            ><svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="m5 12 14-7-4.5 14-3-5.5L5 12Z"></path>
              <path d="m11.5 13.5 3-3"></path>
            </svg></button>
          </div>
        </div>
      </footer>
    </section>`;
    }
    renderPersistenceActions() {
        if (!this.persistence.enabled)
            return nothing;
        const zh = this.locale.toLowerCase().startsWith('zh');
        const newLabel = zh ? '新会话' : 'New chat';
        const switchLabel = zh ? '切换' : 'Switch';
        const createDisabled = this.persistenceOperation !== null ||
            (Boolean(this.state.conversationId) && !this.persistence.binding?.archiveId);
        return html `<div class="header-actions" part="persistence-actions">
      <button
        class="header-action"
        data-tone="add"
        type="button"
        ?disabled=${createDisabled}
        title=${newLabel}
        @click=${() => void this.createConversation()}
      ><span>${this.persistenceOperation === 'create'
            ? (zh ? '创建中' : 'Creating')
            : newLabel}</span></button>
      ${this.persistence.showHistory !== false
            ? html `<button
            class="header-action"
            data-tone="history"
            data-active=${String(this.historyOpen)}
            type="button"
            aria-expanded=${String(this.historyOpen)}
            title=${switchLabel}
            @click=${() => {
                this.historyOpen = !this.historyOpen;
                if (this.historyOpen) {
                    this.providerPanelOpen = false;
                    void this.refreshConversationHistory();
                }
            }}
          ><span>${switchLabel}</span></button>`
            : nothing}
    </div>`;
    }
    renderProviderActions() {
        if (!this.providerControls.enabled)
            return nothing;
        const showModelSwitcher = this.providerControls.showModelSwitcher !== false;
        const showImport = this.providerControls.showImport !== false;
        const zh = this.locale.toLowerCase().startsWith('zh');
        if (!showModelSwitcher && !showImport)
            return nothing;
        const modelLabel = this.currentProviderModelLabel() ??
            (zh ? '未配置模型' : 'No model');
        const modelActionLabel = this.providerOperation === 'load'
            ? (zh ? '加载中' : 'Loading')
            : modelLabel;
        return html `<div class="header-actions" part="provider-actions">
      ${showModelSwitcher
            ? html `<button
            class="header-action"
            data-tone="model"
            data-active=${String(this.providerPanelOpen)}
            type="button"
            aria-expanded=${String(this.providerPanelOpen)}
            ?disabled=${this.providerOperation !== null}
            title=${zh ? `当前模型：${modelLabel}` : `Current model: ${modelLabel}`}
            @click=${() => {
                this.providerPanelOpen = !this.providerPanelOpen;
                if (this.providerPanelOpen) {
                    this.historyOpen = false;
                    if (!this.providerDefinitions)
                        void this.refreshProviderDefinitions();
                }
            }}
          ><span>${modelActionLabel}</span></button>`
            : nothing}
      ${showImport
            ? html `<button
            class="header-action"
            data-tone="add"
            type="button"
            ?disabled=${this.providerOperation !== null}
            title=${zh ? '添加厂商' : 'Add provider'}
            @click=${() => void this.openProviderEditor()}
          ><span>${this.providerOperation === 'import'
                ? (zh ? '添加中' : 'Adding')
                : (zh ? '添加厂商' : 'Add provider')}</span></button>`
            : nothing}
    </div>`;
    }
    renderProviderPanel() {
        if (!this.providerControls.enabled || !this.providerPanelOpen)
            return nothing;
        const zh = this.locale.toLowerCase().startsWith('zh');
        const models = this.providerModels();
        const providers = this.providerDefinitions?.providers ?? [];
        const current = this.currentProviderModelUid();
        return html `<aside class="provider-panel" part="provider-panel">
      ${models.length
            ? html `<label class="provider-field">
            <span class="provider-label">${zh ? '当前模型' : 'Current model'}</span>
            <select
              class="provider-select"
              ?disabled=${this.providerOperation !== null}
              .value=${current === null ? '' : String(current)}
              @change=${(event) => void this.selectProviderModel(event.target.value)}
            >
              ${models.map((model) => html `<option value=${String(model.uid)}>
                ${this.providerModelLabel(model)}
              </option>`)}
            </select>
          </label>
          ${providers.length
                ? html `<div class="provider-list">
                ${providers.map((provider) => html `
                  <div class="provider-list-item">
                    <div class="provider-list-main">
                      <strong>${provider.name ?? `Provider ${provider.uid}`}</strong>
                      <span>${this.providerModelsForProvider(provider.uid).length}
                        ${zh ? '个模型' : 'models'}</span>
                    </div>
                    <button
                      class="secondary"
                      type="button"
                      ?disabled=${this.providerOperation !== null}
                      @click=${() => void this.openProviderEditor(provider.uid)}
                    >${zh ? '编辑' : 'Edit'}</button>
                  </div>
                `)}
              </div>`
                : nothing}
          <div class="provider-panel-actions">
            <button
              class="secondary"
              type="button"
              ?disabled=${this.providerOperation !== null}
              @click=${() => void this.openProviderEditor()}
            >${zh ? '添加厂商' : 'Add provider'}</button>
            <button
              class="secondary"
              type="button"
              ?disabled=${this.providerOperation !== null}
              @click=${() => void this.refreshProviderDefinitions()}
            >${zh ? '刷新' : 'Refresh'}</button>
          </div>`
            : html `<div class="provider-empty">
            ${this.providerOperation === 'load'
                ? (zh ? '正在加载模型' : 'Loading models')
                : (zh ? '未配置厂商模型' : 'No provider models configured')}
            <div class="provider-panel-actions">
              <button
                class="secondary"
                type="button"
                ?disabled=${this.providerOperation !== null}
                @click=${() => void this.openProviderEditor()}
              >${zh ? '添加厂商' : 'Add provider'}</button>
            </div>
          </div>`}
    </aside>`;
    }
    renderProviderEditor() {
        if (!this.providerControls.enabled || !this.providerEditorOpen)
            return nothing;
        const zh = this.locale.toLowerCase().startsWith('zh');
        const providers = this.builtinProviderCatalog?.providers ?? [];
        const models = this.providerEditorBuiltinModels();
        const editing = this.providerEditorProviderUid !== null;
        const editingProvider = editing
            ? this.providerDefinitions?.providers?.find((provider) => provider.uid === this.providerEditorProviderUid)
            : null;
        const apiKeyAlreadySet = Boolean(editingProvider?.api_key_set ?? editingProvider?.apiKeySet);
        return html `<div class="provider-editor-backdrop">
      <form
        class="provider-editor"
        part="provider-editor"
        role="dialog"
        aria-modal="true"
        aria-labelledby="provider-editor-title"
        @submit=${this.submitProviderEditor}
      >
        <h2 id="provider-editor-title">${editing
            ? (zh ? '编辑模型厂商' : 'Edit model provider')
            : (zh ? '添加模型厂商' : 'Add model provider')}</h2>
        <div class="provider-editor-grid">
          <label class="provider-editor-field wide">
            <span>${zh ? '内置预设' : 'Built-in preset'}</span>
            <select
              name="builtin-provider"
              .value=${this.providerEditorBuiltinProviderId}
              @change=${this.selectProviderPreset}
            >
              <option value="">${zh ? '自定义厂商' : 'Custom provider'}</option>
              ${providers.map((provider) => html `<option value=${provider.id}>
                ${provider.name}
              </option>`)}
            </select>
          </label>
          <label class="provider-editor-field">
            <span>${zh ? '厂商名称' : 'Provider name'}</span>
            <input
              name="provider-name"
              required
              autocomplete="off"
              placeholder="OpenAI Compatible"
              .value=${this.providerEditorProviderName}
              @input=${(event) => {
            this.providerEditorProviderName = event.target.value;
        }}
            />
          </label>
          <label class="provider-editor-field">
            <span>${zh ? '接口协议' : 'API protocol'}</span>
            <select
              name="api-paradigm"
              .value=${this.providerEditorApiParadigm}
              @change=${(event) => {
            this.providerEditorApiParadigm = event.target.value;
        }}
            >
              <option value="openai_chat_completions">OpenAI Chat Completions</option>
              <option value="anthropic_messages">Anthropic Messages</option>
            </select>
          </label>
          <label class="provider-editor-field wide">
            <span>API Endpoint</span>
            <input
              name="base-url"
              type="url"
              required
              autocomplete="url"
              placeholder="https://api.example.com/v1"
              .value=${this.providerEditorBaseUrl}
              @input=${(event) => {
            this.providerEditorBaseUrl = event.target.value;
        }}
            />
          </label>
          <label class="provider-editor-field wide">
            <span>API Key</span>
            <input
              name="api-key"
              type="password"
              ?required=${!apiKeyAlreadySet}
              autocomplete="new-password"
              placeholder=${apiKeyAlreadySet
            ? (zh ? '留空则保持已有 Key' : 'Leave blank to keep the existing key')
            : 'sk-...'}
              .value=${this.providerEditorApiKey}
              @input=${(event) => {
            this.providerEditorApiKey = event.target.value;
        }}
            />
          </label>
          <div class="provider-editor-models">
            <div class="provider-editor-models-title">${zh ? '模型' : 'Models'}</div>
            <datalist id="provider-editor-model-presets">
              ${models.map((model) => html `<option value=${model.id}>
                ${this.builtinModelLabel(model)}
              </option>`)}
            </datalist>
            ${this.providerEditorModels.map((row) => html `
              <div class="provider-editor-model-row">
                <label class="provider-editor-field">
                  <span>${zh ? '模型 ID' : 'Model ID'}</span>
                  <input
                    name=${`model-id-${row.key}`}
                    list="provider-editor-model-presets"
                    required
                    autocomplete="off"
                    .value=${row.modelId}
                    placeholder="gpt-4.1"
                    @input=${(event) => this.updateProviderEditorModel(row.key, {
            modelId: event.target.value,
        })}
                  />
                </label>
                <label class="provider-editor-field">
                  <span>Context</span>
                  <input
                    name=${`model-context-${row.key}`}
                    type="number"
                    min="1"
                    step="1"
                    .value=${row.contextWindow === null ? '' : String(row.contextWindow)}
                    placeholder="128000"
                    @input=${(event) => this.updateProviderEditorModel(row.key, {
            contextWindow: this.parseOptionalPositiveInt(event.target.value),
        })}
                  />
                </label>
                <button
                  class="provider-editor-icon-button secondary"
                  type="button"
                  aria-label=${zh ? '删除模型' : 'Remove model'}
                  title=${zh ? '删除模型' : 'Remove model'}
                  ?disabled=${this.providerEditorModels.length <= 1 || this.providerOperation !== null}
                  @click=${() => this.removeProviderEditorModel(row.key)}
                >×</button>
              </div>
            `)}
            <button
              class="provider-editor-add-model secondary"
              type="button"
              ?disabled=${this.providerOperation !== null}
              @click=${() => this.addProviderEditorModel()}
            >${zh ? '添加模型' : 'Add model'}</button>
          </div>
        </div>
        <div class="provider-editor-actions">
          <button
            class="secondary"
            type="button"
            ?disabled=${this.providerOperation !== null}
            @click=${() => { this.providerEditorOpen = false; }}
          >${zh ? '取消' : 'Cancel'}</button>
          <button type="submit" ?disabled=${this.providerOperation !== null}>
            ${this.providerOperation === 'import'
            ? (editing ? (zh ? '正在保存' : 'Saving') : (zh ? '正在添加' : 'Adding'))
            : (editing ? (zh ? '保存变更' : 'Save changes') : (zh ? '添加厂商' : 'Add provider'))}
          </button>
        </div>
      </form>
    </div>`;
    }
    async openProviderEditor(providerUid = null) {
        this.providerPanelOpen = false;
        this.historyOpen = false;
        this.resetProviderEditorDraft(providerUid);
        this.providerEditorOpen = true;
        if (!this.builtinProviderCatalog && this.providerControls.enabled) {
            const loader = this.providerControls.controller.getBuiltinProviderCatalog;
            if (loader) {
                try {
                    this.builtinProviderCatalog = await loader();
                }
                catch (error) {
                    this.emitProviderError('catalog', error);
                }
            }
        }
        if (providerUid !== null)
            this.loadProviderEditorDraft(providerUid);
    }
    resetProviderEditorDraft(providerUid = null) {
        this.providerEditorProviderUid = providerUid;
        this.providerEditorBuiltinProviderId = '';
        this.providerEditorProviderName = '';
        this.providerEditorBaseUrl = '';
        this.providerEditorApiParadigm = 'openai_chat_completions';
        this.providerEditorApiKey = '';
        this.providerEditorModels = [this.createProviderEditorModelRow()];
    }
    loadProviderEditorDraft(providerUid) {
        const provider = this.providerDefinitions?.providers?.find((item) => item.uid === providerUid);
        if (!provider)
            return;
        const providerType = provider.type ?? provider.builtin_type ?? provider.builtinType ?? '';
        this.providerEditorBuiltinProviderId = this.builtinProviderById(providerType) ? providerType : '';
        this.providerEditorProviderName = provider.name ?? `Provider ${provider.uid}`;
        this.providerEditorBaseUrl = provider.base_url ?? provider.baseUrl ?? '';
        this.providerEditorApiParadigm =
            provider.api_paradigm ?? provider.apiParadigm ?? 'openai_chat_completions';
        this.providerEditorApiKey = '';
        const rows = this.providerModelsForProvider(providerUid).map((model) => this.createProviderEditorModelRow(model.model_id ?? model.modelId ?? model.model_name ?? model.modelName ?? model.name ?? '', model.context_window ?? model.contextWindow ?? null, model.uid));
        this.providerEditorModels = rows.length ? rows : [this.createProviderEditorModelRow()];
    }
    selectProviderPreset = (event) => {
        const providerId = event.target.value;
        this.providerEditorBuiltinProviderId = providerId;
        const provider = this.builtinProviderById(providerId);
        if (!provider) {
            this.providerEditorModels = [this.createProviderEditorModelRow()];
            return;
        }
        this.providerEditorProviderName = provider.name;
        this.providerEditorBaseUrl = provider.defaultBaseUrl ?? provider.default_base_url ?? '';
        this.providerEditorApiParadigm = this.providerApiParadigm(provider);
        const defaults = this.providerEditorBuiltinModels().filter((model) => model.default);
        const selectedModels = defaults.length ? defaults : this.providerEditorBuiltinModels().slice(0, 1);
        this.providerEditorModels = selectedModels.length
            ? selectedModels.map((model) => this.createProviderEditorModelRow(model.id, this.builtinModelContext(model)))
            : [this.createProviderEditorModelRow()];
    };
    createProviderEditorModelRow(modelId = '', contextWindow = null, uid = null) {
        this.providerEditorModelSequence += 1;
        return {
            key: String(this.providerEditorModelSequence),
            uid,
            modelId,
            contextWindow,
        };
    }
    addProviderEditorModel() {
        this.providerEditorModels = [
            ...this.providerEditorModels,
            this.createProviderEditorModelRow(),
        ];
    }
    removeProviderEditorModel(key) {
        if (this.providerEditorModels.length <= 1)
            return;
        this.providerEditorModels = this.providerEditorModels.filter((row) => row.key !== key);
    }
    updateProviderEditorModel(key, patch) {
        this.providerEditorModels = this.providerEditorModels.map((row) => {
            if (row.key !== key)
                return row;
            const next = { ...row, ...patch };
            if (patch.modelId !== undefined) {
                const model = this.builtinModelById(patch.modelId);
                if (model && patch.contextWindow === undefined) {
                    next.contextWindow = this.builtinModelContext(model);
                }
            }
            return next;
        });
    }
    providerEditorBuiltinModels() {
        const provider = this.builtinProviderById(this.providerEditorBuiltinProviderId);
        const models = this.builtinProviderCatalog?.models ?? [];
        if (!provider)
            return models.slice(0, 200);
        const prefix = provider.prefix ?? '';
        return models.filter((model) => (model.providerPrefix ?? model.provider_prefix ?? '') === prefix);
    }
    builtinProviderById(id) {
        if (!id)
            return undefined;
        return this.builtinProviderCatalog?.providers?.find((provider) => provider.id === id);
    }
    builtinModelById(id) {
        if (!id)
            return undefined;
        return this.builtinProviderCatalog?.models?.find((model) => model.id === id);
    }
    providerApiParadigm(provider) {
        const apiFormat = provider.apiFormat ?? provider.api_format;
        return apiFormat === 'anthropic' ? 'anthropic_messages' : 'openai_chat_completions';
    }
    builtinModelContext(model) {
        return model.contextWindow ?? model.context_window ?? null;
    }
    builtinModelLabel(model) {
        const context = this.builtinModelContext(model);
        return context ? `${model.name} · ${context}` : model.name;
    }
    parseOptionalPositiveInt(value) {
        const parsed = Number(value);
        return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : null;
    }
    providerEditorModelContext(row, rawValue) {
        const explicit = this.parseOptionalPositiveInt(rawValue);
        if (explicit !== null)
            return explicit;
        const preset = this.builtinModelById(row.modelId);
        return (preset ? this.builtinModelContext(preset) : row.contextWindow) ?? 128000;
    }
    async refreshProviderDefinitions() {
        if (!this.providerControls.enabled)
            return;
        this.providerOperation = 'load';
        try {
            this.providerDefinitions =
                await this.providerControls.controller.getProviderDefinitions();
            this.emit('agent-conversation-provider-action', {
                action: 'definitions-loaded',
                definitions: this.providerDefinitions,
            });
        }
        catch (error) {
            this.emitProviderError('load', error);
        }
        finally {
            this.providerOperation = null;
        }
    }
    async selectProviderModel(value) {
        if (!this.providerControls.enabled)
            return;
        const modelUid = Number(value);
        if (!Number.isFinite(modelUid))
            return;
        this.providerOperation = 'model';
        try {
            const result = await this.providerControls.controller.setCurrentModel({
                modelUid,
            });
            if (result.accepted === false) {
                throw new Error(result.rejectReason ?? 'Model switch was rejected');
            }
            this.providerDefinitions = {
                ...(this.providerDefinitions ?? {}),
                current_model_uid: modelUid,
            };
            this.emit('agent-conversation-provider-action', {
                action: 'model-selected',
                modelUid,
                result,
            });
        }
        catch (error) {
            this.emitProviderError('model', error);
        }
        finally {
            this.providerOperation = null;
        }
    }
    submitProviderEditor = (event) => {
        event.preventDefault();
        const form = event.currentTarget;
        const value = (name) => form.querySelector(`[name="${name}"]`)?.value.trim() ?? '';
        const editingProviderUid = this.providerEditorProviderUid;
        const providerUid = editingProviderUid ?? this.nextProviderUid();
        const usedModelUids = new Set(this.providerModels()
            .filter((model) => (model.provider_uid ?? model.providerUid) !== providerUid)
            .map((model) => model.uid));
        let nextModelUid = this.nextModelUid(usedModelUids);
        const enabledModels = this.providerEditorModels
            .map((row) => {
            const modelId = value(`model-id-${row.key}`);
            if (!modelId)
                return null;
            const uid = row.uid && !usedModelUids.has(row.uid)
                ? row.uid
                : nextModelUid;
            usedModelUids.add(uid);
            while (usedModelUids.has(nextModelUid))
                nextModelUid += 1;
            return {
                uid,
                model_id: modelId,
                max_context_tokens: this.providerEditorModelContext(row, value(`model-context-${row.key}`)),
            };
        })
            .filter((model) => model !== null);
        if (!enabledModels.length)
            return;
        const providerPreset = this.builtinProviderById(value('builtin-provider'));
        const currentProvider = this.providerDefinitions?.providers?.find((provider) => provider.uid === providerUid);
        const providerRegistration = {
            id: providerUid,
            name: value('provider-name'),
            type: providerPreset?.id ??
                currentProvider?.type ??
                currentProvider?.builtin_type ??
                currentProvider?.builtinType ??
                (value('api-paradigm') === 'anthropic_messages' ? 'claude' : 'openai'),
            api_key: value('api-key'),
            base_url: value('base-url'),
            api_paradigm: value('api-paradigm'),
            prompt_cache_control: currentProvider?.prompt_cache_control ?? currentProvider?.promptCacheControl,
            enabled_models: enabledModels,
        };
        const providers = [
            ...this.existingProviderRegistrationsExcept(providerUid),
            providerRegistration,
        ];
        const providerModelUids = new Set(providers.flatMap((provider) => provider.enabled_models.map((model) => model.uid)));
        const currentModelUid = this.currentProviderModelUid();
        const bundle = {
            schema: 'agent-runtime-llm-registration/v1',
            id: 'conversation-provider-editor',
            providers,
            current_model_uid: currentModelUid !== null && providerModelUids.has(currentModelUid)
                ? currentModelUid
                : enabledModels[0].uid,
        };
        void this.configureProviderInput(JSON.stringify(bundle), 'json')
            .then((configured) => {
            if (configured)
                this.providerEditorOpen = false;
        });
    };
    existingProviderRegistrationsExcept(providerUid) {
        const providers = this.providerDefinitions?.providers ?? [];
        return providers
            .filter((provider) => provider.uid !== providerUid)
            .map((provider) => this.providerRegistrationFromDefinition(provider))
            .filter((provider) => provider !== null);
    }
    providerRegistrationFromDefinition(provider) {
        const models = this.providerModelsForProvider(provider.uid)
            .map((model) => ({
            uid: model.uid,
            model_id: model.model_id ?? model.modelId ?? model.model_name ?? model.modelName ?? model.name ?? '',
            max_context_tokens: model.context_window ?? model.contextWindow ?? 128000,
        }))
            .filter((model) => model.model_id);
        if (!models.length)
            return null;
        const apiParadigm = provider.api_paradigm ?? provider.apiParadigm ?? 'openai_chat_completions';
        return {
            id: provider.uid,
            name: provider.name ?? `Provider ${provider.uid}`,
            type: provider.type ?? provider.builtin_type ?? provider.builtinType ??
                (apiParadigm === 'anthropic_messages' ? 'claude' : 'openai'),
            api_key: '',
            base_url: provider.base_url ?? provider.baseUrl ?? '',
            api_paradigm: apiParadigm,
            prompt_cache_control: provider.prompt_cache_control ?? provider.promptCacheControl,
            enabled_models: models,
        };
    }
    async configureProviderInput(input, source) {
        if (!this.providerControls.enabled)
            return false;
        this.providerOperation = 'import';
        try {
            const result = await this.providerControls.controller.configureProviders({
                input,
                source,
            });
            if (result.accepted === false) {
                throw new Error(result.rejectReason ?? 'Provider import was rejected');
            }
            await this.refreshProviderDefinitions();
            this.emit('agent-conversation-provider-action', {
                action: 'providers-imported',
                source,
                result,
            });
            return true;
        }
        catch (error) {
            this.emitProviderError('import', error);
            return false;
        }
        finally {
            if (this.providerOperation === 'import')
                this.providerOperation = null;
        }
    }
    providerModels() {
        return this.providerDefinitions?.models ?? [];
    }
    providerModelsForProvider(providerUid) {
        return this.providerModels().filter((model) => (model.provider_uid ?? model.providerUid) === providerUid);
    }
    nextProviderUid() {
        return Math.max(0, ...(this.providerDefinitions?.providers ?? []).map((provider) => provider.uid)) + 1;
    }
    nextModelUid(used = new Set()) {
        let uid = Math.max(1000, ...this.providerModels().map((model) => model.uid), ...used) + 1;
        while (used.has(uid))
            uid += 1;
        return uid;
    }
    currentProviderModelUid() {
        const definitions = this.providerDefinitions;
        return definitions?.current_model_uid ??
            definitions?.currentModelUid ??
            null;
    }
    currentProviderModelLabel() {
        const current = this.currentProviderModelUid();
        if (current === null)
            return null;
        const model = this.providerModels().find((item) => item.uid === current);
        return model ? this.providerModelLabel(model) : `Model ${current}`;
    }
    providerModelLabel(model) {
        const modelName = model.model_name ??
            model.modelName ??
            model.model_id ??
            model.modelId ??
            model.name ??
            `Model ${model.uid}`;
        const providerUid = model.provider_uid ?? model.providerUid;
        const provider = this.providerDefinitions?.providers?.find((item) => item.uid === providerUid);
        const providerName = provider?.name ?? (providerUid ? `Provider ${providerUid}` : '');
        return providerName ? `${modelName} / ${providerName}` : modelName;
    }
    isCurrentArchive(archive) {
        const binding = this.persistence.enabled ? this.persistence.binding : null;
        return Boolean((binding?.archiveId && binding.archiveId === archive.archiveId) ||
            (!binding?.archiveId &&
                archive.runtimeConversationId &&
                archive.runtimeConversationId === this.state.conversationId));
    }
    formatArchiveTime(value) {
        if (!value)
            return '';
        const date = /^\d+$/.test(value)
            ? new Date(Number(value))
            : new Date(value);
        if (Number.isNaN(date.getTime()))
            return value;
        return new Intl.DateTimeFormat(this.locale, {
            month: '2-digit',
            day: '2-digit',
            hour: '2-digit',
            minute: '2-digit',
        }).format(date);
    }
    emitProviderError(operation, error) {
        this.emit('agent-conversation-error', {
            code: `provider-${operation}-failed`,
            message: error instanceof Error ? error.message : String(error),
            recoverable: true,
            cause: error,
        });
    }
    renderPermissionShelf() {
        const permissions = this.state.pendingPermissions;
        if (!permissions.length)
            return nothing;
        const zh = this.locale.toLowerCase().startsWith('zh');
        return html `<div class="permission-shelf-wrap">
      <section
        class="permission-shelf"
        part="permission-shelf"
        role="region"
        aria-live="polite"
        aria-label=${zh ? '工具批准请求' : 'Tool approval requests'}
      >
        <div class="permission-shelf-header">
          ${permissions.length === 1
            ? (zh ? '等待批准' : 'Waiting for approval')
            : (zh ? `${permissions.length} 个请求等待批准` : `${permissions.length} requests waiting for approval`)}
        </div>
        <div class="permission-tools">
          ${permissions.map((permission) => this.renderShelfPermissionItem(permission))}
        </div>
      </section>
    </div>`;
    }
    renderShelfPermissionItem(permission) {
        const zh = this.locale.toLowerCase().startsWith('zh');
        const busy = this.permissionOperation === permission.tool_call_id;
        const effect = this.permissionEffectLabel(permission, zh);
        const argumentText = this.permissionArguments(permission);
        return html `<div class="permission-tool" data-call-id=${permission.tool_call_id}>
        <div class="permission-copy">
          <div class="permission-tool-name">
            ${permission.display_name || permission.tool_name}
          </div>
          <div class="permission-effect">${effect} · ${permission.tool_name}</div>
          ${argumentText
            ? html `<pre class="permission-arguments">${argumentText}</pre>`
            : nothing}
        </div>
          <div class="permission-actions">
            <button
              class="deny"
              type="button"
              ?disabled=${busy}
              @click=${() => void this.resolveToolPermission(permission.tool_call_id, 'deny')}
            >${zh ? '拒绝' : 'Deny'}</button>
            <button
              type="button"
              ?disabled=${busy}
              @click=${() => void this.resolveToolPermission(permission.tool_call_id, 'allow')}
            >${busy ? (zh ? '处理中' : 'Working') : (zh ? '允许' : 'Allow')}</button>
          </div>
        </div>`;
    }
    permissionEffectLabel(permission, zh) {
        return permission.effect === 'destructive'
            ? (zh ? '破坏性操作' : 'Destructive operation')
            : permission.effect === 'controlled_change'
                ? (zh ? '可控变更' : 'Controlled change')
                : (zh ? '只读操作' : 'Read-only operation');
    }
    permissionArguments(permission) {
        const entries = Object.entries(permission.arguments ?? {});
        if (!entries.length)
            return '';
        const value = Object.fromEntries(entries.slice(0, 12));
        const text = JSON.stringify(value, null, 2);
        return text.length > 2400 ? `${text.slice(0, 2400)}\n…` : text;
    }
    renderHistoryPanel() {
        if (!this.persistence.enabled || !this.historyOpen)
            return nothing;
        const zh = this.locale.toLowerCase().startsWith('zh');
        return html `<aside class="history-panel" part="history-panel">
      ${this.persistenceOperation === 'list'
            ? html `<div class="history-empty">${zh ? '正在加载会话' : 'Loading conversations'}</div>`
            : this.persistedConversations.length === 0
                ? html `<div class="history-empty">${zh ? '还没有会话' : 'No conversations'}</div>`
                : this.persistedConversations.map((archive) => {
                    const current = this.isCurrentArchive(archive);
                    return html `<div class="history-item" data-current=${String(current)}>
                <div class="history-copy">
                  <div class="history-title">${archive.title ?? (zh ? '未命名会话' : 'Untitled conversation')}</div>
                  <div class="history-preview">${archive.preview ?? archive.updatedAt ?? archive.archiveId}</div>
                  <div class="history-meta">
                    ${current || archive.instanceState === 'current'
                        ? (zh ? '当前会话' : 'Current conversation')
                        : archive.instanceState === 'background'
                            ? (zh ? '后台运行' : 'Running in background')
                            : (zh ? '仅数据' : 'Saved data')}
                    · ${this.formatArchiveTime(archive.updatedAt ?? archive.createdAt)}
                  </div>
                </div>
                <button
                  type="button"
                  ?disabled=${current || this.persistenceOperation !== null}
                  @click=${() => void this.restoreConversation(archive.archiveId)}
                >${this.persistenceOperation === `restore:${archive.archiveId}`
                        ? (zh ? '打开中' : 'Opening')
                        : current
                            ? (zh ? '当前' : 'Current')
                            : (zh ? '打开' : 'Open')}</button>
              </div>`;
                })}
    </aside>`;
    }
    renderDisplayItem(item, index, items, scheme) {
        if (item.kind === 'presentation') {
            if (item.item.kind !== 'assistant-markdown' && item.item.kind !== 'notice') {
                return nothing;
            }
            return html `<article class="message assistant" part="assistant-message">
        <agent-conversation-rich-content
          .content=${item.item.content ?? ''}
          .toolCalls=${this.state.toolCalls}
          .colorScheme=${scheme}
          .locale=${this.locale}
          .capabilities=${this.capabilities}
          .contentBaseDir=${typeof item.item.metadata?.baseDir === 'string'
                ? item.item.metadata.baseDir
                : undefined}
          .contentMetadata=${item.item.metadata}
          .hideToolCalls=${this.hideToolCalls}
          .reveal=${item.item.reveal === 'progressive' &&
                !this.completedRevealKeys.has(item.key)}
          .widgetsExpired=${true}
          @agent-content-reveal=${this.followReveal}
          @agent-content-reveal-complete=${() => this.completeReveal(item.key)}
        ></agent-conversation-rich-content>
      </article>`;
        }
        const record = item.record;
        if (record.role === 'user') {
            return html `<article class="message user" part="user-message">
        <div class="user-bubble">${displayText(record)}</div>
      </article>`;
        }
        if (record.role === 'gateway_message') {
            return html `<article class="message assistant" part="assistant-message">
        <agent-conversation-rich-content
          .content=${displayText(record)}
          .toolCalls=${this.state.toolCalls}
          .colorScheme=${scheme}
          .locale=${this.locale}
          .capabilities=${this.capabilities}
          .hideToolCalls=${this.hideToolCalls}
          .reveal=${false}
          .widgetsExpired=${true}
        ></agent-conversation-rich-content>
      </article>`;
        }
        if (record.role !== 'assistant')
            return nothing;
        const latestWidgetRecord = this.latestWidgetRecordKey(items);
        const latestAssistantKey = this.latestAssistantKey(items);
        const reveal = this.progressiveReveal &&
            latestAssistantKey === item.key &&
            !this.completedRevealKeys.has(item.key);
        const showDisclaimer = Boolean(this.disclaimer) &&
            latestAssistantKey === item.key &&
            this.state.runtimeState === 'waiting' &&
            (!reveal || this.completedRevealKeys.has(item.key));
        return html `<article class="message assistant" part="assistant-message">
      <agent-conversation-rich-content
        .content=${displayText(record)}
        .toolCalls=${this.state.toolCalls}
        .colorScheme=${scheme}
        .locale=${this.locale}
        .capabilities=${this.capabilities}
        .hideToolCalls=${this.hideToolCalls}
        .reveal=${reveal}
        .widgetsExpired=${latestWidgetRecord !== item.key}
        @agent-content-reveal=${this.followReveal}
        @agent-content-reveal-complete=${() => this.completeReveal(item.key)}
        @agent-widget-submit=${(event) => void this.send(event.detail.content, { source: 'widget' })}
      ></agent-conversation-rich-content>
      ${showDisclaimer
            ? html `<div class="disclaimer" part="disclaimer">${this.disclaimer}</div>`
            : nothing}
    </article>`;
    }
    createDisplayItems() {
        const records = this.state.records
            .filter((record) => record.role === 'user' ||
            record.role === 'assistant' ||
            isVisibleGatewayRecord(record))
            .map((record) => ({ kind: 'record', key: recordKey(record), record }));
        const items = [...records];
        const anchorInsertCounts = new Map();
        for (const presentation of visiblePresentationItems(this.presentationItems, this.completedRevealKeys)) {
            const presentationKey = `presentation:${presentation.id}`;
            const item = {
                kind: 'presentation',
                key: presentationKey,
                item: presentation,
            };
            if (presentation.anchor.type === 'head') {
                const offset = anchorInsertCounts.get('head') ?? 0;
                items.splice(offset, 0, item);
                anchorInsertCounts.set('head', offset + 1);
            }
            else if (presentation.anchor.type === 'tail') {
                items.push(item);
            }
            else {
                const anchor = presentation.anchor;
                const target = items.findIndex((current) => current.kind === 'record' &&
                    recordKey(current.record) === anchor.recordId);
                if (target < 0)
                    items.push(item);
                else {
                    const anchorKey = `${anchor.type}:${anchor.recordId}`;
                    const offset = anchorInsertCounts.get(anchorKey) ?? 0;
                    items.splice(anchor.type === 'before-record' ? target : target + 1 + offset, 0, item);
                    anchorInsertCounts.set(anchorKey, offset + 1);
                }
            }
        }
        for (const pending of this.state.pendingUserMessages) {
            items.push({
                kind: 'record',
                key: pending.id,
                record: {
                    record_id: pending.id,
                    role: 'user',
                    content: pending.state === 'failed'
                        ? `${pending.content}\n${pending.error ?? ''}`
                        : pending.content,
                },
            });
        }
        return items;
    }
    latestWidgetRecordKey(items) {
        for (let index = items.length - 1; index >= 0; index -= 1) {
            const item = items[index];
            if (item.kind === 'record' &&
                item.record.role === 'assistant' &&
                /\[(?:input:|select:|confirm\b)/.test(displayText(item.record))) {
                return item.key;
            }
        }
        return null;
    }
    latestAssistantKey(items) {
        for (let index = items.length - 1; index >= 0; index -= 1) {
            const item = items[index];
            if (item.kind === 'record' &&
                item.record.role === 'assistant') {
                return item.key;
            }
        }
        return null;
    }
    handleTransportEvent(event) {
        if (event.type === 'conversation-created') {
            this.conversationId = event.conversationId;
            this.dispatch({
                type: 'conversation-created',
                conversationId: event.conversationId,
                eventSeq: event.eventSeq,
            });
            this.emit('agent-conversation-ready', {
                conversationId: event.conversationId,
                state: this.state,
            });
            return;
        }
        if (event.type === 'conversation-closed') {
            this.dispatch({
                type: 'conversation-closed',
                conversationId: event.conversationId,
            });
            return;
        }
        if (event.type === 'state-snapshot') {
            if (event.conversationId &&
                this.state.conversationId &&
                event.conversationId !== this.state.conversationId)
                return;
            this.dispatch({
                type: 'snapshot',
                payload: event.payload,
                eventSeq: event.eventSeq,
            });
            if (event.payload.conversation_state === 'waiting' &&
                event.payload.ledger_records) {
                this.markRecordsRevealComplete(event.payload.ledger_records);
            }
            return;
        }
        if (event.type === 'pending-user-message') {
            if (this.state.conversationId &&
                event.conversationId !== this.state.conversationId)
                return;
            this.dispatch({
                type: 'local-message-added',
                message: createPendingUserMessage(event.messageId, event.content, event.createdAt),
            });
            return;
        }
        if (event.type === 'transport-extension') {
            this.emit('agent-conversation-extension-action', {
                extension: event.extension,
                action: 'transport-event',
            });
        }
    }
    dispatch(action) {
        const next = conversationReducer(this.state, action);
        if (next === this.state)
            return;
        this.state = next;
        this.emit('agent-conversation-state-change', {
            state: next,
            reason: action.type,
        });
    }
    emitConnectionChange(state) {
        this.emit('agent-conversation-connection-change', {
            transportId: this.transport?.id ?? 'unconfigured',
            state,
        });
    }
    emit(name, detail) {
        this.dispatchEvent(new CustomEvent(name, {
            bubbles: true,
            composed: true,
            detail,
        }));
    }
    onDraftInput(event) {
        const textarea = event.target;
        this.draft = textarea.value;
        textarea.style.height = '0px';
        textarea.style.height = `${Math.min(textarea.scrollHeight, 180)}px`;
    }
    resetComposerHeight() {
        requestAnimationFrame(() => {
            const textarea = this.renderRoot.querySelector('textarea');
            if (textarea)
                textarea.style.height = '';
        });
    }
    onComposerKeydown(event) {
        if (event.key !== 'Enter' ||
            event.shiftKey ||
            event.isComposing)
            return;
        event.preventDefault();
        if (this.canCompose() && this.draft.trim())
            void this.send(this.draft);
    }
    onScroll(event) {
        const element = event.currentTarget;
        this.stickToBottom =
            element.scrollHeight - element.scrollTop - element.clientHeight <= 40;
    }
    followReveal = () => {
        if (!this.stickToBottom)
            return;
        requestAnimationFrame(() => {
            const messages = this.renderRoot.querySelector('.messages');
            if (messages)
                messages.scrollTop = messages.scrollHeight;
        });
    };
    completeReveal(key) {
        if (this.completedRevealKeys.has(key))
            return;
        this.completedRevealKeys.add(key);
        this.requestUpdate();
        this.followReveal();
    }
    markRecordsRevealComplete(records) {
        for (const record of records) {
            if (record.role === 'assistant') {
                this.completedRevealKeys.add(recordKey(record));
            }
        }
    }
    refreshRevealTracking() {
        const current = new Map();
        for (const record of this.state.records) {
            if (record.role !== 'assistant')
                continue;
            current.set(recordKey(record), displayText(record).length);
        }
        for (const item of this.presentationItems) {
            current.set(`presentation:${item.id}`, item.content?.length ?? 0);
        }
        for (const [key, length] of current) {
            const previousLength = this.revealContentLengths.get(key);
            if (previousLength !== undefined && length > previousLength) {
                this.completedRevealKeys.delete(key);
            }
            this.revealContentLengths.set(key, length);
        }
        for (const key of this.revealContentLengths.keys()) {
            if (!current.has(key)) {
                this.revealContentLengths.delete(key);
                this.completedRevealKeys.delete(key);
            }
        }
    }
    statusLabel() {
        if (this.state.connection === 'reconnecting')
            return 'Reconnecting';
        if (this.state.connection === 'connecting')
            return 'Connecting';
        if (!this.state.initialized)
            return 'Initializing';
        return this.state.connection === 'connected' ? 'Connected' : 'Offline';
    }
    canCompose() {
        return this.state.initialized &&
            this.state.snapshotReceived &&
            !this.state.awaitingAssistantResponse &&
            this.state.runtimeState === 'waiting';
    }
    canSaveConversation() {
        return this.persistence.enabled &&
            Boolean(this.state.conversationId) &&
            this.state.connection === 'connected' &&
            this.state.snapshotReceived &&
            this.state.runtimeState === 'waiting' &&
            this.persistenceOperation === null;
    }
    configurePersistenceController() {
        const controller = this.persistence.enabled
            ? this.persistence.controller
            : null;
        if (controller === this.persistenceController)
            return;
        this.persistenceUnsubscribe?.();
        this.persistenceUnsubscribe = null;
        this.persistenceController = controller;
        this.persistedConversations = [];
        if (controller?.subscribe) {
            this.persistenceUnsubscribe = controller.subscribe((event) => this.handlePersistenceEvent(event));
        }
        if (controller && this.persistence.enabled && this.persistence.showHistory !== false) {
            void this.refreshConversationHistory();
        }
    }
    handlePersistenceEvent(event) {
        if (event.type === 'archive-updated') {
            this.upsertPersistedConversation(event.archive);
        }
        else if (event.type === 'archive-deleted') {
            this.persistedConversations = this.persistedConversations.filter((archive) => archive.archiveId !== event.archiveId);
        }
        else if (event.type === 'binding-changed' && this.persistence.enabled) {
            this.persistence = { ...this.persistence, binding: event.binding };
        }
        else if (event.type === 'error') {
            this.emit('agent-conversation-error', {
                code: `persistence-${event.operation}-failed`,
                message: event.message,
                recoverable: true,
                metadata: { archiveId: event.archiveId },
            });
        }
    }
    upsertPersistedConversation(archive) {
        this.persistedConversations = [
            archive,
            ...this.persistedConversations.filter((item) => item.archiveId !== archive.archiveId),
        ];
    }
    emitPersistenceError(operation, error, archiveId) {
        this.emit('agent-conversation-error', {
            code: `persistence-${operation}-failed`,
            message: error instanceof Error ? error.message : String(error),
            recoverable: true,
            cause: error,
            metadata: { archiveId },
        });
    }
    activityLabel() {
        if (this.state.runtimeState === 'compacting')
            return '总结中';
        if (this.state.runtimeState === 'waiting')
            return null;
        if (this.state.turnObservedAssistant)
            return null;
        return 'Loading';
    }
    resolvedColorScheme() {
        if (this.colorScheme !== 'system')
            return this.colorScheme;
        return globalThis.matchMedia?.('(prefers-color-scheme: dark)').matches
            ? 'dark'
            : 'light';
    }
}
if (!customElements.get('agent-runtime-conversation')) {
    customElements.define('agent-runtime-conversation', AgentRuntimeConversationElement);
}
//# sourceMappingURL=conversation-element.js.map
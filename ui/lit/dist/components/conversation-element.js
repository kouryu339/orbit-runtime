import { LitElement, css, html, nothing } from 'lit';
import { repeat } from 'lit/directives/repeat.js';
import { createDisplayItems, latestWidgetRecordKey, latestAssistantKey, } from '../content/conversation-view.js';
import { bindProviderModal, providerPresetModels, providerEditorValidation, providerTabKey, } from '../content/provider-editor.js';
import { referenceColor } from '../content/references.js';
import { ComposerSession, effortLabel } from '../content/composer.js';
import { conversationReducer, createConversationState, createPendingUserMessage, displayText, displayTextWithToolAnchors, recordKey, summarizePermissionTargets, snapshotRecords, } from '../protocol/index.js';
import './rich-content.js';
import './plan-card.js';
import './image-reference.js';
export class AgentRuntimeConversationElement extends LitElement {
    static properties = {
        transport: { attribute: false },
        capabilities: { attribute: false },
        composerController: { attribute: false },
        composerPanel: { state: true },
        composerPanelPosition: { state: true },
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
        providerEditorMode: { state: true },
        providerImportJson: { state: true },
        providerEditorError: { state: true },
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
        permissionError: { state: true },
        pendingImages: { state: true },
        imageSending: { state: true },
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
      --conversation-font-display:
        'Aptos Display', 'Segoe UI Variable Display', sans-serif;
      --conversation-font-body: 'Aptos', 'Segoe UI Variable Text', sans-serif;
      --conversation-font-mono: 'Cascadia Code', 'SFMono-Regular', monospace;
      --conversation-shell-radius: 10px;
      --conversation-shell-shadow: none;
      --conversation-composer-radius: 16px;
      --conversation-composer-shadow: 0 10px 24px
        color-mix(in srgb, #24342f 8%, transparent);
      --conversation-message-radius: 14px 14px 5px 14px;
      --conversation-assistant-rule: 2px solid
        color-mix(
          in srgb,
          var(--conversation-accent) 44%,
          var(--conversation-border)
        );
      display: block;
      min-width: 0;
      max-width: 100%;
      height: 100%;
      overflow-x: hidden;
      color: var(--conversation-text);
    }

    :host([theme='studio']),
    :host([theme='green']) {
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

    :host([theme='white']) {
      --conversation-accent: #202020;
      --conversation-accent-contrast: #ffffff;
      --conversation-surface: #ffffff;
      --conversation-surface-raised: #ffffff;
      --conversation-text: #242424;
      --conversation-text-strong: #111111;
      --conversation-text-muted: #666666;
      --conversation-user-background: #f1f1f1;
      --conversation-user-text: #242424;
      --conversation-border: #e5e5e5;
      --conversation-code-background: #f6f6f6;
      --conversation-composer-radius: 20px;
      --conversation-composer-shadow: none;
    }
    :host([theme='white']) .shell {
      background: var(--conversation-surface);
    }

    :host([theme='paper']),
    :host([theme='amber']) {
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

    :host([theme='soft']),
    :host([theme='blue']) {
      --conversation-accent: #3d72c5;
    }
    :host([theme='soft']) {
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

    :host([theme='midnight']) {
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
      --conversation-composer-shadow:
        0 0 0 1px rgba(115, 215, 199, 0.04), 0 12px 28px rgba(0, 0, 0, 0.26);
      --conversation-message-radius: 8px;
      --conversation-assistant-rule: 1px solid #38504f;
    }

    :host([color-scheme='dark']) {
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
      :host([color-scheme='system']) {
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

    * {
      box-sizing: border-box;
    }
    .shell {
      position: relative;
      display: grid;
      grid-template-rows: auto minmax(0, 1fr) auto auto;
      height: 100%;
      min-height: 320px;
      min-width: 0;
      max-width: 100%;
      overflow: hidden;
      border: 1px solid var(--conversation-border);
      border-radius: var(--conversation-shell-radius);
      background:
        linear-gradient(
          180deg,
          color-mix(in srgb, var(--conversation-accent) 3%, transparent),
          transparent 120px
        ),
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
    .header-actions[part='provider-actions'] {
      flex: 1 1 auto;
      justify-content: flex-end;
    }
    .header-actions[part='persistence-actions'] {
      flex: 0 0 auto;
    }
    .header-action {
      display: inline-flex;
      align-items: center;
      justify-content: center;
      gap: 7px;
      flex: 0 0 auto;
      min-width: 0;
      width: auto;
      max-width: 190px;
      height: 34px;
      padding: 0 10px;
      border: 0;
      border-radius: 8px;
      background: transparent;
      color: var(--conversation-text-muted);
      font: 500 13px/1.4 var(--conversation-font-body);
    }
    .header .header-action:hover:not(:disabled),
    .header .header-action[data-active='true'] {
      background: var(--conversation-code-background);
      color: var(--conversation-text-strong);
      transform: none;
    }
    .header-action span {
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .header .header-action svg {
      width: 17px;
      height: 17px;
      flex: none;
    }
    .header .header-action[data-tone='new'] {
      color: var(--conversation-text-strong);
    }
    .header-title {
      color: var(--conversation-text-strong);
      font: 600 15px/1.4 var(--conversation-font-body);
    }
    .connection-label {
      color: var(--conversation-text-muted);
      font: 400 12px/1.4 var(--conversation-font-body);
    }
    @media (max-width: 480px) {
      .header .header-action {
        padding: 0 7px;
        gap: 5px;
      }
      .connection-label {
        display: none;
      }
      .header-control-strip,
      .header-actions {
        gap: 2px;
      }
    }
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
      border: 1px solid
        color-mix(in srgb, var(--conversation-border) 70%, transparent);
      border-radius: 8px;
      background: color-mix(
        in srgb,
        var(--conversation-surface) 84%,
        transparent
      );
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
    .provider-file-input {
      display: none;
    }
    .provider-editor-backdrop {
      position: absolute;
      z-index: 30;
      inset: 0;
      display: grid;
      place-items: center;
      padding: 16px;
      background: color-mix(in srgb, #000 28%, transparent);
    }
    .provider-editor {
      display: grid;
      grid-template-rows: auto auto minmax(0, 1fr) auto auto;
      box-sizing: border-box;
      width: min(100%, 600px);
      max-height: 100%;
      min-height: 0;
      overflow: hidden;
      padding: 0;
      border: 1px solid var(--conversation-border);
      border-radius: 16px;
      background: var(--conversation-surface-raised);
      box-shadow: 0 18px 48px #0003;
      color: var(--conversation-text);
      font: 13px/1.5 var(--conversation-font-body);
    }
    .provider-editor-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 16px;
      padding: 18px 22px 12px;
    }
    .provider-editor h2 {
      margin: 0;
      color: var(--conversation-text-strong);
      font: 600 18px/1.4 var(--conversation-font-body);
    }
    .provider-editor .provider-editor-close {
      width: 32px;
      height: 32px;
      min-width: 32px;
      padding: 0;
      border: 0;
      background: transparent;
      color: var(--conversation-text-muted);
      border-radius: 8px;
      font-size: 22px;
    }
    .provider-editor-tabs {
      display: flex;
      gap: 6px;
      padding: 0 22px 12px;
      border-bottom: 1px solid var(--conversation-border);
    }
    .provider-editor .provider-editor-tab {
      width: auto;
      height: 32px;
      padding: 0 12px;
      border: 0;
      border-radius: 7px;
      background: transparent;
      color: var(--conversation-text-muted);
      font: 500 13px var(--conversation-font-body);
    }
    .provider-editor .provider-editor-tab[aria-selected='true'] {
      color: var(--conversation-text-strong);
      background: var(--conversation-code-background);
    }
    .provider-editor-body {
      display: block;
      min-height: 0;
      overflow: auto;
      padding: 18px 22px;
      margin: 0;
      border: 0;
      min-width: 0;
    }
    .provider-editor [hidden] {
      display: none !important;
    }
    .provider-editor-form {
      display: contents;
    }
    .provider-editor-import {
      display: grid;
      gap: 14px;
    }
    .provider-editor-import textarea {
      box-sizing: border-box;
      width: 100%;
      height: 220px;
      max-height: none;
      min-height: 140px;
      resize: vertical;
      padding: 12px;
      border: 1px solid var(--conversation-border);
      border-radius: 8px;
      background: var(--conversation-surface);
      color: var(--conversation-text);
      font: 12px/1.6 var(--conversation-font-mono);
    }
    .provider-editor .provider-import-file {
      width: auto;
      justify-self: start;
      height: 34px;
      padding: 0 12px;
    }
    .provider-editor-error {
      margin: 0;
      padding: 10px 22px;
      color: var(--conversation-tool-error);
      font: 13px/1.5 var(--conversation-font-body);
    }
    .provider-editor-grid {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 12px;
      margin-top: 0;
    }
    .provider-editor-field {
      display: grid;
      gap: 6px;
      min-width: 0;
    }
    .provider-editor-field.wide {
      grid-column: 1 / -1;
    }
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
      margin-top: 8px;
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
      padding: 12px;
      border: 1px solid var(--conversation-border);
      border-radius: 10px;
    }
    .provider-reasoning-settings {
      grid-column: 1 / -1;
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
      gap: 8px;
    }
    .provider-reasoning-settings > span {
      grid-column: 1 / -1;
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
      align-items: center;
      gap: 8px;
      padding: 14px 22px;
      border-top: 1px solid var(--conversation-border);
      background: var(--conversation-surface-raised);
    }
    .provider-editor .provider-editor-actions button {
      width: auto;
      min-width: 90px;
      height: 36px;
      padding: 0 14px;
      border-radius: 8px;
      font: 500 13px var(--conversation-font-body);
    }
    .provider-editor .provider-editor-actions .secondary {
      background: transparent;
      color: var(--conversation-text);
      border: 1px solid var(--conversation-border);
    }
    .provider-editor .provider-editor-add-model {
      display: inline-flex;
      align-items: center;
      gap: 6px;
    }
    @media (max-width: 560px) {
      .provider-editor-grid {
        grid-template-columns: 1fr;
      }
      .provider-editor-field.wide {
        grid-column: auto;
      }
      .provider-editor-models {
        grid-column: auto;
      }
      .provider-editor-model-row {
        grid-template-columns: minmax(0, 1fr) 34px;
      }
      .provider-editor-model-row > .provider-editor-field:first-child {
        grid-column: 1 / -1;
      }
      .provider-editor-icon-button {
        width: 34px;
      }
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
    .history-item[data-current='true'] {
      border-color: color-mix(
        in srgb,
        var(--conversation-accent) 28%,
        var(--conversation-border)
      );
      background: color-mix(
        in srgb,
        var(--conversation-accent) 7%,
        transparent
      );
    }
    .history-copy {
      min-width: 0;
    }
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
      align-items: baseline;
      gap: 10px;
      min-width: 0;
    }
    .messages {
      min-width: 0;
      max-width: 100%;
      overflow-x: hidden;
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
      min-width: 0;
      max-width: 100%;
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
    .pending {
      opacity: 0.62;
    }
    .assistant {
      min-width: 0;
      max-width: 100%;
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
      border: 1px solid
        color-mix(
          in srgb,
          var(--conversation-tool-error) 48%,
          var(--conversation-border)
        );
      border-radius: 10px;
      color: var(--conversation-tool-error);
      font: 12px/1.5 var(--conversation-font-body);
    }
    .permission-shelf-wrap {
      position: relative;
      z-index: 4;
      min-width: 0;
      max-width: 100%;
      overflow-x: hidden;
      padding: 0 12px;
      background: var(--conversation-surface);
    }
    .permission-shelf {
      width: auto;
      min-width: 0;
      max-width: 100%;
      margin: 0 auto;
      overflow: hidden;
      border: 1px solid
        color-mix(
          in srgb,
          var(--conversation-accent) 38%,
          var(--conversation-border)
        );
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
      box-shadow: 0 0 0 3px
        color-mix(in srgb, var(--conversation-accent) 15%, transparent);
    }
    .permission-tools {
      display: grid;
      min-width: 0;
    }
    .permission-tool {
      display: grid;
      grid-template-columns: minmax(0, 1fr) auto;
      gap: 12px;
      align-items: center;
      min-width: 0;
      max-width: 100%;
      padding: 11px 12px;
    }
    .permission-tool + .permission-tool {
      border-top: 1px solid var(--conversation-border);
    }
    .permission-copy {
      min-width: 0;
      max-width: 100%;
    }
    .permission-tool-name {
      color: var(--conversation-text-strong);
      font: 650 13px var(--conversation-font-body);
      overflow-wrap: anywhere;
      word-break: break-word;
    }
    .permission-effect {
      margin-top: 4px;
      color: var(--conversation-text-muted);
      font: 11px var(--conversation-font-body);
      overflow-wrap: anywhere;
      word-break: break-word;
    }
    .permission-arguments {
      display: grid;
      gap: 4px;
      max-width: 100%;
      margin: 8px 0 0;
      overflow: hidden;
      padding: 7px 8px;
      border-radius: 5px;
      background: var(--conversation-code-background);
      color: var(--conversation-text);
      font: 11px/1.5 var(--conversation-font-mono);
      overflow-wrap: anywhere;
      word-break: break-word;
    }
    .permission-argument {
      display: grid;
      grid-template-columns: minmax(64px, auto) minmax(0, 1fr);
      gap: 8px;
    }
    .permission-argument-label {
      color: var(--conversation-text-muted);
      text-transform: uppercase;
      letter-spacing: 0.04em;
    }
    .permission-error {
      margin-top: 8px;
      color: var(--conversation-tool-error);
      font: 11px/1.45 var(--conversation-font-body);
      overflow-wrap: anywhere;
      word-break: break-word;
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
      background: linear-gradient(
        180deg,
        transparent,
        color-mix(in srgb, var(--conversation-surface-raised) 74%, transparent)
          24%
      );
    }
    .composer {
      display: grid;
      grid-template-columns: minmax(0, 1fr);
      align-items: center;
      gap: 10px;
      max-width: 800px;
      margin: 0 auto;
      min-height: 54px;
      padding: 10px 12px 8px 16px;
      border: 1px solid var(--conversation-border);
      border-radius: var(--conversation-composer-radius);
      background: var(--conversation-surface-raised);
      box-shadow: var(--conversation-composer-shadow);
      transition:
        border-color 140ms ease,
        box-shadow 140ms ease;
    }
    .composer:focus-within {
      border-color: color-mix(
        in srgb,
        var(--conversation-accent) 74%,
        var(--conversation-border)
      );
      box-shadow:
        0 0 0 3px
          color-mix(in srgb, var(--conversation-accent) 14%, transparent),
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
    textarea::placeholder {
      color: var(--conversation-text-muted);
    }
    .image-input {
      display: none;
    }
    .image-attachments {
      grid-column: 1 / -1;
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
      padding: 8px 0;
    }
    .image-attachment {
      display: inline-flex;
      align-items: center;
      gap: 7px;
      max-width: 220px;
      padding: 5px 8px;
      border-radius: 9px;
      border: 1px solid var(--conversation-border);
      font: 12px/1.3 var(--conversation-font-body);
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .image-attachment img {
      width: 28px;
      height: 28px;
      object-fit: cover;
      border-radius: 5px;
    }
    .image-attachment .remove-image {
      width: 22px;
      height: 22px;
      flex: none;
    }
    .at-reference {
      display: inline;
      padding: 2px 5px;
      border: 1px solid var(--reference-color);
      border-radius: 5px;
      background: var(--reference-color);
      color: #fff;
      font: inherit;
      white-space: nowrap;
      box-decoration-break: clone;
    }
    .actions {
      display: flex;
      align-items: center;
      gap: 5px;
    }
    .composer-toolbar {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
      min-width: 0;
    }
    .composer-left,
    .composer-right {
      display: flex;
      align-items: center;
      gap: 6px;
      min-width: 0;
    }
    .composer-right {
      margin-left: auto;
    }
    .composer-toolbar .icon-button {
      width: 32px;
      height: 32px;
      border-radius: 50%;
      background: transparent;
      color: var(--conversation-text);
    }
    .composer-toolbar .primary-action {
      width: 34px;
      height: 34px;
      border-radius: 50%;
      flex: none;
    }
    .composer-toolbar .text-button {
      display: inline-flex;
      gap: 5px;
      width: auto;
      max-width: 100%;
      height: 32px;
      padding: 0 7px;
      background: transparent;
      color: var(--conversation-text);
      border-radius: 8px;
      font: 500 12px/1.4 var(--conversation-font-body);
    }
    .composer-toolbar .model-name {
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      max-width: 190px;
    }
    .composer-toolbar .effort-value {
      color: var(--conversation-text-muted);
      white-space: nowrap;
    }
    .composer-toolbar .text-button svg {
      width: 14px;
      height: 14px;
      flex: none;
    }
    .composer-state {
      max-width: 800px;
      width: 100%;
      margin: 0 auto;
      color: var(--conversation-text-muted);
      font: 11px/1.4 var(--conversation-font-body);
    }
    .composer-menu {
      position: fixed;
      z-index: 30;
      box-sizing: border-box;
      width: 300px;
      max-width: calc(100vw - 24px);
      overflow-y: auto;
      padding: 10px;
      border: 1px solid var(--conversation-border);
      border-radius: 14px;
      background: var(--conversation-surface-raised);
      color: var(--conversation-text);
      box-shadow: 0 8px 24px #0002;
      font: 13px/1.5 var(--conversation-font-body);
    }
    .composer-menu-title {
      margin: 3px 6px 8px;
      color: var(--conversation-text-muted);
      font-size: 12px;
    }
    .composer-menu .menu-item {
      display: flex;
      justify-content: space-between;
      gap: 10px;
      text-align: left;
      width: 100%;
      height: auto;
      min-height: 36px;
      padding: 8px;
      border-radius: 8px;
      background: transparent;
      color: inherit;
      font: inherit;
    }
    .composer-menu .menu-item[aria-checked='true'] {
      background: var(--conversation-code-background);
    }
    .composer-menu .menu-item:hover:not(:disabled) {
      background: var(--conversation-code-background);
      transform: none;
    }
    .composer-menu .menu-item small {
      display: block;
      color: var(--conversation-text-muted);
      font-size: 12px;
    }
    .reasoning-control {
      padding: 6px 6px 12px;
      margin-bottom: 8px;
      border-bottom: 1px solid var(--conversation-border);
    }
    .reasoning-label {
      display: flex;
      align-items: center;
      justify-content: space-between;
      margin-bottom: 8px;
    }
    .reasoning-control input[type='range'] {
      width: 100%;
      accent-color: var(--conversation-accent);
      cursor: pointer;
    }
    .reasoning-control button {
      width: auto;
      height: 26px;
      padding: 0 6px;
      font: inherit;
      background: transparent;
      color: var(--conversation-text-muted);
    }
    .action-spinner {
      animation: composer-spin 900ms linear infinite;
    }
    @keyframes composer-spin {
      to {
        transform: rotate(360deg);
      }
    }
    @media (prefers-reduced-motion: reduce) {
      .action-spinner {
        animation: none;
      }
    }
    @media (max-width: 480px) {
      .composer-toolbar .model-name {
        max-width: 120px;
      }
      .composer-toolbar {
        gap: 4px;
      }
      .composer-toolbar .text-button {
        padding: 0 3px;
      }
    }
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
      transition:
        transform 120ms ease,
        filter 120ms ease;
    }
    button.secondary {
      border: 1px solid var(--conversation-border);
      background: transparent;
      color: var(--conversation-text);
    }
    button:hover:not(:disabled) {
      filter: brightness(1.04);
      transform: translateY(-1px);
    }
    button:active:not(:disabled) {
      transform: translateY(0);
    }
    button:focus-visible {
      outline: 2px solid var(--conversation-accent);
      outline-offset: 2px;
    }
    button:disabled {
      opacity: 0.42;
      cursor: not-allowed;
    }
    button svg {
      width: 18px;
      height: 18px;
      fill: none;
      stroke: currentColor;
      stroke-linecap: round;
      stroke-linejoin: round;
      stroke-width: 1.8;
    }
    :host([density='compact']) .messages {
      padding-top: 14px;
    }
    :host([density='compact']) .message {
      margin-bottom: 13px;
    }
    @media (max-width: 560px) {
      .permission-tool {
        grid-template-columns: minmax(0, 1fr);
      }
      .permission-actions {
        justify-content: stretch;
      }
      .permission-actions button {
        flex: 1 1 0;
      }
    }
    @keyframes pulse {
      50% {
        opacity: 0.28;
        transform: scale(0.76);
      }
    }
    @media (prefers-reduced-motion: reduce) {
      .messages {
        scroll-behavior: auto;
      }
      .pulse {
        animation: none;
      }
    }
  `;
    composerSession = new ComposerSession(() => this.requestUpdate(), (message) => this.emit('agent-conversation-error', {
        code: 'composer-action-failed',
        message,
        recoverable: true,
    }));
    dismissComposerMenu = (event) => {
        if (!event
            .composedPath()
            .some((node) => node instanceof Element &&
            (node.classList.contains('composer-menu') ||
                node.matches('[aria-label="Permissions"],[aria-label="Model and reasoning"]'))))
            this.composerPanel = null;
    };
    escapeComposerMenu = (event) => {
        if (event.key === 'Escape')
            this.composerPanel = null;
    };
    closeComposerMenu = () => {
        this.composerPanel = null;
    };
    providerModalNode = null;
    providerEditorEpoch = 0;
    releaseProviderModal = null;
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
        this.providerEditorMode = 'manual';
        this.providerImportJson = '';
        this.providerEditorError = '';
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
        this.permissionError = null;
        this.pendingImages = [];
        this.imageSending = false;
        this.composerController = null;
        this.composerPanel = null;
        this.composerPanelPosition = '';
    }
    connectedCallback() {
        super.connectedCallback();
        window.addEventListener('pointerdown', this.dismissComposerMenu);
        window.addEventListener('keydown', this.escapeComposerMenu);
        window.addEventListener('resize', this.closeComposerMenu);
    }
    disconnectedCallback() {
        super.disconnectedCallback();
        window.removeEventListener('pointerdown', this.dismissComposerMenu);
        window.removeEventListener('keydown', this.escapeComposerMenu);
        window.removeEventListener('resize', this.closeComposerMenu);
        this.releaseProviderModal?.();
        this.releaseProviderModal = null;
        this.providerModalNode = null;
        this.clearPendingImages();
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
        this.composerSession.dispose();
        this.composerPanel = null;
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
        this.clearPendingImages();
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
            if (this.persistence.enabled &&
                this.persistence.controller === controller) {
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
        const images = [...this.pendingImages];
        const draft = (content || (images.length ? this.draft : content)).trim();
        if (!draft && !images.length)
            return { accepted: false, rejectReason: 'Message is empty.' };
        if (!this.transport)
            return { accepted: false, rejectReason: 'Transport is not configured.' };
        if (!this.state.conversationId)
            return { accepted: false, rejectReason: 'Conversation is not ready.' };
        if (!this.canCompose() || this.imageSending) {
            return {
                accepted: false,
                rejectReason: 'Conversation is still running.',
            };
        }
        const conversationId = this.state.conversationId;
        this.imageSending = true;
        let parts;
        let value = draft;
        try {
            if (images.length) {
                const importer = this.transport.imageInput;
                if (!importer)
                    throw new Error('Image input is not supported by this host.');
                const importedImages = new Map();
                for (const image of images) {
                    const imported = await importer.importImage({
                        conversationId,
                        file: image.file,
                    });
                    if (!imported.imageId)
                        throw new Error(`Image import returned no ID: ${image.file.name}`);
                    importedImages.set(image.number, imported.imageId);
                }
                parts = [];
                const used = new Set();
                const marker = /@图片(\d+)/g;
                let cursor = 0;
                let imageParts = 0;
                for (const match of draft.matchAll(marker)) {
                    const index = match.index ?? 0;
                    const number = Number(match[1]);
                    const imageId = importedImages.get(number);
                    if (!imageId)
                        continue;
                    if (index > cursor)
                        parts.push({ type: 'text', text: draft.slice(cursor, index) });
                    parts.push({ type: 'image', image_id: imageId });
                    imageParts += 1;
                    used.add(number);
                    cursor = index + match[0].length;
                }
                if (cursor < draft.length)
                    parts.push({ type: 'text', text: draft.slice(cursor) });
                for (const image of images) {
                    if (!used.has(image.number)) {
                        parts.push({
                            type: 'image',
                            image_id: importedImages.get(image.number),
                        });
                        imageParts += 1;
                    }
                }
                if (imageParts > 8)
                    throw new Error('At most 8 image references can be sent in one message.');
                value = parts
                    .filter((part) => part.type === 'text')
                    .map((part) => part.text)
                    .join('\n');
                if (!value.trim())
                    value = '[Image]';
            }
        }
        catch (error) {
            this.imageSending = false;
            const message = error instanceof Error ? error.message : String(error);
            this.emit('agent-conversation-error', {
                code: 'image-import-failed',
                message,
                recoverable: true,
            });
            return { accepted: false, rejectReason: message };
        }
        if (this.state.conversationId !== conversationId || !this.canCompose()) {
            this.imageSending = false;
            return {
                accepted: false,
                rejectReason: 'Conversation changed while importing images.',
            };
        }
        const clientMessageId = `local-user-${Date.now()}-${++this.localMessageSequence}`;
        this.dispatch({
            type: 'local-message-added',
            message: createPendingUserMessage(clientMessageId, value, undefined, parts),
        });
        this.resetComposerHeight();
        this.stickToBottom = true;
        try {
            const result = await this.transport.send({
                conversationId: this.state.conversationId,
                content: value,
                parts,
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
            if (result.accepted) {
                this.composerSession.sent();
                this.draft = '';
                this.clearPendingImages();
                this.resetComposerHeight();
            }
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
            this.dispatch({
                type: 'local-message-failed',
                id: clientMessageId,
                error: message,
            });
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
        finally {
            this.imageSending = false;
        }
    }
    async pause() {
        const conversationId = this.state.conversationId;
        if (!conversationId ||
            !this.transport?.pause ||
            this.permissionOperation !== null ||
            this.imageSending) {
            return { accepted: false, rejectReason: 'Pause is not supported.' };
        }
        const result = await this.composerSession.command('pause');
        this.emit('agent-conversation-pause', { conversationId, result });
        return result;
    }
    async resume() {
        const conversationId = this.state.conversationId;
        if (!conversationId)
            return { accepted: false, rejectReason: 'Conversation is not ready.' };
        const result = await this.composerSession.command('resume');
        this.emit('agent-conversation-resume', { conversationId, result });
        return result;
    }
    async resolveToolPermission(toolCallId, decision) {
        const conversationId = this.state.conversationId;
        if (this.permissionOperation !== null ||
            this.state.connection !== 'connected' ||
            this.composerSession.awaiting ||
            !this.state.pendingPermissions.some((p) => p.tool_call_id === toolCallId))
            return {
                accepted: false,
                rejectReason: 'Permission response is unavailable in the current state.',
            };
        if (!conversationId || !this.transport?.resolveToolPermission) {
            return {
                accepted: false,
                rejectReason: 'Tool permission response is not supported.',
            };
        }
        this.permissionOperation = toolCallId;
        this.permissionError = null;
        try {
            const result = await this.transport.resolveToolPermission({
                conversationId,
                toolCallId,
                decision,
            });
            if (result.accepted) {
                // Runtime snapshots remain authoritative; this only closes the visual
                // gap while the resolved snapshot travels over SSE.
                this.dispatch({ type: 'tool-permission-resolved', toolCallId });
            }
            else if (this.permissionIsNoLongerPending(result)) {
                // Timeout/cancellation is terminal. A delayed or missed snapshot must
                // not leave a dead approval card that the user can never dismiss.
                this.dispatch({ type: 'tool-permission-resolved', toolCallId });
                await this.transport.requestSnapshot?.({ conversationId });
            }
            else {
                this.permissionError = {
                    toolCallId,
                    message: result.rejectReason ?? 'Tool permission response was rejected.',
                };
            }
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
            this.permissionError = { toolCallId, message };
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
        this.dispatch({
            type: 'conversation-closed',
            conversationId: conversationId ?? undefined,
        });
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
        this.composerSession.sync(this.state, this.transport, this.composerController, this.state.modelUid, this.providerOperation === 'model');
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
        const modal = this.renderRoot.querySelector('.provider-editor');
        if (modal !== this.providerModalNode) {
            this.releaseProviderModal?.();
            this.releaseProviderModal = null;
            this.providerModalNode = modal;
            if (modal)
                this.releaseProviderModal = bindProviderModal(modal, {
                    close: () => this.closeProviderEditor(),
                    busy: () => this.providerOperation !== null,
                });
        }
        if (this.stickToBottom &&
            (changed.has('state') || changed.has('presentationItems'))) {
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
        const activityLabel = this.state.assistantStream?.content
            ? ''
            : this.activityLabel();
        return html `<section class="shell" part="shell">
      <header class="header" part="header">
        <slot name="header">
          <div class="status">
            <span class="header-title"
              >${this.locale.startsWith('zh') ? '对话' : 'Chat'}</span
            ><span class="connection-label" role="status"
              >${this.statusLabel()}</span
            >
          </div>
        </slot>
        <div class="header-control-strip">
          ${this.renderProviderActions()} ${this.renderPersistenceActions()}
        </div>
      </header>
      ${this.renderProviderPanel()} ${this.renderProviderEditor()}
      ${this.renderHistoryPanel()}
      <main
        class="messages"
        part="message-list"
        aria-live="polite"
        @scroll=${this.onScroll}
        @wheel=${() => {
            this.stickToBottom = false;
        }}
      >
        ${displayItems.length === 0
            ? html `<div class="empty">
              <slot name="empty-state">Start a conversation</slot>
            </div>`
            : repeat(displayItems, (item) => item.key, (item, index) => this.renderDisplayItem(item, index, displayItems, scheme))}
        ${this.state.plan
            ? html `<agent-runtime-plan-card
              .plan=${this.state.plan}
              .locale=${this.locale}
            ></agent-runtime-plan-card>`
            : nothing}
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
          ${this.pendingImages.length
            ? html `<div class="image-attachments" aria-label="Selected images">
                ${this.pendingImages.map((image) => html `<span class="image-attachment">
                      ${image.previewUrl
                ? html `<img src=${image.previewUrl} alt="" />`
                : nothing}
                      <span title=${image.file.name}>${image.file.name}</span>
                      <button
                        class="secondary remove-image"
                        type="button"
                        aria-label=${`Remove ${image.file.name}`}
                        ?disabled=${this.imageSending}
                        @click=${() => this.removePendingImage(image)}
                      >
                        ×
                      </button>
                    </span>`)}
              </div>`
            : nothing}
          <textarea
            rows="1"
            aria-label="Message"
            placeholder=${this.state.initialized
            ? 'Message the agent'
            : 'Waiting for conversation'}
            .value=${this.draft}
            ?disabled=${!this.canCompose() || this.imageSending}
            @input=${this.onDraftInput}
            @keydown=${this.onComposerKeydown}
            @paste=${this.onComposerPaste}
          ></textarea>
          ${this.renderComposerControls()}
        </div>
      </footer>
    </section>`;
    }
    renderComposerControls() {
        const zh = this.locale.startsWith('zh'), session = this.composerSession;
        const phase = this.imageSending ? 'sending' : session.phase;
        const busy = [
            'thinking',
            'executing',
            'approval',
            'pausing',
            'resuming',
            'compacting',
            'sending',
        ].includes(phase);
        const resume = phase === 'paused' && !this.draft.trim() && !this.pendingImages.length;
        const label = busy
            ? 'Pause response'
            : resume
                ? 'Continue response'
                : 'Send message';
        const disabled = this.permissionOperation !== null ||
            (busy
                ? !session.canPause || this.imageSending
                : resume
                    ? !session.canResume
                    : !this.canCompose() ||
                        (!this.draft.trim() && !this.pendingImages.length));
        const model = this.providerModels().find((m) => m.uid === this.currentConversationModelUid());
        const effort = session.settings.effort ?? model?.reasoning?.effort;
        const permission = session.settings.permissionModes?.find((m) => m.id === session.settings.permissionMode);
        const stateLabels = {
            offline: ['未连接', 'Offline'],
            initializing: ['正在连接会话', 'Initializing'],
            sending: ['正在发送', 'Sending'],
            thinking: ['正在思考', 'Thinking'],
            executing: ['正在执行', 'Executing'],
            approval: ['等待批准', 'Waiting for approval'],
            pausing: ['正在暂停', 'Pausing'],
            resuming: ['正在继续', 'Resuming'],
            compacting: ['正在整理上下文', 'Compacting'],
            paused: ['已暂停', 'Paused'],
        };
        return html `${stateLabels[phase]
            ? html `<div class="composer-state" role="status">
            ${stateLabels[phase][zh ? 0 : 1]}
          </div>`
            : nothing}
      <div class="composer-toolbar" data-state=${phase}>
        <div class="composer-left">
          ${this.transport?.imageInput
            ? html `<input
                  class="image-input"
                  type="file"
                  accept="image/png,image/jpeg,image/webp,image/gif"
                  multiple
                  @change=${this.onImageInput}
                />
                <button
                  class="icon-button"
                  type="button"
                  aria-label="Attach images"
                  title=${zh ? '添加图片' : 'Attach images'}
                  ?disabled=${!this.canCompose() ||
                this.imageSending ||
                this.pendingImages.length >= 8}
                  @click=${() => this.renderRoot
                .querySelector('.image-input')
                ?.click()}
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path d="M12 5v14M5 12h14" />
                  </svg>
                </button>`
            : nothing}
          <button
            class="text-button"
            type="button"
            aria-label="Permissions"
            aria-expanded=${String(this.composerPanel === 'permissions')}
            title=${zh ? '工具权限' : 'Tool permissions'}
            @click=${(event) => this.toggleComposerPanel('permissions', event)}
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="m12 3 8 4v5c0 4-3 7-8 9-5-2-8-5-8-9V7z" />
              <path d="m8 12 3 3 5-6" />
            </svg>
            <span
              >${permission?.label ?? (zh ? '权限' : 'Permissions')}${this.state
            .pendingPermissions.length
            ? ` · ${this.state.pendingPermissions.length}`
            : ''}</span
            >
          </button>
        </div>
        <div class="composer-right">
          <slot name="composer-suffix"></slot>
          ${this.providerControls.enabled &&
            this.providerControls.showModelSwitcher !== false
            ? html `<button
                class="text-button"
                data-tone="model"
                type="button"
                aria-label="Model and reasoning"
                aria-expanded=${String(this.composerPanel === 'model')}
                ?disabled=${!session.canConfigure ||
                this.providerOperation !== null ||
                this.imageSending}
                title=${this.currentProviderModelLabel() ?? ''}
                @click=${(event) => this.toggleComposerPanel('model', event)}
              >
                <span class="model-name"
                  >${model
                ? this.providerModelName(model)
                : zh
                    ? '选择模型'
                    : 'Select model'}</span
                ><span class="effort-value">${effortLabel(effort, zh)}</span
                ><svg viewBox="0 0 24 24" aria-hidden="true">
                  <path d="m8 10 4 4 4-4" />
                </svg>
              </button>`
            : nothing}
          <button
            class="primary-action"
            type="button"
            data-state=${phase}
            aria-label=${label}
            title=${label}
            ?disabled=${disabled}
            @click=${() => void (busy
            ? this.pause()
            : resume
                ? this.resume()
                : this.send(this.draft))}
          >
            <svg
              class=${['pausing', 'resuming', 'sending', 'compacting'].includes(phase)
            ? 'action-spinner'
            : ''}
              viewBox="0 0 24 24"
              aria-hidden="true"
            >
              ${['pausing', 'resuming', 'sending', 'compacting'].includes(phase)
            ? html `<path d="M20 12a8 8 0 1 1-8-8" />`
            : busy
                ? html `<rect
                      x="7"
                      y="7"
                      width="10"
                      height="10"
                      rx="1"
                      fill="currentColor"
                      stroke="none"
                    />`
                : resume
                    ? html `<path
                        d="m9 5 11 7-11 7z"
                        fill="currentColor"
                        stroke="none"
                      />`
                    : html `<path d="M12 19V5m-6 6 6-6 6 6" />`}
            </svg>
          </button>
        </div>
      </div>
      ${this.renderComposerMenu(model)}`;
    }
    toggleComposerPanel(panel, event) {
        this.composerSession.clearEffortPreview();
        if (this.composerPanel === panel) {
            this.composerPanel = null;
            return;
        }
        const rect = event.currentTarget.getBoundingClientRect();
        this.composerPanelPosition = `left:${Math.max(12, Math.min(rect.right - 300, window.innerWidth - 312))}px;bottom:${Math.max(12, window.innerHeight - rect.top + 8)}px;max-height:${Math.max(120, rect.top - 24)}px`;
        this.composerPanel = panel;
    }
    renderComposerMenu(model) {
        if (!this.composerPanel)
            return nothing;
        const zh = this.locale.startsWith('zh'), session = this.composerSession, settings = session.settings;
        const efforts = settings.supportedEfforts ?? [];
        const displayedEffort = session.displayedEffort;
        const index = displayedEffort ? efforts.indexOf(displayedEffort) : -1;
        return html `<aside
      class="composer-menu"
      style=${this.composerPanelPosition}
      aria-label=${this.composerPanel === 'model'
            ? 'Model settings'
            : 'Permission settings'}
      @keydown=${(event) => {
            if (event.key === 'Escape')
                this.composerPanel = null;
        }}
    >
      ${this.composerPanel === 'model'
            ? html ` <div class="reasoning-control">
              <div class="reasoning-label">
                <span
                  >${zh ? '推理强度' : 'Reasoning effort'} ·
                  ${effortLabel(displayedEffort, zh)}</span
                ><button
                  type="button"
                  aria-label="Reset reasoning"
                  ?disabled=${!session.canConfigure ||
                !this.composerController?.setEffort}
                  @click=${() => {
                session.clearEffortPreview();
                void session.configure('effort', null);
            }}
                >
                  ${zh ? '重置' : 'Reset'}
                </button>
              </div>
              <input
                type="range"
                aria-label="Reasoning effort"
                aria-valuetext=${effortLabel(displayedEffort, zh)}
                min="-1"
                max=${Math.max(-1, efforts.length - 1)}
                step="1"
                .value=${String(index)}
                ?disabled=${!session.canConfigure ||
                !efforts.length ||
                !this.composerController?.setEffort}
                @input=${(event) => session.previewEffort(efforts[Number(event.target.value)] ??
                null)}
                @change=${(event) => void session.configure('effort', efforts[Number(event.target.value)] ??
                null)}
              />
            </div>
            <div class="composer-menu-title">
              ${zh ? '选择模型' : 'Select model'}
            </div>
            <div role="menu" aria-label="Models">
              ${this.providerModels().map((m) => html `<button
                    class="menu-item"
                    type="button"
                    role="menuitemradio"
                    aria-checked=${String(m.uid === this.currentConversationModelUid())}
                    ?disabled=${!session.canConfigure ||
                this.providerOperation !== null}
                    @click=${() => void this.selectProviderModel(String(m.uid))}
                  >
                    <span>${this.providerModelName(m)}</span>${m.uid ===
                this.currentConversationModelUid()
                ? html `<span aria-hidden="true">✓</span>`
                : nothing}
                  </button>`)}
            </div>
            ${this.providerControls.enabled &&
                this.providerControls.showImport !== false
                ? html `<button
                  class="menu-item"
                  type="button"
                  @click=${() => {
                    this.composerPanel = null;
                    this.providerPanelOpen = true;
                    this.historyOpen = false;
                }}
                >
                  ${zh ? '管理厂商' : 'Manage providers'}
                </button>`
                : nothing}`
            : html `<div class="composer-menu-title">
              ${zh ? '工具权限' : 'Tool permissions'}
            </div>
            <div role="menu">
              ${settings.permissionModes?.length
                ? settings.permissionModes.map((mode) => html `<button
                        class="menu-item"
                        type="button"
                        role="menuitemradio"
                        aria-checked=${String(mode.id === settings.permissionMode)}
                        ?disabled=${!session.canConfigure ||
                    !this.composerController?.setPermissionMode}
                        @click=${() => void session.configure('permission', mode.id)}
                      >
                        <span
                          >${mode.label}${mode.description
                    ? html `<small>${mode.description}</small>`
                    : nothing}</span
                        >${mode.id === settings.permissionMode
                    ? html `<span aria-hidden="true">✓</span>`
                    : nothing}
                      </button>`)
                : html `<div class="composer-menu-title">
                    ${zh ? '由宿主管理权限' : 'Permissions managed by host'}
                  </div>`}
            </div>
            ${this.state.pendingPermissions.length
                ? html `<button
                  class="menu-item"
                  type="button"
                  @click=${() => {
                    this.composerPanel = null;
                    this.renderRoot
                        .querySelector('.permission-shelf')
                        ?.scrollIntoView?.({ block: 'nearest' });
                }}
                >
                  ${zh ? '查看待批准请求' : 'View pending approvals'} ·
                  ${this.state.pendingPermissions.length}
                </button>`
                : nothing}`}
    </aside>`;
    }
    renderPersistenceActions() {
        if (!this.persistence.enabled)
            return nothing;
        const zh = this.locale.toLowerCase().startsWith('zh');
        const newLabel = zh ? '新会话' : 'New chat';
        const switchLabel = zh ? '历史会话' : 'History';
        const createDisabled = this.persistenceOperation !== null || !this.persistence.controller.create;
        return html `<div class="header-actions" part="persistence-actions">
      <button
        class="header-action"
        data-tone="new"
        type="button"
        ?disabled=${createDisabled}
        title=${newLabel}
        @click=${() => void this.createConversation()}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M12 5v14M5 12h14" /></svg
        ><span
          >${this.persistenceOperation === 'create'
            ? zh
                ? '创建中'
                : 'Creating'
            : newLabel}</span
        >
      </button>
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
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M3 11a9 9 0 1 1 2.6 7M3 5v6h6M12 7v5l3 2" /></svg
            ><span>${switchLabel}</span>
          </button>`
            : nothing}
    </div>`;
    }
    renderProviderActions() {
        if (!this.providerControls.enabled ||
            this.providerControls.showImport === false)
            return nothing;
        const zh = this.locale.startsWith('zh');
        return html `<div class="header-actions" part="provider-actions">
      <button
        class="header-action"
        data-tone="add"
        type="button"
        ?disabled=${this.providerOperation !== null}
        @click=${() => void this.openProviderEditor()}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="m12 3 8 4-8 4-8-4zM4 12l8 4 8-4M4 17l8 4 8-4" /></svg
        ><span
          >${this.providerOperation === 'import'
            ? zh
                ? '添加中'
                : 'Adding'
            : zh
                ? '添加厂商'
                : 'Add provider'}</span
        >
      </button>
    </div>`;
    }
    renderProviderPanel() {
        if (!this.providerControls.enabled || !this.providerPanelOpen)
            return nothing;
        const zh = this.locale.toLowerCase().startsWith('zh');
        const models = this.providerModels();
        const providers = this.providerDefinitions?.providers ?? [];
        const current = this.currentConversationModelUid();
        return html `<aside class="provider-panel" part="provider-panel">
      ${models.length
            ? html `<label class="provider-field">
              <span class="provider-label"
                >${zh ? '当前模型' : 'Current model'}</span
              >
              <select
                class="provider-select"
                .value=${current === null ? '' : String(current)}
                ?disabled=${this.providerOperation !== null ||
                !this.composerSession.canConfigure}
                @change=${(event) => void this.selectProviderModel(event.target.value)}
              >
                ${models.map((model) => html `<option
                      value=${String(model.uid)}
                      ?selected=${model.uid === current}
                    >
                      ${this.providerModelLabel(model)}
                    </option>`)}
              </select>
            </label>
            ${providers.length
                ? html `<div class="provider-list">
                  ${providers.map((provider) => html `
                      <div class="provider-list-item">
                        <div class="provider-list-main">
                          <strong
                            >${provider.name ??
                    `Provider ${provider.uid}`}</strong
                          >
                          <span
                            >${this.providerModelsForProvider(provider.uid)
                    .length}
                            ${zh ? '个模型' : 'models'}</span
                          >
                        </div>
                        <button
                          class="secondary"
                          type="button"
                          ?disabled=${this.providerOperation !== null}
                          @click=${() => void this.openProviderEditor(provider.uid)}
                        >
                          ${zh ? '编辑' : 'Edit'}
                        </button>
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
              >
                ${zh ? '添加厂商' : 'Add provider'}
              </button>
              <button
                class="secondary"
                type="button"
                ?disabled=${this.providerOperation !== null}
                @click=${() => void this.refreshProviderDefinitions()}
              >
                ${zh ? '刷新' : 'Refresh'}
              </button>
            </div>`
            : html `<div class="provider-empty">
            ${this.providerOperation === 'load'
                ? zh
                    ? '正在加载模型'
                    : 'Loading models'
                : zh
                    ? '未配置厂商模型'
                    : 'No provider models configured'}
            <div class="provider-panel-actions">
              <button
                class="secondary"
                type="button"
                ?disabled=${this.providerOperation !== null}
                @click=${() => void this.openProviderEditor()}
              >
                ${zh ? '添加厂商' : 'Add provider'}
              </button>
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
      <div
        class="provider-editor"
        part="provider-editor"
        role="dialog"
        aria-modal="true"
        aria-labelledby="provider-editor-title"
        tabindex="-1"
        aria-busy=${String(this.providerOperation !== null)}
      >
        <form class="provider-editor-form" @submit=${this.submitProviderEditor}>
          <header class="provider-editor-header">
            <h2 id="provider-editor-title">
              ${editing
            ? zh
                ? '编辑模型厂商'
                : 'Edit model provider'
            : zh
                ? '添加模型厂商'
                : 'Add model provider'}
            </h2>
            <button
              class="provider-editor-close"
              type="button"
              aria-label=${zh ? '关闭厂商配置' : 'Close provider settings'}
              ?disabled=${this.providerOperation !== null}
              @click=${() => this.closeProviderEditor()}
            >
              ×
            </button>
          </header>
          <div
            class="provider-editor-tabs"
            role="tablist"
            aria-label=${zh ? '配置方式' : 'Configuration mode'}
          >
            ${['manual', 'import'].map((mode) => html `<button
                  class="provider-editor-tab"
                  type="button"
                  role="tab"
                  tabindex=${this.providerEditorMode === mode ? 0 : -1}
                  @keydown=${providerTabKey}
                  aria-selected=${String(this.providerEditorMode === mode)}
                  ?disabled=${this.providerOperation !== null}
                  @click=${() => {
            this.providerEditorMode = mode;
            this.providerEditorError = '';
        }}
                >
                  ${mode === 'manual'
            ? zh
                ? '手动配置'
                : 'Configure'
            : zh
                ? '导入 JSON'
                : 'Import JSON'}
                </button>`)}
          </div>
          <fieldset
            class="provider-editor-body"
            ?hidden=${this.providerEditorMode !== 'manual'}
            ?disabled=${this.providerOperation !== null ||
            this.providerEditorMode !== 'manual'}
          >
            <div class="provider-editor-grid">
              <label class="provider-editor-field wide">
                <span>${zh ? '内置预设' : 'Built-in preset'}</span>
                <select
                  name="builtin-provider"
                  .value=${this.providerEditorBuiltinProviderId}
                  @change=${this.selectProviderPreset}
                >
                  <option value="">
                    ${zh ? '自定义厂商' : 'Custom provider'}
                  </option>
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
                  <option value="openai_chat_completions">
                    OpenAI Chat Completions
                  </option>
                  <option value="openai_responses">OpenAI Responses</option>
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
                  autocomplete="new-password"
                  placeholder=${apiKeyAlreadySet
            ? zh
                ? '留空则保持已有 Key'
                : 'Leave blank to keep the existing key'
            : 'sk-...'}
                  .value=${this.providerEditorApiKey}
                  @input=${(event) => {
            this.providerEditorApiKey = event.target.value;
        }}
                />
              </label>
              <div class="provider-editor-models">
                <div class="provider-editor-models-title">
                  ${zh ? '模型' : 'Models'}
                </div>
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
                        <span>${zh ? '上下文长度' : 'Context length'}</span>
                        <input
                          name=${`model-context-${row.key}`}
                          type="number"
                          min="1"
                          required
                          step="1"
                          .value=${row.contextWindow === null
            ? ''
            : String(row.contextWindow)}
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
                        ?disabled=${this.providerEditorModels.length <= 1 ||
            this.providerOperation !== null}
                        @click=${() => this.removeProviderEditorModel(row.key)}
                      >
                        ×
                      </button>
                      <div class="provider-reasoning-settings">
                        <label class="provider-editor-field">
                          <span>${zh ? '推理强度' : 'Reasoning effort'}</span>
                          <select
                            name=${`model-effort-${row.key}`}
                            .value=${row.reasoning?.effort ?? ''}
                            @change=${(event) => this.updateProviderEditorModel(row.key, {
            reasoning: {
                ...row.reasoning,
                effort: event.target
                    .value ||
                    undefined,
            },
        })}
                          >
                            <option value="">
                              ${zh ? '服务端默认' : 'Server default'}
                            </option>
                            ${(this.providerEditorApiParadigm ===
            'anthropic_messages'
            ? ['low', 'medium', 'high', 'xhigh', 'max']
            : [
                'none',
                'minimal',
                'low',
                'medium',
                'high',
                'xhigh',
                'max',
            ]).map((effort) => html `<option value=${effort}>
                                  ${effort}
                                </option>`)}
                          </select>
                        </label>
                        ${this.providerEditorApiParadigm === 'openai_responses'
            ? html ` <label class="provider-editor-field">
                              <span
                                >${zh ? '思考摘要' : 'Reasoning summary'}</span
                              >
                              <select
                                name=${`model-summary-${row.key}`}
                                .value=${row.reasoning?.summary ?? ''}
                                @change=${(event) => this.updateProviderEditorModel(row.key, {
                reasoning: {
                    ...row.reasoning,
                    summary: event.target
                        .value ||
                        undefined,
                },
            })}
                              >
                                <option value="">
                                  ${zh ? '不指定' : 'Unspecified'}
                                </option>
                                ${['auto', 'concise', 'detailed'].map((summary) => html `<option value=${summary}>
                                      ${summary}
                                    </option>`)}
                              </select>
                            </label>`
            : nothing}
                        ${this.providerEditorApiParadigm ===
            'anthropic_messages'
            ? html `
                              <label class="provider-editor-field">
                                <span
                                  >${zh ? '思考模式' : 'Thinking mode'}</span
                                >
                                <select
                                  name=${`model-thinking-${row.key}`}
                                  .value=${row.reasoning?.thinking ?? ''}
                                  @change=${(event) => this.updateProviderEditorModel(row.key, {
                reasoning: {
                    ...row.reasoning,
                    thinking: event.target
                        .value ||
                        undefined,
                },
            })}
                                >
                                  <option value="">
                                    ${zh ? '不指定' : 'Unspecified'}
                                  </option>
                                  <option value="adaptive">
                                    ${zh ? '自适应' : 'Adaptive'}
                                  </option>
                                  <option value="enabled">
                                    ${zh ? '固定预算' : 'Fixed budget'}
                                  </option>
                                  <option value="disabled">
                                    ${zh ? '关闭' : 'Disabled'}
                                  </option>
                                </select>
                              </label>
                              ${row.reasoning?.thinking === 'enabled'
                ? html ` <label class="provider-editor-field">
                                    <span
                                      >${zh
                    ? '思考预算（tokens）'
                    : 'Thinking budget (tokens)'}</span
                                    >
                                    <input
                                      name=${`model-budget-${row.key}`}
                                      type="number"
                                      min="1024"
                                      step="1"
                                      required
                                      .value=${row.reasoning?.budget_tokens
                    ? String(row.reasoning.budget_tokens)
                    : ''}
                                      @input=${(event) => this.updateProviderEditorModel(row.key, {
                    reasoning: {
                        ...row.reasoning,
                        budget_tokens: this.parseOptionalPositiveInt(event.target.value) ?? undefined,
                    },
                })}
                                    />
                                  </label>`
                : nothing}
                            `
            : nothing}
                      </div>
                    </div>
                  `)}
                <button
                  class="provider-editor-add-model secondary"
                  type="button"
                  ?disabled=${this.providerOperation !== null}
                  @click=${() => this.addProviderEditorModel()}
                >
                  ${zh ? '添加模型' : 'Add model'}
                </button>
              </div>
            </div>
          </fieldset>
          <section
            class="provider-editor-body"
            ?hidden=${this.providerEditorMode !== 'import'}
          >
            <div class="provider-editor-import">
              <label class="provider-editor-field"
                ><span>${zh ? '厂商配置 JSON' : 'Provider JSON'}</span
                ><textarea
                  aria-label="Provider JSON"
                  spellcheck="false"
                  placeholder='{"schema":"agent-runtime-llm-registration/v1","providers":[...]}'
                  .value=${this.providerImportJson}
                  ?disabled=${this.providerOperation !== null}
                  @input=${(event) => (this.providerImportJson = event.target.value)}
                ></textarea></label
              ><input
                class="provider-file-input"
                type="file"
                accept=".json,application/json"
                aria-label=${zh ? '导入配置文件' : 'Import configuration file'}
                @change=${this.readProviderImportFile}
              /><button
                class="secondary provider-import-file"
                type="button"
                ?disabled=${this.providerOperation !== null}
                @click=${() => this.renderRoot
            .querySelector('.provider-file-input')
            ?.click()}
              >
                ${zh ? '选择 JSON 文件' : 'Choose JSON file'}
              </button>
            </div>
          </section>
          ${this.providerEditorError
            ? html `<div class="provider-editor-error" role="alert">
                ${this.providerEditorError}
              </div>`
            : nothing}
          <div class="provider-editor-actions">
            <button
              class="secondary"
              type="button"
              ?disabled=${this.providerOperation !== null}
              @click=${() => {
            this.closeProviderEditor();
        }}
            >
              ${zh ? '取消' : 'Cancel'}
            </button>
            <button type="submit" ?disabled=${this.providerOperation !== null}>
              ${this.providerOperation === 'import'
            ? zh
                ? '保存中…'
                : 'Saving…'
            : this.providerEditorMode === 'import'
                ? zh
                    ? '导入配置'
                    : 'Import configuration'
                : zh
                    ? '保存厂商'
                    : 'Save provider'}
            </button>
          </div>
        </form>
      </div>
    </div>`;
    }
    closeProviderEditor() {
        if (this.providerOperation !== null)
            return;
        this.providerEditorOpen = false;
        this.providerEditorEpoch++;
        this.providerEditorApiKey = '';
    }
    readProviderImportFile = async (event) => {
        const input = event.target, file = input.files?.[0];
        input.value = '';
        if (!file || this.providerOperation !== null)
            return;
        const epoch = this.providerEditorEpoch;
        try {
            const text = await file.text();
            if (epoch !== this.providerEditorEpoch || !this.providerEditorOpen)
                return;
            this.providerImportJson = text;
            this.providerEditorError = '';
        }
        catch (error) {
            if (epoch !== this.providerEditorEpoch || !this.providerEditorOpen)
                return;
            this.providerEditorError =
                error instanceof Error ? error.message : String(error);
        }
    };
    async openProviderEditor(providerUid = null) {
        if (this.providerOperation !== null)
            return;
        this.providerEditorEpoch++;
        this.composerPanel = null;
        this.providerEditorMode = 'manual';
        this.providerEditorError = '';
        this.providerImportJson = '';
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
        this.providerEditorBuiltinProviderId = this.builtinProviderById(providerType)
            ? providerType
            : '';
        this.providerEditorProviderName =
            provider.name ?? `Provider ${provider.uid}`;
        this.providerEditorBaseUrl = provider.base_url ?? provider.baseUrl ?? '';
        this.providerEditorApiParadigm =
            provider.api_paradigm ??
                provider.apiParadigm ??
                'openai_chat_completions';
        this.providerEditorApiKey = '';
        const rows = this.providerModelsForProvider(providerUid).map((model) => this.createProviderEditorModelRow(model.model_id ??
            model.modelId ??
            model.model_name ??
            model.modelName ??
            model.name ??
            '', model.context_window ?? model.contextWindow ?? null, model.uid, model.reasoning));
        this.providerEditorModels = rows.length
            ? rows
            : [this.createProviderEditorModelRow()];
    }
    selectProviderPreset = (event) => {
        const providerId = event.target.value;
        this.providerEditorBuiltinProviderId = providerId;
        const provider = this.builtinProviderById(providerId);
        if (!provider)
            return;
        this.providerEditorProviderName = provider.name;
        this.providerEditorBaseUrl =
            provider.defaultBaseUrl ?? provider.default_base_url ?? '';
        this.providerEditorApiParadigm = this.providerApiParadigm(provider);
        const selectedModels = providerPresetModels(this.builtinProviderCatalog ?? {}, providerId);
        this.providerEditorModels = selectedModels.length
            ? selectedModels.map((model) => this.createProviderEditorModelRow(model.id, this.builtinModelContext(model)))
            : [this.createProviderEditorModelRow()];
    };
    createProviderEditorModelRow(modelId = '', contextWindow = 128000, uid = null, reasoning) {
        this.providerEditorModelSequence += 1;
        return {
            key: String(this.providerEditorModelSequence),
            uid,
            modelId,
            contextWindow,
            reasoning,
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
        return apiFormat === 'anthropic'
            ? 'anthropic_messages'
            : 'openai_chat_completions';
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
        return ((preset ? this.builtinModelContext(preset) : row.contextWindow) ?? 128000);
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
        if (!this.providerControls.enabled ||
            !this.composerSession.canConfigure ||
            this.imageSending ||
            this.providerOperation !== null)
            return;
        const conversationId = this.state.conversationId ?? this.conversationId;
        if (!conversationId)
            return;
        const modelUid = Number(value);
        if (!Number.isFinite(modelUid))
            return;
        const model = this.providerModels().find((item) => item.uid === modelUid);
        if (!model)
            return;
        const modelName = this.providerModelName(model);
        const controller = this.providerControls.controller;
        this.providerOperation = 'model';
        try {
            const result = await controller.setConversationModel({
                conversationId,
                modelUid,
            });
            if (result.accepted === false) {
                throw new Error(result.rejectReason ?? 'Model switch was rejected');
            }
            if (conversationId !== this.state.conversationId ||
                !this.providerControls.enabled ||
                controller !== this.providerControls.controller)
                return;
            this.dispatch({
                type: 'conversation-model-selected',
                model: modelName,
                modelUid,
            });
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
    providerEditorReasoning(row) {
        const config = { effort: row.reasoning?.effort };
        if (this.providerEditorApiParadigm === 'openai_responses')
            config.summary = row.reasoning?.summary;
        if (this.providerEditorApiParadigm === 'anthropic_messages') {
            if (config.effort === 'none' || config.effort === 'minimal')
                config.effort = undefined;
            config.thinking = row.reasoning?.thinking;
            if (config.thinking === 'enabled')
                config.budget_tokens = row.reasoning?.budget_tokens;
        }
        return Object.values(config).some((value) => value !== undefined)
            ? config
            : undefined;
    }
    submitProviderEditor = (event) => {
        event.preventDefault();
        const form = event.currentTarget;
        const value = (name) => form
            .querySelector(`[name="${name}"]`)
            ?.value.trim() ?? '';
        if (this.providerOperation !== null)
            return;
        this.providerEditorError = '';
        if (this.providerEditorMode === 'import') {
            try {
                JSON.parse(this.providerImportJson);
            }
            catch {
                this.providerEditorError = this.locale.startsWith('zh')
                    ? 'JSON 格式无效，请检查后重试。'
                    : 'Invalid JSON. Check the configuration and retry.';
                return;
            }
            void this.configureProviderInput(this.providerImportJson, 'json').then((ok) => {
                if (ok)
                    this.providerEditorOpen = false;
            });
            return;
        }
        const invalid = providerEditorValidation(value('provider-name'), value('base-url'), this.providerEditorModels.map((row) => ({
            name: value(`model-id-${row.key}`),
            context: Number(value(`model-context-${row.key}`)),
        })), this.locale.startsWith('zh'));
        if (invalid) {
            this.providerEditorError = invalid;
            return;
        }
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
            const uid = row.uid && !usedModelUids.has(row.uid) ? row.uid : nextModelUid;
            usedModelUids.add(uid);
            while (usedModelUids.has(nextModelUid))
                nextModelUid += 1;
            return {
                uid,
                model_id: modelId,
                max_context_tokens: this.providerEditorModelContext(row, value(`model-context-${row.key}`)),
                reasoning: this.providerEditorReasoning(row),
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
            prompt_cache_control: currentProvider?.prompt_cache_control ??
                currentProvider?.promptCacheControl,
            enabled_models: enabledModels,
        };
        const providers = [
            ...this.existingProviderRegistrationsExcept(providerUid),
            providerRegistration,
        ];
        const providerModelUids = new Set(providers.flatMap((provider) => provider.enabled_models.map((model) => model.uid)));
        const currentModelUid = this.globalProviderModelUid();
        const bundle = {
            schema: 'agent-runtime-llm-registration/v1',
            id: 'conversation-provider-editor',
            providers,
            current_model_uid: currentModelUid !== null && providerModelUids.has(currentModelUid)
                ? currentModelUid
                : enabledModels[0].uid,
        };
        void this.configureProviderInput(JSON.stringify(bundle), 'json').then((configured) => {
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
            model_id: model.model_id ??
                model.modelId ??
                model.model_name ??
                model.modelName ??
                model.name ??
                '',
            max_context_tokens: model.context_window ?? model.contextWindow ?? 128000,
            reasoning: model.reasoning,
        }))
            .filter((model) => model.model_id);
        if (!models.length)
            return null;
        const apiParadigm = provider.api_paradigm ??
            provider.apiParadigm ??
            'openai_chat_completions';
        return {
            id: provider.uid,
            name: provider.name ?? `Provider ${provider.uid}`,
            type: provider.type ??
                provider.builtin_type ??
                provider.builtinType ??
                (apiParadigm === 'anthropic_messages' ? 'claude' : 'openai'),
            api_key: '',
            base_url: provider.base_url ?? provider.baseUrl ?? '',
            api_paradigm: apiParadigm,
            prompt_cache_control: provider.prompt_cache_control ?? provider.promptCacheControl,
            enabled_models: models,
        };
    }
    async configureProviderInput(input, source) {
        if (!this.providerControls.enabled || this.providerOperation !== null)
            return false;
        const controller = this.providerControls.controller;
        this.providerEditorError = '';
        this.providerOperation = 'import';
        try {
            const result = await controller.configureProviders({
                input,
                source,
            });
            if (result.accepted === false) {
                throw new Error(result.rejectReason ?? 'Provider import was rejected');
            }
            if (!this.providerControls.enabled ||
                this.providerControls.controller !== controller)
                return false;
            this.providerDefinitions = await controller.getProviderDefinitions();
            this.emit('agent-conversation-provider-action', {
                action: 'providers-imported',
                source,
                result,
            });
            return true;
        }
        catch (error) {
            this.providerEditorError =
                error instanceof Error ? error.message : String(error);
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
        return (Math.max(0, ...(this.providerDefinitions?.providers ?? []).map((provider) => provider.uid)) + 1);
    }
    nextModelUid(used = new Set()) {
        let uid = Math.max(1000, ...this.providerModels().map((model) => model.uid), ...used) + 1;
        while (used.has(uid))
            uid += 1;
        return uid;
    }
    globalProviderModelUid() {
        const definitions = this.providerDefinitions;
        return (definitions?.current_model_uid ?? definitions?.currentModelUid ?? null);
    }
    permissionIsNoLongerPending(result) {
        return (result.metadata?.resolved === false ||
            result.rejectReason === 'Permission request is no longer pending.');
    }
    currentConversationModelUid() {
        if (this.state.modelUid !== undefined)
            return this.state.modelUid;
        if (!this.state.model)
            return null;
        return (this.providerModels().find((model) => this.providerModelName(model) === this.state.model)?.uid ?? null);
    }
    currentProviderModelLabel() {
        const current = this.currentConversationModelUid();
        if (current === null)
            return null;
        const model = this.providerModels().find((item) => item.uid === current);
        return model ? this.providerModelLabel(model) : `Model ${current}`;
    }
    providerModelLabel(model) {
        const modelName = this.providerModelName(model);
        const providerUid = model.provider_uid ?? model.providerUid;
        const provider = this.providerDefinitions?.providers?.find((item) => item.uid === providerUid);
        const providerName = provider?.name ?? (providerUid ? `Provider ${providerUid}` : '');
        return providerName ? `${modelName} / ${providerName}` : modelName;
    }
    providerModelName(model) {
        return (model.model_name ??
            model.modelName ??
            model.model_id ??
            model.modelId ??
            model.name ??
            `Model ${model.uid}`);
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
            ? zh
                ? '等待批准'
                : 'Waiting for approval'
            : zh
                ? `${permissions.length} 个请求等待批准`
                : `${permissions.length} requests waiting for approval`}
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
        const targets = summarizePermissionTargets(permission);
        const error = this.permissionError?.toolCallId === permission.tool_call_id
            ? this.permissionError.message
            : null;
        return html `<div
      class="permission-tool"
      data-call-id=${permission.tool_call_id}
    >
      <div class="permission-copy">
        <div class="permission-tool-name">
          ${permission.display_name || permission.tool_name}
        </div>
        <div class="permission-effect">${effect} · ${permission.tool_name}</div>
        ${targets.length
            ? html `<div class="permission-arguments">
              ${targets.map((target) => html `<div class="permission-argument">
                    <span class="permission-argument-label"
                      >${target.label}</span
                    >
                    <span>${target.value}</span>
                  </div>`)}
            </div>`
            : nothing}
        ${error
            ? html `<div class="permission-error" role="alert">${error}</div>`
            : nothing}
      </div>
      <div class="permission-actions">
        <button
          class="deny"
          type="button"
          ?disabled=${this.permissionOperation !== null ||
            this.state.connection !== 'connected' ||
            Boolean(this.composerSession.awaiting)}
          @click=${() => void this.resolveToolPermission(permission.tool_call_id, 'deny')}
        >
          ${zh ? '拒绝' : 'Deny'}
        </button>
        <button
          type="button"
          ?disabled=${this.permissionOperation !== null ||
            this.state.connection !== 'connected' ||
            Boolean(this.composerSession.awaiting)}
          @click=${() => void this.resolveToolPermission(permission.tool_call_id, 'allow')}
        >
          ${busy ? (zh ? '处理中' : 'Working') : zh ? '允许' : 'Allow'}
        </button>
      </div>
    </div>`;
    }
    permissionEffectLabel(permission, zh) {
        return permission.effect === 'destructive'
            ? zh
                ? '破坏性操作'
                : 'Destructive operation'
            : permission.effect === 'controlled_change'
                ? zh
                    ? '可控变更'
                    : 'Controlled change'
                : zh
                    ? '只读操作'
                    : 'Read-only operation';
    }
    renderHistoryPanel() {
        if (!this.persistence.enabled || !this.historyOpen)
            return nothing;
        const zh = this.locale.toLowerCase().startsWith('zh');
        return html `<aside class="history-panel" part="history-panel">
      ${this.persistenceOperation === 'list'
            ? html `<div class="history-empty">
            ${zh ? '正在加载会话' : 'Loading conversations'}
          </div>`
            : this.persistedConversations.length === 0
                ? html `<div class="history-empty">
              ${zh ? '还没有会话' : 'No conversations'}
            </div>`
                : this.persistedConversations.map((archive) => {
                    const current = this.isCurrentArchive(archive);
                    return html `<div
                class="history-item"
                data-current=${String(current)}
              >
                <div class="history-copy">
                  <div class="history-title">
                    ${archive.title ??
                        (zh ? '未命名会话' : 'Untitled conversation')}
                  </div>
                  <div class="history-preview">
                    ${archive.preview ?? archive.updatedAt ?? archive.archiveId}
                  </div>
                  <div class="history-meta">
                    ${current || archive.instanceState === 'current'
                        ? zh
                            ? '当前会话'
                            : 'Current conversation'
                        : archive.instanceState === 'background'
                            ? zh
                                ? '后台运行'
                                : 'Running in background'
                            : zh
                                ? '仅数据'
                                : 'Saved data'}
                    ·
                    ${this.formatArchiveTime(archive.updatedAt ?? archive.createdAt)}
                  </div>
                </div>
                <button
                  type="button"
                  ?disabled=${current || this.persistenceOperation !== null}
                  @click=${() => void this.restoreConversation(archive.archiveId)}
                >
                  ${this.persistenceOperation === `restore:${archive.archiveId}`
                        ? zh
                            ? '打开中'
                            : 'Opening'
                        : current
                            ? zh
                                ? '当前'
                                : 'Current'
                            : zh
                                ? '打开'
                                : 'Open'}
                </button>
              </div>`;
                })}
    </aside>`;
    }
    renderDisplayItem(item, index, items, scheme) {
        if (item.kind === 'presentation') {
            if (item.item.kind !== 'assistant-markdown' &&
                item.item.kind !== 'notice') {
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
        <div class="user-bubble">${this.renderUserRecord(record)}</div>
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
        .content=${displayTextWithToolAnchors(record)}
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
            ? html `<div class="disclaimer" part="disclaimer">
            ${this.disclaimer}
          </div>`
            : nothing}
    </article>`;
    }
    createDisplayItems() {
        return createDisplayItems(this.state, this.presentationItems, this.completedRevealKeys);
    }
    latestWidgetRecordKey(items) {
        return latestWidgetRecordKey(items);
    }
    latestAssistantKey(items) {
        return latestAssistantKey(items);
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
            const initialSnapshot = !this.state.snapshotReceived;
            this.dispatch({
                type: 'snapshot',
                payload: event.payload,
                eventSeq: event.eventSeq,
            });
            if (initialSnapshot && event.payload.conversation_state === 'waiting') {
                this.markRecordsRevealComplete(snapshotRecords(event.payload));
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
            if (event.extension.kind === 'tool:permission-resolved') {
                const payload = event.extension.payload;
                if (payload && typeof payload === 'object') {
                    const toolCallId = payload.tool_call_id;
                    if (typeof toolCallId === 'string' && toolCallId) {
                        if (this.permissionError?.toolCallId === toolCallId) {
                            this.permissionError = null;
                        }
                        this.dispatch({ type: 'tool-permission-resolved', toolCallId });
                    }
                }
            }
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
        this.composerSession.sync(this.state, this.transport, this.composerController, this.state.modelUid);
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
        const retained = this.pendingImages.filter((image) => this.draft.includes(`@图片${image.number}`));
        for (const image of this.pendingImages) {
            if (!retained.includes(image) && image.previewUrl)
                URL.revokeObjectURL(image.previewUrl);
        }
        if (retained.length !== this.pendingImages.length) {
            this.pendingImages = retained;
            this.renumberPendingImages();
        }
        textarea.style.height = '0px';
        textarea.style.height = `${Math.min(textarea.scrollHeight, 180)}px`;
    }
    onImageInput(event) {
        const input = event.target;
        const files = Array.from(input.files ?? []);
        input.value = '';
        this.addPendingImages(files);
    }
    onComposerPaste(event) {
        if (!this.transport?.imageInput || !this.canCompose() || this.imageSending)
            return;
        const items = Array.from(event.clipboardData?.items ?? []);
        const itemFiles = items
            .filter((item) => item.kind === 'file')
            .map((item) => item.getAsFile())
            .filter((file) => file !== null && file.type.startsWith('image/'));
        const files = itemFiles.length
            ? itemFiles
            : Array.from(event.clipboardData?.files ?? []).filter((file) => file.type.startsWith('image/'));
        if (!files.length)
            return;
        event.preventDefault();
        const text = event.clipboardData?.getData?.('text/plain') ?? '';
        this.addPendingImages(files, text);
    }
    addPendingImages(files, pastedText = '') {
        const markers = [];
        for (const file of files) {
            if (!['image/png', 'image/jpeg', 'image/webp', 'image/gif'].includes(file.type) ||
                file.size > 20 * 1024 * 1024 ||
                this.pendingImages.length >= 8) {
                this.emit('agent-conversation-error', {
                    code: 'invalid-image',
                    message: `Unsupported image, image larger than 20 MiB, or more than 8 images: ${file.name}`,
                    recoverable: true,
                });
                continue;
            }
            const previewUrl = typeof URL.createObjectURL === 'function'
                ? URL.createObjectURL(file)
                : '';
            const number = this.pendingImages.reduce((max, image) => Math.max(max, image.number), 0) + 1;
            this.pendingImages = [
                ...this.pendingImages,
                { file, previewUrl, number },
            ];
            markers.push(`@图片${number}`);
        }
        if (pastedText || markers.length) {
            this.insertIntoDraft([pastedText, ...markers].filter(Boolean).join(' '));
        }
    }
    insertIntoDraft(value) {
        const textarea = this.renderRoot.querySelector('textarea');
        const start = textarea?.selectionStart ?? this.draft.length;
        const end = textarea?.selectionEnd ?? this.draft.length;
        const before = this.draft.slice(0, start);
        const after = this.draft.slice(end);
        const prefix = /@图片\d+$/.test(before) && /^\S/.test(value) ? ' ' : '';
        const suffix = /@图片\d+$/.test(value) && /^[A-Za-z0-9]/.test(after) ? ' ' : '';
        this.draft = `${before}${prefix}${value}${suffix}${after}`;
        void this.updateComplete.then(() => {
            const current = this.renderRoot.querySelector('textarea');
            const cursor = start + prefix.length + value.length + suffix.length;
            current?.setSelectionRange(cursor, cursor);
        });
    }
    removePendingImage(image, removeMarker = true) {
        this.pendingImages = this.pendingImages.filter((item) => item !== image);
        if (image.previewUrl)
            URL.revokeObjectURL(image.previewUrl);
        if (removeMarker) {
            const label = `@图片${image.number}`;
            const index = this.draft.indexOf(label);
            if (index >= 0) {
                let before = this.draft.slice(0, index);
                let after = this.draft.slice(index + label.length);
                if (before.endsWith(' ') && after.startsWith(' '))
                    before = before.slice(0, -1);
                else if (!before && after.startsWith(' '))
                    after = after.slice(1);
                this.draft = before + after;
            }
        }
        this.renumberPendingImages();
    }
    renumberPendingImages() {
        const numbers = new Map(this.pendingImages.map((image, index) => [image.number, index + 1]));
        this.draft = this.draft.replace(/@图片(\d+)/g, (marker, number) => {
            const next = numbers.get(Number(number));
            return next ? `@图片${next}` : marker;
        });
        this.pendingImages = this.pendingImages.map((image, index) => ({
            ...image,
            number: index + 1,
        }));
    }
    clearPendingImages() {
        for (const image of this.pendingImages) {
            if (image.previewUrl)
                URL.revokeObjectURL(image.previewUrl);
        }
        this.pendingImages = [];
    }
    recordParts(record) {
        const parts = record.metadata?.extra?.parts;
        if (!Array.isArray(parts))
            return [];
        const result = [];
        for (const part of parts) {
            if (!part || typeof part !== 'object')
                continue;
            const value = part;
            if (value.type === 'image' && typeof value.image_id === 'string') {
                result.push({ type: 'image', image_id: value.image_id });
            }
            else if (value.type === 'text' && typeof value.text === 'string') {
                result.push({ type: 'text', text: value.text });
            }
        }
        return result;
    }
    renderReference(label, objectKey) {
        return html `<span
      class="at-reference"
      style=${`--reference-color: ${referenceColor(objectKey)}`}
      data-reference-key=${objectKey}
      >${label}</span
    >`;
    }
    renderMentionText(value) {
        const tokens = value.split(/(@[\p{L}\p{N}_-]+)/gu);
        let offset = 0;
        return tokens.map((token) => {
            const previous = value[offset - 1] ?? '';
            offset += token.length;
            return token.startsWith('@') &&
                token.length > 1 &&
                !/[A-Za-z0-9._%+-]/.test(previous)
                ? this.renderReference(token, `mention:${token.slice(1).toLocaleLowerCase()}`)
                : token;
        });
    }
    renderUserRecord(record) {
        const parts = this.recordParts(record);
        const message = displayText(record);
        if (!parts.length)
            return this.renderMentionText(message);
        const hasText = parts.some((part) => part.type === 'text');
        let imageNumber = 0;
        const pendingError = record.metadata?.extra?.pending_error;
        return html `${!hasText && message !== '[Image]' && !pendingError
            ? this.renderMentionText(message)
            : nothing}${parts.map((part) => {
            if (part.type === 'text')
                return this.renderMentionText(part.text);
            imageNumber += 1;
            return html `<agent-runtime-image-reference
        data-reference-key=${`image:${part.image_id}`}
        style=${`--reference-color: ${referenceColor(`image:${part.image_id}`)}`}
        .imageId=${part.image_id}
        .conversationId=${this.state.conversationId ?? undefined}
        .capabilities=${this.capabilities}
        .label=${this.locale.startsWith('zh')
                ? `预览图片 ${imageNumber}`
                : `Preview image ${imageNumber}`}
        >${`@图片${imageNumber}`}</agent-runtime-image-reference
      >`;
        })}${pendingError ? html `<br />${pendingError}` : nothing}`;
    }
    resetComposerHeight() {
        requestAnimationFrame(() => {
            const textarea = this.renderRoot.querySelector('textarea');
            if (textarea)
                textarea.style.height = '';
        });
    }
    onComposerKeydown(event) {
        if (event.key !== 'Enter' || event.shiftKey || event.isComposing)
            return;
        event.preventDefault();
        if (this.canCompose() && (this.draft.trim() || this.pendingImages.length))
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
        const zh = this.locale.startsWith('zh');
        if (this.state.connection === 'reconnecting')
            return zh ? '重新连接中' : 'Reconnecting';
        if (this.state.connection === 'connecting')
            return zh ? '连接中' : 'Connecting';
        if (this.state.connection !== 'connected')
            return zh ? '未连接' : 'Offline';
        if (!this.state.initialized)
            return zh ? '初始化中' : 'Initializing';
        return zh ? '已连接' : 'Connected';
    }
    canCompose() {
        return (this.composerSession.canCompose && this.providerOperation !== 'model');
    }
    canSaveConversation() {
        return (this.persistence.enabled &&
            Boolean(this.state.conversationId) &&
            this.state.connection === 'connected' &&
            this.state.snapshotReceived &&
            this.state.runtimeState === 'waiting' &&
            this.persistenceOperation === null);
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
        if (controller &&
            this.persistence.enabled &&
            this.persistence.showHistory !== false) {
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
        if (this.state.runtimeState === 'executing' ||
            this.state.toolCalls.some((call) => call.status === 'placeholder' ||
                call.status === 'running' ||
                call.status === 'waiting_permission')) {
            return this.locale.toLowerCase().startsWith('zh')
                ? '工具执行中'
                : 'Running tools';
        }
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
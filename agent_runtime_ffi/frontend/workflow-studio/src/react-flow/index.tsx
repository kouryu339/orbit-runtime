import React from 'react';
import ReactDOM from 'react-dom/client';
import '@xyflow/react/dist/style.css';
import { WorkflowEditor } from './WorkflowEditor';
import type { EditorProps, EditorEvent, EditorMode, BlueprintJson, FlowchartData, FlowchartStepStatus } from './types';

class WorkflowEditorElement extends HTMLElement {
  private root: ReactDOM.Root | null = null;
  private _props: EditorProps = { mode: 'normal' };

  connectedCallback() {
    const container = document.createElement('div');
    container.style.cssText = 'width:100%;height:100%';
    this.appendChild(container);
    this.root = ReactDOM.createRoot(container);
    this._render();
  }

  disconnectedCallback() {
    this.root?.unmount();
    this.root = null;
  }

  set mode(val: EditorMode) {
    this._props.mode = val;
    this._render();
  }

  set blueprintData(val: BlueprintJson | undefined) {
    this._props.blueprintData = val;
    this._render();
  }

  set flowchartData(val: FlowchartData | undefined) {
    this._props.flowchartData = val;
    this._render();
  }

  set readOnly(val: boolean) {
    this._props.readOnly = val;
    this._render();
  }

  set nodeDescriptions(val: Record<string, string> | undefined) {
    this._props.nodeDescriptions = val;
    this._render();
  }

  set currentStepId(val: string | undefined) {
    this._props.currentStepId = val;
    this._render();
  }

  set stepStatuses(val: Record<string, FlowchartStepStatus> | undefined) {
    this._props.stepStatuses = val;
    this._render();
  }

  private _render() {
    this.root?.render(
      <WorkflowEditor
        {...this._props}
        onEvent={(e: EditorEvent) => this._dispatch(e)}
      />
    );
  }

  private _dispatch(event: EditorEvent) {
    this.dispatchEvent(
      new CustomEvent(event.type, { detail: event.payload, bubbles: true })
    );
  }
}

customElements.define('workflow-editor', WorkflowEditorElement);

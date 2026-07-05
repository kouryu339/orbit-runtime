import React from 'react';
import type { EditorProps } from './types';
import { NormalFlowEditor } from './normal/NormalFlowEditor';
import { ProfessionalFlowEditor } from './professional/ProfessionalFlowEditor';

export function WorkflowEditor(props: EditorProps) {
  const { mode, flowchartData, blueprintData, onEvent, readOnly, nodeDescriptions, currentStepId, stepStatuses } = props;

  if (mode === 'normal') {
    return (
      <NormalFlowEditor
        data={flowchartData ?? null}
        onEvent={onEvent}
        readonly={readOnly}
        currentStepId={currentStepId}
        stepStatuses={stepStatuses}
      />
    );
  }

  return (
    <ProfessionalFlowEditor
      data={blueprintData ?? null}
      onEvent={onEvent}
      readonly={readOnly}
      nodeDescriptions={nodeDescriptions}
    />
  );
}

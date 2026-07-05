import { useEffect, useState } from 'react';
import type { BlueprintNodeJson, EditorEvent, NodePin } from './react-flow/types';
import { getArrayElementType, isArrayDataType, normalizeDataType } from './react-flow/theme';

type Props = {
  node: BlueprintNodeJson | null;
  connectedInputNames: Set<string>;
  onEvent: (event: EditorEvent) => void;
};

type BasicValueType = 'String' | 'Number' | 'Boolean';

const BASIC_VALUE_TYPES: BasicValueType[] = ['String', 'Number', 'Boolean'];

function basicTypeFor(dataType: string): BasicValueType | null {
  const normalized = normalizeDataType(dataType);
  if (normalized === 'String' || normalized === 'Path' || normalized === 'Date' || normalized === 'Time') return 'String';
  if (normalized === 'i64' || normalized === 'f64' || normalized === 'Number') return 'Number';
  if (normalized === 'bool' || normalized === 'Boolean') return 'Boolean';
  return null;
}

function inferArrayElementType(value: unknown): BasicValueType | null {
  if (!Array.isArray(value) || value.length === 0) return null;
  const firstType = typeof value[0];
  if (firstType === 'string') return value.every((item) => typeof item === 'string') ? 'String' : null;
  if (firstType === 'number') return value.every((item) => typeof item === 'number' && Number.isFinite(item)) ? 'Number' : null;
  if (firstType === 'boolean') return value.every((item) => typeof item === 'boolean') ? 'Boolean' : null;
  return null;
}

function defaultForType(type: BasicValueType) {
  if (type === 'String') return '';
  if (type === 'Number') return 0;
  return false;
}

function numberText(value: unknown) {
  return typeof value === 'number' && Number.isFinite(value) ? String(value) : '';
}

function jsonText(value: unknown) {
  if (value == null) return '';
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return '';
  }
}

function commitValue(nodeId: string, pinName: string, value: unknown, onEvent: Props['onEvent']) {
  onEvent({
    type: 'pindefaultchange',
    payload: { nodeId, pinName, value },
  });
}

function ScalarEditor({
  type,
  value,
  disabled,
  onCommit,
}: {
  type: BasicValueType;
  value: unknown;
  disabled: boolean;
  onCommit: (value: unknown) => void;
}) {
  const [numberDraft, setNumberDraft] = useState(() => numberText(value));
  const [numberError, setNumberError] = useState<string | null>(null);

  useEffect(() => {
    setNumberDraft(numberText(value));
    setNumberError(null);
  }, [value]);

  if (type === 'Boolean') {
    return (
      <label className="typed-bool">
        <input
          type="checkbox"
          checked={value === true}
          disabled={disabled}
          onChange={(event) => onCommit(event.target.checked)}
        />
      </label>
    );
  }

  if (type === 'String') {
    return (
      <div className="typed-string-shell">
        <span>"</span>
        <input
          type="text"
          value={typeof value === 'string' ? value : ''}
          disabled={disabled}
          onChange={(event) => onCommit(event.target.value)}
        />
        <span>"</span>
      </div>
    );
  }

  return (
    <div className="typed-number-shell">
      <input
        type="text"
        inputMode="decimal"
        value={numberDraft}
        disabled={disabled}
        aria-invalid={numberError != null}
        onChange={(event) => {
          setNumberDraft(event.target.value);
          setNumberError(null);
        }}
        onBlur={() => {
          const trimmed = numberDraft.trim();
          if (!trimmed) {
            setNumberError('Number required');
            return;
          }
          const parsed = Number(trimmed);
          if (!Number.isFinite(parsed)) {
            setNumberError('Invalid number');
            return;
          }
          setNumberError(null);
          onCommit(parsed);
        }}
      />
      {numberError && <em>{numberError}</em>}
    </div>
  );
}

function AnyJsonEditor({
  pin,
  nodeId,
  connected,
  onEvent,
}: {
  pin: NodePin;
  nodeId: string;
  connected: boolean;
  onEvent: Props['onEvent'];
}) {
  const [text, setText] = useState(() => jsonText(pin.default_value));
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setText(jsonText(pin.default_value));
    setError(null);
  }, [pin.default_value]);

  return (
    <>
      <textarea
        value={text}
        disabled={connected}
        placeholder={connected ? 'Value supplied by connection' : 'JSON value only'}
        rows={Math.min(8, Math.max(2, text.split('\n').length))}
        aria-invalid={error != null}
        onChange={(event) => {
          setText(event.target.value);
          setError(null);
        }}
        onBlur={() => {
          if (connected) return;
          try {
            commitValue(nodeId, pin.name, JSON.parse(text), onEvent);
          } catch {
            setError('Use valid JSON, for example "text", 12, true, [], or {}.');
          }
        }}
      />
      {error && <em className="inspector-error">{error}</em>}
    </>
  );
}

function TypeChooser({
  value,
  onChange,
  disabled,
}: {
  value: BasicValueType | null;
  onChange: (value: BasicValueType) => void;
  disabled: boolean;
}) {
  return (
    <div className="typed-array-typebar" role="group" aria-label="Array item type">
      {BASIC_VALUE_TYPES.map((type) => (
        <button
          key={type}
          type="button"
          disabled={disabled}
          className={value === type ? 'active' : ''}
          onClick={() => onChange(type)}
        >
          {type}
        </button>
      ))}
    </div>
  );
}

function ArrayEditor({
  pin,
  nodeId,
  connected,
  onEvent,
  fixedElementType,
}: {
  pin: NodePin;
  nodeId: string;
  connected: boolean;
  onEvent: Props['onEvent'];
  fixedElementType: BasicValueType | null;
}) {
  const currentItems = Array.isArray(pin.default_value) ? pin.default_value : [];
  const [chosenType, setChosenType] = useState<BasicValueType | null>(() => fixedElementType ?? inferArrayElementType(pin.default_value));
  const elementType = fixedElementType ?? chosenType;
  const canEditItems = elementType != null;

  useEffect(() => {
    setChosenType(fixedElementType ?? inferArrayElementType(pin.default_value));
  }, [fixedElementType, pin.default_value]);

  const updateItems = (items: unknown[]) => commitValue(nodeId, pin.name, items, onEvent);

  return (
    <div className="typed-array-editor">
      {!fixedElementType && (
        <TypeChooser
          value={chosenType}
          disabled={connected}
          onChange={(nextType) => {
            setChosenType(nextType);
            updateItems([]);
          }}
        />
      )}
      {!canEditItems ? (
        <p className="typed-array-empty">Choose a generic type before editing items.</p>
      ) : (
        <>
          <div className={currentItems.length > 10 ? 'typed-array-items is-scrollable' : 'typed-array-items'}>
            {currentItems.map((item, index) => (
              <div className="typed-array-row" key={index}>
                <span>{index + 1}</span>
                <ScalarEditor
                  type={elementType}
                  value={item}
                  disabled={connected}
                  onCommit={(nextValue) => {
                    const nextItems = [...currentItems];
                    nextItems[index] = nextValue;
                    updateItems(nextItems);
                  }}
                />
                <button
                  type="button"
                  disabled={connected}
                  aria-label={`Remove item ${index + 1}`}
                  onClick={() => updateItems(currentItems.filter((_, itemIndex) => itemIndex !== index))}
                >
                  -
                </button>
              </div>
            ))}
          </div>
          <button
            type="button"
            className="typed-array-add"
            disabled={connected}
            onClick={() => updateItems([...currentItems, defaultForType(elementType)])}
          >
            Add item
          </button>
        </>
      )}
    </div>
  );
}

function PinDefaultEditor({
  nodeId,
  pin,
  connected,
  onEvent,
}: {
  nodeId: string;
  pin: NodePin;
  connected: boolean;
  onEvent: Props['onEvent'];
}) {
  const normalizedType = normalizeDataType(pin.data_type);
  const arrayElementType = isArrayDataType(pin.data_type) ? getArrayElementType(pin.data_type) : null;
  const fixedArrayElementType = arrayElementType ? basicTypeFor(arrayElementType) : null;
  const scalarType = basicTypeFor(pin.data_type);
  const isAny = normalizedType === 'Any';
  const isArrayAny = arrayElementType === 'Any';

  return (
    <label className={`inspector-pin${connected ? ' is-connected' : ''}`}>
      <span>
        <b>{pin.name}</b>
        <small>{connected ? 'Connected' : pin.data_type}</small>
      </span>
      {isAny ? (
        <AnyJsonEditor pin={pin} nodeId={nodeId} connected={connected} onEvent={onEvent} />
      ) : isArrayDataType(pin.data_type) ? (
        <ArrayEditor
          pin={pin}
          nodeId={nodeId}
          connected={connected}
          onEvent={onEvent}
          fixedElementType={isArrayAny ? null : fixedArrayElementType}
        />
      ) : scalarType ? (
        <ScalarEditor
          type={scalarType}
          value={pin.default_value}
          disabled={connected}
          onCommit={(value) => commitValue(nodeId, pin.name, value, onEvent)}
        />
      ) : (
        <AnyJsonEditor pin={pin} nodeId={nodeId} connected={connected} onEvent={onEvent} />
      )}
      {connected && <em>Default disabled while this pin is connected.</em>}
    </label>
  );
}

export function NodeInspector({ node, connectedInputNames, onEvent }: Props) {
  if (!node) return <p>Select a node to edit it.</p>;
  const dataInputs = node.pins.filter((pin) => pin.kind === 'DataInput');
  if (dataInputs.length === 0) return <p>This node has no data input pins.</p>;

  return (
    <div className="inspector-form">
      {dataInputs.map((pin) => (
        <PinDefaultEditor
          key={pin.name}
          nodeId={node.id}
          pin={pin}
          connected={connectedInputNames.has(pin.name)}
          onEvent={onEvent}
        />
      ))}
    </div>
  );
}

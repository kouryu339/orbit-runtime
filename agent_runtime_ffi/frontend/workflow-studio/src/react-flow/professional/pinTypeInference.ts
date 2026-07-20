import type { BlueprintNodeJson, ConnectionJson, NodePin } from '../types';
import { getArrayElementType, normalizeDataType } from '../theme';

function pinKey(nodeId: string, pinName: string) {
  return `${nodeId}::${pinName}`;
}

function isAny(dataType: string) {
  return normalizeDataType(dataType) === 'Any';
}

function canonicalArray(elementType: string) {
  return `Array<${normalizeDataType(elementType)}>`;
}

function inferFlexibleType(current: string, connected: string): string {
  if (isAny(current) && !isAny(connected)) return connected;

  const currentElement = getArrayElementType(current);
  const connectedElement = getArrayElementType(connected);
  if (currentElement && connectedElement && isAny(currentElement) && !isAny(connectedElement)) {
    return canonicalArray(connectedElement);
  }

  return current;
}

export function resolveConnectedPinTypes(
  nodes: BlueprintNodeJson[],
  connections: ConnectionJson[],
) {
  const resolved = new Map<string, string>();

  for (const node of nodes) {
    for (const pin of node.pins) {
      const runtimeResolved = typeof pin.resolved_type === 'string' ? pin.resolved_type : '';
      resolved.set(pinKey(node.id, pin.name), runtimeResolved || pin.data_type || 'Any');
    }
  }

  // Iterate because a concrete type may need to cross several Any pins.
  for (let pass = 0; pass < connections.length + 1; pass += 1) {
    let changed = false;
    for (const connection of connections) {
      if (connection.connection_type === 'Exec') continue;
      const sourceKey = pinKey(connection.source_node, connection.source_pin);
      const targetKey = pinKey(connection.target_node, connection.target_pin);
      const sourceType = resolved.get(sourceKey) ?? 'Any';
      const targetType = resolved.get(targetKey) ?? 'Any';
      const nextSource = inferFlexibleType(sourceType, targetType);
      const nextTarget = inferFlexibleType(targetType, sourceType);

      if (nextSource !== sourceType) {
        resolved.set(sourceKey, nextSource);
        changed = true;
      }
      if (nextTarget !== targetType) {
        resolved.set(targetKey, nextTarget);
        changed = true;
      }
    }
    if (!changed) break;
  }

  return resolved;
}

export function withResolvedPinType(pin: NodePin, resolvedType: string | undefined): NodePin {
  if (!resolvedType || resolvedType === pin.data_type) return pin;
  return { ...pin, data_type: resolvedType, resolved_type: resolvedType };
}

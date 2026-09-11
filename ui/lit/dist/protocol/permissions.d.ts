import type { PendingToolPermission } from './types.js';
export type PermissionTargetSummary = {
    label: string;
    value: string;
};
/**
 * Build a short, non-JSON approval summary. Runtime keeps the full arguments
 * for execution and audit; the regular UI only exposes safe target fields.
 */
export declare function summarizePermissionTargets(permission: PendingToolPermission): PermissionTargetSummary[];
//# sourceMappingURL=permissions.d.ts.map
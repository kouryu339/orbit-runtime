import React, { useState, useRef, useCallback } from 'react';
import type { NodeProps } from '@xyflow/react';
import { NodeResizer } from '@xyflow/react';

interface CommentBoxData {
  text: string;
  color?: string;
  width: number;
  height: number;
  [key: string]: unknown;
}

export function CommentBoxNode({ data, selected, id }: NodeProps) {
  const d = data as unknown as CommentBoxData;
  const accent = d.color ?? '#f59e0b';
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(d.text);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const startEdit = useCallback((e: React.MouseEvent) => {
    e.stopPropagation();
    setDraft(d.text);
    setEditing(true);
    setTimeout(() => textareaRef.current?.focus(), 0);
  }, [d.text]);

  const commitEdit = useCallback(() => {
    setEditing(false);
    if (draft !== d.text) {
      // Dispatch custom event up to Angular — same pattern as other editor events
      const el = document.querySelector('workflow-editor');
      el?.dispatchEvent(new CustomEvent('commentupdate', {
        detail: { commentId: id, text: draft },
        bubbles: true,
      }));
    }
  }, [draft, d.text, id]);

  const onKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); commitEdit(); }
    if (e.key === 'Escape') { setEditing(false); setDraft(d.text); }
  }, [commitEdit, d.text]);

  return (
    <>
      <NodeResizer
        isVisible={selected}
        minWidth={140}
        minHeight={60}
        handleStyle={{ background: accent, border: 'none', width: 8, height: 8, borderRadius: 2 }}
        lineStyle={{ border: `1px dashed ${accent}` }}
      />
      <div
        style={{
          width: '100%', height: '100%',
          background: `${accent}18`,
          border: `1.5px dashed ${accent}88`,
          borderRadius: 10,
          padding: '8px 12px 10px',
          boxSizing: 'border-box',
          display: 'flex',
          flexDirection: 'column',
          gap: 4,
        }}
      >
        <div style={{
          fontSize: 11, fontWeight: 700, color: accent,
          letterSpacing: '0.04em', textTransform: 'uppercase',
          opacity: 0.85, lineHeight: 1,
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        }}>
          <span>▪ Comment</span>
          {!editing && (
            <span
              title="双击编辑"
              onDoubleClick={startEdit}
              style={{ opacity: 0.5, cursor: 'text', fontSize: 10, fontWeight: 400, textTransform: 'none' }}
            >
              双击编辑
            </span>
          )}
        </div>

        {editing ? (
          <textarea
            ref={textareaRef}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={commitEdit}
            onKeyDown={onKeyDown}
            style={{
              flex: 1, resize: 'none', background: '#fffefa',
              border: `1px solid ${accent}88`, borderRadius: 4,
              color: '#3c514e', fontSize: 12, lineHeight: 1.5,
              padding: '4px 6px', outline: 'none', fontFamily: '"Segoe UI", sans-serif',
            }}
          />
        ) : (
          <div
            onDoubleClick={startEdit}
            style={{
              fontSize: 12, color: '#536863', lineHeight: 1.5,
              flex: 1, overflow: 'hidden', wordBreak: 'break-word',
              cursor: 'text',
            }}
          >
            {d.text || <span style={{ opacity: 0.3 }}>双击添加注释...</span>}
          </div>
        )}
      </div>
    </>
  );
}

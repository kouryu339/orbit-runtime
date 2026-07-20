import { useEffect, useMemo, useState, type CSSProperties } from 'react';
import {
  Activity,
  Bot,
  ChevronRight,
  FileText,
  MessageSquareText,
  Radio,
  ShieldCheck,
  Sparkles,
  UserRound,
} from 'lucide-react';
import { StudioConversation } from './StudioConversation';

type PairStatus = 'running' | 'concluding' | 'reported';

type PairPersona = {
  identity: string;
  goal: string;
  strategy?: string;
};

type PairSummary = {
  pair_id: string;
  persona: PairPersona;
  status: PairStatus;
  runtime_status?: string;
  turns?: number;
  updated_at?: string;
  failure?: string | null;
};

type ToolEvidence = {
  call_id: string;
  tool_name?: string;
  status: string;
  success?: boolean;
  content?: string;
};

type PairMessage = {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  side?: 'target' | 'adversary';
  target_tool_evidence?: ToolEvidence[];
};

type PairDetail = {
  pair_id: string;
  adversary_name?: string;
  target_name?: string;
  messages: PairMessage[];
  report?: unknown;
  failure?: string | null;
  relay_state?: string;
  status?: PairStatus;
};

type StudioContext = {
  session_id?: string;
  target_name?: string;
  supervisor_name?: string;
  supervisor_state?: string;
  conversation_state?: 'waiting' | 'thinking' | 'executing' | 'compacting' | 'stopping';
  pairs?: PairSummary[];
};

type StudioEvent = {
  type?: string;
  payload?: Record<string, unknown>;
};

const token = new URLSearchParams(location.search).get('token') ?? '';

function apiUrl(path: string) {
  return `${path}${path.includes('?') ? '&' : '?'}token=${encodeURIComponent(token)}`;
}

async function api<T>(path: string, options: RequestInit = {}): Promise<T> {
  const response = await fetch(apiUrl(path), {
    ...options,
    headers: { 'Content-Type': 'application/json', ...options.headers },
  });
  const contentType = response.headers.get('content-type') ?? '';
  if (!contentType.includes('application/json')) {
    throw new Error('运行时 API 尚未接入 Agent Test Studio 前端');
  }
  const value = await response.json();
  if (!response.ok || value?.error) {
    throw new Error(value?.error ?? `Request failed: ${response.status}`);
  }
  return value as T;
}

function statusLabel(status: PairStatus) {
  if (status === 'running') return '正在运行';
  if (status === 'concluding') return '正在生成报告';
  return '已经报告';
}

function timeLabel(value?: string) {
  if (!value) return '刚刚';
  if (/^\d+$/.test(value)) {
    const date = new Date(Number(value));
    return Number.isNaN(date.getTime()) ? '刚刚' : formatTime(date);
  }
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : formatTime(date);
}

function formatTime(date: Date) {
  return new Intl.DateTimeFormat('zh-CN', {
    hour: '2-digit',
    minute: '2-digit',
  }).format(date);
}

function normalizePair(pair: PairSummary): PairSummary | null {
  if (
    pair.status !== 'running'
    && pair.status !== 'concluding'
    && pair.status !== 'reported'
  ) return null;
  return pair;
}

function reportText(report: unknown, failure?: string | null) {
  if (failure) return `Relay failure\n\n${failure}`;
  if (!report) return '';
  if (typeof report === 'string') return report;
  return JSON.stringify(report, null, 2);
}

export function App() {
  const [context, setContext] = useState<StudioContext>({});
  const [pairs, setPairs] = useState<PairSummary[]>([]);
  const [selectedPairId, setSelectedPairId] = useState<string | null>(null);
  const [pairDetail, setPairDetail] = useState<PairDetail | null>(null);
  const [connected, setConnected] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  const selectedPair = useMemo(
    () => pairs.find((pair) => pair.pair_id === selectedPairId) ?? null,
    [pairs, selectedPairId],
  );

  useEffect(() => {
    let disposed = false;
    api<StudioContext>('/api/context')
      .then((value) => {
        if (disposed) return;
        const nextPairs = (value.pairs ?? []).map(normalizePair).filter(Boolean) as PairSummary[];
        setContext(value);
        setPairs(nextPairs);
        setSelectedPairId(nextPairs[0]?.pair_id ?? null);
        setError('');
      })
      .catch((reason) => {
        if (!disposed) setError(reason instanceof Error ? reason.message : String(reason));
      })
      .finally(() => {
        if (!disposed) setLoading(false);
      });

    const source = new EventSource(apiUrl('/events'));
    source.onopen = () => setConnected(true);
    source.onerror = () => setConnected(false);
    source.onmessage = (event) => {
      try {
        applyStudioEvent(JSON.parse(event.data) as StudioEvent);
      } catch {
        // Keep diagnostics from breaking the live stream.
      }
    };

    return () => {
      disposed = true;
      source.close();
    };
  }, []);

  useEffect(() => {
    if (!selectedPairId) {
      setPairDetail(null);
      return;
    }
    api<PairDetail>(`/api/pairs/${encodeURIComponent(selectedPairId)}`)
      .then(setPairDetail)
      .catch(() => setPairDetail(null));
  }, [selectedPairId, pairs]);

  function applyStudioEvent(event: StudioEvent) {
    const payload = event.payload ?? {};
    if (event.type === 'agent-test.snapshot') {
      const nextPairs = (Array.isArray(payload.pairs) ? payload.pairs : [])
        .map((pair) => normalizePair(pair as PairSummary))
        .filter(Boolean) as PairSummary[];
      setPairs(nextPairs);
      setSelectedPairId((current) =>
        current && nextPairs.some((pair) => pair.pair_id === current)
          ? current
          : nextPairs[0]?.pair_id ?? null,
      );
      setContext((current) => ({
        ...current,
        supervisor_state:
          typeof payload.supervisor_state === 'string'
            ? payload.supervisor_state
            : current.supervisor_state,
        conversation_state:
          typeof payload.conversation_state === 'string'
            ? payload.conversation_state as StudioContext['conversation_state']
            : current.conversation_state,
      }));
    }
  }

  const runningCount = pairs.filter(
    (pair) => pair.status === 'running' || pair.status === 'concluding',
  ).length;
  const reportedCount = pairs.filter((pair) => pair.status === 'reported').length;
  const detailReport = reportText(pairDetail?.report, pairDetail?.failure);

  return (
    <main className="studio-shell">
      <header className="topbar">
        <div className="brand">
          <div className="brand-mark"><ShieldCheck size={18} /></div>
          <div>
            <strong>Agent Test Studio</strong>
            <span>{context.target_name ? `正在评测 ${context.target_name}` : '对抗测试工作台'}</span>
          </div>
        </div>
        <div className="session-meta">
          <span className={`connection ${connected ? 'is-live' : ''}`}>
            <Radio size={13} />
            {connected ? '实时快照已连接' : '等待运行时连接'}
          </span>
          {context.session_id && <code>{context.session_id}</code>}
        </div>
      </header>

      <section className="workspace">
        <aside className="pair-rail">
          <div className="panel-heading">
            <div>
              <span className="eyebrow">Active adversaries</span>
              <h1>对抗测试</h1>
            </div>
            <div className="count-badge">{pairs.length}</div>
          </div>

          <div className="rail-stats">
            <div><Activity size={14} /><strong>{runningCount}</strong><span>运行中</span></div>
            <div><FileText size={14} /><strong>{reportedCount}</strong><span>已报告</span></div>
          </div>

          <div className="pair-list">
            {pairs.map((pair, index) => (
              <button
                className={`pair-card ${pair.pair_id === selectedPairId ? 'is-selected' : ''}`}
                key={pair.pair_id}
                onClick={() => setSelectedPairId(pair.pair_id)}
                style={{ '--delay': `${index * 35}ms` } as CSSProperties}
              >
                <div className="pair-card-top">
                  <span className={`status-dot ${pair.status}`} />
                  <span className="pair-id">{pair.pair_id}</span>
                  <span className={`status-pill ${pair.status}`}>{statusLabel(pair.status)}</span>
                </div>
                <strong>{pair.persona.identity}</strong>
                <p>{pair.persona.goal}</p>
                {pair.failure && <small className="pair-failure">{pair.failure}</small>}
                <div className="pair-card-foot">
                  <span>{pair.turns ?? 0} 轮对话</span>
                  <span>{timeLabel(pair.updated_at)}</span>
                  <ChevronRight size={14} />
                </div>
              </button>
            ))}

            {!loading && pairs.length === 0 && (
              <div className="rail-empty">
                <Sparkles size={22} />
                <strong>还没有运行中的对抗测试</strong>
                <p>在右侧告诉总管测试目标，它会创建多个独立用户画像。</p>
              </div>
            )}
          </div>
        </aside>

        <section className="evidence-stage">
          <div className="conversation-pane">
            <div className="pane-title">
              <div>
                <span className="eyebrow">Paired transcript</span>
                <h2>{selectedPair?.persona.identity ?? '选择一个对抗测试'}</h2>
              </div>
              {selectedPair && (
                <span className={`status-pill ${selectedPair.status}`}>{statusLabel(selectedPair.status)}</span>
              )}
            </div>

            <div className="transcript">
              {pairDetail?.messages.map((message) => (
                <article className={`message ${message.role}`} key={message.id}>
                  <div className="avatar">
                    {message.role === 'user' ? <UserRound size={15} /> : <Bot size={15} />}
                  </div>
                  <div>
                    <span className="speaker">
                      {message.role === 'user'
                        ? pairDetail.adversary_name ?? '对抗用户'
                        : pairDetail.target_name ?? context.target_name ?? '目标 Agent'}
                    </span>
                    <p>{message.content}</p>
                    {!!message.target_tool_evidence?.length && (
                      <div className="tool-evidence">
                        {message.target_tool_evidence.map((tool) => (
                          <span key={`${message.id}-${tool.call_id}-${tool.status}`}>
                            {tool.tool_name ?? tool.call_id} · {tool.status}
                          </span>
                        ))}
                      </div>
                    )}
                  </div>
                </article>
              ))}

              {!pairDetail?.messages.length && (
                <div className="stage-empty">
                  <MessageSquareText size={30} />
                  <strong>{selectedPair ? '正在等待会话证据' : '对话证据会显示在这里'}</strong>
                  <p>控制器只把可见回复跨边界转发；目标工具调用作为开发者证据保留。</p>
                </div>
              )}
            </div>
          </div>

          <div className="report-pane">
            <div className="pane-title compact">
              <div>
                <span className="eyebrow">Adversary report</span>
                <h2>报告与证据</h2>
              </div>
              <FileText size={17} />
            </div>
            <div className="report-body">
              {detailReport
                ? <pre>{detailReport}</pre>
                : (
                  <div className="report-placeholder">
                    <span className="report-line wide" />
                    <span className="report-line" />
                    <span className="report-line short" />
                    <p>
                      {selectedPair?.status === 'concluding'
                        ? '对抗者已经提交结论，正在生成报告。'
                        : selectedPair?.status === 'running'
                          ? '对抗者提交结论后会出现在这里。'
                          : '暂无可用报告。'}
                    </p>
                  </div>
                )}
            </div>
          </div>
        </section>

        <aside className="supervisor-panel">
          <div className="supervisor-toolbar">
            <button className="active" type="button">Chat</button>
            <button type="button" disabled>Runs</button>
            <button type="button" disabled>Contract</button>
          </div>

          <div className="studio-chat">
            <StudioConversation
              token={token}
              locale="zh-CN"
              theme="green"
              onError={setError}
            />
            {error && (
              <div className="runtime-error">
                <span>{error}</span>
              </div>
            )}

          </div>
        </aside>
      </section>
    </main>
  );
}

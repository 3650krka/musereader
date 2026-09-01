import { useEffect, useRef, useState } from 'react';
import { SendHorizonal, Trash2, X } from 'lucide-react';
import type { useAiAssistant } from './useAiAssistant';
import { UI } from './readerShared';

interface AiChatPanelProps {
  assistant: ReturnType<typeof useAiAssistant>;
}

// 右侧 AI 会话区：消息流 + 引用条 + 提问输入。
// 划选文本可经 selection 菜单/拖拽进入提问框（quote 条），发送时随并提交。
export function AiChatPanel({ assistant }: AiChatPanelProps) {
  const [draft, setDraft] = useState('');
  const [dragOver, setDragOver] = useState(false);
  const listRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const el = listRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [assistant.messages, assistant.pending]);

  const send = () => {
    const text = draft.trim();
    if (!text || assistant.pending) return;
    setDraft('');
    void assistant.ask(text);
  };

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 animate-fade-in">
      <div ref={listRef} className="ai-chat-list">
        {assistant.messages.length === 0 && !assistant.pending && (
          <div className="py-8 text-center text-sm text-muted">{UI.aiEmptyChat}</div>
        )}
        {assistant.messages.map((message) => (
          <div
            key={message.id}
            className={`ai-chat-bubble ${message.role === 'user' ? 'ai-chat-bubble-user' : 'ai-chat-bubble-assistant'}`}
            data-failed={message.failed || undefined}
          >
            {message.content}
          </div>
        ))}
        {assistant.pending && (
          <div className="ai-chat-bubble ai-chat-bubble-assistant ai-chat-pending">
            {UI.aiThinking}
          </div>
        )}
      </div>

      {assistant.quote && (
        <div className="ai-quote-bar">
          <p className="min-w-0 flex-1 truncate text-xs text-muted">{assistant.quote.text}</p>
          <button
            type="button"
            className="iconbtn shrink-0"
            style={{ width: 26, height: 26, borderRadius: 9 }}
            title={UI.cancel}
            aria-label={UI.cancel}
            onClick={assistant.clearQuote}
          >
            <X size={13} />
          </button>
        </div>
      )}

      <div
        className="ai-chat-input-row"
        data-drag-over={dragOver || undefined}
        onDragOver={(event) => {
          event.preventDefault();
          setDragOver(true);
        }}
        onDragLeave={() => setDragOver(false)}
        onDrop={(event) => {
          event.preventDefault();
          setDragOver(false);
          const text = event.dataTransfer.getData('text/plain');
          if (text.trim()) assistant.attachQuote(text);
        }}
      >
        <textarea
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && !event.shiftKey) {
              event.preventDefault();
              send();
            }
          }}
          className="min-h-[44px] flex-1 resize-none px-3 py-2 text-sm"
          placeholder={UI.aiAskPlaceholder}
          rows={1}
        />
        <button
          type="button"
          className="iconbtn"
          title={UI.aiSend}
          aria-label={UI.aiSend}
          disabled={assistant.pending || !draft.trim()}
          onClick={send}
        >
          <SendHorizonal size={16} />
        </button>
        <button
          type="button"
          className="iconbtn"
          title={UI.cancel}
          aria-label={UI.cancel}
          disabled={assistant.messages.length === 0}
          onClick={assistant.clearMessages}
        >
          <Trash2 size={16} />
        </button>
      </div>
    </div>
  );
}

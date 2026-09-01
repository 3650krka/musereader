import { invoke } from '@tauri-apps/api/core';
import { useCallback, useMemo, useRef, useState } from 'react';
import { useT } from '@/i18n';
import type { AiAssistAction, AiAssistResponse, AiChatTurn } from '@/types';

export interface AiChatMessage extends AiChatTurn {
  id: string;
  model?: string;
  failed?: boolean;
}

interface PendingQuote {
  text: string;
}

const MAX_MESSAGES = 100;

// 段落动作类型（不含 ask/define——ask 走会话、define 走 WordWise 弹卡直调）。
export type ParagraphAction = 'summarize' | 'explain' | 'translate';

// AI 阅读会话运行时：维护对话历史、引用段落、请求状态。
// 后端 ai_assist_paragraph 复用 BYOK 路由（号池/fallback 与翻译链路一致）。
export function useAiAssistant() {
  const t = useT();
  const [messages, setMessages] = useState<AiChatMessage[]>([]);
  const [pending, setPending] = useState(false);
  /* 并发防护 ref：state 闭包在同步双发（双击回车）时两个调用都读到 false。 */
  const pendingRef = useRef(false);
  const [quote, setQuote] = useState<PendingQuote | null>(null);
  const messageIdRef = useRef(0);

  const nextId = () => {
    messageIdRef.current += 1;
    return `ai-${messageIdRef.current}`;
  };

  const history = useMemo<AiChatTurn[]>(
    () =>
      messages
        .filter((message) => !message.failed)
        .map((message) => ({ role: message.role, content: message.content })),
    [messages],
  );

  const request = useCallback(
    async (
      action: AiAssistAction,
      sourceText: string,
      question?: string,
    ): Promise<string | null> => {
      /* 并发防护用 ref（state 闭包在同 tick 双发时双双看到 false）。 */
      if (pendingRef.current) return null;
      pendingRef.current = true;
      setPending(true);
      try {
        const response = await invoke<AiAssistResponse>('ai_assist_paragraph', {
          action,
          sourceText,
          question: question ?? null,
          targetLanguage: '中文',
          history,
        });
        return response.answer;
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        setMessages((current) => [
          ...current.slice(-MAX_MESSAGES),
          {
            id: nextId(),
            role: 'assistant',
            content: `请求失败：${message}`,
            failed: true,
          },
        ]);
        return null;
      } finally {
        pendingRef.current = false;
        setPending(false);
      }
    },
    [history],
  );

  // 段落动作（总结/释义/重译）：结果作为助手消息直接入会话。
  const runParagraphAction = useCallback(
    async (action: ParagraphAction, paragraphText: string) => {
      const text = paragraphText.trim();
      if (!text) return;
      setMessages((current) => [
        ...current.slice(-MAX_MESSAGES),
        { id: nextId(), role: 'user', content: `[${actionLabel(action, t.reader.paraMenu)}] ${clip(text, 120)}` },
      ]);
      const answer = await request(action, text);
      if (answer) {
        setMessages((current) => [
          ...current.slice(-MAX_MESSAGES),
          { id: nextId(), role: 'assistant', content: answer },
        ]);
      }
    },
    [request, t.reader.paraMenu],
  );

  // 会话提问：可携带引用段落（划选拖入或红点「提问」）。
  const ask = useCallback(
    async (question: string) => {
      const trimmed = question.trim();
      if (!trimmed) return;
      const context = quote?.text ?? '';
      setMessages((current) => [
        ...current.slice(-MAX_MESSAGES),
        {
          id: nextId(),
          role: 'user',
          content: context ? `[引用] ${clip(context, 120)}\n${trimmed}` : trimmed,
        },
      ]);
      const answer = await request('ask', context || trimmed, trimmed);
      if (answer) {
        setMessages((current) => [
          ...current.slice(-MAX_MESSAGES),
          { id: nextId(), role: 'assistant', content: answer },
        ]);
      }
      setQuote(null);
    },
    [quote, request],
  );

  const attachQuote = useCallback((text: string) => {
    const trimmed = text.trim();
    if (trimmed) setQuote({ text: trimmed });
  }, []);

  const clearQuote = useCallback(() => setQuote(null), []);

  const clearMessages = useCallback(() => setMessages([]), []);

  return {
    messages,
    pending,
    quote,
    historyLength: history.length,
    runParagraphAction,
    ask,
    attachQuote,
    clearQuote,
    clearMessages,
  };
}

function actionLabel(action: ParagraphAction, labels: { summarize: string; explain: string; retranslate: string }): string {
  switch (action) {
    case 'summarize':
      return labels.summarize;
    case 'explain':
      return labels.explain;
    case 'translate':
      return labels.retranslate;
  }
}

function clip(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max)}…` : text;
}

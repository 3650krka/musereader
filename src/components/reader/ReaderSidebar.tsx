import { motion } from 'motion/react';
import { Sparkles, NotebookPen, ScanText, Search, SpellCheck2, Trash2, X } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Book, EpubSearchResult, ReaderState, ReaderTab, VocabCard } from '@/types';
import { useT } from '@/i18n';
import { AiChatPanel } from './AiChatPanel';
import { ReaderReviewPanel } from './ReaderReviewPanel';
import type { useAiAssistant } from './useAiAssistant';
import { UI } from './readerShared';

interface ReaderSidebarProps {
  book: Book;
  open: boolean;
  activeTab: ReaderTab;
  selectionText: string | null;
  noteDraft: string;
  readerState: ReaderState | null;
  aiAssistant: ReturnType<typeof useAiAssistant>;
  /** 点击生词行打开词卡（复用正文 WordWise 卡；context/contextZh 来自生词库成卡语境）。 */
  onOpenWord?: (
    info: { word: string; level: string; phonetic: string; definition: string },
    context?: string,
    contextZh?: string,
  ) => void;
  onClose: () => void;
  onChangeTab: (tab: ReaderTab) => void;
  onChangeDraft: (value: string) => void;
  onCancelSelection: () => void;
  onSaveNote: (quote: string, note: string) => void;
  /** 笔记/书签/搜索命中点击跳转 EPUB 位置。 */
  onJumpToHref?: (href: string, quote?: string) => void;
  /** 删除划线/书签与批注（允许删除）。 */
  onRemoveBookmark?: (bookmarkId: string) => void;
  onRemoveNote?: (noteId: string) => void;
}

interface WordLevelInfo {
  word: string;
  level: string;
  phonetic: string;
  definition: string;
}

/** 阅读器右侧栏：AI 对话 / 本页生词透析 / 笔记 / 全书搜索。非遮挡并排布局。
    目录已迁至左侧抽屉（TocDrawer），不在此栏。 */
export function ReaderSidebar({
  book, open, activeTab, selectionText, noteDraft, readerState, aiAssistant,
  onOpenWord, onClose, onChangeTab, onChangeDraft, onCancelSelection, onSaveNote,
  onJumpToHref, onRemoveBookmark, onRemoveNote,
}: ReaderSidebarProps) {
  const t = useT();
  /* 窄屏（手机）侧栏加宽：36vw 在 412px 下仅 148px，搜索/笔记不可读；
     桌面保持内容列宽。motion 内联动画无法被 CSS 覆盖，需 JS 侧响应式。 */
  const [narrowScreen, setNarrowScreen] = useState(
    () => typeof window !== 'undefined' && window.matchMedia('(max-width: 640px)').matches,
  );
  useEffect(() => {
    const media = window.matchMedia('(max-width: 640px)');
    const onChange = () => setNarrowScreen(media.matches);
    media.addEventListener('change', onChange);
    return () => media.removeEventListener('change', onChange);
  }, []);

  /* 全书搜索：EPUB 阅读包全文检索（后端 search_epub_reader_content），350ms 防抖。
     无打包 EPUB 的书籍（Markdown/PDF 直读）走 catch 分支显示不可用提示。 */
  const [searchQuery, setSearchQuery] = useState('');
  const [searchResults, setSearchResults] = useState<EpubSearchResult[] | null>(null);
  const [searchLoading, setSearchLoading] = useState(false);
  const [searchUnavailable, setSearchUnavailable] = useState(false);
  useEffect(() => {
    if (!open || activeTab !== 'search') return;
    const query = searchQuery.trim();
    if (!query) {
      setSearchResults(null);
      setSearchLoading(false);
      setSearchUnavailable(false);
      return;
    }
    let cancelled = false;
    setSearchLoading(true);
    const timer = window.setTimeout(() => {
      invoke<EpubSearchResult[]>('search_epub_reader_content', { taskId: book.id, query, limit: 40 })
        .then((rows) => {
          if (cancelled) return;
          setSearchResults(rows);
          setSearchUnavailable(false);
        })
        .catch(() => {
          if (cancelled) return;
          setSearchResults([]);
          setSearchUnavailable(true);
        })
        .finally(() => {
          if (!cancelled) setSearchLoading(false);
        });
    }, 350);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [open, activeTab, searchQuery, book.id]);

  /* 本书生词全集（词汇透析语义）：书生词库 vocabCards ∪ 黑白名单 wordMarks。
     WordWise 标注会自动把超档词播种入 vocabCards，因此该列表即全书透析词。 */
  interface BookWordRow extends WordLevelInfo {
    mark?: 'learning' | 'mastered';
    card?: VocabCard;
  }
  const [bookWords, setBookWords] = useState<BookWordRow[]>([]);
  const [wordFilter, setWordFilter] = useState('');
  useEffect(() => {
    if (!open || activeTab !== 'words') return;
    const cards = readerState?.vocabCards ?? {};
    const marks = readerState?.wordMarks ?? {};
    const words = Array.from(new Set([...Object.keys(cards), ...Object.keys(marks)]));
    if (words.length === 0) {
      setBookWords([]);
      return;
    }
    let cancelled = false;
    invoke<WordLevelInfo[]>('lookup_words_info_batch', { words })
      .then((rows) => {
        if (cancelled) return;
        const rank = new Map([
          ['中考', 1], ['高考', 2], ['四级', 3], ['六级', 4], ['考研', 5],
          ['雅思', 6], ['托福', 7], ['专四', 8], ['专八', 9], ['GRE', 10],
        ]);
        setBookWords(
          rows
            .map((row) => ({
              ...row,
              mark: marks[row.word]?.status,
              card: cards[row.word],
            }))
            .sort(
              (a, b) =>
                (rank.get(b.level) ?? 11) - (rank.get(a.level) ?? 11) ||
                a.word.localeCompare(b.word),
            ),
        );
      })
      .catch(() => {
        if (!cancelled) setBookWords([]);
      });
    return () => {
      cancelled = true;
    };
  }, [open, activeTab, readerState?.vocabCards, readerState?.wordMarks]);

  const shownWords = useMemo(() => {
    const query = wordFilter.trim().toLowerCase();
    if (!query) return bookWords;
    return bookWords.filter(
      (row) => row.word.includes(query) || row.definition.toLowerCase().includes(query),
    );
  }, [bookWords, wordFilter]);

  if (!open) return null;

  const tabs: { id: ReaderTab; label: string; icon: typeof Sparkles }[] = [
    { id: 'search', label: t.reader.search, icon: Search },
    { id: 'ai', label: t.reader.ai, icon: Sparkles },
    { id: 'words', label: t.reader.words, icon: ScanText },
    { id: 'notes', label: t.reader.notesTab, icon: NotebookPen },
    { id: 'review', label: t.reader.reviewTab, icon: SpellCheck2 },
  ];

  return (
    <motion.aside
      initial={{ width: 0, opacity: 0 }}
      animate={{ width: narrowScreen ? '86vw' : 'min(380px, 36vw)', opacity: 1 }}
      exit={{ width: 0, opacity: 0 }}
      transition={{ duration: 0.38, ease: [0.22, 1, 0.36, 1] }}
      className="rside open"
    >
      <div className="rtabs">
        {tabs.map(({ id, label, icon: Icon }) => (
          <button key={id} type="button" className={`rtab ${activeTab === id ? 'on' : ''}`} onClick={() => onChangeTab(id)}>
            <Icon size={14} />
            {label}
          </button>
        ))}
        <button type="button" className="rtab" style={{ flex: 'none', padding: '11px 10px' }} onClick={onClose} aria-label={UI.close}>
          <X size={15} />
        </button>
      </div>

      <div className="rbody">
        {activeTab === 'search' && (
          <div className="rpane on">
            <input
              className="rsearch-input"
              type="search"
              value={searchQuery}
              onChange={(event) => setSearchQuery(event.target.value)}
              placeholder={t.reader.searchPlaceholder}
              autoFocus
            />
            {searchLoading && (
              <p style={{ color: 'var(--ink-3)', fontSize: 13 }}>…</p>
            )}
            {searchUnavailable && (
              <p style={{ color: 'var(--ink-3)', fontSize: 13 }}>{t.reader.searchUnavailable}</p>
            )}
            {!searchUnavailable && !searchLoading && searchResults && searchResults.length === 0 && (
              <p style={{ color: 'var(--ink-3)', fontSize: 13 }}>{t.reader.searchNoResults}</p>
            )}
            {(searchResults ?? []).map((result, index) => (
              <button key={`${result.href}-${index}`} type="button" className="rword rsearch-hit"
                style={{ border: 0, background: 'var(--paper-2)', cursor: 'pointer', width: '100%', font: 'inherit', textAlign: 'left' }}
                onClick={() => onJumpToHref?.(result.href)}>
                <span className="w" style={{ fontWeight: 600, fontSize: 13 }}>{result.title}</span>
                <span className="tier">{result.occurrences}</span>
                <span
                  className="m"
                  style={{ flexBasis: '100%', fontSize: 12 }}
                  dangerouslySetInnerHTML={{ __html: result.snippetHtml || result.snippet }}
                />
              </button>
            ))}
          </div>
        )}

        {activeTab === 'ai' && (
          <div className="rpane on"><AiChatPanel assistant={aiAssistant} /></div>
        )}

        {activeTab === 'words' && (
          <div className="rpane on">
            <input
              className="rsearch-input"
              type="search"
              value={wordFilter}
              onChange={(event) => setWordFilter(event.target.value)}
              placeholder={t.reader.filterWords}
            />
            <p style={{ fontSize: 11.5, letterSpacing: '.1em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>
              {t.reader.bookWords} · {shownWords.length}
              {wordFilter.trim() && bookWords.length !== shownWords.length ? ` / ${bookWords.length}` : ''}
            </p>
            {shownWords.map((w) => (
              <button
                key={w.word}
                type="button"
                className="rword rsearch-hit"
                style={{ border: 0, background: 'var(--paper-2)', cursor: 'pointer', width: '100%', font: 'inherit', textAlign: 'left' }}
                onClick={() => onOpenWord?.(w, w.card?.context, w.card?.contextZh || undefined)}
              >
                <span className="w" style={{ fontWeight: 600, fontSize: 13 }}>{w.word}</span>
                <span className="tier">{w.level}</span>
                <span
                  className="m"
                  style={{ flexBasis: '100%', fontSize: 12, color: w.mark === 'mastered' ? 'var(--ink-3)' : undefined }}
                >
                  {w.mark === 'mastered' ? `${t.reader.wordMastered} · ` : w.mark === 'learning' ? `${t.reader.wordLearning} · ` : ''}
                  {w.definition || w.phonetic}
                </span>
              </button>
            ))}
            {shownWords.length === 0 && (
              <p style={{ color: 'var(--ink-3)', fontSize: 13 }}>
                {bookWords.length === 0 ? t.reader.wordsEmptyHint : '—'}
              </p>
            )}
          </div>
        )}

        {activeTab === 'notes' && (
          <div className="rpane on">
            {selectionText && (
              <div className="rnote">
                <q>“{selectionText}”</q>
                <textarea
                  value={noteDraft}
                  onChange={(e) => onChangeDraft(e.target.value)}
                  placeholder={UI.notePlaceholder}
                  style={{ width: '100%', marginTop: 10, background: 'var(--card)', border: 0, borderRadius: 10, padding: 10, font: 'inherit', fontSize: 13, resize: 'vertical', minHeight: 70 }}
                />
                <div style={{ display: 'flex', gap: 8, marginTop: 8, justifyContent: 'flex-end' }}>
                  <button type="button" className="chip" onClick={onCancelSelection}>{UI.cancel}</button>
                  <button type="button" className="chip on" onClick={() => onSaveNote(selectionText, noteDraft)}>{UI.save}</button>
                </div>
              </div>
            )}
            <p style={{ fontSize: 11.5, letterSpacing: '.1em', textTransform: 'uppercase', color: 'var(--ink-3)' }}>
              {t.reader.pageNotes} · {(readerState?.notes.length ?? 0) + (readerState?.bookmarks.length ?? 0)}
            </p>
            {readerState?.bookmarks.map((b) => (
              <div key={b.id} className="rnote" role={onJumpToHref && b.href ? 'button' : undefined}
                onClick={() => { if (onJumpToHref && b.href) onJumpToHref(b.href, b.quote); }}
                style={onJumpToHref && b.href ? { cursor: 'pointer' } : undefined}
              >
                <div className="rnote-head">
                  <div className={`markline${b.style ? ` mark-${b.style}` : ''}`} />
                  {onRemoveBookmark && (
                    <button
                      type="button"
                      className="rnote-del"
                      aria-label={t.reader.remove}
                      onClick={(e) => { e.stopPropagation(); onRemoveBookmark(b.id); }}
                    >
                      <Trash2 size={12} />
                    </button>
                  )}
                </div>
                <q>“{b.quote}”</q>
                <p style={{ color: 'var(--ink-3)', fontSize: 11.5 }}>{b.chapter} · {b.progress}%</p>
              </div>
            ))}
            {readerState?.notes.map((n) => (
              <div key={n.id} className="rnote" role={onJumpToHref && n.href ? 'button' : undefined}
                onClick={() => { if (onJumpToHref && n.href) onJumpToHref(n.href, n.quote); }}
                style={onJumpToHref && n.href ? { cursor: 'pointer' } : undefined}
              >
                <div className="rnote-head">
                  <div className="markline mark-note" />
                  {onRemoveNote && (
                    <button
                      type="button"
                      className="rnote-del"
                      aria-label={t.reader.remove}
                      onClick={(e) => { e.stopPropagation(); onRemoveNote(n.id); }}
                    >
                      <Trash2 size={12} />
                    </button>
                  )}
                </div>
                <q>“{n.quote}”</q>
                <p>{n.note}</p>
                <p style={{ color: 'var(--ink-3)', fontSize: 11.5 }}>{n.chapter} · {n.progress}%</p>
              </div>
            ))}
            {!readerState?.bookmarks.length && !readerState?.notes.length && (
              <p style={{ color: 'var(--ink-3)', fontSize: 13 }}>{UI.emptyNotes}</p>
            )}
          </div>
        )}

        {activeTab === 'review' && (
          <ReaderReviewPanel bookId={book.id} chapterTitle={readerState?.chapter ?? ''} />
        )}
      </div>
    </motion.aside>
  );
}

import { motion } from 'motion/react';
import { ChevronRight, List, Search, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { EpubSearchResult } from '@/types';
import { useT } from '@/i18n';
import { useDismissOnEscape } from '@/hooks/useDismissOnEscape';
import type { ReaderFrameTocItem } from './epubReaderTypes';
import { UI } from './readerShared';

interface TocDrawerProps {
  open: boolean;
  toc: ReaderFrameTocItem[];
  /** 当前阅读位置 href（用于当前章高亮）。 */
  currentHref?: string | null;
  /** 全书搜索所属任务（book.id）。 */
  taskId: string;
  onClose: () => void;
  onGoTo: (href: string) => void;
}

/**
 * 目录抽屉（左侧滑出）：目录列表 + 全文搜索。
 * 空查询显示目录树；输入后同时给出「章节标题匹配」与「全文命中」两组结果。
 */
export function TocDrawer({ open, toc, currentHref, taskId, onClose, onGoTo }: TocDrawerProps) {
  const t = useT();
  const [query, setQuery] = useState('');
  const [hits, setHits] = useState<EpubSearchResult[] | null>(null);
  const [searchUnavailable, setSearchUnavailable] = useState(false);

  /* 全文搜索（350ms 防抖），复用 EPUB 阅读包检索 */
  useEffect(() => {
    if (!open) return;
    const trimmed = query.trim();
    if (!trimmed) {
      setHits(null);
      setSearchUnavailable(false);
      return;
    }
    let cancelled = false;
    const timer = window.setTimeout(() => {
      invoke<EpubSearchResult[]>('search_epub_reader_content', { taskId, query: trimmed, limit: 30 })
        .then((rows) => {
          if (cancelled) return;
          setHits(rows);
          setSearchUnavailable(false);
        })
        .catch(() => {
          if (cancelled) return;
          setHits([]);
          setSearchUnavailable(true);
        });
    }, 350);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [open, query, taskId]);

  /* Esc 关闭 */
  useDismissOnEscape(open ? onClose : null);

  if (!open) return null;

  const trimmed = query.trim();
  const titleMatches = trimmed
    ? toc.filter((item) => item.title.toLowerCase().includes(trimmed.toLowerCase()))
    : [];

  return (
    <>
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        className="tocdrawer-backdrop"
        onClick={onClose}
      />
      <motion.aside
        initial={{ x: '-100%' }}
        animate={{ x: 0 }}
        exit={{ x: '-100%' }}
        transition={{ duration: 0.32, ease: [0.22, 1, 0.36, 1] }}
        className="tocdrawer"
        role="dialog"
        aria-label={t.reader.toc}
      >
        <div className="tocdrawer-head">
          <List size={16} />
          <span>{t.reader.toc} · {toc.length}</span>
          <button type="button" className="iconbtn" style={{ width: 30, height: 30, marginLeft: 'auto' }} onClick={onClose} aria-label={UI.close}>
            <X size={15} />
          </button>
        </div>
        <div className="tocdrawer-search">
          <Search size={14} />
          <input
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t.reader.searchPlaceholder}
            autoFocus
          />
        </div>

        <div className="tocdrawer-body">
          {!trimmed && (
            <TocList toc={toc} currentHref={currentHref} onGoTo={onGoTo} />
          )}
          {trimmed && titleMatches.length > 0 && (
            <>
              <p className="tocdrawer-group">{t.reader.toc} · {titleMatches.length}</p>
              <TocList toc={titleMatches} currentHref={currentHref} onGoTo={onGoTo} />
            </>
          )}
          {trimmed && (
            <>
              <p className="tocdrawer-group">{t.reader.search} · {hits?.length ?? 0}</p>
              {searchUnavailable && <p className="tocdrawer-empty">{t.reader.searchUnavailable}</p>}
              {!searchUnavailable && hits && hits.length === 0 && (
                <p className="tocdrawer-empty">{t.reader.searchNoResults}</p>
              )}
              {(hits ?? []).map((result, index) => (
                <button
                  key={`${result.href}-${index}`}
                  type="button"
                  className="tocdrawer-item hit"
                  onClick={() => onGoTo(result.href)}
                >
                  <span className="t">{result.title}</span>
                  <span className="n">{result.occurrences}</span>
                  <span className="s">{result.snippet}</span>
                </button>
              ))}
            </>
          )}
        </div>
      </motion.aside>
    </>
  );
}

function TocList({
  toc,
  currentHref,
  onGoTo,
}: {
  toc: ReaderFrameTocItem[];
  currentHref?: string | null;
  onGoTo: (href: string) => void;
}) {
  if (toc.length === 0) return <p className="tocdrawer-empty">—</p>;
  const currentPath = splitHrefPath(currentHref);
  return (
    <>
      {toc.map((item, index) => {
        const active = currentPath !== '' && splitHrefPath(item.href) === currentPath;
        /* 归并子条目（NCX 未收录的章节分片）缩进弱化，从属于其章节；
           一级条目保持主导航观感。 */
        const sub = item.depth > 0;
        return (
          <button
            key={`${item.href}-${index}`}
            type="button"
            className={`tocdrawer-item ${active ? 'on' : ''} ${sub ? 'sub' : ''}`}
            onClick={() => onGoTo(item.href)}
          >
            <span className="t">{item.title}</span>
            <ChevronRight size={14} className="go" />
          </button>
        );
      })}
    </>
  );
}

/** href 归一化为路径部分（去锚点/查询），用于当前章匹配。 */
function splitHrefPath(href?: string | null): string {
  if (!href) return '';
  const hash = href.indexOf('#');
  return (hash < 0 ? href : href.slice(0, hash)).split('/').pop() ?? '';
}

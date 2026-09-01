import { ArrowLeft, MoreHorizontal, Search } from 'lucide-react';
import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { cn } from '@/lib/utils';
import { displayBookTitle } from '@/components/library/bookDisplay';
import { useT } from '@/i18n';
import type { Book } from '@/types';
import type { ReaderFlow, ReaderLang } from './useEpubReader';
import {
  IconBilingual,
  IconNote,
  IconPageScroll,
  IconPageSingle,
  IconPageSpread,
  IconReadAloud,
  IconReadingLamp,
  IconReviewDesk,
  IconReviewMode,
  IconToc,
  IconWordWise,
} from './readerIcons';

interface ReaderChromeProps {
  book: Book;
  visible: boolean;
  chapter: string;
  bilingualMode: boolean;
  wordWiseEnabled: boolean;
  spreadMode: boolean;
  /** 当前翻页模式（单页/双页/滚动），顶栏双栏钮按此显示与循环切换。 */
  flowMode: ReaderFlow;
  typographyOpen: boolean;
  sidebarOpen: boolean;
  ttsSpeaking: boolean;
  ttsLoading: boolean;
  onBack: () => void;
  onCycleTheme: () => void;
  onToggleBilingual: () => void;
  onToggleWordWise: () => void;
  onToggleSpread: () => void;
  onToggleTypography: () => void;
  onOpenToc: () => void;
  onOpenSearch: () => void;
  onOpenNotes: () => void;
  onToggleTts: () => void;
  /** 打开当前书的校对工作台（可选：非翻译产物书不显示）。 */
  onOpenReview?: () => void;
  /** 阅读器内校对模式（直接在阅读器修改正文与目录）。 */
  reviewMode?: boolean;
  onToggleReviewMode?: () => void;
  /** 语言模式（双语/中文/英文），替代单纯 bilingual 开关。 */
  langMode?: ReaderLang;
  onSetLang?: (lang: ReaderLang) => void;
}

/** 沉浸阅读器顶栏：默认隐藏，随 chromeVisible 滑入。
    按钮序（按阅读动线排布）：导航(目录/搜索) → 内容形态(语言/生词/版式) →
    校对(模式/工作台) → 随读工具(笔记/排版/朗读/主题) → 更多。 */
export function ReaderChrome({
  book, visible, chapter, bilingualMode, wordWiseEnabled, spreadMode, flowMode,
  typographyOpen, sidebarOpen, ttsSpeaking, ttsLoading,
  onBack, onCycleTheme, onToggleBilingual, onToggleWordWise, onToggleSpread,
  onToggleTypography, onOpenToc, onOpenSearch, onOpenNotes, onToggleTts, onOpenReview,
  langMode = 'both', onSetLang, reviewMode = false, onToggleReviewMode,
}: ReaderChromeProps) {
  const t = useT();
  const langLabel = langMode === 'both' ? t.reader.bilingual : langMode === 'zh' ? '中文' : 'EN';
  /* 翻页模式三态：单页 / 双页展开 / 滚动，点击循环切换。 */
  const flowLabel =
    flowMode === 'spread'
      ? t.reader.spread
      : flowMode === 'scrolled'
        ? t.reader.scrolled
        : t.reader.single;
  const FlowIcon =
    flowMode === 'spread' ? IconPageSpread : flowMode === 'scrolled' ? IconPageScroll : IconPageSingle;
  /* 移动端「更多」底栏弹层：≤1080px 顶栏只留 返回/标题/目录/排版/更多，
     其余动作（对照/生词/版式/校对/工作台/搜索/笔记/朗读/主题）收进底部动作面板。 */
  const [moreOpen, setMoreOpen] = useState(false);
  useEffect(() => {
    if (!visible) setMoreOpen(false);
  }, [visible]);
  const moreActions = [
    { key: 'lang', icon: IconBilingual, label: langLabel, active: langMode === 'both', run: onToggleBilingual },
    { key: 'words', icon: IconWordWise, label: t.reader.words, active: wordWiseEnabled, run: onToggleWordWise },
    { key: 'spread', icon: FlowIcon, label: flowLabel, active: spreadMode, run: onToggleSpread },
    /* 校对模式必须进更多面板：窄屏下顶栏两枚校对控件（chip+图标）都会
       display:none，缺此入口则校对在手机上完全不可达。 */
    ...(onToggleReviewMode
      ? [{ key: 'review', icon: IconReviewMode, label: t.reader.reviewModeShort, active: reviewMode, run: onToggleReviewMode }]
      : []),
    /* 工作台入口与模式开关并置；≤900px 中间档顶栏图标钮收起后由此触达。 */
    ...(onOpenReview
      ? [{ key: 'reviewDesk', icon: IconReviewDesk, label: t.reader.openReview, active: false, run: onOpenReview }]
      : []),
    { key: 'search', icon: Search, label: t.reader.search, active: false, run: onOpenSearch },
    { key: 'notes', icon: IconNote, label: t.reader.notesTab, active: sidebarOpen, run: onOpenNotes },
    { key: 'tts', icon: IconReadAloud, label: t.reader.tts, active: ttsSpeaking, run: onToggleTts },
    { key: 'theme', icon: IconReadingLamp, label: t.app.theme, active: false, run: onCycleTheme },
  ];
  return (
    <div className={cn('chrome top', visible && 'show')}>
      <button type="button" className="iconbtn" onClick={onBack} aria-label={t.reader.back} style={{ width: 38, height: 38 }}>
        <ArrowLeft size={18} />
      </button>
      <div className="min-w-0">
        <div className="title truncate">{displayBookTitle(book, t.library.untitledBook)}</div>
        <div className="where truncate">{chapter}</div>
      </div>
      <div className="grow" />

      {/* 导航 */}
      <button type="button" className="iconbtn" style={{ width: 38, height: 38 }} onClick={onOpenToc} aria-label={t.reader.toc} title={t.reader.toc}>
        <IconToc size={17} />
      </button>
      <button type="button" className="iconbtn chrome-opt" style={{ width: 38, height: 38 }} onClick={onOpenSearch} aria-label={t.reader.search} title={t.reader.search}>
        <Search size={17} />
      </button>

      {/* 内容形态 */}
      {onSetLang ? (
        <button type="button" className={cn('chip chrome-chip', langMode === 'both' && 'on')} onClick={onToggleBilingual} title={langLabel}>
          <IconBilingual size={14} />
          {langLabel}
        </button>
      ) : (
        <button type="button" className={cn('chip chrome-chip', bilingualMode && 'on')} onClick={onToggleBilingual}>
          <IconBilingual size={14} />
          {t.reader.bilingual}
        </button>
      )}
      <button type="button" className={cn('chip chrome-chip', wordWiseEnabled && 'on')} onClick={onToggleWordWise} title={t.reader.words}>
        <IconWordWise size={14} />
        {t.reader.words}
      </button>
      <button type="button" className={cn('chip chrome-chip', spreadMode && 'on')} onClick={onToggleSpread} title={flowLabel}>
        <FlowIcon size={14} />
        {flowLabel}
      </button>

      {/* 校对：模式开关（章内直改）+ 工作台入口（整书修订），并置以免混淆 */}
      {onToggleReviewMode && (
        <button
          type="button"
          className={`chip chrome-chip ${reviewMode ? 'on' : ''}`}
          onClick={onToggleReviewMode}
          title={t.reader.reviewMode}
        >
          <IconReviewMode size={14} />
          {t.reader.reviewModeShort}
        </button>
      )}
      {onOpenReview && (
        <button type="button" className="iconbtn chrome-opt" style={{ width: 38, height: 38 }} onClick={onOpenReview} aria-label={t.reader.openReview} title={t.reader.openReview}>
          <IconReviewDesk size={17} />
        </button>
      )}

      {/* 随读工具 */}
      <button type="button" className="iconbtn chrome-opt" style={{ width: 38, height: 38 }} onClick={onOpenNotes} aria-label={t.reader.notesTab} data-active={sidebarOpen} title={t.reader.notesTab}>
        <IconNote size={17} />
      </button>
      <button type="button" className={cn('iconbtn', typographyOpen && 'on')} style={{ width: 38, height: 38 }} onClick={onToggleTypography} aria-label={t.reader.typography} title={t.reader.typography}>
        <SlidersIcon size={17} />
      </button>
      <button type="button" className="iconbtn chrome-opt" style={{ width: 38, height: 38 }} onClick={onToggleTts} disabled={ttsLoading} aria-label={t.reader.tts} data-active={ttsSpeaking} title={t.reader.tts}>
        <IconReadAloud size={17} />
      </button>
      <button type="button" className="iconbtn chrome-opt" style={{ width: 38, height: 38 }} onClick={onCycleTheme} aria-label={t.app.theme} title={t.app.theme}>
        <IconReadingLamp size={17} />
      </button>
      <button type="button" className="iconbtn chrome-more" style={{ width: 38, height: 38 }} onClick={() => setMoreOpen((v) => !v)} aria-label={t.app.more} data-active={moreOpen}>
        <MoreHorizontal size={17} />
      </button>

      {moreOpen && createPortal(
        <>
          <button type="button" className="sheet-backdrop reader-more-backdrop" onClick={() => setMoreOpen(false)} aria-label={t.app.more} />
          <div className="more-sheet reader-more-sheet" role="menu">
            <div className="more-sheet-grid">
              {moreActions.map(({ key, icon: Icon, label, active, run }) => (
                <button key={key} type="button" role="menuitem"
                  className={cn('more-item', active && 'active')}
                  onClick={() => { setMoreOpen(false); run(); }}>
                  <Icon size={18} />
                  <span>{label}</span>
                </button>
              ))}
            </div>
          </div>
        </>,
        document.body,
      )}
    </div>
  );
}

/** 排版面板：滑块齿轮（保留 lucide 原语，语义即「调节」）。 */
function SlidersIcon({ size = 17 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor"
      strokeWidth={1.75} strokeLinecap="round" strokeLinejoin="round">
      <path d="M4 8.5h9M17.5 8.5H20M4 15.5h2.5M11 15.5h9" />
      <circle cx="15.2" cy="8.5" r="2.3" />
      <circle cx="8.8" cy="15.5" r="2.3" />
    </svg>
  );
}

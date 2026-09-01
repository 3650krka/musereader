import { AnimatePresence, motion } from 'motion/react';
import {
  Globe,
  MoreHorizontal,
  Settings as SettingsIcon,
  Sun,
} from 'lucide-react';
import { useEffect, useState } from 'react';
import { LangContext } from './i18n/context';
import { useT } from './i18n';
import { cn } from './lib/utils';
import {
  IconInsights,
  IconLearning,
  IconLibrary,
  IconNotes,
  IconReview,
  IconTasks,
  IconTerms,
} from './components/common/appIcons';
import { Logo } from './components/common/Logo';
import type { Palette, Theme } from './types';
import type { AppView } from './appShared';

interface AppShellProps {
  currentView: AppView;
  theme: Theme;
  /** 强调色系（与明暗主题正交），落到 data-palette。 */
  palette: Palette;
  isReader: boolean;
  stageKey: string;
  lang: 'zh' | 'en';
  setLang: (lang: 'zh' | 'en') => void;
  children: React.ReactNode;
  onNavigate: (view: AppView) => void;
  onCycleTheme: () => void;
}

interface RailEntry {
  view: AppView;
  icon: typeof IconLibrary;
  labelKey: 'library' | 'notes' | 'learning' | 'review' | 'terms' | 'insights' | 'tasks' | 'settings';
  /** 底部栏主槽（竖屏精简模式）；缺省进「更多」面板。 */
  dock?: boolean;
}

const RAIL_ENTRIES: RailEntry[] = [
  { view: 'library', icon: IconLibrary, labelKey: 'library', dock: true },
  { view: 'notes', icon: IconNotes, labelKey: 'notes', dock: true },
  { view: 'learning', icon: IconLearning, labelKey: 'learning', dock: true },
  { view: 'review', icon: IconReview, labelKey: 'review' },
  { view: 'terms', icon: IconTerms, labelKey: 'terms' },
  { view: 'insights', icon: IconInsights, labelKey: 'insights', dock: true },
  { view: 'tasks', icon: IconTasks, labelKey: 'tasks', dock: true },
];

const SLIM_NAV_MEDIA = '(hover: none) and (pointer: coarse), (max-width: 640px)';

/** 触控设备走底部栏；Windows 鼠标窗口即使较窄也保留左侧栏。 */
function useSlimNav() {
  const [slim, setSlim] = useState(
    () => typeof window !== 'undefined' &&
      window.matchMedia(SLIM_NAV_MEDIA).matches,
  );
  useEffect(() => {
    const mq = window.matchMedia(SLIM_NAV_MEDIA);
    const onChange = () => setSlim(mq.matches);
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
  }, []);
  return slim;
}

export function AppShell({
  currentView,
  theme,
  palette,
  isReader,
  stageKey,
  lang,
  setLang,
  children,
  onNavigate,
  onCycleTheme,
}: AppShellProps) {
  const t = useT();
  const slim = useSlimNav();
  const [moreOpen, setMoreOpen] = useState(false);

  /* 主题/色系必须镜像到 <html>：CSS 变量在 body/:root 就已被解析（body { color: var(--ink) }），
     data-theme 只挂 app-frame 时，body 与一切 portal 到 body 的浮层（学习全屏/菜单/模态/Toast）
     仍拿浅色 --ink → 暗黑下黑字不可见。挂到根节点后全局与 portal 一致重解析。 */
  useEffect(() => {
    const root = document.documentElement;
    root.dataset.theme = theme;
    root.dataset.palette = palette;
  }, [theme, palette]);

  const navTo = (view: AppView) => {
    setMoreOpen(false);
    onNavigate(view);
  };

  /* 全局快捷键支持：Ctrl/Cmd + 1~6 切换核心主视图，Ctrl/Cmd + , 进设置，Ctrl/Cmd + T 切主题 */
  useEffect(() => {
    const onGlobalKeyDown = (e: KeyboardEvent) => {
      const isCmdOrCtrl = e.metaKey || e.ctrlKey;
      if (!isCmdOrCtrl) return;
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement || (e.target as HTMLElement)?.isContentEditable) {
        return;
      }
      if (e.key === '1') { e.preventDefault(); onNavigate('library'); }
      else if (e.key === '2') { e.preventDefault(); onNavigate('notes'); }
      else if (e.key === '3') { e.preventDefault(); onNavigate('learning'); }
      else if (e.key === '4') { e.preventDefault(); onNavigate('review'); }
      else if (e.key === '5') { e.preventDefault(); onNavigate('terms'); }
      else if (e.key === '6') { e.preventDefault(); onNavigate('insights'); }
      else if (e.key === ',') { e.preventDefault(); onNavigate('settings'); }
      else if (e.key.toLowerCase() === 't' && !e.shiftKey) { e.preventDefault(); onCycleTheme(); }
    };
    window.addEventListener('keydown', onGlobalKeyDown);
    return () => window.removeEventListener('keydown', onGlobalKeyDown);
  }, [onNavigate, onCycleTheme]);

  return (
    <LangContext.Provider value={{ lang, setLang }}>
      <div className="app-frame" data-theme={theme} data-palette={palette}>
        {!isReader && (
          <div className="app-shell">
            {/* 纯图标栏（原型 .rail）；竖屏/窄屏由 CSS 隐藏，走底部栏 */}
            <aside className="app-rail">
              <button
                type="button"
                className="rail-logo"
                onClick={() => onNavigate('library')}
                onDoubleClick={() => setLang(lang === 'zh' ? 'en' : 'zh')}
                title="MuseReader"
                aria-label="MuseReader"
              >
                <Logo size={22} />
              </button>

              <nav className="rail-nav">
                {RAIL_ENTRIES.map(({ view, icon: Icon, labelKey }) => (
                  <button
                    key={view}
                    type="button"
                    className={cn('rail-btn', currentView === view && 'active')}
                    onClick={() => onNavigate(view)}
                    aria-label={t.app[labelKey]}
                    aria-current={currentView === view ? 'page' : undefined}
                  >
                    <Icon size={20} strokeWidth={1.7} />
                    <span className="tip">{t.app[labelKey]}</span>
                  </button>
                ))}
              </nav>

              <div className="rail-spacer" />

              <button type="button" className="rail-btn" onClick={onCycleTheme} aria-label={t.app.theme}>
                <Sun size={20} strokeWidth={1.7} />
                <span className="tip">{t.app.theme}</span>
              </button>
              <button
                type="button"
                className={cn('rail-btn', currentView === 'settings' && 'active')}
                onClick={() => onNavigate('settings')}
                aria-label={t.app.settings}
              >
                <SettingsIcon size={20} strokeWidth={1.7} />
                <span className="tip">{t.app.settings}</span>
              </button>
            </aside>

            <main className="app-main relative flex min-h-screen flex-col">
              <div className="app-stage flex-1">
                <AnimatePresence mode="wait" initial={false}>
                  <motion.div
                    key={stageKey}
                    initial={{ opacity: 0, y: 14 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: -10 }}
                    transition={{ duration: 0.34, ease: [0.22, 1, 0.36, 1] }}
                    className="app-route-shell"
                  >
                    {children}
                  </motion.div>
                </AnimatePresence>
              </div>

              {/* 移动端底部图标条：竖屏精简主槽(5) + 更多面板（被精简的按钮进这里） */}
              <nav className="app-rail-mobile">
                {RAIL_ENTRIES.filter((e) => e.dock).map(
                  ({ view, icon: Icon, labelKey }) => (
                    <button
                      key={view}
                      type="button"
                      className={cn('rail-btn', currentView === view && 'active')}
                      onClick={() => navTo(view)}
                      aria-label={t.app[labelKey]}
                    >
                      <Icon size={20} strokeWidth={1.7} />
                    </button>
                  ),
                )}
                <button
                  type="button"
                  className={cn('rail-btn', moreOpen && 'active')}
                  onClick={() => setMoreOpen((v) => !v)}
                  aria-label={t.app.more}
                  aria-expanded={moreOpen}
                >
                  <MoreHorizontal size={20} strokeWidth={1.7} />
                </button>
              </nav>

              {/* 竖屏「更多」面板：校对/术语/设置 + 主题/语言/剪藏 */}
              {slim && moreOpen && (
                <>
                  <button
                    type="button"
                    className="sheet-backdrop"
                    onClick={() => setMoreOpen(false)}
                    aria-label={t.app.more}
                  />
                  <div className="more-sheet" role="dialog" aria-modal="true" aria-label={t.app.more}>
                    <div className="more-sheet-grid">
                      {RAIL_ENTRIES.filter((e) => !e.dock).map(({ view, icon: Icon, labelKey }) => (
                        <button
                          key={view}
                          type="button"
                          className={cn('more-item', currentView === view && 'active')}
                          onClick={() => navTo(view)}
                        >
                          <Icon size={20} strokeWidth={1.7} />
                          <span>{t.app[labelKey]}</span>
                        </button>
                      ))}
                      <button type="button" className="more-item" onClick={() => { onCycleTheme(); setMoreOpen(false); }}>
                        <Sun size={20} strokeWidth={1.7} />
                        <span>{t.app.theme}</span>
                      </button>
                      <button
                        type="button"
                        className={cn('more-item', currentView === 'settings' && 'active')}
                        onClick={() => navTo('settings')}
                      >
                        <SettingsIcon size={20} strokeWidth={1.7} />
                        <span>{t.app.settings}</span>
                      </button>
                      <button
                        type="button"
                        className="more-item"
                        onClick={() => { setLang(lang === 'zh' ? 'en' : 'zh'); setMoreOpen(false); }}
                      >
                        <Globe size={20} strokeWidth={1.7} />
                        <span>{lang === 'zh' ? 'EN' : '中文'}</span>
                      </button>
                    </div>
                  </div>
                </>
              )}
            </main>
          </div>
        )}
        {isReader && children}
      </div>
    </LangContext.Provider>
  );
}

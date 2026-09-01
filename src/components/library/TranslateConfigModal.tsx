import { invoke } from '@tauri-apps/api/core';
import { open as openFileDialog } from '@tauri-apps/plugin-dialog';
import { Search, Upload, X } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { useT } from '@/i18n';
import type { Book, GlossaryDeckView, SkillLibraryEntry } from '@/types';

export interface TranslateConfig {
  articleType: string;
  concurrent: boolean;
  maxWorkers: number;
  deckIds: string[];
  /** 注入的 skill（全景翻译等，本次会话生效）。 */
  skillIds: string[];
  /** 号池名（写回 runtime config active_pool）。 */
  poolName: string | null;
}

interface RoutePoolLite {
  name: string;
  active: boolean;
}

interface TranslateConfigModalProps {
  book: Book;
  glossaryDecks: GlossaryDeckView[];
  selectedDeckIds: string[];
  toggleDeck: (deckId: string) => void;
  /** 可注入的 skill（翻译类）。 */
  skills?: SkillLibraryEntry[];
  onClose: () => void;
  onStart: (config: TranslateConfig) => Promise<void>;
  /* 新建翻译选书模式：提供书库可选与上传入口；确认后回传所选路径。 */
  pick?: {
    books: Book[];
    onPicked: (sourcePath: string, fallbackTitle: string, config: TranslateConfig) => void;
  };
}

interface RoutePoolLite {
  name: string;
  active: boolean;
}

/** 书库候选行：封面 + 书名 + 原文件路径。 */
interface BookOption {
  id: string;
  title: string;
  cover?: string | null;
  sourcePath: string;
}

const ARTICLE_TYPES = [
  'fiction', 'academic', 'nonfiction', 'tech_doc', 'textbook',
  'business', 'children', 'blog', 'news', 'legal', 'general',
] as const;

/* 默认项记忆（localStorage）：下次打开自动填充。 */
const DEFAULTS_KEY = 'mt.translate.defaults';
interface TranslateDefaults {
  articleType?: string;
  concurrent?: boolean;
  maxWorkers?: number;
  skillIds?: string[];
  poolName?: string | null;
}
function loadDefaults(): TranslateDefaults {
  try {
    return JSON.parse(localStorage.getItem(DEFAULTS_KEY) ?? '{}') as TranslateDefaults;
  } catch {
    return {};
  }
}
function saveDefaults(cfg: TranslateConfig) {
  try {
    localStorage.setItem(DEFAULTS_KEY, JSON.stringify({
      articleType: cfg.articleType,
      concurrent: cfg.concurrent,
      maxWorkers: cfg.maxWorkers,
      skillIds: cfg.skillIds,
      poolName: cfg.poolName,
    } satisfies TranslateDefaults));
  } catch { /* 忽略存储不可用 */ }
}

/** 翻译前配置弹窗：可配置项全览 + 默认项记忆。
    与直接导入（上传即翻）不同——所有配置先确认再开翻。 */
export function TranslateConfigModal({
  book,
  glossaryDecks,
  selectedDeckIds,
  toggleDeck,
  skills = [],
  onClose,
  onStart,
  pick,
}: TranslateConfigModalProps) {
  const t = useT();
  const defaults = useMemo(loadDefaults, []);
  const [articleType, setArticleType] = useState<string>(defaults.articleType ?? book.category ?? 'academic');
  const [skillIds, setSkillIds] = useState<string[]>(defaults.skillIds ?? []);
  const [pools, setPools] = useState<RoutePoolLite[]>([]);
  const [poolName, setPoolName] = useState<string | null>(defaults.poolName ?? null);
  const [deckQuery, setDeckQuery] = useState('');
  const [starting, setStarting] = useState(false);
  /* 并发：并发由号池自适应调度（健康度×速度+全局令牌桶）管理，
     不在任务配置里手填——固定 concurrent=true / max_workers 默认档。 */
  const concurrent = true;
  const maxWorkers = 10;
  /* 选书模式：上传或书库选择（bookQuery 搜索过滤）。 */
  const [pickedPath, setPickedPath] = useState('');
  const [pickedTitle, setPickedTitle] = useState('');
  const [bookQuery, setBookQuery] = useState('');
  const bookOptions = useMemo<BookOption[]>(() => {
    if (!pick) return [];
    const seen = new Set<string>();
    const options: BookOption[] = [];
    for (const b of pick.books) {
      const sourcePath = b.originalPath ?? b.primaryArtifactPath ?? '';
      if (!sourcePath || seen.has(sourcePath)) continue;
      seen.add(sourcePath);
      options.push({ id: b.id, title: b.title || sourcePath.split(/[\\/]/).pop() || '—', cover: b.cover, sourcePath });
    }
    return options;
  }, [pick]);
  const filteredBooks = useMemo(() => {
    const q = bookQuery.trim().toLowerCase();
    return q ? bookOptions.filter((b) => b.title.toLowerCase().includes(q)) : bookOptions;
  }, [bookOptions, bookQuery]);
  const pickReady = !pick || Boolean(pickedPath);

  /* 号池列表 + 当前激活（展示与选择，不改保存——开翻时写回） */
    /* 滚动穿透锁：模态打开期间冻结底层页面滚动。 */
  useEffect(() => {
    const prev = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    return () => { document.body.style.overflow = prev; };
  }, []);

  useEffect(() => {
    invoke<RoutePoolLite[]>('list_route_pools')
      .then((rows) => {
        setPools(rows);
        setPoolName((cur) => cur ?? rows.find((p) => p.active)?.name ?? rows[0]?.name ?? null);
      })
      .catch(() => setPools([]));
  }, []);

  const filteredDecks = useMemo(() => {
    const q = deckQuery.trim().toLowerCase();
    return q ? glossaryDecks.filter((d) => d.name.toLowerCase().includes(q)) : glossaryDecks;
  }, [glossaryDecks, deckQuery]);

  const toggleSkill = (id: string) => {
    setSkillIds((cur) => (cur.includes(id) ? cur.filter((s) => s !== id) : [...cur, id]));
  };

  const start = async () => {
    if (starting || !pickReady) return;
    setStarting(true);
    try {
      const cfg: TranslateConfig = { articleType, concurrent, maxWorkers, deckIds: selectedDeckIds, skillIds, poolName };
      /* 号池切换：读-改-写 runtime config（activePool 即时生效于本次翻译管线）。 */
      if (poolName) {
        try {
          const current = await invoke<Record<string, unknown>>('get_runtime_config');
          await invoke('update_runtime_config', { config: { ...current, activePool: poolName } });
        } catch (error) {
          console.error('switch pool failed:', error);
        }
      }
      saveDefaults(cfg);
      if (pick && pickedPath) {
        pick.onPicked(pickedPath, pickedTitle, cfg);
        return;
      }
      await onStart(cfg);
    } finally {
      setStarting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-[120]" role="dialog" aria-label={t.library.configTitle}>
      <button type="button" className="absolute inset-0 bg-black/25" onClick={onClose} aria-label={t.tasks.cancel} />
      <div className="translate-config-modal">
        <div className="tcm-head">
          <div className="min-w-0">
            <h3 className="text-lg font-semibold truncate">{book.title}</h3>
            <p className="text-xs text-muted mt-1">{t.library.configTitle}</p>
          </div>
          <button type="button" className="iconbtn" onClick={onClose} aria-label={t.tasks.cancel}>
            <X size={16} />
          </button>
        </div>

        <div className="tcm-body">
          {/* 选书模式：上传长条 dropzone + 书库选书 */}
          {pick && (
            <div className="tcm-row tcm-decks">
              <span className="tcm-label">{t.library.cfgSource}</span>
              <button
                type="button"
                className="tcm-dropzone"
                onClick={() => {
                  void openFileDialog({
                    multiple: false,
                    filters: [{ name: 'Books', extensions: ['pdf', 'epub', 'docx', 'md', 'txt'] }],
                  }).then((path) => {
                    if (typeof path === 'string' && path) {
                      setPickedPath(path);
                      setPickedTitle(path.split(/[\\/]/).pop() ?? path);
                    }
                  });
                }}
              >
                <Upload size={20} strokeWidth={1.6} />
                <span>{t.library.importFile}</span>
                <em>PDF / EPUB / DOCX / MD / TXT</em>
              </button>
              <div className="tcm-deck-search">
                <Search size={13} />
                <input
                  value={bookQuery}
                  placeholder={t.library.cfgPickBook}
                  onChange={(e) => setBookQuery(e.target.value)}
                />
              </div>
              <div className="tcm-deck-list" style={{ maxHeight: 150 }}>
                {filteredBooks.slice(0, 30).map((option) => (
                  <label key={option.id} className={`tcm-deck ${pickedPath === option.sourcePath ? 'picked' : ''}`}>
                    <input
                      type="radio"
                      name="tcm-book-pick"
                      checked={pickedPath === option.sourcePath}
                      onChange={() => {
                        setPickedPath(option.sourcePath);
                        setPickedTitle(option.title);
                      }}
                    />
                    <span className="truncate">{option.title}</span>
                  </label>
                ))}
                {filteredBooks.length === 0 && <p className="tcm-empty">{t.library.cfgNoBook}</p>}
              </div>
              {pickedPath && (
                <p style={{ fontSize: 12, color: 'var(--ink-3)', wordBreak: 'break-all' }}>
                  {t.library.cfgSelected}: {pickedTitle || pickedPath}
                </p>
              )}
            </div>
          )}

          {/* 文章类型 + Prompt/Skill 注入（同类归组——文章类型决定 prompt 模板，Skill 是注入项） */}
          <div className="tcm-group">
            <div className="tcm-group-title">{t.library.cfgPromptGroup}</div>
            <div className="tcm-row">
              <span className="tcm-label">{t.library.cfgArticleType}</span>
              <div className="tcm-types">
                {ARTICLE_TYPES.map((type) => (
                  <button
                    key={type}
                    type="button"
                    className={`chip sm ${articleType === type ? 'on' : ''}`}
                    onClick={() => setArticleType(type)}
                  >
                    {type}
                  </button>
                ))}
              </div>
            </div>
            {skills.length > 0 && (
              <div className="tcm-row tcm-decks">
                <span className="tcm-label">{t.library.cfgSkillInject}</span>
                <div className="tcm-deck-list">
                  {skills.map((skill) => (
                    <label key={skill.id} className="tcm-deck">
                      <input
                        type="checkbox"
                        checked={skillIds.includes(skill.id)}
                        onChange={() => toggleSkill(skill.id)}
                      />
                      <span className="truncate">{skill.title}</span>
                    </label>
                  ))}
                </div>
              </div>
            )}
          </div>

          {/* 号池/模型路由（自适应并发在此管理，任务配置不再提供并发滑条） */}
          {pools.length > 0 && (
            <div className="tcm-group">
              <div className="tcm-group-title">{t.library.cfgRouteGroup}</div>
              <div className="tcm-row">
                <span className="tcm-label">{t.library.cfgPool}</span>
                <div className="tcm-types">
                  {pools.map((pool) => (
                    <button
                      key={pool.name}
                      type="button"
                      className={`chip sm ${poolName === pool.name ? 'on' : ''}`}
                      onClick={() => setPoolName(pool.name)}
                      title={pool.active ? '当前激活号池' : undefined}
                    >
                      {pool.name}
                      {pool.active && <span style={{ fontSize: 8, opacity: 0.7, borderRadius: '50%', width: 5, height: 5, background: 'currentColor', display: 'inline-block', verticalAlign: 'middle' }} />}
                    </button>
                  ))}
                </div>
              </div>
            </div>
          )}

          {/* 词表选择（带搜索，问题 F.7） */}
          <div className="tcm-group">
            <div className="tcm-group-title">{t.library.cfgDecks}</div>
            <div className="tcm-row tcm-decks">
              <div className="tcm-deck-search">
                <Search size={13} />
                <input
                  value={deckQuery}
                  placeholder={t.terms.searchTerms}
                  onChange={(e) => setDeckQuery(e.target.value)}
                />
              </div>
              <div className="tcm-deck-list">
                {filteredDecks.map((deck) => (
                  <label key={deck.id} className="tcm-deck">
                    <input
                      type="checkbox"
                      checked={deck.enabled && selectedDeckIds.includes(deck.id)}
                      disabled={!deck.enabled}
                      onChange={() => toggleDeck(deck.id)}
                    />
                    <span className="truncate">{deck.name}</span>
                    <span className="tcm-deck-count">{deck.entryCount}</span>
                  </label>
                ))}
                {filteredDecks.length === 0 && (
                  <p className="tcm-empty">{t.terms.emptyUser}</p>
                )}
              </div>
            </div>
          </div>
        </div>

        <div className="tcm-foot">
          <button type="button" className="btn ghost sm" onClick={onClose} disabled={starting}>
            {t.tasks.cancel}
          </button>
          <button type="button" className="btn sm" onClick={() => void start()} disabled={starting || !pickReady}>
            {starting ? '…' : t.tasks.new}
          </button>
        </div>
      </div>
    </div>
  );
}

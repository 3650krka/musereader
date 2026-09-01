import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { useToast } from '@/components/common/Toast';
import { useAppDialog } from '@/components/common/AppDialog';
import { motion } from 'motion/react';
import {
  BookMarked,
  BookOpenText,
  BrainCircuit,
  Database,
  FileText,
  KeyRound,
  MessageSquareText,
  Palette,
  Plus,
  ScanLine,
  Server,
  Share2,
  Trash2,
  X,
  Zap,
} from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { useT } from '@/i18n';
import type {
  AgentDefinition,
  AgentPromptPreview,
  AppSettings,
  RoutePoolInput,
  RoutePoolView,
  SkillLibraryEntry,
  Theme,
} from '@/types';
import { AppearanceSettingsPanel } from './AppearanceSettingsPanel';
import { ReadingSettingsPanel } from './ReaderSettingsPanels';
import { ProvidersPanelV2 } from './ProvidersPanelV2';
import { SkillPromptPanel } from './SkillPromptPanel';
import { WordlistPanel } from './WordlistPanel';
import { useSettingsPageState } from './useSettingsPageState';
import { Slider } from '@/components/common/Slider';

type PanelId =
  | 'providers'
  | 'pools'
  | 'ocr'
  | 'skills'
  | 'prompts'
  | 'wordlist'
  | 'reading'
  | 'appearance'
  | 'data';

interface NavItem {
  id: PanelId;
  group: 'gServices' | 'gTranslation' | 'gExperience';
  icon: typeof Server;
  labelKey: 'providers' | 'pools' | 'ocr' | 'skills' | 'prompts' | 'wordlist' | 'reading' | 'appearance' | 'data';
}

const NAV: NavItem[] = [
  { id: 'providers', group: 'gServices', icon: Server, labelKey: 'providers' },
  { id: 'pools', group: 'gServices', icon: Share2, labelKey: 'pools' },
  { id: 'ocr', group: 'gServices', icon: ScanLine, labelKey: 'ocr' },
  { id: 'skills', group: 'gTranslation', icon: FileText, labelKey: 'skills' },
  { id: 'prompts', group: 'gTranslation', icon: MessageSquareText, labelKey: 'prompts' },
  { id: 'wordlist', group: 'gTranslation', icon: BookMarked, labelKey: 'wordlist' },
  { id: 'reading', group: 'gExperience', icon: BookOpenText, labelKey: 'reading' },
  { id: 'appearance', group: 'gExperience', icon: Palette, labelKey: 'appearance' },
  { id: 'data', group: 'gExperience', icon: Database, labelKey: 'data' },
];

interface SettingsPageProps {
  theme: Theme;
  setTheme: (theme: Theme) => void;
  settings: AppSettings;
  onSettingsChange: (settings: AppSettings) => void | Promise<void>;
  skillEntries: SkillLibraryEntry[];
  agentDefinitions: AgentDefinition[];
  translationPromptPreview: AgentPromptPreview | null;
  onSkillEntrySave: (skillId: string, content: string) => Promise<void>;
  onEntriesChanged: () => void;
  saveState: 'idle' | 'saving' | 'saved' | 'error';
  saveMessage: string;
}

export function SettingsPage(props: SettingsPageProps) {
  const t = useT();
  const [panel, setPanel] = useState<PanelId>('providers');
  const state = useSettingsPageState({
    settings: props.settings,
    onSettingsChange: props.onSettingsChange,
    skillEntries: props.skillEntries,
    onSkillEntrySave: props.onSkillEntrySave,
  });

  let lastGroup = '';
  return (
    <motion.div
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.34, ease: [0.22, 1, 0.36, 1] }}
      className="page-stage"
    >
      <header className="page-head">
        <h1 className="page-title">{t.set.title}</h1>
        {props.saveState !== 'idle' && props.saveMessage ? (
          <span className="chip on">{props.saveMessage}</span>
        ) : null}
      </header>

      <div className="settings-v2">
        <aside>
          <nav className="settings-nav2">
            {NAV.map((item) => {
              const groupHeader = item.group !== lastGroup ? (lastGroup = item.group, item.group) : null;
              const Icon = item.icon;
              return (
                <div key={item.id}>
                  {groupHeader && <div className="sgroup">{t.set[groupHeader]}</div>}
                  <button
                    type="button"
                    className={`snav ${panel === item.id ? 'on' : ''}`}
                    onClick={() => setPanel(item.id)}
                  >
                    <Icon size={16} />
                    <span>{t.set[item.labelKey]}</span>
                  </button>
                </div>
              );
            })}
          </nav>
        </aside>

        <div>
          {panel === 'providers' && <ProvidersPanelV2 settings={props.settings} updateSetting={state.updateSetting} updateRuntimeConfig={state.updateRuntimeConfig} />}
          {panel === 'pools' && <PoolsPanel settings={props.settings} />}
          {panel === 'ocr' && <OcrPanel settings={props.settings} updateRuntimeConfig={state.updateRuntimeConfig} />}
          {panel === 'skills' && (
            <>
            <div className="card" style={{ marginBottom: 16 }}>
              <div className="row2">
                <div className="grow">
                  <div className="name">{t.set.coreSkillInject}</div>
                  <div className="desc">{t.set.coreSkillInjectDesc}</div>
                </div>
                <button
                  type="button"
                  className={`switch ${props.settings.runtimeConfig.inject_core_skill ?? true ? 'on' : ''}`}
                  onClick={() => state.updateRuntimeConfig('inject_core_skill', !(props.settings.runtimeConfig.inject_core_skill ?? true))}
                  aria-pressed={props.settings.runtimeConfig.inject_core_skill ?? true}
                  aria-label={t.set.coreSkillInject}
                />
              </div>
              <div className="row2" style={{ marginTop: 12 }}>
                <div className="grow">
                  <div className="name">{t.set.chunkSize}</div>
                  <div className="desc">{t.set.chunkSizeDesc}</div>
                </div>
                <input
                  type="number"
                  min={500}
                  max={20000}
                  step={100}
                  defaultValue={props.settings.runtimeConfig.chunk_size ?? 3000}
                  onChange={(e) => {
                    const n = Number(e.target.value);
                    if (Number.isFinite(n)) state.updateRuntimeConfig('chunk_size', Math.max(500, Math.min(20000, Math.round(n))));
                  }}
                  aria-label={t.set.chunkSize}
                  style={{ width: 110 }}
                />
              </div>
            </div>
            <SkillPromptPanel
              skillEntries={props.skillEntries.filter((e) => e.kind === 'core')}
              agentDefinitions={props.agentDefinitions}
              translationPreview={props.translationPromptPreview}
              readingPreview={state.readingPreview}
              selectedSkillId={state.selectedSkillId}
              setSelectedSkillId={state.setSelectedSkillId}
              skillDraft={state.skillDraft}
              setSkillDraft={state.setSkillDraft}
              skillSaveState={state.skillSaveState}
              onSaveSkill={state.saveSelectedSkill}
              onEntriesChanged={props.onEntriesChanged}
            />
            </>
          )}
          {panel === 'wordlist' && <WordlistPanel />}
          {panel === 'prompts' && (
            <SkillPromptPanel
              skillEntries={props.skillEntries.filter((e) => e.kind !== 'core')}
              agentDefinitions={props.agentDefinitions}
              translationPreview={props.translationPromptPreview}
              readingPreview={state.readingPreview}
              selectedSkillId={state.selectedSkillId}
              setSelectedSkillId={state.setSelectedSkillId}
              skillDraft={state.skillDraft}
              setSkillDraft={state.setSkillDraft}
              skillSaveState={state.skillSaveState}
              onSaveSkill={state.saveSelectedSkill}
              onEntriesChanged={props.onEntriesChanged}
            />
          )}
          {panel === 'reading' && (
            <div className="card">
              <div className="card-title"><BookOpenText size={18} /><span>{t.set.reading}</span></div>
              <ReadingSettingsPanel settings={props.settings} updateSetting={state.updateSetting} />
            </div>
          )}
          {panel === 'appearance' && (
            <div className="card">
              <div className="card-title"><Palette size={18} /><span>{t.set.appearance}</span></div>
              <AppearanceSettingsPanel theme={props.theme} setTheme={props.setTheme} settings={props.settings} updateSetting={state.updateSetting} />
            </div>
          )}
          {panel === 'data' && <DataPanel />}
        </div>
      </div>
    </motion.div>
  );
}

/* 内置供应商（号池添加模型用）：默认端点 + 常用模型建议 */
const BUILTIN_POOL_PROVIDERS = [
  { id: 'sensenova', label: 'SenseNova', apiUrl: 'https://token.sensenova.cn/v1', models: ['deepseek-v4-flash', 'sensenova-6.7-flash-lite'] },
  { id: 'minimax', label: 'MiniMax', apiUrl: 'https://api.minimaxi.com/v1', models: ['MiniMax-M2.7'] },
  { id: 'cohere', label: 'Cohere', apiUrl: 'https://api.cohere.ai/compatibility/v1', models: ['command-a-plus-05-2026'] },
  { id: 'nvidia', label: 'NVIDIA', apiUrl: 'https://integrate.api.nvidia.com/v1', models: ['stepfun-ai/step-3.7-flash', 'google/diffusiongemma-26b-a4b-it'] },
];

/* ── 号池：池编辑器（接 pools.local.json）；激活池即时生效 ── */
function PoolsPanel({ settings }: { settings: AppSettings }) {
  const t = useT();
  const toast = useToast();
  const { appConfirm } = useAppDialog();
  const [pools, setPools] = useState<RoutePoolView[]>([]);
  const [selected, setSelected] = useState('');
  const [dirty, setDirty] = useState<RoutePoolInput[] | null>(null);
  const [savedTick, setSavedTick] = useState(0);
  const [addModelProvider, setAddModelProvider] = useState('');
  const [addModelModel, setAddModelModel] = useState('');
  const [keyDrafts, setKeyDrafts] = useState<Record<string, string>>({});

  /* 添加模型建议：严格按所选供应商——自定义=该供应商在模型服务里
     添加的模型列表；内置=该供应商常用模型。不再混入全局全集。 */
  const addModelSuggestions = useMemo(() => {
    if (addModelProvider.startsWith('custom:')) {
      const provider = (settings.runtimeConfig.customProviders ?? []).find((p) => `custom:${p.id}` === addModelProvider);
      return provider?.models ?? [];
    }
    const builtin = BUILTIN_POOL_PROVIDERS.find((p) => p.id === addModelProvider);
    return builtin?.models ?? [];
  }, [addModelProvider, settings.runtimeConfig.customProviders]);

  /* 既有路由行模型建议：按该路由 provider 匹配（自定义名或内置标签）。 */
  const suggestionsForRoute = (provider: string, apiUrl: string): string[] => {
    const custom = (settings.runtimeConfig.customProviders ?? []).find(
      (p) => p.name === provider || p.apiUrl === apiUrl);
    if (custom) return custom.models;
    const builtin = BUILTIN_POOL_PROVIDERS.find((p) => p.label === provider || p.apiUrl === apiUrl);
    return builtin?.models ?? [];
  };

  const applyView = (rows: RoutePoolView[]) => {
    setPools(rows);
    setDirty(rows.map((p) => ({
      name: p.name,
      active: p.active,
      routes: p.routes.map((r) => ({
        provider: r.provider, apiUrl: r.apiUrl, model: r.model, weight: r.weight, apiKeys: [],
      })),
    })));
  };

  useEffect(() => {
    invoke<RoutePoolView[]>('list_route_pools')
      .then((rows) => {
        applyView(rows);
        if (!selected && rows[0]) setSelected(rows[0].name);
      })
      .catch(() => setPools([]));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [savedTick]);

  const current = dirty?.find((p) => p.name === selected) ?? dirty?.[0];
  const currentView = pools.find((p) => p.name === current?.name);
  const totalW = current?.routes.reduce((s, r) => s + r.weight, 0) || 1;

  const persistPools = async (next: RoutePoolInput[]) => {
    await invoke('save_route_pools', { pools: next });
    setSavedTick((v) => v + 1);
  };

  const persist = async () => {
    if (!dirty) return;
    await persistPools(dirty);
  };

  /* 激活池切换：即时落盘并反馈——此前只改本地态不保存，点着"没反应"。 */
  const activatePool = async (name: string) => {
    const next = (dirty ?? []).map((p) => ({ ...p, active: p.name === name }));
    setDirty(next);
    try {
      await persistPools(next);
      toast.success(`${t.set.poolActivated} · ${name}`);
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error));
    }
  };

  /* 删除池：确认后即时落盘（不可撤销操作，AppDialog 确认）。 */
  const deletePool = async (name: string) => {
    if (!(await appConfirm({ title: t.set.deletePoolConfirmTitle, message: name, danger: true }))) return;
    const next = (dirty ?? []).filter((p) => p.name !== name);
    setDirty(next);
    try {
      await persistPools(next);
      if (selected === name) setSelected(next[0]?.name ?? '');
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error));
    }
  };

  /* 路由 key 增删：前端不持有明文，专用命令直改文件后刷新。 */
  const addRouteKey = async (poolName: string, model: string, key: string) => {
    if (!key.trim()) return;
    try {
      applyView(await invoke<RoutePoolView[]>('pool_route_add_key', { poolName, model, key }));
      setKeyDrafts((d) => ({ ...d, [`${poolName}:${model}`]: '' }));
      toast.success(t.set.keyAdded);
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error));
    }
  };
  const removeRouteKey = async (poolName: string, model: string, keyIndex: number) => {
    try {
      applyView(await invoke<RoutePoolView[]>('pool_route_remove_key', { poolName, model, keyIndex }));
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error));
    }
  };

  return (
    <>
      <div className="flow-hint">
        <b>{t.set.pools}</b><span className="arr">→</span><span>{t.set.poolsDesc}</span>
      </div>

      <div className="card">
        <div className="card-title"><KeyRound size={18} /><span>{t.set.pools}</span></div>
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap', marginBottom: 14 }}>
          {(dirty ?? []).map((p) => (
            <span key={p.name} className={`chip pool-chip ${current?.name === p.name ? 'on' : ''}`}>
              <button
                type="button"
                className="pool-chip-name"
                onClick={() => setSelected(p.name)}
                title={p.name}
              >
                {p.name}
              </button>
              {/* 激活池切换：此前无任何 UI 能写 active，save 后 active 恒为首个池 */}
              <button
                type="button"
                className={`pool-chip-zap ${p.active ? 'live' : ''}`}
                title={p.active ? t.set.poolActive : t.set.setActivePool}
                aria-pressed={p.active}
                onClick={() => void activatePool(p.name)}
              >
                <Zap size={11} />
              </button>
            </span>
          ))}
          <button
            type="button"
            className="chip"
            onClick={() => {
              // 唯一命名：pool-{n} 取未占用的最小序号（防删除后重建重名导致 key 冲突）
              const used = new Set((dirty ?? []).map((p) => p.name));
              let n = 1;
              while (used.has(`pool-${n}`)) n += 1;
              const name = `pool-${n}`;
              setDirty((d) => [...(d ?? []), { name, active: false, routes: [] }]);
              setSelected(name);
            }}
          >
            <Plus size={12} /> {t.set.newPool}
          </button>
        </div>

        {current ? (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            {current.routes.map((r, i) => (
              <div key={`${r.provider}-${r.model}-${i}`} className="row2 pool-route-card" style={{ flexWrap: 'wrap' }}>
                {/* 删除模型：卡片右上角小号，替代行尾大按钮 */}
                <button
                  type="button"
                  className="pool-route-del"
                  aria-label={t.set.cancel}
                  title={t.set.cancel}
                  onClick={() => setDirty((d) => d?.map((p) => p.name === current.name
                    ? { ...p, routes: p.routes.filter((_, j) => j !== i) }
                    : p) ?? null)}
                >
                  <X size={11} />
                </button>
                <div className="grow" style={{ minWidth: 160 }}>
                  {/* 模型可编辑 + 建议下拉 */}
                  <input
                    list={`pool-models-${current.name}-${i}`}
                    value={r.model}
                    style={{ width: '100%' }}
                    placeholder={t.set.modelPlaceholder}
                    onChange={(e) => {
                      const model = e.target.value;
                      setDirty((d) => d?.map((p) => p.name === current.name
                        ? { ...p, routes: p.routes.map((x, j) => (j === i ? { ...x, model } : x)) }
                        : p) ?? null);
                    }}
                  />
                  <datalist id={`pool-models-${current.name}-${i}`}>
                    {suggestionsForRoute(r.provider, r.apiUrl).map((m) => <option key={m} value={m} />)}
                  </datalist>
                  <div className="desc">{r.provider} · {r.apiUrl || '—'}</div>
                  {/* 路由 key 掩码 + 增删（key 只在 pools.local.json，编辑走专用命令） */}
                  <div className="pool-key-row">
                    {currentView?.routes[i]?.keyMasks.map((mask, ki) => (
                      <span key={ki} className="chip" style={{ fontSize: 10.5, gap: 4 }}>
                        {mask}
                        <button
                          type="button"
                          aria-label={t.set.removeKey}
                          onClick={() => void removeRouteKey(current.name, r.model, ki)}
                          style={{ background: 'none', border: 0, cursor: 'pointer', color: 'var(--ink-3)', padding: 0, lineHeight: 1 }}
                        >
                          <X size={10} />
                        </button>
                      </span>
                    ))}
                    <input
                      type="password"
                      className="pool-key-add"
                      placeholder={t.set.addKeyPlaceholder}
                      value={keyDrafts[`${current.name}:${r.model}`] ?? ''}
                      onChange={(e) => setKeyDrafts((d) => ({ ...d, [`${current.name}:${r.model}`]: e.target.value }))}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') void addRouteKey(current.name, r.model, keyDrafts[`${current.name}:${r.model}`] ?? '');
                      }}
                    />
                  </div>
                </div>
                <Slider
                  min={1} max={10} value={r.weight} style={{ width: 100, flex: 'none' }}
                  ariaLabel={`${current.name} ${r.model} weight`}
                  onChange={(w) => {
                    setDirty((d) => d?.map((p) => p.name === current.name
                      ? { ...p, routes: p.routes.map((x, j) => (j === i ? { ...x, weight: w } : x)) }
                      : p) ?? null);
                  }}
                />
                <span style={{ width: 26, textAlign: 'center', fontWeight: 700, fontSize: 12.5 }}>{r.weight}</span>
                <span className="chip" style={{ fontSize: 11 }}>{Math.round((r.weight / totalW) * 100)}%</span>
              </div>
            ))}
            {/* 向号池添加模型：选供应商（含自定义）+ 模型建议 */}
            <div className="pool-add-model">
              <select
                className="settings-select"
                value={addModelProvider}
                onChange={(e) => { setAddModelProvider(e.target.value); setAddModelModel(''); }}
                aria-label={t.set.addPoolModel}
              >
                <option value="">{t.set.addPoolModel}</option>
                {(settings.runtimeConfig.customProviders ?? []).map((p) => (
                  <option key={p.id} value={`custom:${p.id}`}>{p.name}</option>
                ))}
                {BUILTIN_POOL_PROVIDERS.map((p) => <option key={p.id} value={p.id}>{p.label}</option>)}
              </select>
              <input
                list="pool-add-models"
                value={addModelModel}
                placeholder={t.set.modelPlaceholder}
                onChange={(e) => setAddModelModel(e.target.value)}
              />
              <datalist id="pool-add-models">
                {addModelSuggestions.map((m) => <option key={m} value={m} />)}
              </datalist>
              <button
                type="button"
                className="chip"
                disabled={!addModelModel.trim() || !addModelProvider}
                onClick={() => {
                  const providerMeta = BUILTIN_POOL_PROVIDERS.find((p) => p.id === addModelProvider);
                  const providerObj = (settings.runtimeConfig.customProviders ?? []).find((p) => `custom:${p.id}` === addModelProvider);
                  const provider = providerObj?.name ?? providerMeta?.label ?? 'custom';
                  const apiUrl = providerObj?.apiUrl ?? providerMeta?.apiUrl ?? '';
                  setDirty((d) => d?.map((p) => p.name === current.name
                    ? {
                        ...p,
                        routes: [...p.routes, {
                          provider,
                          apiUrl,
                          model: addModelModel.trim(),
                          weight: 1,
                          apiKeys: [],
                        }],
                      }
                    : p) ?? null);
                  setAddModelModel('');
                }}
              >
                <Plus size={12} />
              </button>
            </div>
            {/* 删除号池：即时落盘 */}
            <div style={{ display: 'flex', justifyContent: 'flex-end', marginTop: 4 }}>
              <button
                type="button"
                className="btn ghost sm"
                style={{ color: 'var(--danger, #c0392b)' }}
                onClick={() => void deletePool(current.name)}
              >
                <Trash2 size={13} />
                {t.set.deletePool}
              </button>
            </div>
          </div>
        ) : (
          <p className="card-desc" style={{ textAlign: 'center', padding: '12px 0' }}>
            {pools.length === 0 ? 'pools.local.json —' : ''}{t.set.poolsDesc}
          </p>
        )}

        <div className="divider" />
        <div style={{ display: 'flex', gap: 10, alignItems: 'center' }}>
          <button type="button" className="btn sm" onClick={() => void persist()}>{t.set.save}</button>
          <BrainCircuit size={15} style={{ color: 'var(--ink-3)' }} />
          <span style={{ fontSize: 12, color: 'var(--ink-3)' }}>{t.set.poolHealthDesc}</span>
        </div>
      </div>
    </>
  );
}

/* ── OCR ── */
function OcrPanel({
  settings,
  updateRuntimeConfig,
}: {
  settings: AppSettings;
  updateRuntimeConfig: ReturnType<typeof useSettingsPageState>['updateRuntimeConfig'];
}) {
  const t = useT();
  return (
    <>
      <div className="flow-hint"><b>OCR</b><span className="arr">→</span><span>{t.set.ocrCacheDesc}</span></div>
      <div className="card">
        <div className="card-title"><ScanLine size={18} /><span>OCR</span></div>
        <div className="grid2" style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
          <div className="field">
            <label>API URL</label>
            <input
              value={settings.runtimeConfig.ocr_api_url}
              onChange={(e) => updateRuntimeConfig('ocr_api_url', e.target.value)}
              placeholder="https://…"
            />
          </div>
          <div className="field">
            <label>Token</label>
            <input
              type="password"
              value={settings.runtimeConfig.ocr_token}
              onChange={(e) => updateRuntimeConfig('ocr_token', e.target.value)}
              placeholder="••••••••"
            />
          </div>
        </div>
        <div className="divider" />
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <button
            type="button"
            className={`switch ${settings.runtimeConfig.use_ocr_cache ? 'on' : ''}`}
            onClick={() => updateRuntimeConfig('use_ocr_cache', !settings.runtimeConfig.use_ocr_cache)}
            aria-label="cache"
          />
          <div>
            <b style={{ fontSize: 13.5 }}>{t.set.ocrCache}</b>
            <p className="card-desc" style={{ margin: '2px 0 0' }}>{t.set.ocrCacheDesc}</p>
          </div>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginTop: 12 }}>
          <button
            type="button"
            className={`switch ${settings.runtimeConfig.use_front_matter_cache ? 'on' : ''}`}
            onClick={() => updateRuntimeConfig('use_front_matter_cache', !settings.runtimeConfig.use_front_matter_cache)}
            aria-label="front-matter-cache"
          />
          <div>
            <b style={{ fontSize: 13.5 }}>{t.set.frontMatterCache}</b>
            <p className="card-desc" style={{ margin: '2px 0 0' }}>{t.set.frontMatterCacheNote}</p>
          </div>
        </div>
      </div>
    </>
  );
}

/* ── 数据与同步 ── */
function DataPanel() {
  const t = useT();
  const { appConfirm } = useAppDialog();
  const [backupState, setBackupState] = useState<'idle' | 'busy' | 'done' | 'failed'>('idle');
  const [restorePath, setRestorePath] = useState('');
  const [restoreState, setRestoreState] = useState<'idle' | 'busy' | 'done' | 'failed'>('idle');

  /* 导出：ZIP 打包任务/阅读状态/收藏标签/运行配置/号池（不含 artifacts 大文件） */
  const exportBackup = async () => {
    setBackupState('busy');
    try {
      const stamp = new Date().toISOString().slice(0, 10);
      const path = await save({
        defaultPath: `musereader-backup-${stamp}.zip`,
        filters: [{ name: 'Backup ZIP', extensions: ['zip'] }],
      });
      if (!path) {
        setBackupState('idle');
        return;
      }
      await invoke('export_backup_zip', { outputPath: path });
      setBackupState('done');
    } catch (error) {
      console.error('Backup export failed:', error);
      setBackupState('failed');
    } finally {
      window.setTimeout(() => setBackupState('idle'), 2500);
    }
  };

  /* 恢复：整表覆盖（任务/阅读状态/收藏标签/配置/号池），操作不可撤销 */
  const restoreBackup = async () => {
    const path = restorePath.trim();
    if (!path) return;
    if (!(await appConfirm({ title: t.set.restoreConfirm, danger: true }))) return;
    setRestoreState('busy');
    try {
      await invoke('restore_backup_zip', { archivePath: path });
      setRestoreState('done');
      // 恢复替换了内存态：重载界面刷新书库/洞察等前端缓存
      if (await appConfirm({ title: t.set.restartToApply })) window.location.reload();
    } catch (error) {
      console.error('Backup restore failed:', error);
      setRestoreState('failed');
    } finally {
      window.setTimeout(() => setRestoreState('idle'), 2500);
    }
  };

  return (
    <>
      <div className="card" style={{ marginBottom: 16 }}>
        <div className="card-title"><Database size={18} /><span>{t.set.data}</span></div>
        <p className="card-desc">{t.set.dataDesc}</p>
        <div className="row2" style={{ opacity: 0.55 }}>
          <div className="grow"><div className="name">WebDAV <span className="chip" style={{ fontSize: 11 }}>{t.set.soon}</span></div><div className="desc">{t.set.webdavDesc}</div></div>
        </div>
        <div className="row2" style={{ opacity: 0.55, marginTop: 8 }}>
          <div className="grow"><div className="name">{t.set.nas} <span className="chip" style={{ fontSize: 11 }}>{t.set.soon}</span></div><div className="desc">{t.set.nasDesc}</div></div>
        </div>
      </div>
      <div className="card">
        <div className="card-title"><span>{t.set.backup}</span></div>
        <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap', alignItems: 'center' }}>
          <button type="button" className="btn ghost sm" disabled={backupState === 'busy'} onClick={() => void exportBackup()}>
            {backupState === 'done' ? t.set.backupDone : backupState === 'failed' ? t.set.backupFailed : t.set.backupExport}
          </button>
        </div>
        <p className="card-desc" style={{ marginTop: 8 }}>{t.set.backupNote}</p>
        <div style={{ display: 'flex', gap: 8, marginTop: 12, flexWrap: 'wrap' }}>
          <input
            className="backup-path-input"
            value={restorePath}
            onChange={(e) => setRestorePath(e.target.value)}
            placeholder={t.set.restorePlaceholder}
          />
          <button type="button" className="btn ghost sm" disabled={restoreState === 'busy' || !restorePath.trim()} onClick={() => void restoreBackup()}>
            {restoreState === 'done' ? t.set.restoreDone : restoreState === 'failed' ? t.set.restoreFailed : t.set.restore}
          </button>
        </div>
      </div>
    </>
  );
}

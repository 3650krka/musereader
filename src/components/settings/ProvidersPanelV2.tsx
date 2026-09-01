import { invoke } from '@tauri-apps/api/core';
import { Bot, Eye, EyeOff, Globe, KeyRound, PlugZap, Plus, Server, Sparkles, Trash2 } from 'lucide-react';
import { useState } from 'react';
import { useT } from '@/i18n';
import { useAppDialog } from '@/components/common/AppDialog';
import type { AppSettings, CustomProvider, ProviderApiKey } from '@/types';

/** 供应商管理（CherryStudio 式）：左侧供应商可滚动列表（底部新增），右侧信息/编辑。 */

const API_PROTOCOLS = [
  { id: 'openai', label: 'OpenAI', desc: 'OpenAI 官方 chat/completions' },
  { id: 'openai-compatible', label: 'OpenAI 兼容', desc: 'DeepSeek/OpenRouter 等兼容端点' },
  { id: 'openai-responses', label: 'Responses API', desc: 'OpenAI responses 端点（新协议）' },
  { id: 'anthropic-messages', label: 'Anthropic', desc: 'Anthropic messages 端点（专用请求格式）' },
] as const;

const BUILTIN_PROVIDERS: { id: AppSettings['provider']; name: string }[] = [
  { id: 'sensenova', name: 'SenseNova' },
  { id: 'minimax', name: 'MiniMax' },
  { id: 'cohere', name: 'Cohere' },
  { id: 'nvidia', name: 'NVIDIA' },
  { id: 'openai-compatible', name: 'OpenAI' },
];

type ProvidersPanelProps = {
  settings: AppSettings;
  updateSetting: <K extends keyof AppSettings>(key: K, value: AppSettings[K]) => void;
  updateRuntimeConfig: <K extends keyof AppSettings['runtimeConfig']>(
    key: K,
    value: AppSettings['runtimeConfig'][K],
  ) => void;
};

export function ProvidersPanelV2({ settings, updateSetting, updateRuntimeConfig }: ProvidersPanelProps) {
  const t = useT();
  const { appConfirm } = useAppDialog();
  const customProviders = settings.runtimeConfig.customProviders ?? [];
  const builtin = BUILTIN_PROVIDERS.find((p) => p.id === settings.provider);
  const [selected, setSelected] = useState<string>(builtin?.id ?? 'sensenova');
  const [addingModel, setAddingModel] = useState('');
  const [addingKey, setAddingKey] = useState('');

  const selectedCustom = customProviders.find((p) => p.id === selected) ?? null;
  const selectedBuiltin = selectedCustom ? null : BUILTIN_PROVIDERS.find((p) => p.id === selected) ?? null;
  const builtinFields = selectedBuiltin ? resolveBuiltinFields(selectedBuiltin.id) : null;

  const patchCustom = (id: string, patch: Partial<CustomProvider>) => {
    updateRuntimeConfig(
      'customProviders',
      customProviders.map((p) => (p.id === id ? { ...p, ...patch } : p)),
    );
  };

  const addProvider = () => {
    const id = `custom-${Date.now().toString(36)}`;
    const next: CustomProvider = { id, name: t.set.newProviderName, apiUrl: '', apiKey: '', models: [], protocol: 'openai-compatible', apiKeys: [] };
    updateRuntimeConfig('customProviders', [...customProviders, next]);
    setSelected(id);
  };

  const removeProvider = async (id: string) => {
    if (!(await appConfirm({ title: t.set.removeProviderConfirm, danger: true }))) return;
    updateRuntimeConfig('customProviders', customProviders.filter((p) => p.id !== id));
    setSelected(builtin?.id ?? 'sensenova');
  };

  const addModel = (id: string) => {
    const model = addingModel.trim();
    if (!model) return;
    const provider = customProviders.find((p) => p.id === id);
    if (!provider || provider.models.includes(model)) return;
    patchCustom(id, { models: [...provider.models, model] });
    setAddingModel('');
  };

  const [testState, setTestState] = useState<'idle' | 'busy' | 'ok' | 'failed'>('idle');
  const [testMessage, setTestMessage] = useState('');

  const testConnection = async (apiUrl: string, apiKey: string, model: string) => {
    setTestState('busy');
    setTestMessage('');
    try {
      const message = await invoke<string>('test_provider_connection', { apiUrl, apiKey, model });
      setTestState('ok');
      setTestMessage(message);
    } catch (error) {
      setTestState('failed');
      const msg = error instanceof Error ? error.message : typeof error === 'string' ? error : JSON.stringify(error);
      setTestMessage(msg);
    } finally {
      window.setTimeout(() => setTestState('idle'), 4000);
    }
  };

  const useProvider = (provider: CustomProvider) => {
    updateSetting('provider', 'custom');
    updateRuntimeConfig('llm_provider', 'custom');
    updateRuntimeConfig('llm_api_url', provider.apiUrl);
    /* 多 key：取首个启用的 key；全部停用时回落旧单 key 字段/空。 */
    const keys = provider.apiKeys ?? [];
    const activeKey = keys.find((k) => k.enabled && k.value.trim())?.value.trim() || provider.apiKey;
    updateRuntimeConfig('llm_api_key', activeKey);
    updateRuntimeConfig('llm_model', provider.models[0] ?? '');
  };

  /* ── 内置供应商扩展：多模型 + 多 key + 单值字段回写 ──
     providerExtras 以 provider id 为键存 models/apiKeys；编辑时同步写回
     legacy 单值字段（{provider}_model / {provider}_api_key 取首个启用 key），
     后端轮换与池路由逻辑零改动。 */
  const extras = settings.runtimeConfig.providerExtras ?? [];
  const builtinExtra = (id: string): CustomProvider =>
    extras.find((e) => e.id === id) ?? { id, name: '', apiUrl: '', apiKey: '', models: [], protocol: 'openai-compatible', apiKeys: [] };
  const patchBuiltinExtra = (id: string, patch: Partial<CustomProvider>, legacySync: { modelField?: string; keyField?: string }) => {
    const next = { ...builtinExtra(id), ...patch };
    const list = extras.some((e) => e.id === id)
      ? extras.map((e) => (e.id === id ? next : e))
      : [...extras, next];
    updateRuntimeConfig('providerExtras', list);
    if (legacySync.modelField && next.models.length > 0) {
      updateRuntimeConfig(legacySync.modelField as keyof AppSettings['runtimeConfig'], next.models[0] as never);
    }
    if (legacySync.keyField) {
      const activeKey = (next.apiKeys ?? []).find((k) => k.enabled && k.value.trim())?.value.trim() ?? '';
      updateRuntimeConfig(legacySync.keyField as keyof AppSettings['runtimeConfig'], activeKey as never);
    }
  };
  const [builtinAddingModel, setBuiltinAddingModel] = useState('');
  const [builtinAddingKey, setBuiltinAddingKey] = useState('');
  const [keyVisible, setKeyVisible] = useState<Record<string, boolean>>({});

  return (
    <div className="providers-v2">
      {/* 左侧：可滚动供应商列表，底部新增按钮 */}
      <div className="prov-list">
        <div className="prov-list-head">{t.set.providers}</div>
        <div className="prov-list-scroll">
          {BUILTIN_PROVIDERS.map((p) => (
            <button
              key={p.id}
              type="button"
              className={`prov-item ${selected === p.id ? 'on' : ''}`}
              onClick={() => setSelected(p.id)}
            >
              <Server size={15} />
              <span className="truncate">{p.name}</span>
              {settings.provider === p.id && <span className="dot" title={t.set.currentProvider} />}
            </button>
          ))}
          {customProviders.map((p) => (
            <button
              key={p.id}
              type="button"
              className={`prov-item ${selected === p.id ? 'on' : ''}`}
              onClick={() => setSelected(p.id)}
            >
              <Sparkles size={15} />
              <span className="truncate">{p.name}</span>
              {settings.provider === 'custom' && settings.runtimeConfig.llm_api_url === p.apiUrl && (
                <span className="dot" title={t.set.currentProvider} />
              )}
            </button>
          ))}
        </div>
        <button type="button" className="prov-add" onClick={addProvider}>
          <Plus size={15} />
          {t.set.addProvider}
        </button>
      </div>

      {/* 右侧：所选供应商信息 */}
      <div className="prov-detail">
        {selectedBuiltin && builtinFields && (
          <>
            <div className="prov-detail-head">
              <span className="prov-name-static">{selectedBuiltin.name}</span>
              <button
                type="button"
                className="btn ghost sm"
                onClick={() => {
                  updateSetting('provider', selectedBuiltin.id);
                  updateRuntimeConfig('llm_provider', selectedBuiltin.id);
                }}
              >
                {t.set.useThisProvider}
              </button>
              <button
                type="button"
                className="btn ghost sm"
                disabled={testState === 'busy'}
                onClick={() => void testConnection(
                  settings.runtimeConfig[builtinFields.apiUrl],
                  settings.runtimeConfig[builtinFields.apiKey],
                  settings.runtimeConfig[builtinFields.model],
                )}
              >
                <PlugZap size={13} />
                {testState === 'busy' ? '…' : t.set.testConnection}
              </button>
            </div>
            <div className="field">
              <label><Globe size={13} />{t.set.llmUrl}</label>
              <input
                value={settings.runtimeConfig[builtinFields.apiUrl]}
                placeholder={t.set.llmUrlPlaceholder}
                onChange={(e) => updateRuntimeConfig(builtinFields.apiUrl, e.target.value)}
              />
            </div>
            <div className="field">
              <label><KeyRound size={13} />{t.set.llmKey}</label>
              {/* key 眼睛按钮：显示/隐藏明文 */}
              <div style={{ display: 'flex', gap: 6 }}>
                <input
                  type={keyVisible[selectedBuiltin.id] ? 'text' : 'password'}
                  value={settings.runtimeConfig[builtinFields.apiKey]}
                  placeholder={t.set.keyPlaceholder}
                  onChange={(e) => updateRuntimeConfig(builtinFields.apiKey, e.target.value)}
                  style={{ flex: 1 }}
                />
                <button
                  type="button"
                  className="iconbtn"
                  style={{ width: 34, height: 34, flex: 'none' }}
                  aria-label={keyVisible[selectedBuiltin.id] ? t.set.hideKey : t.set.showKey}
                  title={keyVisible[selectedBuiltin.id] ? t.set.hideKey : t.set.showKey}
                  onClick={() => setKeyVisible((v) => ({ ...v, [selectedBuiltin.id]: !v[selectedBuiltin.id] }))}
                >
                  {keyVisible[selectedBuiltin.id] ? <EyeOff size={14} /> : <Eye size={14} />}
                </button>
              </div>
            </div>
            <div className="field">
              <label><Bot size={13} />{t.set.model}</label>
              <input
                value={settings.runtimeConfig[builtinFields.model]}
                placeholder={t.set.modelPlaceholder}
                list={`models-${builtinFields.model}`}
                onChange={(e) => updateRuntimeConfig(builtinFields.model, e.target.value)}
              />
              <datalist id={`models-${builtinFields.model}`}>
                {modelSuggestions(selectedBuiltin.id).map((m) => (
                  <option key={m} value={m} />
                ))}
              </datalist>
            </div>
            {/* 多模型：内置供应商同享 custom 的模型清单编辑 */}
            <div className="field">
              <label><Bot size={13} />{t.set.pmodels}</label>
              <div className="prov-models">
                {builtinExtra(selectedBuiltin.id).models.map((model) => (
                  <span key={model} className="chip on">
                    {model}
                    <button
                      type="button"
                      className="chip-x"
                      onClick={() => {
                        const rest = builtinExtra(selectedBuiltin.id).models.filter((m) => m !== model);
                        patchBuiltinExtra(selectedBuiltin.id, { models: rest }, { modelField: builtinFields.model, keyField: builtinFields.apiKey });
                      }}
                      aria-label={t.terms.remove}
                    >
                      ×
                    </button>
                  </span>
                ))}
              </div>
              <div style={{ display: 'flex', gap: 8, marginTop: 8 }}>
                <input
                  value={builtinAddingModel}
                  placeholder={t.set.modelPlaceholder}
                  onChange={(e) => setBuiltinAddingModel(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key !== 'Enter') return;
                    const model = builtinAddingModel.trim();
                    if (!model || builtinExtra(selectedBuiltin.id).models.includes(model)) return;
                    patchBuiltinExtra(
                      selectedBuiltin.id,
                      { models: [...builtinExtra(selectedBuiltin.id).models, model] },
                      { modelField: builtinFields.model, keyField: builtinFields.apiKey },
                    );
                    setBuiltinAddingModel('');
                  }}
                  style={{ flex: 1 }}
                />
                <button
                  type="button"
                  className="btn ghost sm"
                  onClick={() => {
                    const model = builtinAddingModel.trim();
                    if (!model || builtinExtra(selectedBuiltin.id).models.includes(model)) return;
                    patchBuiltinExtra(
                      selectedBuiltin.id,
                      { models: [...builtinExtra(selectedBuiltin.id).models, model] },
                      { modelField: builtinFields.model, keyField: builtinFields.apiKey },
                    );
                    setBuiltinAddingModel('');
                  }}
                >
                  <Plus size={13} />
                  {t.terms.add}
                </button>
              </div>
            </div>
            {/* 多 key：内置供应商同享多 key 清单（启停+删除+眼睛） */}
            <div className="field">
              <label><KeyRound size={13} />{t.set.llmKeyMulti}</label>
              <div className="prov-keys">
                {(builtinExtra(selectedBuiltin.id).apiKeys ?? []).map((entry, idx) => (
                  <div key={idx} className="prov-key-row">
                    <label className="toggle" title={entry.enabled ? t.set.keyEnabled : t.set.keyDisabled}>
                      <input
                        type="checkbox"
                        checked={entry.enabled}
                        onChange={(e) => {
                          const keys = (builtinExtra(selectedBuiltin.id).apiKeys ?? []).map((k, j) =>
                            j === idx ? { ...k, enabled: e.target.checked } : k);
                          patchBuiltinExtra(selectedBuiltin.id, { apiKeys: keys }, { modelField: builtinFields.model, keyField: builtinFields.apiKey });
                        }}
                      />
                      <span className="toggle-track" />
                    </label>
                    <input
                      type={keyVisible[`${selectedBuiltin.id}:${idx}`] ? 'text' : 'password'}
                      className="prov-key-input"
                      value={entry.value}
                      placeholder={t.set.keyPlaceholder}
                      onChange={(e) => {
                        const keys = (builtinExtra(selectedBuiltin.id).apiKeys ?? []).map((k, j) =>
                          j === idx ? { ...k, value: e.target.value } : k);
                        patchBuiltinExtra(selectedBuiltin.id, { apiKeys: keys }, { modelField: builtinFields.model, keyField: builtinFields.apiKey });
                      }}
                    />
                    <button
                      type="button"
                      className="iconbtn"
                      style={{ width: 28, height: 28 }}
                      aria-label={keyVisible[`${selectedBuiltin.id}:${idx}`] ? t.set.hideKey : t.set.showKey}
                      onClick={() => setKeyVisible((v) => ({ ...v, [`${selectedBuiltin.id}:${idx}`]: !v[`${selectedBuiltin.id}:${idx}`] }))}
                    >
                      {keyVisible[`${selectedBuiltin.id}:${idx}`] ? <EyeOff size={13} /> : <Eye size={13} />}
                    </button>
                    <button
                      type="button"
                      className="iconbtn danger"
                      style={{ width: 28, height: 28 }}
                      aria-label={t.terms.remove}
                      onClick={() => patchBuiltinExtra(
                        selectedBuiltin.id,
                        { apiKeys: (builtinExtra(selectedBuiltin.id).apiKeys ?? []).filter((_, j) => j !== idx) },
                        { modelField: builtinFields.model, keyField: builtinFields.apiKey },
                      )}
                    >
                      <Trash2 size={13} />
                    </button>
                  </div>
                ))}
                <div style={{ display: 'flex', gap: 8 }}>
                  <input
                    type="password"
                    value={builtinAddingKey}
                    placeholder={t.set.addKeyPlaceholder}
                    onChange={(e) => setBuiltinAddingKey(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key !== 'Enter') return;
                      const value = builtinAddingKey.trim();
                      if (!value) return;
                      patchBuiltinExtra(
                        selectedBuiltin.id,
                        { apiKeys: [...(builtinExtra(selectedBuiltin.id).apiKeys ?? []), { value, enabled: true }] },
                        { modelField: builtinFields.model, keyField: builtinFields.apiKey },
                      );
                      setBuiltinAddingKey('');
                    }}
                    style={{ flex: 1 }}
                  />
                  <button
                    type="button"
                    className="btn ghost sm"
                    onClick={() => {
                      const value = builtinAddingKey.trim();
                      if (!value) return;
                      patchBuiltinExtra(
                        selectedBuiltin.id,
                        { apiKeys: [...(builtinExtra(selectedBuiltin.id).apiKeys ?? []), { value, enabled: true }] },
                        { modelField: builtinFields.model, keyField: builtinFields.apiKey },
                      );
                      setBuiltinAddingKey('');
                    }}
                  >
                    <Plus size={13} />
                  </button>
                </div>
              </div>
            </div>
          </>
        )}
        {selectedCustom ? (
          <>
            <div className="prov-detail-head">
              <input
                className="prov-name-input"
                value={selectedCustom.name}
                onChange={(e) => patchCustom(selectedCustom.id, { name: e.target.value })}
                aria-label={t.set.pname}
              />
              <button type="button" className="btn ghost sm" onClick={() => useProvider(selectedCustom)}>
                {t.set.useThisProvider}
              </button>
              <button
                type="button"
                className="btn ghost sm"
                disabled={testState === 'busy'}
                onClick={() => void testConnection(selectedCustom.apiUrl, selectedCustom.apiKey, selectedCustom.models[0] ?? '')}
              >
                <PlugZap size={13} />
                {testState === 'busy' ? '…' : t.set.testConnection}
              </button>
              <button
                type="button"
                className="iconbtn danger"
                onClick={() => void removeProvider(selectedCustom.id)}
                aria-label={t.terms.remove}
              >
                <Trash2 size={14} />
              </button>
            </div>
            <div className="field">
              <label><Globe size={13} />{t.set.llmUrl}</label>
              <input
                value={selectedCustom.apiUrl}
                placeholder={t.set.llmUrlPlaceholder}
                onChange={(e) => patchCustom(selectedCustom.id, { apiUrl: e.target.value })}
              />
            </div>
            <div className="field">
              <label><KeyRound size={13} />{t.set.llmKey}</label>
              {/* 多 key 管理：逐个启停 + 删除；空态回落旧单 key 字段 */}
              <div className="prov-keys">
                {(selectedCustom.apiKeys ?? []).map((entry, idx) => (
                  <div key={idx} className="prov-key-row">
                    <label className="toggle" title={entry.enabled ? t.set.keyEnabled : t.set.keyDisabled}>
                      <input
                        type="checkbox"
                        checked={entry.enabled}
                        onChange={(e) => {
                          const keys = (selectedCustom.apiKeys ?? []).map((k, j) =>
                            j === idx ? { ...k, enabled: e.target.checked } : k);
                          patchCustom(selectedCustom.id, { apiKeys: keys });
                        }}
                      />
                      <span className="toggle-track" />
                    </label>
                    <input
                      type={keyVisible[`custom:${idx}`] ? 'text' : 'password'}
                      className="prov-key-input"
                      value={entry.value}
                      placeholder={t.set.keyPlaceholder}
                      onChange={(e) => {
                        const keys = (selectedCustom.apiKeys ?? []).map((k, j) =>
                          j === idx ? { ...k, value: e.target.value } : k);
                        patchCustom(selectedCustom.id, { apiKeys: keys });
                      }}
                    />
                    <button
                      type="button"
                      className="iconbtn"
                      style={{ width: 28, height: 28 }}
                      aria-label={keyVisible[`custom:${idx}`] ? t.set.hideKey : t.set.showKey}
                      title={keyVisible[`custom:${idx}`] ? t.set.hideKey : t.set.showKey}
                      onClick={() => setKeyVisible((v) => ({ ...v, [`custom:${idx}`]: !v[`custom:${idx}`] }))}
                    >
                      {keyVisible[`custom:${idx}`] ? <EyeOff size={13} /> : <Eye size={13} />}
                    </button>
                    <button
                      type="button"
                      className="iconbtn danger"
                      style={{ width: 28, height: 28 }}
                      aria-label={t.terms.remove}
                      onClick={() => patchCustom(selectedCustom.id, {
                        apiKeys: (selectedCustom.apiKeys ?? []).filter((_, j) => j !== idx),
                      })}
                    >
                      <Trash2 size={13} />
                    </button>
                  </div>
                ))}
                {/* 旧数据兼容：有 apiKey 无 apiKeys 时显示迁移提示行 */}
                {!selectedCustom.apiKeys?.length && selectedCustom.apiKey && (
                  <p className="prov-key-legacy">…{selectedCustom.apiKey.slice(-4)}</p>
                )}
                <div style={{ display: 'flex', gap: 8 }}>
                  <input
                    type="password"
                    value={addingKey}
                    placeholder={t.set.addKeyPlaceholder}
                    onChange={(e) => setAddingKey(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key !== 'Enter') return;
                      const value = addingKey.trim();
                      if (!value) return;
                      const entry: ProviderApiKey = { value, enabled: true };
                      patchCustom(selectedCustom.id, { apiKeys: [...(selectedCustom.apiKeys ?? []), entry] });
                      setAddingKey('');
                    }}
                    style={{ flex: 1 }}
                  />
                  <button
                    type="button"
                    className="btn ghost sm"
                    onClick={() => {
                      const value = addingKey.trim();
                      if (!value) return;
                      const entry: ProviderApiKey = { value, enabled: true };
                      patchCustom(selectedCustom.id, { apiKeys: [...(selectedCustom.apiKeys ?? []), entry] });
                      setAddingKey('');
                    }}
                  >
                    <Plus size={13} />
                  </button>
                </div>
              </div>
            </div>
            {/* API 协议选择：OpenAI 系三变体 + Anthropic Messages */}
            <div className="field">
              <label><Bot size={13} />{t.set.protocol}</label>
              <div className="proto-chips">
                {API_PROTOCOLS.map((proto) => (
                  <button
                    key={proto.id}
                    type="button"
                    className={`chip sm ${(selectedCustom.protocol ?? 'openai-compatible') === proto.id ? 'on' : ''}`}
                    onClick={() => patchCustom(selectedCustom.id, { protocol: proto.id })}
                    title={proto.desc}
                  >
                    {proto.label}
                  </button>
                ))}
              </div>
              <p className="proto-hint">
                {(selectedCustom.protocol ?? 'openai-compatible') === 'anthropic-messages'
                  ? t.set.protocolAnthropicHint
                  : t.set.protocolOpenaiHint}
              </p>
            </div>
            <div className="field">
              <label><Bot size={13} />{t.set.pmodels}</label>
              <div className="prov-models">
                {selectedCustom.models.map((model) => (
                  <span key={model} className="chip on">
                    {model}
                    <button
                      type="button"
                      className="chip-x"
                      onClick={() => patchCustom(selectedCustom.id, {
                        models: selectedCustom.models.filter((m) => m !== model),
                      })}
                      aria-label={t.terms.remove}
                    >
                      ×
                    </button>
                  </span>
                ))}
              </div>
              <div style={{ display: 'flex', gap: 8, marginTop: 8 }}>
                <input
                  value={addingModel}
                  placeholder={t.set.modelPlaceholder}
                  onChange={(e) => setAddingModel(e.target.value)}
                  onKeyDown={(e) => { if (e.key === 'Enter') addModel(selectedCustom.id); }}
                  style={{ flex: 1 }}
                />
                <button type="button" className="btn ghost sm" onClick={() => addModel(selectedCustom.id)}>
                  <Plus size={13} />
                  {t.terms.add}
                </button>
              </div>
            </div>
          </>
        ) : null}
        {!selectedCustom && !selectedBuiltin && (
          <p className="card-desc" style={{ padding: 24, textAlign: 'center' }}>—</p>
        )}
      </div>

      {testMessage && (
        <p className={`prov-test-msg ${testState === 'ok' ? 'ok' : 'err'}`}>{testMessage}</p>
      )}
    </div>
  );
}

function resolveBuiltinFields(provider: AppSettings['provider']) {
  if (provider === 'sensenova') {
    return { apiUrl: 'sensenova_api_url', apiKey: 'sensenova_api_key', model: 'sensenova_model' } as const;
  }
  if (provider === 'minimax') {
    return { apiUrl: 'minimax_api_url', apiKey: 'minimax_api_key', model: 'minimax_model' } as const;
  }
  if (provider === 'cohere') {
    return { apiUrl: 'cohere_api_url', apiKey: 'cohere_api_key', model: 'cohere_model' } as const;
  }
  if (provider === 'nvidia') {
    return { apiUrl: 'nvidia_api_url', apiKey: 'nvidia_api_key', model: 'nvidia_model' } as const;
  }
  return { apiUrl: 'llm_api_url', apiKey: 'llm_api_key', model: 'llm_model' } as const;
}

function modelSuggestions(provider: AppSettings['provider']) {
  if (provider === 'sensenova') return ['deepseek-v4-flash', 'sensenova-6.7-flash-lite'];
  if (provider === 'minimax') return ['MiniMax-M2.7'];
  if (provider === 'cohere') return ['command-a-plus-05-2026'];
  if (provider === 'nvidia') return ['stepfun-ai/step-3.7-flash', 'google/diffusiongemma-26b-a4b-it'];
  return [];
}

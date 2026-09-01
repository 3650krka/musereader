import { Bot, Database, FileJson, Globe, KeyRound, ShieldCheck, Sparkles } from 'lucide-react';
import { useId, type ReactNode } from 'react';
import { useT } from '@/i18n';
import type { AppSettings } from '@/types';
import type { ServicePanelProps } from './panelTypes';

type RuntimeServicePanelProps = Pick<
  ServicePanelProps,
  'settings' | 'updateSetting' | 'updateRuntimeConfig'
>;

export function RuntimeServicePanel({
  settings,
  updateSetting,
  updateRuntimeConfig,
}: RuntimeServicePanelProps) {
  const t = useT();
  const provider = settings.runtimeConfig.llm_provider || settings.provider;
  const llmFields = resolveProviderFields(provider);
  const modelSuggestions = modelSuggestionsForProvider(provider);

  return (
    <>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
        <div className="row2">
          <div className="grow">
            <div className="name"><KeyRound size={15} style={{ color: 'var(--ink-3)' }} />{t.set.currentProvider}</div>
            <div className="desc">{t.set.providerNote}</div>
          </div>
          <select
            value={provider}
            onChange={(event) => {
              const nextProvider = event.target.value as AppSettings['provider'];
              updateSetting('provider', nextProvider);
              updateRuntimeConfig('llm_provider', nextProvider);
            }}
            className="v2select"
          >
            <option value="sensenova">SenseNova</option>
            <option value="minimax">MiniMax</option>
            <option value="cohere">Cohere</option>
            <option value="nvidia">NVIDIA</option>
            <option value="openai-compatible">OpenAI</option>
            <option value="custom">{t.set.customOverride}</option>
          </select>
        </div>

        <div className="row2">
          <div className="grow">
            <div className="name"><ShieldCheck size={15} style={{ color: 'var(--ink-3)' }} />{t.set.currentMode}</div>
            <div className="desc">{t.set.currentModeNote}</div>
          </div>
          <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
            <span className="chip on">{t.set.builtin}</span>
            <span className="chip">{t.set.envInjected}</span>
            <span className="chip">{t.set.customOverride}</span>
          </div>
        </div>

        <div className="row2">
          <div className="grow">
            <div className="name"><Database size={15} style={{ color: 'var(--ink-3)' }} />{t.set.ocrCache}</div>
            <div className="desc">{t.set.ocrCacheDesc}</div>
          </div>
          <button
            type="button"
            className={`switch ${settings.runtimeConfig.use_ocr_cache ? 'on' : ''}`}
            onClick={() => updateRuntimeConfig('use_ocr_cache', !settings.runtimeConfig.use_ocr_cache)}
            aria-pressed={settings.runtimeConfig.use_ocr_cache}
            aria-label={t.set.ocrCache}
          />
        </div>

        <div className="row2">
          <div className="grow">
            <div className="name"><FileJson size={15} style={{ color: 'var(--ink-3)' }} />{t.set.frontMatterCache}</div>
            <div className="desc">{t.set.frontMatterCacheNote}</div>
          </div>
          <button
            type="button"
            className={`switch ${settings.runtimeConfig.use_front_matter_cache ? 'on' : ''}`}
            onClick={() => updateRuntimeConfig('use_front_matter_cache', !settings.runtimeConfig.use_front_matter_cache)}
            aria-pressed={settings.runtimeConfig.use_front_matter_cache}
            aria-label={t.set.frontMatterCache}
          />
        </div>

        <div className="row2">
          <div className="grow">
            <div className="name"><Sparkles size={15} style={{ color: 'var(--ink-3)' }} />{t.set.coreSkillInject}</div>
            <div className="desc">{t.set.coreSkillInjectDesc}</div>
          </div>
          <button
            type="button"
            className={`switch ${settings.runtimeConfig.inject_core_skill ?? true ? 'on' : ''}`}
            onClick={() => updateRuntimeConfig('inject_core_skill', !(settings.runtimeConfig.inject_core_skill ?? true))}
            aria-pressed={settings.runtimeConfig.inject_core_skill ?? true}
            aria-label={t.set.coreSkillInject}
          />
        </div>
      </div>

      <div className="divider" />

      <div className="grid2">
        <RuntimeField
          icon={<Globe size={14} />}
          label={t.set.llmUrl}
          value={settings.runtimeConfig[llmFields.apiUrl]}
          onChange={(value) => updateRuntimeConfig(llmFields.apiUrl, value)}
          placeholder={t.set.llmUrlPlaceholder}
        />
        <RuntimeField
          icon={<Bot size={14} />}
          label={t.set.model}
          value={settings.runtimeConfig[llmFields.model]}
          onChange={(value) => updateRuntimeConfig(llmFields.model, value)}
          placeholder={t.set.modelPlaceholder}
          suggestions={modelSuggestions}
        />
        <RuntimeField
          icon={<ShieldCheck size={14} />}
          label={t.set.llmKey}
          value={settings.runtimeConfig[llmFields.apiKey]}
          onChange={(value) => updateRuntimeConfig(llmFields.apiKey, value)}
          placeholder={t.set.keyPlaceholder}
          type="password"
        />
        <RuntimeField
          icon={<Globe size={14} />}
          label={t.set.ocrUrl}
          value={settings.runtimeConfig.ocr_api_url}
          onChange={(value) => updateRuntimeConfig('ocr_api_url', value)}
          placeholder={t.set.keyPlaceholder}
        />
        <RuntimeField
          icon={<ShieldCheck size={14} />}
          label={t.set.ocrToken}
          value={settings.runtimeConfig.ocr_token}
          onChange={(value) => updateRuntimeConfig('ocr_token', value)}
          placeholder={t.set.keyPlaceholder}
          type="password"
        />
      </div>
    </>
  );
}

function resolveProviderFields(provider: AppSettings['provider']) {
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

function modelSuggestionsForProvider(provider: AppSettings['provider']) {
  if (provider === 'sensenova') return ['deepseek-v4-flash', 'sensenova-6.7-flash-lite'];
  if (provider === 'minimax') return ['MiniMax-M2.7'];
  if (provider === 'cohere') return ['command-a-plus-05-2026'];
  if (provider === 'nvidia') return ['stepfun-ai/step-3.7-flash', 'google/diffusiongemma-26b-a4b-it'];
  return [];
}

interface RuntimeFieldProps {
  icon: ReactNode;
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
  type?: 'text' | 'password';
  suggestions?: string[];
}

function RuntimeField({ icon, label, value, onChange, placeholder, type = 'text', suggestions = [] }: RuntimeFieldProps) {
  const inputId = useId();
  const listId = suggestions.length > 0 ? `${inputId}-suggestions` : undefined;

  return (
    <div className="field">
      <label htmlFor={inputId} style={{ display: 'inline-flex', alignItems: 'center', gap: 6 }}>
        {icon}
        {label}
      </label>
      <input
        id={inputId}
        type={type}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        list={listId}
      />
      {listId ? (
        <datalist id={listId}>
          {suggestions.map((suggestion) => (
            <option key={suggestion} value={suggestion} />
          ))}
        </datalist>
      ) : null}
    </div>
  );
}

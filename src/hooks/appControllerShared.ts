import type { AppSettings, Book, RuntimeConfig, Theme } from '@/types';

export const SETTINGS_STORAGE_KEY = 'musereader-settings';
/** 旧键（musetranslate 时代）：新键缺数据时回落读取，保住升级用户的设置。 */
export const LEGACY_SETTINGS_STORAGE_KEY = 'musetranslate-settings';

export const DEFAULT_SETTINGS: AppSettings = {
  provider: 'sensenova',
  librarySurfaceStyle: 'canvas',
  bilingualDefault: false,
  wordwiseDefault: true,
  paragraphDensity: 'standard',
  learningFeaturesEnabled: true,
  notesFeaturesEnabled: true,
  learningDefaultView: 'cards',
  notesDefaultView: 'feed',
  palette: 'coral',
  runtimeConfig: {
    ocr_api_url: '',
    ocr_token: '',
    llm_provider: 'sensenova',
    llm_api_url: '',
    llm_api_key: '',
    llm_model: '',
    sensenova_api_url: '',
    sensenova_api_key: '',
    sensenova_model: '',
    minimax_api_url: '',
    minimax_api_key: '',
    minimax_model: '',
    cohere_api_url: '',
    cohere_api_key: '',
    cohere_model: '',
    nvidia_api_url: '',
    nvidia_api_key: '',
    nvidia_model: '',
    use_ocr_cache: true,
    use_front_matter_cache: true,
  },
};

export interface StoredSettingsPayload extends Partial<AppSettings> {
  theme?: Theme;
}

export function parseStoredSettings(raw: string | null) {
  if (!raw) return null;
  return JSON.parse(raw) as StoredSettingsPayload;
}

export function mergeLoadedSettings(
  previous: AppSettings,
  runtimeConfig: RuntimeConfig,
  stored: StoredSettingsPayload | null,
) {
  if (!stored) {
    return {
      ...previous,
      runtimeConfig,
    };
  }

  return {
    ...previous,
    ...stored,
    /* 旧版本存储数据没有 palette 字段：展开合并会把 undefined 写入，
       必须回落到默认值，否则 data-palette="undefined" 导致色系失效。 */
    palette: stored.palette ?? previous.palette,
    runtimeConfig,
  };
}

export function buildStoredSettingsPayload(settings: AppSettings, theme: Theme) {
  return JSON.stringify({
    ...settings,
    theme,
  });
}

export function buildDisplayBooks(books: Book[]) {
  return books;
}

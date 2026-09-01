import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type {
  AgentDefinition,
  AgentPromptPreview,
  AppSettings,
  Book,
  RuntimeConfig,
  SkillLibraryEntry,
  Theme,
  TranslationTask,
} from '@/types';
import { detectInitialLang, LANG_STORAGE_KEY, type Lang } from '@/i18n/context';
import { useTaskRuntime } from './useTaskRuntime';
import type { AppView } from '../appShared';
import {
  buildDisplayBooks,
  buildStoredSettingsPayload,
  DEFAULT_SETTINGS,
  LEGACY_SETTINGS_STORAGE_KEY,
  mergeLoadedSettings,
  parseStoredSettings,
  SETTINGS_STORAGE_KEY,
} from './appControllerShared';

export function useAppController() {
  const [currentView, setCurrentView] = useState<AppView>('library');
  const [currentBook, setCurrentBook] = useState<Book | null>(null);
  /** 打开书时指定跳转锚点（笔记/书签跳书用；消费后清除）。 */
  const [pendingBookAnchor, setPendingBookAnchor] = useState<string | null>(null);
  /** 学习视图聚焦书（书库右键「词汇透析」跳入该书生词库）。 */
  const [vocabBookId, setVocabBookId] = useState<string | null>(null);
  const [theme, setTheme] = useState<Theme>('light');
  const [lang, setLangState] = useState<Lang>(detectInitialLang);
  const { books, tasks, upsertTask, removeTask, toggleBookFavorite, setBookTags, setBookFolder } = useTaskRuntime();
  const [settingsSaveState, setSettingsSaveState] = useState<'idle' | 'saving' | 'saved' | 'error'>('idle');
  const [settingsSaveMessage, setSettingsSaveMessage] = useState('');
  const [skillEntries, setSkillEntries] = useState<SkillLibraryEntry[]>([]);
  const [agentDefinitions, setAgentDefinitions] = useState<AgentDefinition[]>([]);
  const [translationPromptPreview, setTranslationPromptPreview] = useState<AgentPromptPreview | null>(null);
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  /* 持久化门闩：初始异步加载完成前禁止写回——否则挂载效应会先用
     DEFAULT_SETTINGS 覆盖 localStorage，再被加载读到，主题等本地设置每次启动被重置。 */
  const settingsHydratedRef = useRef(false);

  const setLang = (next: Lang) => {
    setLangState(next);
    localStorage.setItem(LANG_STORAGE_KEY, next);
  };

  useEffect(() => {
    const loadSettings = async () => {
      try {
        const runtimeConfig = await invoke<RuntimeConfig>('get_runtime_config');
        /* 旧键迁移：musereader 新键无数据时回落 musetranslate 旧键（升级用户设置不丢）。 */
        const stored =
          localStorage.getItem(SETTINGS_STORAGE_KEY) ??
          localStorage.getItem(LEGACY_SETTINGS_STORAGE_KEY);
        const parsed = parseStoredSettings(stored);
        if (parsed?.theme) setTheme(parsed.theme);
        setSettings((prev) => mergeLoadedSettings(prev, runtimeConfig, parsed));
      } catch (error) {
        console.error('Failed to load settings:', error);
      } finally {
        settingsHydratedRef.current = true;
      }
    };

    void loadSettings();
  }, []);

  const reloadSkillEntries = useCallback(async () => {
    try {
      const [entries, agents, preview] = await Promise.all([
        invoke<SkillLibraryEntry[]>('list_skill_library_entries'),
        invoke<AgentDefinition[]>('list_agents_registry'),
        invoke<AgentPromptPreview>('preview_agent_prompt', {
          agent: 'translation',
          articleType: 'fiction',
          readingRequest: null,
        }),
      ]);
      setSkillEntries(entries);
      setAgentDefinitions(agents);
      setTranslationPromptPreview(preview);
    } catch (error) {
      console.error('Failed to load skill library:', error);
    }
  }, []);

  useEffect(() => {
    void reloadSkillEntries();
  }, [reloadSkillEntries]);

  useEffect(() => {
    if (settingsSaveState !== 'saved') return;
    const timer = window.setTimeout(() => {
      setSettingsSaveState('idle');
      setSettingsSaveMessage('');
    }, 1800);
    return () => window.clearTimeout(timer);
  }, [settingsSaveState]);

  useEffect(() => {
    if (!settingsHydratedRef.current) return;
    localStorage.setItem(SETTINGS_STORAGE_KEY, buildStoredSettingsPayload(settings, theme));
  }, [settings, theme]);

  useEffect(() => {
    if (currentView === 'learning' && !settings.learningFeaturesEnabled) setCurrentView('library');
    if (currentView === 'notes' && !settings.notesFeaturesEnabled) setCurrentView('library');
  }, [currentView, settings.learningFeaturesEnabled, settings.notesFeaturesEnabled]);

  const handleSettingsChange = async (nextSettings: AppSettings) => {
    setSettings(nextSettings);
    setSettingsSaveState('saving');
    setSettingsSaveMessage('');
    localStorage.setItem(SETTINGS_STORAGE_KEY, buildStoredSettingsPayload(nextSettings, theme));

    try {
      await invoke('update_runtime_config', { config: nextSettings.runtimeConfig });
      setSettingsSaveState('saved');
      setSettingsSaveMessage('已保存');
    } catch (error) {
      console.error('Failed to update runtime config:', error);
      setSettingsSaveState('error');
      setSettingsSaveMessage('保存失败');
    }
  };

  const handleSkillEntrySave = async (skillId: string, content: string) => {
    const updatedEntry = await invoke<SkillLibraryEntry>('update_skill_library_entry', { skillId, content });
    setSkillEntries((prev) => prev.map((entry) => (entry.id === skillId ? updatedEntry : entry)));

    if (updatedEntry.agent === 'translation') {
      const preview = await invoke<AgentPromptPreview>('preview_agent_prompt', {
        agent: 'translation',
        articleType: updatedEntry.articleType ?? 'fiction',
        readingRequest: null,
      });
      setTranslationPromptPreview(preview);
    }
  };

  const handleTaskCreated = (task: TranslationTask) => upsertTask(task);
  const handleTaskUpdated = (task: TranslationTask) => upsertTask(task);

  const handleDeleteTask = async (taskId: string) => {
    try {
      await invoke('delete_task', { taskId });
      removeTask(taskId);
    } catch (error) {
      console.error('Failed to delete task:', error);
    }
  };

  const openBook = (book: Book, anchorHref?: string | null) => {
    setCurrentBook(book);
    setPendingBookAnchor(anchorHref?.trim() || null);
    setCurrentView('reader');
  };

  /** 从阅读器直达该书的校对工作台（review 视图自动聚焦此书）。 */
  /** 跳到学习视图并聚焦该书的生词背诵台（词汇透析入口）。 */
  const openVocab = (book: Book) => {
    setVocabBookId(book.id);
    setCurrentView('learning');
  };

  const openReview = (book: Book) => {
    setCurrentBook(book);
    setCurrentView('review');
  };

  const displayBooks = useMemo(() => {
    return buildDisplayBooks(books);
  }, [books]);

  return {
    agentDefinitions,
    books,
    currentBook,
    currentView,
    displayBooks,
    handleDeleteTask,
    handleSettingsChange,
    handleSkillEntrySave,
    handleTaskCreated,
    handleTaskUpdated,
    lang,
    setLang,
    openBook,
    openVocab,
    vocabBookId,
    clearVocabBookId: () => setVocabBookId(null),
    pendingBookAnchor,
    clearPendingBookAnchor: () => setPendingBookAnchor(null),
    openReview,
    reloadSkillEntries,
    setCurrentView,
    settings,
    settingsSaveMessage,
    settingsSaveState,
    skillEntries,
    tasks,
    theme,
    translationPromptPreview,
    setTheme,
    toggleBookFavorite,
    setBookTags,
    setBookFolder,
  };
}

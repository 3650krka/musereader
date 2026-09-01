import { invoke } from '@tauri-apps/api/core';
import { useEffect, useMemo, useState } from 'react';
import type { AgentPromptPreview, AppSettings, SkillLibraryEntry } from '@/types';
import type { SettingsSection } from './settingsShared';

interface UseSettingsPageStateArgs {
  settings: AppSettings;
  onSettingsChange: (settings: AppSettings) => void | Promise<void>;
  skillEntries: SkillLibraryEntry[];
  onSkillEntrySave: (skillId: string, content: string) => Promise<void>;
}

export function useSettingsPageState({
  settings,
  onSettingsChange,
  skillEntries,
  onSkillEntrySave,
}: UseSettingsPageStateArgs) {
  const [activeSection, setActiveSection] = useState<SettingsSection>('appearance');
  const [selectedSkillId, setSelectedSkillId] = useState('');
  const [skillDraft, setSkillDraft] = useState('');
  const [skillSaveState, setSkillSaveState] = useState<'idle' | 'saving' | 'saved' | 'error'>('idle');
  /* 阅读 agent 预览：固定示例片段，随 skill 列表刷新 */
  const [readingPreview, setReadingPreview] = useState<AgentPromptPreview | null>(null);

  const selectedSkill = useMemo(
    () => skillEntries.find((entry) => entry.id === selectedSkillId) ?? skillEntries[0] ?? null,
    [selectedSkillId, skillEntries],
  );

  useEffect(() => {
    if (!selectedSkillId && skillEntries[0]) {
      setSelectedSkillId(skillEntries[0].id);
    }
  }, [selectedSkillId, skillEntries]);

  useEffect(() => {
    if (selectedSkill) {
      setSkillDraft(selectedSkill.content);
    }
  }, [selectedSkill]);

  useEffect(() => {
    let cancelled = false;
    invoke<AgentPromptPreview>('preview_agent_prompt', {
      agent: 'reading',
      articleType: null,
      readingRequest: {
        taskKind: 'sentence',
        bookTitle: null,
        chapter: null,
        excerpt: 'The light in the valley faded slowly as they crossed the old stone bridge.',
        focusText: null,
        userNote: null,
      },
    })
      .then((preview) => { if (!cancelled) setReadingPreview(preview); })
      .catch(() => { if (!cancelled) setReadingPreview(null); });
    return () => { cancelled = true; };
  }, [skillEntries]);

  const updateSetting = <K extends keyof AppSettings>(key: K, value: AppSettings[K]) => {
    void onSettingsChange({
      ...settings,
      [key]: value,
    });
  };

  const updateRuntimeConfig = <K extends keyof AppSettings['runtimeConfig']>(
    key: K,
    value: AppSettings['runtimeConfig'][K],
  ) => {
    void onSettingsChange({
      ...settings,
      runtimeConfig: {
        ...settings.runtimeConfig,
        [key]: value,
      },
    });
  };

  const saveSelectedSkill = async () => {
    if (!selectedSkill) return;
    setSkillSaveState('saving');
    try {
      await onSkillEntrySave(selectedSkill.id, skillDraft);
      setSkillSaveState('saved');
    } catch (error) {
      console.error('Failed to save skill:', error);
      setSkillSaveState('error');
    }
  };

  return {
    activeSection,
    readingPreview,
    selectedSkill,
    selectedSkillId,
    setActiveSection,
    setSelectedSkillId,
    skillDraft,
    setSkillDraft,
    skillSaveState,
    updateRuntimeConfig,
    updateSetting,
    saveSelectedSkill,
  };
}

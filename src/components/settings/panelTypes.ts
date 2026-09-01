import type { Variants } from 'motion/react';
import type { AgentDefinition, AgentPromptPreview, AppSettings, SkillLibraryEntry, Theme } from '@/types';

export interface MotionProps {
  listVariants: Variants;
  rowVariants: Variants;
}

export interface SettingsControlProps extends MotionProps {
  settings: AppSettings;
  updateSetting: <K extends keyof AppSettings>(key: K, value: AppSettings[K]) => void;
}

export interface AppearancePanelProps extends SettingsControlProps {
  theme: Theme;
  setTheme: (theme: Theme) => void;
}

export interface ServicePanelProps extends MotionProps {
  settings: AppSettings;
  updateSetting: <K extends keyof AppSettings>(key: K, value: AppSettings[K]) => void;
  updateRuntimeConfig: <K extends keyof AppSettings['runtimeConfig']>(
    key: K,
    value: AppSettings['runtimeConfig'][K],
  ) => void;
  skillEntries: SkillLibraryEntry[];
  agentDefinitions: AgentDefinition[];
  translationPromptPreview: AgentPromptPreview | null;
  selectedSkill: SkillLibraryEntry | null;
  selectedSkillId: string;
  setSelectedSkillId: (id: string) => void;
  skillDraft: string;
  setSkillDraft: (draft: string) => void;
  skillSaveState: 'idle' | 'saving' | 'saved' | 'error';
  onSaveSkill: () => Promise<void>;
}

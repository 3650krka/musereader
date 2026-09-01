import type { LucideIcon } from 'lucide-react';
import {
  BookOpenText,
  BrainCircuit,
  Coffee,
  Eye,
  KeyRound,
  LaptopMinimal,
  Moon,
  Palette,
  PanelsTopLeft,
  Sun,
} from 'lucide-react';
import type { Theme } from '@/types';

export type SettingsSection =
  | 'appearance'
  | 'reading'
  | 'learning'
  | 'service'
  | 'layout'
  | 'about';

export interface ThemeOption {
  id: Theme;
  label: string;
  icon: LucideIcon;
  note: string;
  swatch: string;
}

export interface SettingsSectionOption {
  id: SettingsSection;
  label: string;
  icon: LucideIcon;
  note: string;
}

export const UI = {
  pureWhite: '\u7eaf\u767d',
  pureWhiteNote: '\u4eae\u5e95\uff0c\u65e0\u5206\u533a\u80cc\u666f',
  softPaper: '\u67d4\u7eb8',
  softPaperNote: '\u5fae\u7070\uff0c\u6574\u5757\u767d\u533a\u57df',
  coolGray: '\u51b7\u7070',
  coolGrayNote: '\u51b7\u8c03\u7eb8\u611f',
  night: '\u591c\u95f4',
  nightNote: '\u6697\u5149\u9605\u8bfb',
  appearance: '\u5916\u89c2',
  appearanceNote: '\u4e3b\u9898\u4e0e\u9996\u9875',
  reading: '\u9605\u8bfb',
  readingNote: '\u6392\u7248\u4e0e\u663e\u793a',
  learning: '\u5b66\u4e60',
  learningNote: '\u751f\u8bcd\u4e0e\u7b14\u8bb0',
  service: '\u670d\u52a1',
  serviceNote: 'OCR \u4e0e LLM',
  device: '\u8bbe\u5907',
  deviceNote: '\u6a2a\u7ad6\u5c4f\u9002\u914d',
  about: '\u5173\u4e8e',
  aboutNote: '\u7248\u672c\u4fe1\u606f',
  current: '\u5f53\u524d',
  switch: '\u5207\u6362',
  homeLayout: '\u9996\u9875\u5e03\u5c40',
  homeLayoutNote: '\u6574\u5757\u6a21\u5f0f\u6216\u65e0\u5361\u6a21\u5f0f',
  blockMode: '\u6574\u5757\u6a21\u5f0f',
  canvasMode: '\u65e0\u5361\u6a21\u5f0f',
  appearanceTitle: '\u4e3b\u9898\u4e0e\u9996\u9875',
  readingTitle: '\u6392\u7248\u4e0e\u663e\u793a',
  bilingual: '\u53cc\u8bed\u663e\u793a',
  bilingualNote: '\u9ed8\u8ba4\u540c\u65f6\u663e\u793a\u539f\u6587\u4e0e\u8bd1\u6587',
  wordAssist: '\u8bcd\u4e49\u8f85\u52a9',
  wordAssistNote: '\u9ed8\u8ba4\u663e\u793a\u8bcd\u4e49\u63d0\u793a',
  density: '\u6bb5\u843d\u5bc6\u5ea6',
  densityNote: '\u63a7\u5236\u6bb5\u95f4\u8ddd\u4e0e\u9605\u8bfb\u8282\u594f',
  compact: '\u7d27\u51d1',
  standard: '\u6807\u51c6',
  relaxed: '\u5bbd\u677e',
  learningTitle: '\u5b66\u4e60\u4e0e\u7b14\u8bb0',
  learningEntry: '\u82f1\u8bed\u5b66\u4e60\u5165\u53e3',
  learningEntryNote: '\u542f\u7528\u751f\u8bcd\u3001\u8bed\u5883\u548c\u53e5\u6cd5\u5b66\u4e60',
  notesEntry: '\u7b14\u8bb0\u4e0e\u4e66\u7b7e\u5165\u53e3',
  notesEntryNote: '\u542f\u7528\u6458\u5f55\u3001\u4e66\u7b7e\u4e0e\u7b14\u8bb0\u9875\u9762',
  cards: '\u5361\u7247',
  serviceTitle: '\u6a21\u578b\u4e0e OCR',
  promptTitle: 'Skill \u4e0e Agent',
  promptNote: '\u5185\u7f6e\u63d0\u793a\u5e93\uff0c\u53ef\u76f4\u63a5\u7f16\u8f91',
  skills: 'Skill',
  promptPreview: '\u6700\u7ec8 Prompt',
  skillPlaceholder: '\u5728\u8fd9\u91cc\u7f16\u8f91 skill',
  saveSkill: '\u4fdd\u5b58 Skill',
  savingSkill: '\u4fdd\u5b58\u4e2d',
  translationAgent: '\u7ffb\u8bd1 Agent',
  readingAgent: '\u9605\u8bfb Agent',
  provider: '\u670d\u52a1\u6765\u6e90',
  providerNote: '\u5185\u7f6e\u670d\u52a1\u53ef\u76f4\u63a5\u6d4b\u8bd5\uff0c\u4e5f\u53ef\u8986\u76d6\u4e3a\u81ea\u5b9a\u4e49\u63a5\u53e3',
  builtInSenseNova: '\u5185\u7f6e SenseNova',
  builtInMiniMax: '\u5185\u7f6e MiniMax',
  builtInCohere: '\u5185\u7f6e Cohere',
  builtInNvidia: '\u5185\u7f6e NVIDIA',
  openAICompatible: '\u517c\u5bb9 OpenAI',
  custom: '\u81ea\u5b9a\u4e49',
  currentProviderConfig: '\u5f53\u524d\u4f9b\u5e94\u5546\u914d\u7f6e',
  providerConfigNote: '\u4ec5\u4fdd\u5b58\u5f53\u524d\u9009\u4e2d\u4f9b\u5e94\u5546\u4f7f\u7528\u7684 LLM \u53c2\u6570\uff0cOCR \u4ecd\u4e3a\u5168\u5c40\u914d\u7f6e',
  sensenovaConfig: 'SenseNova \u914d\u7f6e',
  minimaxConfig: 'MiniMax \u914d\u7f6e',
  cohereConfig: 'Cohere \u914d\u7f6e',
  nvidiaConfig: 'NVIDIA \u914d\u7f6e',
  currentMode: '\u5f53\u524d\u6a21\u5f0f',
  currentModeNote: '\u542f\u52a8\u65f6\u8bfb\u53d6 .env \u5e76\u6ce8\u5165 OCR \u4e0e LLM \u914d\u7f6e',
  builtIn: '\u5185\u7f6e',
  envInjected: '\u542f\u52a8\u6ce8\u5165',
  customOverride: '\u53ef\u8986\u76d6',
  cachePolicy: '\u5904\u7406\u7f13\u5b58',
  cachePolicyNote: '\u8de8\u4efb\u52a1\u590d\u7528 OCR \u539f\u59cb\u7ed3\u679c\u4e0e\u5934\u90e8\u5143\u6570\u636e\u62bd\u53d6',
  ocrCache: 'OCR \u539f\u59cb\u7f13\u5b58',
  ocrCacheNote: '\u540c\u4e00 PDF \u53ef\u8df3\u8fc7 OCR \u8bf7\u6c42\uff1b\u5173\u95ed\u540e\u4ecd\u4fdd\u7559\u672c artifact \u65ad\u70b9\u590d\u7528',
  frontMatterCache: '\u5934\u90e8\u5143\u6570\u636e\u7f13\u5b58',
  frontMatterCacheNote: '\u590d\u7528\u5df2\u62bd\u53d6\u7684\u6807\u9898\u3001\u4f5c\u8005\u548c\u673a\u6784\uff1b\u5173\u95ed\u540e\u4f1a\u91cd\u65b0\u62bd\u53d6',
  llmUrl: 'LLM \u5730\u5740',
  model: '\u6a21\u578b',
  llmKey: 'LLM \u5bc6\u94a5',
  ocrUrl: 'OCR \u5730\u5740',
  ocrToken: 'OCR \u4ee4\u724c',
  llmUrlPlaceholder: '\u542f\u52a8\u65f6\u81ea\u52a8\u6ce8\u5165\uff0c\u4e5f\u53ef\u5728\u8fd9\u91cc\u8986\u76d6',
  modelPlaceholder: '\u9ed8\u8ba4\u4f7f\u7528\u5185\u7f6e\u6a21\u578b',
  llmKeyPlaceholder: '\u7559\u7a7a\u5219\u4f7f\u7528 .env',
  ocrUrlPlaceholder: '\u7559\u7a7a\u5219\u4f7f\u7528 .env',
  ocrTokenPlaceholder: '\u7559\u7a7a\u5219\u4f7f\u7528 .env',
  deviceTitle: '\u5e03\u5c40\u9002\u914d',
  windowsLandscape: 'Windows \u6a2a\u5c4f',
  windowsLandscapeNote: '\u4e66\u5e93\u533a\u4e0e\u4fa7\u680f\u540c\u65f6\u53ef\u89c1\uff0c\u66f4\u9002\u5408\u6574\u7406\u4e0e\u5bfc\u5165',
  twoColumn: '\u53cc\u680f',
  androidPortrait: 'Android \u7ad6\u5c4f',
  androidPortraitNote: '\u5185\u5bb9\u4f18\u5148\uff0c\u8be6\u60c5\u901a\u8fc7\u4fa7\u9875\u4e0e\u5c42\u7ea7\u8fdb\u5165',
  singleColumn: '\u5355\u5217',
  aboutTitle: '\u5e94\u7528\u4fe1\u606f',
  version: '\u7248\u672c',
  settings: '\u8bbe\u7f6e',
  settingsTitle: '\u504f\u597d\u4e0e\u670d\u52a1',
  on: '\u5f00\u542f',
  off: '\u5173\u95ed',
} as const;

export const THEME_OPTIONS: ThemeOption[] = [
  { id: 'light', label: UI.pureWhite, icon: Sun, note: UI.pureWhiteNote, swatch: '#ffffff' },
  { id: 'paper', label: UI.softPaper, icon: PanelsTopLeft, note: UI.softPaperNote, swatch: '#f5f5f3' },
  { id: 'sepia', label: UI.coolGray, icon: Coffee, note: UI.coolGrayNote, swatch: '#edf2f4' },
  { id: 'dark', label: UI.night, icon: Moon, note: UI.nightNote, swatch: '#14161b' },
];

export const SETTINGS_SECTIONS: SettingsSectionOption[] = [
  { id: 'appearance', label: UI.appearance, icon: Palette, note: UI.appearanceNote },
  { id: 'reading', label: UI.reading, icon: BookOpenText, note: UI.readingNote },
  { id: 'learning', label: UI.learning, icon: BrainCircuit, note: UI.learningNote },
  { id: 'service', label: UI.service, icon: KeyRound, note: UI.serviceNote },
  { id: 'layout', label: UI.device, icon: LaptopMinimal, note: UI.deviceNote },
  { id: 'about', label: UI.about, icon: Eye, note: UI.aboutNote },
];

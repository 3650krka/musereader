export type Theme = 'light' | 'paper' | 'sepia' | 'dark';

/** 色系（类 Material 动态取色）：与明暗主题（Theme）正交，
    通过 data-palette 属性切换 signal/accent/滑块渐变族。 */
export type Palette = 'coral' | 'jade' | 'indigo' | 'gold' | 'violet';
export type ReaderTab = 'analysis' | 'notes' | 'toc' | 'ai' | 'words' | 'search' | 'review';
export type ProviderId = 'custom' | 'sensenova' | 'minimax' | 'cohere' | 'nvidia' | 'openai-compatible';

/** 供应商 API key 条目（多 key 管理；enabled=false 跳过）。 */
export interface ProviderApiKey {
  value: string;
  enabled: boolean;
}

/** 用户自定义供应商（CherryStudio 式管理，runtime_config.customProviders 持久化）。 */
export interface CustomProvider {
  id: string;
  name: string;
  apiUrl: string;
  apiKey: string;
  models: string[];
  /** API 协议：openai | openai-compatible | openai-responses | anthropic-messages */
  protocol?: string;
  /** 多 key：逐个启停；解析时 apiKeys 优先、回落 apiKey。 */
  apiKeys?: ProviderApiKey[];
}

export interface Book {
  id: string;
  title: string;
  author: string;
  cover: string;
  /** 阅读进度，0–100 整数百分比。 */
  progress: number;
  lastRead: string;
  category: string;
  level: string;
  content?: string;
  originalPath?: string;
  /** 文件夹分组名（book_profiles 合并）。 */
  folder?: string;
  /** 用户可直接打开/导出的最终成书，双语 EPUB 优先。 */
  primaryArtifactPath?: string;
  translatedEpubPath?: string;
  format?: string;
  tocCount?: number;
  toc?: ReaderTocItem[];
  segments?: BilingualSegment[];
  /** 书籍真实元数据（EPUB/文档内置），用于替代哈希文件名做展示。 */
  metaTitle?: string;
  metaAuthor?: string;
  /** 收藏标记（book_profiles.json 持久化）。 */
  favorite?: boolean;
  /** 用户标签（book_profiles.json 持久化）。 */
  tags?: string[];
}

/** 书籍档案（后端 BookProfile，收藏/标签独立持久化）。 */
export interface BookProfile {
  bookId: string;
  favorite: boolean;
  tags: string[];
  /** 文件夹分组名（空=未分组）。 */
  folder?: string;
  updatedAt: string;
}

export interface BilingualSegment {
  source: string;
  target: string;
  anchorId?: string;
}

export interface ReaderTocItem {
  title: string;
  level: number;
  order: number;
  href?: string | null;
  progress: number;
  sourceTitle?: string;
}

/** EPUB 阅读包（prepare_epub_reader 返回）：解包后的章节清单与阅读元信息。 */
export interface EpubReaderPackage {
  cacheKey: string;
  rootPath: string;
  sections: EpubReaderSection[];
  hasBilingualMarkup: boolean;
  fixedLayout: boolean;
  pageProgressionDirection?: string | null;
}

export interface EpubReaderSection {
  index: number;
  href: string;
  filePath: string;
  title: string;
  /** 正文文本字符数：全书进度按章节权重加权的依据。 */
  textLength: number;
  /** 是否被书籍目录（NCX/nav）收录：目录树仅列收录条目，
      未收录的连续子文件归并到前一目录条目之下。 */
  tocCovered: boolean;
}

export interface EpubSearchResult {
  href: string;
  title: string;
  snippet: string;
  /** 关键词高亮版 snippet（含 <mark class="mt-hit">）。 */
  snippetHtml: string;
  occurrences: number;
}

export interface DocumentReaderPackage {
  cacheKey: string;
  rootPath: string;
  htmlPath: string;
}

export type TaskStatus = 'pending' | 'processing' | 'paused' | 'completed' | 'failed';

export type TaskPhase =
  | 'imported'
  | 'ocrRunning'
  | 'ocrReady'
  | 'chunking'
  | 'translating'
  | 'rendering'
  | 'writingArtifacts'
  | 'paused'
  | 'completed'
  | 'failed'
  | 'cancelled';

export interface AppError {
  code: string;
  message: string;
  retryable: boolean;
  retryAfterMs?: number;
}

export interface TaskError extends AppError {}

export interface TaskArtifactPaths {
  artifactDir?: string;
  sourceMarkdownPath?: string;
  sourceTocPath?: string;
  translatedTocPath?: string;
  outputMarkdownPath?: string;
  outputHtmlPath?: string;
  translatedEpubPath?: string;
  bilingualEpubPath?: string;
  checkpointPath?: string;
  manifestPath?: string;
  eventLogPath?: string;
  validationReportPath?: string;
  glossaryPath?: string;
  metricsPath?: string;
}

export interface TranslationTask {
  id: string;
  filename: string;
  pdfPath: string;
  status: TaskStatus;
  phase: TaskPhase;
  progress: number;
  message: string;
  articleType: string;
  concurrent: boolean;
  maxWorkers: number;
  createdAt: string;
  updatedAt: string;
  outputPath?: string;
  htmlOutputPath?: string;
  coverPath?: string;
  artifactPaths: TaskArtifactPaths;
  totalChunks: number;
  translatedChunks: number;
  retryCount: number;
  sourceHash?: string;
  lastError?: TaskError | null;
}

export interface RuntimeNotice {
  timestamp: string;
  scope: string;
  taskId?: string | null;
  detail: string;
  code: string;
  message: string;
}

export interface RuntimeConfig {
  ocr_api_url: string;
  ocr_token: string;
  llm_provider: ProviderId;
  llm_api_url: string;
  llm_api_key: string;
  llm_model: string;
  sensenova_api_url: string;
  sensenova_api_key: string;
  sensenova_model: string;
  minimax_api_url: string;
  minimax_api_key: string;
  minimax_model: string;
  cohere_api_url: string;
  cohere_api_key: string;
  cohere_model: string;
  nvidia_api_url: string;
  nvidia_api_key: string;
  nvidia_model: string;
  use_ocr_cache: boolean;
  use_front_matter_cache: boolean;
  customProviders?: CustomProvider[];
  /** 内置供应商扩展（多模型+多 key）：以 provider id 为键，仅用 models/apiKeys 字段。 */
  providerExtras?: CustomProvider[];
  /** 翻译 prompt 是否注入核心翻译 skill（全景翻译 skill）。 */
  inject_core_skill?: boolean;
  /** 翻译分块尺寸（字符数）：未设置用默认 3000；后端钳制到 [500, 20000]。 */
  chunk_size?: number;
}

export type SkillEntryKind = 'core' | 'reference' | 'fallback';

export interface SkillLibraryEntry {
  id: string;
  agent: string;
  kind: SkillEntryKind;
  articleType?: string | null;
  title: string;
  description: string;
  content: string;
  filePath: string;
  updatedAt: string;
}

export interface AgentPromptPreview {
  agent: string;
  articleType?: string | null;
  systemPrompt: string;
  fallbackPrompt?: string | null;
  skillIds: string[];
  systemPromptHash: string;
}

export type AgentCapability =
  | 'translateDocument'
  | 'explainSentence'
  | 'explainWord'
  | 'explainGrammar'
  | 'explainContext'
  | 'analyzeReading'
  | 'linkNotes';

export interface AgentDefinition {
  id: string;
  title: string;
  description: string;
  articleTypeRequired: boolean;
  defaultSkillIds: string[];
  capabilities: AgentCapability[];
}

export interface ReaderState {
  bookId: string;
  /** 阅读进度，0–100 整数百分比。 */
  progress: number;
  chapter: string;
  /** 最近阅读位置的 EPUB 锚点（href#anchor），重开书籍时精确恢复。 */
  href?: string | null;
  /** 锚点之后的页内偏移。 */
  offset?: number;
  bookmarks: ReaderBookmark[];
  notes: ReaderNote[];
  activities: ReaderActivity[];
  wordMarks: Record<string, WordMark>;
  vocabCards: Record<string, VocabCard>;
  /** 复习日志（90 天窗口，Insights 复习趋势数据源）。 */
  reviewLog?: ReviewLogEntry[];
  updatedAt: string;
}

export interface ReviewLogEntry {
  bookId: string;
  word: string;
  quality: number;
  createdAt: string;
}

export interface VocabCard {
  word: string;
  context: string;
  /** 例句译文（双语书成卡时由前端从配对块提取）。 */
  contextZh?: string;
  chapter: string;
  createdAt: string;
  /** AI 语境释义覆盖（优先于词表默认释义）。 */
  customDefinition?: string;
  /** 用户单词笔记（列表模式词详情内编辑）。 */
  note?: string;
  repetitions: number;
  intervalDays: number;
  easeFactor: number;
  dueAt?: string | null;
}

export interface VocabCardFace {
  word: string;
  phonetic: string;
  definition: string;
  level: string;
  context: string;
  /** 例句译文（双语书成卡时提取；单语书为空串）。 */
  contextZh: string;
  chapter: string;
  repetitions: number;
  intervalDays: number;
  /** 四档评分的下次间隔预览需要 EF（前端重放 SM-2）。 */
  easeFactor: number;
  dueAt?: string | null;
  /** —— 词库富信息（背面分层展示；未导入词库/未收录时缺省）—— */
  /** 完整中文翻译（词性分段）。 */
  translation?: string;
  /** 考试标签（zk/gk/cet4…，空格分隔）。 */
  tag?: string;
  /** 词形变换编码（d/p/i/3/s/r/t）。 */
  exchange?: string;
  /** 词根助记（分号分隔，最多两条）。 */
  root?: string;
  /** 柯林斯星级 1–5。 */
  collins?: number;
  /** 牛津3000（0/1）。 */
  oxford?: number;
  /** BNC 词频排名。 */
  bnc?: number;
  /** COCA 词频排名。 */
  frq?: number;
}

/** 跨书全局复习队列条目（list_due_vocab_cards_all）。 */
export interface GlobalDueCard extends VocabCardFace {
  bookId: string;
}

export interface ReaderBookmark {
  id: string;
  bookId: string;
  chapter: string;
  quote: string;
  progress: number;
  /** EPUB 章节定位（href#anchor），用于从书签跳回原文位置。 */
  href?: string | null;
  /** 划线/高亮样式：highlight/underline/wave/bookmark（空=老数据纯书签）。 */
  style?: string;
  createdAt: string;
}

export interface ReaderNote {
  id: string;
  bookId: string;
  chapter: string;
  quote: string;
  note: string;
  progress: number;
  /** EPUB 章节定位（href#anchor），用于从笔记跳回原文位置。 */
  href?: string | null;
  createdAt: string;
}

export interface ReaderActivity {
  id: string;
  bookId: string;
  minutes: number;
  progress: number;
  chapter: string;
  createdAt: string;
}

export type WordMarkStatus = 'mastered' | 'learning';

export interface WordMark {
  word: string;
  status: WordMarkStatus;
  context: string;
  /** AI 语境释义覆盖（用户点击「AI 释义」保存的语境义，优先于词表默认释义） */
  customDefinition?: string;
  updatedAt: string;
}

export type AiAssistAction = 'summarize' | 'explain' | 'translate' | 'define' | 'ask';

/** 目录编辑视图行（list_toc_edit_items，叠加 toc_overrides 覆盖层）。 */
export interface TocEditItem {
  id: string;
  title: string;
  sourceTitle?: string | null;
  level: number;
  page?: number | null;
  href?: string | null;
  /** original=产物条目 / added=用户新建。 */
  kind: 'original' | 'added' | string;
}

export interface GlossaryEntry {
  source: string;
  translation: string;
  category: string;
  occurrences: number;
  /** static=用户词库 / dynamic=翻译锁定 / user=术语页新增。 */
  origin?: string;
}

/** 用户术语条目（save_user_glossary 入参，runtime_config.static_glossary_entries 持久化）。 */
export interface UserGlossaryEntry {
  source: string;
  target: string;
  enforcement?: string;
  scope?: string;
  notes?: string;
}

/** 用户术语词表（多词表体系，user_glossaries.json 持久化）。 */
export interface GlossaryDeckView {
  id: string;
  name: string;
  enabled: boolean;
  createdAt: string;
  updatedAt: string;
  entryCount: number;
  entries: UserGlossaryEntry[];
}

/** 校对段对（list_review_segments）：源文 + 机翻 + 修订层 + 有效译文。 */
export interface ReviewOverride {
  blockId: string;
  editedTarget: string;
  comment?: string;
  status?: string;
  updatedAt?: string;
}

export interface ReviewSegment {
  blockId: string;
  order: number;
  source: string;
  target: string;
  chapter?: string | null;
  overrideEdited?: ReviewOverride | null;
  effectiveTarget: string;
}

/** 用户自建生词本词条。 */
export interface VocabNotebookEntry {
  word: string;
  definition: string;
  context: string;
  /** 用户单词笔记。 */
  note?: string;
  bookId: string;
  chapter: string;
  createdAt: string;
  repetitions: number;
  intervalDays: number;
  easeFactor: number;
  dueAt?: string | null;
}

export interface VocabNotebook {
  id: string;
  name: string;
  createdAt: string;
  updatedAt: string;
  entries: VocabNotebookEntry[];
}

/** 生成术语表的用户修订（旁路 overrides，不修改 artifact 本体）。 */
export interface GlossaryOverrideInput {
  source: string;
  translation: string;
  /** source 重命名来源（书目区英文原文可编辑）；普通修订省略。 */
  renameFrom?: string | null;
}

/** 号池视图（后端 list_route_pools 返回，key 只回掩码）。 */
export interface RoutePoolView {
  name: string;
  active: boolean;
  routes: PoolRouteView[];
}

export interface PoolRouteView {
  provider: string;
  apiUrl: string;
  model: string;
  weight: number;
  keyCount: number;
  keyMasks: string[];
}

/** 号池保存入参（save_route_pools）。 */
export interface RoutePoolInput {
  name: string;
  active: boolean;
  routes: PoolRouteInput[];
}

export interface PoolRouteInput {
  provider: string;
  apiUrl: string;
  model: string;
  weight: number;
  apiKeys: string[];
}

export interface AiChatTurn {
  role: 'user' | 'assistant';
  content: string;
}

export interface AiAssistResponse {
  answer: string;
  model: string;
}

export interface LearningItem {
  id: string;
  word: string;
  note: string;
  context: string;
  source: string;
  chapter: string;
  progress: number;
}

export interface AppSettings {
  runtimeConfig: RuntimeConfig;
  provider: ProviderId;
  librarySurfaceStyle: 'cards' | 'canvas';
  bilingualDefault: boolean;
  wordwiseDefault: boolean;
  paragraphDensity: 'compact' | 'standard' | 'relaxed';
  learningFeaturesEnabled: boolean;
  notesFeaturesEnabled: boolean;
  learningDefaultView: 'cards' | 'feed';
  notesDefaultView: 'cards' | 'feed';
  /** 强调色系（与明暗主题独立）；缺省 夕珊瑚。 */
  palette: Palette;
}

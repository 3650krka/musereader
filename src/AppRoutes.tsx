import type {
  AgentDefinition,
  AgentPromptPreview,
  AppSettings,
  Book,
  SkillLibraryEntry,
  Theme,
  TranslationTask,
} from './types';
import type { AppView } from './appShared';
import { LibraryView } from './components/library/LibraryView';
import { InsightsView } from './components/insights/InsightsView';
import { LearningView } from './components/learning/LearningView';
import { NotesView } from './components/notes/NotesView';
import { ReaderView } from './components/reader/ReaderView';
import { ReviewView } from './components/review/ReviewView';
import { SettingsPage } from './components/settings/SettingsPage';
import { GlossaryView } from './components/terms/GlossaryView';
import { TasksView } from './components/tasks/TasksView';

interface AppRoutesProps {
  currentView:
    | 'library'
    | 'reader'
    | 'insights'
    | 'learning'
    | 'notes'
    | 'review'
    | 'terms'
    | 'tasks'
    | 'settings';
  books: Book[];
  displayBooks: Book[];
  tasks: TranslationTask[];
  settings: AppSettings;
  theme: Theme;
  currentBook: Book | null;
  /** 打开书时携带的跳转锚点（笔记/书签跳书）。 */
  pendingBookAnchor: string | null;
  clearPendingBookAnchor: () => void;
  /** 学习视图聚焦书（书库右键词汇透析）。 */
  vocabBookId: string | null;
  clearVocabBookId: () => void;
  skillEntries: SkillLibraryEntry[];
  agentDefinitions: AgentDefinition[];
  translationPromptPreview: AgentPromptPreview | null;
  settingsSaveState: 'idle' | 'saving' | 'saved' | 'error';
  settingsSaveMessage: string;
  onOpenBook: (book: Book, anchorHref?: string | null) => void;
  onOpenReview: (book: Book) => void;
  onOpenVocab: (book: Book) => void;
  onTaskCreated: (task: TranslationTask) => void;
  onTaskUpdated: (task: TranslationTask) => void;
  onDeleteTask: (taskId: string) => void;
  onBackToLibrary: () => void;
  onNavigate: (view: AppView) => void;
  onThemeChange: (theme: Theme) => void;
  onSettingsChange: (settings: AppSettings) => void;
  onSkillEntrySave: (skillId: string, content: string) => Promise<void>;
  onEntriesChanged: () => void;
  onToggleFavorite: (bookId: string, favorite: boolean) => void;
  onSetTags: (bookId: string, tags: string[]) => void;
  onSetFolder: (bookId: string, folder: string) => void;
}

export function AppRoutes({
  currentView,
  books,
  displayBooks,
  tasks,
  settings,
  theme,
  currentBook,
  pendingBookAnchor,
  clearPendingBookAnchor,
  vocabBookId,
  clearVocabBookId,
  skillEntries,
  agentDefinitions,
  translationPromptPreview,
  settingsSaveState,
  settingsSaveMessage,
  onOpenBook,
  onOpenReview,
  onOpenVocab,
  onTaskCreated,
  onTaskUpdated,
  onDeleteTask,
  onBackToLibrary,
  onNavigate,
  onThemeChange,
  onSettingsChange,
  onSkillEntrySave,
  onEntriesChanged,
  onToggleFavorite,
  onSetTags,
  onSetFolder,
}: AppRoutesProps) {
  if (currentView === 'library') {
    return (
      <LibraryView
        books={displayBooks}
        tasks={tasks}
        onOpenBook={onOpenBook}
        onTaskCreated={onTaskCreated}
        onTaskUpdated={onTaskUpdated}
        onDeleteTask={onDeleteTask}
        surfaceStyle={settings.librarySurfaceStyle}
        skillEntries={skillEntries}
        onToggleFavorite={onToggleFavorite}
        onSetTags={onSetTags}
        onSetFolder={onSetFolder}
        onOpenReview={onOpenReview}
        onOpenVocab={onOpenVocab}
      />
    );
  }

  if (currentView === 'insights') return <InsightsView books={books} tasks={tasks} />;

  if (currentView === 'learning' && settings.learningFeaturesEnabled) {
    return <LearningView books={displayBooks} defaultView={settings.learningDefaultView} initialBookId={vocabBookId} onInitialBookConsumed={clearVocabBookId} />;
  }

  if (currentView === 'notes' && settings.notesFeaturesEnabled) {
    return <NotesView books={displayBooks} defaultView={settings.notesDefaultView} onOpenBook={onOpenBook} />;
  }

  if (currentView === 'terms') {
    return <GlossaryView books={displayBooks} />;
  }

  if (currentView === 'tasks') {
    return (
      <TasksView
        tasks={tasks}
        onDeleteTask={onDeleteTask}
        onOpenBook={onOpenBook}
        books={displayBooks}
        onNavigate={onNavigate}
      />
    );
  }

  if (currentView === 'review') {
    return <ReviewView tasks={tasks} books={displayBooks} initialTaskId={currentBook?.id ?? null} />;
  }

  if (currentView === 'settings') {
    return (
      <SettingsPage
        theme={theme}
        setTheme={onThemeChange}
        settings={settings}
        onSettingsChange={onSettingsChange}
        skillEntries={skillEntries}
        agentDefinitions={agentDefinitions}
        translationPromptPreview={translationPromptPreview}
        onSkillEntrySave={onSkillEntrySave}
        onEntriesChanged={onEntriesChanged}
        saveState={settingsSaveState}
        saveMessage={settingsSaveMessage}
      />
    );
  }

  if (currentView === 'reader' && currentBook) {
    return <ReaderView book={currentBook} onBack={onBackToLibrary} currentTheme={theme} setTheme={onThemeChange} pendingAnchor={pendingBookAnchor} onPendingAnchorConsumed={clearPendingBookAnchor} onOpenReview={() => onOpenReview(currentBook)} readingDefaults={{ bilingualDefault: settings.bilingualDefault, wordwiseDefault: settings.wordwiseDefault, paragraphDensity: settings.paragraphDensity }} />;
  }

  return null;
}

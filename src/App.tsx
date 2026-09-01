import { AppRoutes } from './AppRoutes';
import { ToastProvider } from './components/common/Toast';
import { AppDialogProvider } from './components/common/AppDialog';
import { AppShell } from './AppShell';
import { useAppController } from './hooks/useAppController';

export default function App() {
  const {
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
    pendingBookAnchor,
    clearPendingBookAnchor,
    vocabBookId,
    clearVocabBookId,
    openReview,
    openVocab,
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
  } = useAppController();

  const isReader = currentView === 'reader';
  const stageKey = `${currentView}-${lang}`;

  /* 侧栏/快捷键固定明暗两态直达（纸/墨蓝等四态选择器在 设置→外观）：
     旧四态轮切需连点三次才到暗色，用户感知为「按钮不生效」。 */
  const cycleTheme = () => {
    setTheme(theme === 'dark' ? 'light' : 'dark');
  };

  return (
    <ToastProvider>
      <AppDialogProvider>
      <AppShell
      currentView={currentView}
      theme={theme}
      palette={settings.palette}
      isReader={isReader}
      stageKey={stageKey}
      lang={lang}
      setLang={setLang}
      onNavigate={setCurrentView}
      onCycleTheme={cycleTheme}
    >
      <AppRoutes
        currentView={currentView}
        books={books}
        displayBooks={displayBooks}
        tasks={tasks}
        settings={settings}
        theme={theme}
        currentBook={currentBook}
        pendingBookAnchor={pendingBookAnchor}
        clearPendingBookAnchor={clearPendingBookAnchor}
        vocabBookId={vocabBookId}
        clearVocabBookId={clearVocabBookId}
        skillEntries={skillEntries}
        agentDefinitions={agentDefinitions}
        translationPromptPreview={translationPromptPreview}
        settingsSaveState={settingsSaveState}
        settingsSaveMessage={settingsSaveMessage}
        onOpenBook={openBook}
        onOpenReview={openReview}
        onOpenVocab={openVocab}
        onEntriesChanged={() => void reloadSkillEntries()}
        onTaskCreated={handleTaskCreated}
        onTaskUpdated={handleTaskUpdated}
        onDeleteTask={handleDeleteTask}
        onBackToLibrary={() => setCurrentView('library')}
        onNavigate={setCurrentView}
        onThemeChange={setTheme}
        onSettingsChange={handleSettingsChange}
        onSkillEntrySave={handleSkillEntrySave}
        onToggleFavorite={toggleBookFavorite}
        onSetTags={setBookTags}
        onSetFolder={setBookFolder}
      />
      </AppShell>
      </AppDialogProvider>
    </ToastProvider>
  );
}

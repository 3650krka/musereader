import coverOtherMothers from '../../prototype/assets/cover-other-mothers.jpg';
import type { Book, ReaderState, TranslationTask } from '@/types';

// 构建期门控（vite define）：默认发布构建为 false，整块演示数据（含真实译文
// 节选与本机路径）被 tree-shake 出 bundle；宣传截图/演示站用 `--mode preview`。
export const isWebPreview = __WEB_PREVIEW__;

const TASK_ID = 'gate-epub-full-book-env-001-the-other-mothers-002';
const SOURCE_EPUB = 'D:/tools/musetranslate/click/test_file/The_Other_Mothers_-_Katherine_Faulkner_US.epub';
const ARTIFACT_ROOT = `D:/tools/musetranslate/click/musetranslate/src-tauri/.runtime/artifacts/${TASK_ID}`;
const BILINGUAL_EPUB = `${ARTIFACT_ROOT}/packed14/book_bilingual.epub`;
const TRANSLATED_EPUB = `${ARTIFACT_ROOT}/packed14/book_zh.epub`;

const TOC = [
  { title: '封面', sourceTitle: 'Cover', level: 1, order: 0, progress: 0 },
  { title: '扉页', sourceTitle: 'Title Page', level: 1, order: 2, progress: 0 },
  { title: '献词', sourceTitle: 'Dedication', level: 1, order: 3, progress: 0 },
  { title: '第1章：塔什', sourceTitle: 'Chapter 1: Tash', level: 1, order: 4, progress: 0 },
  { title: '第二章：塔什', sourceTitle: 'Chapter 2: Tash', level: 1, order: 6, progress: 0 },
  { title: '第三章：塔什', sourceTitle: 'Chapter 3: Tash', level: 1, order: 7, progress: 0 },
];

const TRANSLATED_EXCERPT = `# 第1章：塔什

# 塔什

北康沃尔警察局

2019年4月

我们在一间没有窗户的房间里见面，这座小镇的房子都是碎石墙面。高街两旁是木板封起的店面和大大小小的博彩店。他们把我带到了内陆，我想是去了最近的警察局。这里没有海浪拍岸，没有鸟鸣，也没有从屋顶后探出的那一抹明快的蓝色条纹。

来的路上，在车里，汤姆和我和芬恩玩了个游戏：谁先看到海。其实是我先看到的，但我没吭声，好让芬恩赢。看到那条蓝宝石般的缎带横亘在地平线上，尽管发生了这一切，我的心还是为之一振——为了那个假期的许诺。那些在海滩上搭堡垒、筑城堡的日子。芬恩的小脚丫在海湾湿沙上踩出一个个完美的脚印。

我琢磨着他们会不会给我戴上手铐，但他们没有。车上的警官们几乎带着歉意似的。他们不停问我冷不冷，要不要开窗，要不要喝水。我摇摇头，努力把目光投向窗外的风景。`;

export const WEB_PREVIEW_BOOKS: Book[] = [
  {
    id: TASK_ID,
    title: 'The Other Mothers',
    author: 'Katherine Faulkner',
    cover: coverOtherMothers,
    progress: 0,
    lastRead: '',
    category: 'fiction',
    level: '原文',
    content: TRANSLATED_EXCERPT,
    originalPath: SOURCE_EPUB,
    primaryArtifactPath: BILINGUAL_EPUB,
    translatedEpubPath: TRANSLATED_EPUB,
    format: 'EPUB',
    tocCount: 103,
    toc: TOC,
    metaTitle: 'The Other Mothers',
    metaAuthor: 'Katherine Faulkner',
  },
];

export const WEB_PREVIEW_TASKS: TranslationTask[] = [
  {
    id: TASK_ID,
    filename: 'The_Other_Mothers_-_Katherine_Faulkner_US.epub',
    pdfPath: SOURCE_EPUB,
    status: 'completed',
    phase: 'completed',
    progress: 100,
    message: '翻译完成',
    articleType: 'fiction',
    concurrent: true,
    maxWorkers: 10,
    createdAt: '2026-07-31T08:17:34.805104300Z',
    updatedAt: '2026-08-02T03:27:22.670954100Z',
    outputPath: `${ARTIFACT_ROOT}/translated.md`,
    htmlOutputPath: `${ARTIFACT_ROOT}/translated.html`,
    coverPath: coverOtherMothers,
    artifactPaths: {
      artifactDir: ARTIFACT_ROOT,
      sourceMarkdownPath: `${ARTIFACT_ROOT}/source.md`,
      sourceTocPath: `${ARTIFACT_ROOT}/source_toc.json`,
      translatedTocPath: `${ARTIFACT_ROOT}/translated_toc.json`,
      outputMarkdownPath: `${ARTIFACT_ROOT}/translated.md`,
      outputHtmlPath: `${ARTIFACT_ROOT}/translated.html`,
      translatedEpubPath: TRANSLATED_EPUB,
      bilingualEpubPath: BILINGUAL_EPUB,
      checkpointPath: `${ARTIFACT_ROOT}/checkpoint.json`,
      manifestPath: `${ARTIFACT_ROOT}/manifest.json`,
      eventLogPath: `${ARTIFACT_ROOT}/events.ndjson`,
      validationReportPath: `${ARTIFACT_ROOT}/validation_report.json`,
      glossaryPath: `${ARTIFACT_ROOT}/glossary.json`,
    },
    totalChunks: 223,
    translatedChunks: 223,
    retryCount: 0,
    sourceHash: '04dbb8b1ffb14f71',
    lastError: null,
  },
];

// Web 预览没有持久化阅读行为；笔记、活动和生词保持为空。
export const WEB_PREVIEW_READER_STATES: ReaderState[] = [];

/**
 * 发布构建的 webPreviewFixtures 替身（vite alias 在非 --mode preview 时指向本文件）：
 * 演示夹具模块整体不进模块图 → 真实译文节选、本机路径、封面资产零 emit。
 * isWebPreview 恒为 false，消费侧的演示分支为死代码被消除。
 */
import type { Book, ReaderState, TranslationTask } from '@/types';

export const isWebPreview = false;
export const WEB_PREVIEW_BOOKS: Book[] = [];
export const WEB_PREVIEW_TASKS: TranslationTask[] = [];
export const WEB_PREVIEW_READER_STATES: ReaderState[] = [];

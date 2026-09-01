import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import type { Book, DocumentReaderPackage, TranslationTask } from '@/types';
import { buildDocumentReaderHtml } from './documentHtml';
import {
  injectReaderBase,
  loadReaderSectionDocument,
  type ReaderSectionDocument,
} from './epubDocument';
import type { ReaderLang } from './epubReaderTypes';

interface DocumentSourceState {
  document: ReaderSectionDocument | null;
  loading: boolean;
  error: string | null;
}

const IDLE_STATE: DocumentSourceState = {
  document: null,
  loading: false,
  error: null,
};

const IS_TAURI = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export function useDocumentReaderSource(
  book: Book,
  lang: ReaderLang,
  enabled: boolean,
): DocumentSourceState {
  const [state, setState] = useState<DocumentSourceState>(IDLE_STATE);

  useEffect(() => {
    if (!enabled) {
      setState(IDLE_STATE);
      return;
    }

    const controller = new AbortController();
    let disposed = false;
    setState({ document: null, loading: true, error: null });

    void resolveDocumentSource(book, lang, controller.signal)
      .then((document) => {
        if (!disposed) setState({ document, loading: false, error: null });
      })
      .catch((error: unknown) => {
        if (disposed || controller.signal.aborted) return;
        setState({
          document: null,
          loading: false,
          error: error instanceof Error ? error.message : String(error),
        });
      });

    return () => {
      disposed = true;
      controller.abort();
    };
  }, [
    book.content,
    book.id,
    book.metaTitle,
    book.segments,
    book.title,
    enabled,
    lang,
  ]);

  return state;
}

async function resolveDocumentSource(
  book: Book,
  lang: ReaderLang,
  signal: AbortSignal,
): Promise<ReaderSectionDocument> {
  const prepared = IS_TAURI
    ? await prepareDocumentPackage(book.id, signal)
    : null;
  if (lang === 'zh' && prepared) {
    try {
      return await loadReaderSectionDocument(
        prepared.htmlPath,
        `${book.id}:html:zh:${prepared.cacheKey}`,
        signal,
      );
    } catch (error) {
      if (signal.aborted) throw error;
      console.warn('Document HTML artifact could not be loaded; using safe Markdown rendering:', error);
    }
  }

  const task = IS_TAURI ? await loadTask(book.id) : null;
  const source = buildDocumentReaderHtml({
    title: book.metaTitle?.trim() || book.title,
    content: book.content ?? '',
    segments: book.segments ?? [],
    lang,
  });
  const fallbackPath = task?.artifactPaths.outputMarkdownPath ?? task?.outputPath;
  const basePath = prepared?.htmlPath ?? (!IS_TAURI ? fallbackPath : undefined);
  const assetUrl = basePath ? convertFileSrc(basePath) : '';
  const srcDoc = assetUrl
    ? injectReaderBase(source, assetUrl)
    : source;
  return {
    assetUrl,
    key: `${book.id}:generated:${lang}:${contentFingerprint(source)}`,
    srcDoc,
  };
}

async function prepareDocumentPackage(
  taskId: string,
  signal: AbortSignal,
): Promise<DocumentReaderPackage | null> {
  try {
    const prepared = await invoke<DocumentReaderPackage>('prepare_document_reader', { taskId });
    if (signal.aborted) throw new DOMException('Aborted', 'AbortError');
    return prepared;
  } catch (error) {
    if (signal.aborted) throw error;
    console.warn(`Document reader package unavailable for ${taskId}:`, error);
    return null;
  }
}

async function loadTask(taskId: string): Promise<TranslationTask | null> {
  try {
    return await invoke<TranslationTask>('get_task_status', { taskId });
  } catch (error) {
    console.warn(`Reader task metadata unavailable for ${taskId}:`, error);
    return null;
  }
}

function contentFingerprint(value: string): string {
  let hash = 2_166_136_261;
  for (let index = 0; index < value.length; index += 1) {
    hash = Math.imul(hash ^ value.charCodeAt(index), 16_777_619);
  }
  return (hash >>> 0).toString(36);
}

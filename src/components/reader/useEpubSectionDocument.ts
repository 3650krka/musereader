import { useEffect, useState } from 'react';
import type { EpubReaderSection } from '@/types';
import { loadReaderSectionDocument, type ReaderSectionDocument } from './epubDocument';

interface SectionDocumentState {
  document: ReaderSectionDocument | null;
  loading: boolean;
  error: string | null;
}

const IDLE_STATE: SectionDocumentState = {
  document: null,
  loading: false,
  error: null,
};

export function useEpubSectionDocument(
  section: EpubReaderSection | undefined,
  enabled: boolean,
): SectionDocumentState {
  const [state, setState] = useState<SectionDocumentState>(IDLE_STATE);
  const filePath = section?.filePath;
  const sectionKey = section ? `${section.index}:${section.href}` : '';

  useEffect(() => {
    if (!enabled || !filePath || !sectionKey) {
      setState(IDLE_STATE);
      return;
    }

    const controller = new AbortController();
    setState({ document: null, loading: true, error: null });

    void loadReaderSectionDocument(filePath, sectionKey, controller.signal)
      .then((document) => {
        setState({ document, loading: false, error: null });
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) return;
        setState({
          document: null,
          loading: false,
          error: error instanceof Error ? error.message : String(error),
        });
      });

    return () => controller.abort();
  }, [enabled, filePath, sectionKey]);

  return state;
}

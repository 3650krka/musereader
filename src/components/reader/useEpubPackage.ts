import { invoke } from '@tauri-apps/api/core';
import { useEffect, useMemo, useState } from 'react';
import type { EpubReaderPackage } from '@/types';

const IS_TAURI = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'object' && error !== null && 'message' in error) {
    return String(error.message);
  }
  return String(error);
}

export function useEpubPackage(bookId: string, enabled: boolean) {
  const [pkg, setPkg] = useState<EpubReaderPackage | null>(null);
  const [loading, setLoading] = useState(enabled);
  const [error, setError] = useState<string | null>(null);
  const [sectionIndex, setSectionIndex] = useState(0);

  useEffect(() => {
    setPkg(null);
    setSectionIndex(0);
    if (!enabled || !IS_TAURI) {
      setLoading(false);
      setError(null);
      return;
    }

    let mounted = true;
    setLoading(true);
    setError(null);
    void invoke<EpubReaderPackage>('prepare_epub_reader', { taskId: bookId })
      .then((result) => {
        if (!mounted) return;
        if (result.sections.length === 0) {
          throw new Error('EPUB 未包含可阅读章节');
        }
        setPkg(result);
      })
      .catch((reason: unknown) => {
        if (mounted) setError(errorMessage(reason));
      })
      .finally(() => {
        if (mounted) setLoading(false);
      });

    return () => {
      mounted = false;
    };
  }, [bookId, enabled]);

  const sections = useMemo(() => pkg?.sections ?? [], [pkg]);
  /* 目录树：书籍目录（NCX/nav）收录的 section 为一级条目；
     未收录的连续子文件（ch01_sub01 之类，head-title 兜底标题如 "1. Tash"）
     归并到其前一个目录条目之下——它们是同一章的内容分片，
     不应与章节并列成独立假目录项。
     兜底：全书无目录条目（纯 spine 书）时所有 section 平铺为一级条目。 */
  const toc = useMemo(() => {
    let depth = 0;
    let seenCovered = false;
    return sections.map((section) => {
      if (section.tocCovered) {
        seenCovered = true;
        depth = 0;
      } else if (seenCovered) {
        depth = 1;
      } else {
        depth = 0; // 目录条目出现前的前置页（封面等）保持一级
      }
      return {
        title: section.title,
        href: section.href,
        index: section.index,
        depth,
      };
    });
  }, [sections]);
  const currentSection = sections[sectionIndex];
  /* 章题对外展示用目录标题：归并子文件（ch01_sub01 之类）位于章节中段时，
     顶栏章题沿用其所属章节的目录标题（最近一个目录条目），
     与目录树的归并语义一致；目录条目自身/前置页照旧。 */
  const chapterTitle = useMemo(() => {
    const section = sections[sectionIndex];
    if (!section) return '';
    if (section.tocCovered) return section.title;
    let hasCoveredBefore = false;
    let parentTitle = '';
    for (let i = 0; i < sectionIndex; i += 1) {
      if (sections[i].tocCovered) {
        hasCoveredBefore = true;
        parentTitle = sections[i].title;
      }
    }
    return hasCoveredBefore ? parentTitle : section.title;
  }, [sections, sectionIndex]);

  return {
    pkg,
    loading,
    error,
    sections,
    toc,
    sectionIndex,
    setSectionIndex,
    currentSection,
    chapterTitle,
  };
}

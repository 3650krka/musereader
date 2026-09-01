import { invoke } from '@tauri-apps/api/core';
import { motion } from 'motion/react';
import { useEffect, useMemo, useState } from 'react';
import { useT } from '@/i18n';
import { isWebPreview, WEB_PREVIEW_READER_STATES } from '@/hooks/webPreviewFixtures';
import type { Book, ReaderState, TranslationTask } from '@/types';
import { bucketVocabWords, buildBookMinutes, buildInsightsSummary, buildReviewData } from './insightsShared';
import { ActivitySplit, BookProgressBars, MinutesBars, MinutesTrend, QualityBars, VocabDonut, YearHeat } from './LieflatCharts';

interface InsightsViewProps {
  books: Book[];
  tasks: TranslationTask[];
}

export function InsightsView({ books, tasks }: InsightsViewProps) {
  const t = useT();
  const [readerStates, setReaderStates] = useState<ReaderState[]>([]);
  const [range, setRange] = useState<7 | 30>(7);
  const [dailyGoalMinutes, setDailyGoalMinutes] = useState(() => {
    // 防护：localStorage 异常/脏值（NaN/0/负数）回退 30
    try {
      const raw = Number(localStorage.getItem('mt_daily_goal_mins') ?? 30);
      return Number.isFinite(raw) && raw > 0 ? raw : 30;
    } catch {
      return 30;
    }
  });

  const saveDailyGoal = (mins: number) => {
    setDailyGoalMinutes(mins);
    try {
      localStorage.setItem('mt_daily_goal_mins', String(mins));
    } catch {
      /* 存储不可用：仅本次会话生效 */
    }
  };

  useEffect(() => {
    if (isWebPreview) {
      setReaderStates(WEB_PREVIEW_READER_STATES);
      return;
    }
    let mounted = true;
    const load = async () => {
      try {
        const states = await invoke<ReaderState[]>('list_reader_states');
        if (mounted) setReaderStates(states);
      } catch (error) {
        console.error('Failed to load reader states:', error);
      }
    };

    void load();
    return () => {
      mounted = false;
    };
  }, []);

  const summary = useMemo(() => buildInsightsSummary(books, readerStates, tasks, range), [books, readerStates, tasks, range]);
  const vocabWords = useMemo(
    () => readerStates.flatMap((s) => [...Object.keys(s.wordMarks), ...Object.keys(s.vocabCards)]),
    [readerStates],
  );
  const vocabBuckets = useMemo(() => bucketVocabWords([...new Set(vocabWords)]), [vocabWords]);
  const vocabTotal = useMemo(() => vocabBuckets.reduce((s, b) => s + b.count, 0), [vocabBuckets]);
  const activities = useMemo(() => readerStates.flatMap((state) => state.activities), [readerStates]);
  /* 复习统计（reviewLog 90 天窗口）：趋势图 + 记忆率（quality>=3 占比） */
  const reviewLog = useMemo(
    () => readerStates.flatMap((state) => state.reviewLog ?? []),
    [readerStates],
  );
  const reviewData = useMemo(() => buildReviewData(reviewLog, range), [reviewLog, range]);
  const reviewTotal = useMemo(() => reviewData.reduce((sum, item) => sum + item.minutes, 0), [reviewData]);
  /* 每书时长（全历史，不限窗口） */
  const bookMinutes = useMemo(() => buildBookMinutes(readerStates), [readerStates]);
  const bookTitleOf = (bookId: string): string =>
    books.find((book) => book.id === bookId)?.title ?? bookId;

  const retention = useMemo(() => {
    if (reviewLog.length === 0) return 0;
    const kept = reviewLog.filter((entry) => entry.quality >= 3).length;
    return Math.round((kept / reviewLog.length) * 100);
  }, [reviewLog]);

  return (
    <motion.div
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.36, ease: [0.22, 1, 0.36, 1] }}
      className="page-stage"
    >
      <header className="page-head">
        <h1 className="page-title">{t.insights.title}</h1>
        <div className="actions">
          <button type="button" className={`chip ${range === 7 ? 'on' : ''}`} onClick={() => setRange(7)}>{t.insights.week}</button>
          <button type="button" className={`chip ${range === 30 ? 'on' : ''}`} onClick={() => setRange(30)}>{t.insights.month}</button>
        </div>
      </header>

      {/* 每日阅读目标追踪（移除纯文字建议卡，图表化主导） */}
      <div className="grid-2" style={{ marginBottom: 18 }}>
        <div className="card" style={{ padding: 18, display: 'flex', flexDirection: 'column', gap: 12 }}>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontWeight: 600, fontSize: 14 }}>
              <span>{t.insights.dailyGoalTitle}</span>
            </div>
            <div style={{ display: 'flex', gap: 6 }}>
              {[15, 30, 45, 60].map((m) => (
                <button
                  key={m}
                  type="button"
                  className={`chip sm ${dailyGoalMinutes === m ? 'on' : ''}`}
                  onClick={() => saveDailyGoal(m)}
                >
                  {m} 分钟
                </button>
              ))}
            </div>
          </div>
          <div>
            <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 12.5, marginBottom: 6 }}>
              <span className="text-muted">今日已读: <b>{summary.readingData[summary.readingData.length - 1]?.minutes ?? 0}</b> 分钟</span>
              <span style={{ fontWeight: 700, color: (summary.readingData[summary.readingData.length - 1]?.minutes ?? 0) >= dailyGoalMinutes ? 'var(--ok, #3F7D54)' : 'var(--ink)' }}>
                {Math.min(100, Math.round(((summary.readingData[summary.readingData.length - 1]?.minutes ?? 0) / Math.max(1, dailyGoalMinutes)) * 100))}%
              </span>
            </div>
            <div style={{ height: 8, background: 'var(--paper-2)', borderRadius: 99, overflow: 'hidden' }}>
              <div
                style={{
                  height: '100%',
                  borderRadius: 99,
                  background: (summary.readingData[summary.readingData.length - 1]?.minutes ?? 0) >= dailyGoalMinutes ? 'var(--ok, #3F7D54)' : 'var(--accent)',
                  width: `${Math.min(100, Math.round(((summary.readingData[summary.readingData.length - 1]?.minutes ?? 0) / Math.max(1, dailyGoalMinutes)) * 100))}%`,
                  transition: 'width 0.4s ease',
                }}
              />
            </div>
          </div>
        </div>


      </div>



      <div className="card chart-card" style={{ marginBottom: 18 }}>
        <h2>{t.insights.activitySplit}</h2>
        <p className="legend">{t.insights.activitySplitL}</p>
        <ActivitySplit items={[
          { label: t.insights.actReading, minutes: summary.totalMinutes, color: 'var(--accent)' },
          { label: t.insights.actReview, minutes: reviewTotal, color: 'var(--ok, #3F7D54)' },
          { label: t.insights.completedTranslations, minutes: summary.completedTranslations * 15, color: 'var(--signal)' },
        ]} />
      </div>

      <div className="grid-2">
        <div className="card chart-card">
          <h2>{range === 7 ? t.insights.c1t : t.insights.c1tm}</h2>
          <p className="legend">{t.insights.c1l} · <b>{summary.readingData[0]?.date} – {summary.readingData[summary.readingData.length - 1]?.date}</b> · {t.insights.streak} {summary.streak}{t.insights.days} · {t.insights.minutes} {summary.totalMinutes}{t.insights.min}</p>
          {summary.totalMinutes > 0 ? (
            <>
              {range === 7 ? <MinutesBars data={summary.readingData} /> : <MinutesTrend data={summary.readingData} />}
              <p className="src">{t.insights.c1s}</p>
            </>
          ) : (
            <p className="empty-hint">{t.insights.noReading}</p>
          )}
        </div>

        {reviewLog.length > 0 && (
          <div className="card chart-card">
            <h2>{t.insights.c4t}</h2>
            <p className="legend">{t.insights.c4l} · <b>{retention}% {t.insights.retention}</b></p>
            {reviewTotal > 0 ? (
              <>
                {range === 7 ? <MinutesBars data={reviewData} /> : <MinutesTrend data={reviewData} />}
                <p className="src">{t.insights.c4s}</p>
              </>
            ) : (
              <p className="empty-hint">{t.insights.noReading}</p>
            )}
          </div>
        )}

        {bookMinutes.length > 0 && (
          <div className="card chart-card">
            <h2>{t.insights.c5t}</h2>
            <div className="ins2-cats">
              {bookMinutes.slice(0, 6).map((item) => {
                const max = bookMinutes[0]?.minutes ?? 1;
                return (
                  <div key={item.bookId} className="ins2-cat">
                    <div className="ins2-cat-head">
                      <span className="truncate">{bookTitleOf(item.bookId)}</span>
                      <span>{item.minutes} {t.insights.min}</span>
                    </div>
                    <div className="ins2-track">
                      <i style={{ width: `${Math.max(6, Math.round((item.minutes / max) * 100))}%` }} />
                    </div>
                  </div>
                );
              })}
            </div>
            <p className="src">{t.insights.c5s}</p>
          </div>
        )}

        {vocabTotal > 0 && (
          <div className="card chart-card">
            <h2>{t.insights.c2t}</h2>
            <p className="legend">{vocabTotal} {t.insights.wordsUnit} · <b>{t.insights.c2l}</b></p>
            <VocabDonut buckets={vocabBuckets} total={vocabTotal} />
            <p className="src">{t.insights.c2s}</p>
          </div>
        )}
      </div>

      <div className="card chart-card" style={{ marginTop: 18, marginBottom: 18 }}>
        <h2>{t.insights.c3t}</h2>
        <p className="legend">{t.insights.c3l}</p>
        {activities.length > 0 ? (
          <>
            <YearHeat activities={activities} />
            <p className="src">{t.insights.c3s}</p>
          </>
        ) : (
          <p className="empty-hint">{t.insights.noReading}</p>
        )}
      </div>

      <div className="grid-2" style={{ marginBottom: 18 }}>
        <div className="card chart-card">
          <h2>{t.insights.c6t}</h2>
          {books.length > 0 ? (
            <BookProgressBars items={books.map((b) => ({ title: b.title, progress: b.progress }))} />
          ) : (
            <p className="empty-hint">{t.insights.noReading}</p>
          )}
        </div>
        <div className="card chart-card">
          <h2>{t.insights.c7t}</h2>
          {reviewLog.length > 0 ? (
            <QualityBars distribution={[0, 1, 2, 3, 4, 5].map((q) => ({ quality: q, count: reviewLog.filter((e) => e.quality === q).length }))} />
          ) : (
            <p className="empty-hint">{t.insights.noReading}</p>
          )}
        </div>
      </div>
    </motion.div>
  );
}


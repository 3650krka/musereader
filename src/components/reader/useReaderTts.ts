import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useRef, useState } from 'react';

export interface TtsVoice {
  id: string;
  name: string;
  locale: string;
  gender: string;
}

interface TtsSynthesizeResult {
  audioBase64: string;
  contentType: string;
  chunks: number;
}

const DEFAULT_VOICE_ZH = 'zh-CN-XiaoxiaoNeural';
const DEFAULT_VOICE_EN = 'en-US-JennyNeural';

export function useReaderTts() {
  const [voices, setVoices] = useState<TtsVoice[]>([]);
  const [speaking, setSpeaking] = useState(false);
  const [loading, setLoading] = useState(false);
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const voiceCacheRef = useRef<TtsVoice[]>([]);
  /* 请求 id 竞态防护：连续点击/切换朗读目标或中途 stop 时，
     旧一次 synthesize 的异步返回直接丢弃（不会把过期音频播出来）。
     speaking/loading 镜像用 ref：speak 内的守卫判断不受闭包过期影响。 */
  const sessionIdRef = useRef(0);
  const speakingRef = useRef(false);
  const loadingRef = useRef(false);

  useEffect(() => {
    if (voiceCacheRef.current.length > 0) return;
    invoke<TtsVoice[]>('list_tts_voices')
      .then((list) => {
        voiceCacheRef.current = list;
        setVoices(list);
      })
      .catch((error) => console.error('Failed to list TTS voices:', error));
  }, []);

  const stop = useCallback(() => {
    sessionIdRef.current += 1; // 在途合成全部作废
    if (audioRef.current) {
      audioRef.current.pause();
      audioRef.current.currentTime = 0;
      audioRef.current = null;
    }
    speakingRef.current = false;
    loadingRef.current = false;
    setSpeaking(false);
    setLoading(false);
  }, []);

  const speak = useCallback(
    async (text: string, voiceId?: string) => {
      if (speakingRef.current || loadingRef.current) {
        stop();
        return;
      }
      const mySession = ++sessionIdRef.current;
      const voice = voiceId ?? pickDefaultVoice(text, voices);
      loadingRef.current = true;
      setLoading(true);
      try {
        const result = await invoke<TtsSynthesizeResult>('synthesize_tts', {
          text,
          voice,
          rate: null,
        });
        if (sessionIdRef.current !== mySession) return; // 已被新请求/停止取代
        const audio = new Audio(`data:${result.contentType};base64,${result.audioBase64}`);
        audioRef.current = audio;
        audio.onended = () => {
          if (sessionIdRef.current !== mySession) return;
          speakingRef.current = false;
          setSpeaking(false);
        };
        audio.onerror = () => {
          if (sessionIdRef.current !== mySession) return;
          speakingRef.current = false;
          setSpeaking(false);
        };
        speakingRef.current = true;
        setSpeaking(true);
        await audio.play();
      } catch (error) {
        if (sessionIdRef.current !== mySession) return;
        console.error('TTS synthesize failed:', error);
        speakingRef.current = false;
        setSpeaking(false);
      } finally {
        if (sessionIdRef.current === mySession) {
          loadingRef.current = false;
          setLoading(false);
        }
      }
    },
    [voices, stop],
  );

  useEffect(() => stop, [stop]);

  return { voices, speaking, loading, speak, stop };
}

function pickDefaultVoice(text: string, voices: TtsVoice[]): string {
  const hasCjk = /[一-鿿]/.test(text);
  const preferred = hasCjk ? DEFAULT_VOICE_ZH : DEFAULT_VOICE_EN;
  if (voices.some((voice) => voice.id === preferred)) return preferred;
  const localePrefix = hasCjk ? 'zh-' : 'en-';
  const fallback = voices.find((voice) => voice.locale.startsWith(localePrefix));
  return fallback?.id ?? preferred;
}

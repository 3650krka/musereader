import { motion } from 'motion/react';
import { Coffee, Moon, PanelsTopLeft, RectangleHorizontal, ScanLine, Sun, SwatchBook } from 'lucide-react';
import { useT } from '@/i18n';
import type { Palette, Theme } from '@/types';
import type { AppearancePanelProps } from './panelTypes';

const THEME_ICONS = { light: Sun, paper: PanelsTopLeft, sepia: Coffee, dark: Moon } as const;

/* 色板预览色：与 index.css 色系 token（--sc-a → --sc-b 渐变对）同源，
   仅作选择器色块展示，不随当前选中色系联动。 */
const PALETTES: Array<{ id: Palette; from: string; to: string }> = [
  { id: 'coral', from: '#FF0844', to: '#FFB199' },
  { id: 'jade', from: '#16A06B', to: '#7FDEC0' },
  { id: 'indigo', from: '#3D55D8', to: '#9DB4FF' },
  { id: 'gold', from: '#DFA22F', to: '#FFE9A6' },
  { id: 'violet', from: '#8C52D9', to: '#D6A9FF' },
];

export function AppearanceSettingsPanel({
  theme,
  setTheme,
  settings,
  updateSetting,
}: Pick<AppearancePanelProps, 'theme' | 'setTheme' | 'settings' | 'updateSetting'>) {
  const t = useT();
  const themes: Array<{ id: Theme; label: string; note: string }> = [
    { id: 'light', label: t.set.themeLight, note: t.set.themeLightNote },
    { id: 'paper', label: t.set.themePaper, note: t.set.themePaperNote },
    { id: 'sepia', label: t.set.themeSepia, note: t.set.themeSepiaNote },
    { id: 'dark', label: t.set.themeDark, note: t.set.themeDarkNote },
  ];
  const paletteLabels: Record<Palette, string> = {
    coral: t.set.paletteCoral,
    jade: t.set.paletteJade,
    indigo: t.set.paletteIndigo,
    gold: t.set.paletteGold,
    violet: t.set.paletteViolet,
  };

  return (
    <motion.div initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ duration: 0.2 }} style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      {themes.map((item) => {
        const Icon = THEME_ICONS[item.id];
        const active = theme === item.id;
        return (
          <button key={item.id} type="button" className="row2 theme2" onClick={() => setTheme(item.id)} aria-pressed={active}>
            <div className="grow">
              <div className="name"><Icon size={15} style={{ color: 'var(--ink-3)' }} />{item.label}</div>
              <div className="desc">{item.note}</div>
            </div>
            <span className={`chip ${active ? 'on' : ''}`}>{active ? t.set.cur : t.set.switchTo}</span>
          </button>
        );
      })}

      <div className="row2">
        <div className="grow">
          <div className="name"><SwatchBook size={15} style={{ color: 'var(--ink-3)' }} />{t.set.paletteSection}</div>
          <div className="desc">{t.set.paletteSectionNote}</div>
        </div>
        <div className="palette-picker">
          {PALETTES.map((item) => {
            const active = settings.palette === item.id;
            return (
              <button
                key={item.id}
                type="button"
                className={`palette-dot ${active ? 'on' : ''}`}
                style={{ background: `linear-gradient(135deg, ${item.from}, ${item.to})` }}
                title={paletteLabels[item.id]}
                aria-label={paletteLabels[item.id]}
                aria-pressed={active}
                onClick={() => updateSetting('palette', item.id)}
              />
            );
          })}
        </div>
      </div>

      <div className="row2">
        <div className="grow">
          <div className="name"><RectangleHorizontal size={15} style={{ color: 'var(--ink-3)' }} />{t.set.homeLayout}</div>
          <div className="desc">{t.set.homeLayoutNote}</div>
        </div>
        <div style={{ display: 'flex', gap: 8 }}>
          <button
            type="button"
            className={`chip ${settings.librarySurfaceStyle === 'cards' ? 'on' : ''}`}
            onClick={() => updateSetting('librarySurfaceStyle', 'cards')}
            aria-pressed={settings.librarySurfaceStyle === 'cards'}
          >
            <RectangleHorizontal size={13} />
            {t.set.blockMode}
          </button>
          <button
            type="button"
            className={`chip ${settings.librarySurfaceStyle === 'canvas' ? 'on' : ''}`}
            onClick={() => updateSetting('librarySurfaceStyle', 'canvas')}
            aria-pressed={settings.librarySurfaceStyle === 'canvas'}
          >
            <ScanLine size={13} />
            {t.set.canvasMode}
          </button>
        </div>
      </div>
    </motion.div>
  );
}

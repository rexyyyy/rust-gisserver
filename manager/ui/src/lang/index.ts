// Vue I18n setup. Message catalogs live in ./messages.ts.
import { createI18n } from 'vue-i18n';

import { LanguageEnum } from '@/enums/LanguageEnum';
import { messages } from './messages';

/** One-time default switch to English for existing browsers that persisted the
 * old Chinese default. Runs once; later user selections are never overwritten. */
const DEFAULT_EN_MIGRATION_KEY = 'language_default_en_v1';
try {
  if (!localStorage.getItem(DEFAULT_EN_MIGRATION_KEY)) {
    localStorage.setItem('language', LanguageEnum.en_US);
    localStorage.setItem(DEFAULT_EN_MIGRATION_KEY, '1');
  }
} catch {
  // localStorage may be unavailable (private mode); fall back to in-memory default.
}

/** Resolve the persisted UI language. Default is English; the encoding region
 * is a separate server-side setting (`GIS_ENCODING_REGION`) and is not derived
 * from this value. */
export const getLanguage = (): LanguageEnum => {
  const language = useStorage<LanguageEnum>('language', LanguageEnum.en_US);
  return language.value || LanguageEnum.en_US;
};

const i18n = createI18n({
  globalInjection: true,
  allowComposition: true,
  legacy: false,
  locale: getLanguage(),
  fallbackLocale: 'zh_CN',
  messages
});

export default i18n;

export type LanguageType = typeof messages.zh_CN;

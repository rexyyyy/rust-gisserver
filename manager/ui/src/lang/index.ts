// Vue I18n setup. Message catalogs live in ./messages.ts.
import { createI18n } from 'vue-i18n';

import { LanguageEnum } from '@/enums/LanguageEnum';
import { messages } from './messages';

/** Resolve the persisted UI language, defaulting to Simplified Chinese. */
export const getLanguage = (): LanguageEnum => {
  const language = useStorage<LanguageEnum>('language', LanguageEnum.zh_CN);
  return language.value || LanguageEnum.zh_CN;
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

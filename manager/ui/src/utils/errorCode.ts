import i18n from '@/lang';

// Resolve messages at call time so a language switch takes effect immediately.
export const getErrorCode = (): Record<string, string> => ({
  '401': i18n.global.t('misc.auto.failedSystem'),
  '403': i18n.global.t('misc.auto.actionspermission'),
  '404': i18n.global.t('misc.auto.text17'),
  default: i18n.global.t('misc.auto.system2')
});

export default getErrorCode;

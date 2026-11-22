// Aggregated bilingual message catalogs.
// Keep this file free of runtime side effects so unit tests can import it directly.
import legacyZh from './zh_CN';
import legacyEn from './en_US';
import commonZh from './modules/common.zh_CN';
import commonEn from './modules/common.en_US';
import layoutZh from './modules/layout.zh_CN';
import layoutEn from './modules/layout.en_US';
import mapZh from './modules/map.zh_CN';
import mapEn from './modules/map.en_US';
import systemZh from './modules/system.zh_CN';
import systemEn from './modules/system.en_US';
import monitorZh from './modules/monitor.zh_CN';
import monitorEn from './modules/monitor.en_US';
import miscZh from './modules/misc.zh_CN';
import miscEn from './modules/misc.en_US';

export const messages = {
  zh_CN: {
    ...legacyZh,
    common: commonZh,
    layout: layoutZh,
    map: mapZh,
    system: systemZh,
    monitor: monitorZh,
    misc: miscZh
  },
  en_US: {
    ...legacyEn,
    common: commonEn,
    layout: layoutEn,
    map: mapEn,
    system: systemEn,
    monitor: monitorEn,
    misc: miscEn
  }
};

export default messages;

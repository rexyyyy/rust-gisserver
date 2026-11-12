import defaultSettings from '@/settings';
import { GS_THEME, THEME_VERSION, LEGACY_THEME_MAP } from './gsTheme';

// 配色真源统一放在 ./gsTheme，此处转出，业务代码统一从 '@/utils/theme' 引用即可
export { GS_THEME, THEME_VERSION } from './gsTheme';

/**
 * 是否处于暗黑模式（不引第三方依赖，便于在任意环境复用）
 */
export const isDarkMode = (): boolean => {
  const stored = typeof localStorage !== 'undefined' ? localStorage.getItem('useDarkKey') : null;
  if (stored === 'dark' || stored === 'light') return stored === 'dark';
  return typeof document !== 'undefined' && document.documentElement.classList.contains('dark');
};

// 计算颜色亮度
const getLuminance = (hex: string): number => {
  const rgb = hexToRgb(hex);
  if (rgb.length < 3) return 0;
  const [r, g, b] = rgb.map((c) => {
    const v = Number(c) / 255;
    return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};

// 处理主题样式
export const handleThemeStyle = (theme: string) => {
  document.documentElement.style.setProperty('--el-color-primary', theme);
  for (let i = 1; i <= 9; i++) {
    document.documentElement.style.setProperty(`--el-color-primary-light-${i}`, `${getLightColor(theme, i / 10)}`);
  }
  for (let i = 1; i <= 9; i++) {
    document.documentElement.style.setProperty(`--el-color-primary-dark-${i}`, `${getDarkColor(theme, i / 10)}`);
  }
  applyButtonAccent(theme);
  updateSidebarTheme(theme);
};

/**
 * 主按钮配色：默认 GeoServer 主题用「浅绿底 + 深绿字」的经典按钮，
 * 切换到其它主题色时按钮跟随主题色，保证同一套主题下观感一致。
 */
const buttonAccentPresets: Record<string, Record<string, string>> = {
  [GS_THEME.primary]: {
    '--gs-btn-bg': GS_THEME.accent,
    '--gs-btn-border': GS_THEME.accentBorder,
    '--gs-btn-text': GS_THEME.accentText,
    '--gs-btn-hover-bg': GS_THEME.accentHover,
    '--gs-btn-hover-border': GS_THEME.accentBorder,
    '--gs-btn-hover-text': GS_THEME.accentText,
    '--gs-btn-active-bg': GS_THEME.accentActive,
    '--gs-btn-active-border': GS_THEME.accentBorder,
    '--gs-btn-plain-bg': GS_THEME.accentFaint,
    '--gs-btn-plain-border': GS_THEME.accent,
    '--gs-btn-plain-text': GS_THEME.accentText
  }
};

const buttonAccentVars = Object.keys(buttonAccentPresets[GS_THEME.primary]);

const applyButtonAccent = (theme: string) => {
  const root = document.documentElement.style;
  const preset = buttonAccentPresets[theme];
  if (preset) {
    Object.entries(preset).forEach(([key, value]) => root.setProperty(key, value));
    return;
  }
  buttonAccentVars.forEach((key) => root.removeProperty(key));
};

// 各主题的侧边栏预设颜色
const sidebarPresets: Record<string, Record<string, string>> = {
  '#409EFF': {
    menuBg: '#1f2d3d', menuColor: '#bfcbd9', menuActiveText: '#f4f4f5',
    menuHover: '#263445', subMenuBg: '#1f2d3d', subMenuActiveText: '#f4f4f5',
    subMenuHover: '#001528', subMenuTitleHover: '#293444',
    menuGroupTitle: '#4ba3c4', menuDivider: '#2b3a4a'
  },
  '#009688': {
    menuBg: '#1f2d3d', menuColor: '#bfcbd9', menuActiveText: '#f4f4f5',
    menuHover: '#263445', subMenuBg: '#1f2d3d', subMenuActiveText: '#f4f4f5',
    subMenuHover: '#001528', subMenuTitleHover: '#293444',
    menuGroupTitle: '#4ba3c4', menuDivider: '#2b3a4a'
  },
  '#1b2a47': {
    menuBg: '#1b2a47', menuColor: '#ffffff', menuActiveText: '#ffffff',
    menuHover: '#263850', subMenuBg: '#1b2a47', subMenuActiveText: '#ffffff',
    subMenuHover: '#263850', subMenuTitleHover: '#263850',
    menuGroupTitle: '#7cc3e0', menuDivider: '#2f415f'
  },
  '#646cff': {
    menuBg: '#ffffff', menuColor: '#333333', menuActiveText: '#333333',
    menuHover: '#f0f1f5', subMenuBg: '#fafafa', subMenuActiveText: '#333333',
    subMenuHover: '#f0f1f5', subMenuTitleHover: '#f0f1f5',
    menuGroupTitle: '#0082b6', menuDivider: '#dadada'
  },
  // GeoServer 经典主题：白底 + 青蓝链接 + 蓝灰分隔线
  [GS_THEME.primary]: {
    menuBg: GS_THEME.sidebarBg, menuColor: GS_THEME.link, menuActiveText: GS_THEME.primary,
    menuHover: GS_THEME.sidebarHover, subMenuBg: GS_THEME.sidebarBg, subMenuActiveText: GS_THEME.primary,
    subMenuHover: GS_THEME.sidebarHover, subMenuTitleHover: GS_THEME.sidebarHover,
    menuGroupTitle: GS_THEME.groupTitle, menuDivider: GS_THEME.divider
  },
  // 兼容上一版默认色（旧值直接映射到 GeoServer 主题，避免历史用户停留在旧皮肤）
  '#1D9CC9': {
    menuBg: GS_THEME.sidebarBg, menuColor: GS_THEME.link, menuActiveText: GS_THEME.primary,
    menuHover: GS_THEME.sidebarHover, subMenuBg: GS_THEME.sidebarBg, subMenuActiveText: GS_THEME.primary,
    subMenuHover: GS_THEME.sidebarHover, subMenuTitleHover: GS_THEME.sidebarHover,
    menuGroupTitle: GS_THEME.groupTitle, menuDivider: GS_THEME.divider
  }
};

// 菜单变量清单（用于清理内联值，回落到 variables.module.scss 的默认值）
const sidebarVars = [
  'menuBg', 'menuColor', 'menuActiveText', 'menuHover',
  'subMenuBg', 'subMenuActiveText', 'subMenuHover', 'subMenuTitleHover',
  'menuGroupTitle', 'menuDivider'
];

/**
 * 清除内联侧边栏变量，交还给 CSS 默认值
 * （html.dark 下由 variables.module.scss 提供暗色菜单配色）
 */
export const clearSidebarTheme = () => {
  const root = document.documentElement.style;
  sidebarVars.forEach((key) => root.removeProperty('--' + key));
};

// 根据主题色更新侧边栏CSS变量
const updateSidebarTheme = (bgColor: string) => {
  // 暗黑模式下侧边栏配色由 html.dark 定义，不覆盖
  if (isDarkMode()) {
    clearSidebarTheme();
    return;
  }
  const root = document.documentElement.style;
  const preset = sidebarPresets[bgColor];
  if (preset) {
    Object.entries(preset).forEach(([key, value]) => root.setProperty('--' + key, value));
    return;
  }
  // 无预设时，根据亮度计算（自由取色回退）
  const isDark = getLuminance(bgColor) <= 0.179;
  const textColor = isDark ? '#ffffff' : '#333333';
  const hoverBg = isDark ? getLightColor(bgColor, 0.15) : getDarkColor(bgColor, 0.08);
  const subBg = isDark ? getDarkColor(bgColor, 0.15) : getLightColor(bgColor, 0.05);
  const subHover = isDark ? getLightColor(bgColor, 0.1) : getDarkColor(bgColor, 0.05);
  root.setProperty('--menuBg', bgColor);
  root.setProperty('--menuColor', textColor);
  root.setProperty('--menuActiveText', isDark ? '#ffffff' : '#333333');
  root.setProperty('--menuHover', hoverBg);
  root.setProperty('--subMenuBg', subBg);
  root.setProperty('--subMenuActiveText', isDark ? '#ffffff' : '#333333');
  root.setProperty('--subMenuHover', subHover);
  root.setProperty('--subMenuTitleHover', hoverBg);
  root.setProperty('--menuGroupTitle', isDark ? '#7cc3e0' : GS_THEME.groupTitle);
  root.setProperty('--menuDivider', isDark ? '#2b3a4a' : GS_THEME.divider);
};

// 页面圆角
export const applyRadiusBase = (val: number) => {
  const el = document.documentElement;
  el.style.setProperty('--app-radius-base', `${val}px`);
  el.style.setProperty('--app-radius-sm', `${Math.round(val * 0.6)}px`);
  el.style.setProperty('--app-radius-md', `${val}px`);
  el.style.setProperty('--app-radius-lg', `${Math.round(val * 1.4)}px`);
  el.style.setProperty('--el-border-radius-base', `${val}px`);
  el.style.setProperty('--el-border-radius-small', `${Math.round(val * 0.6)}px`);
};

// 默认亮色落位标记：主题功能升级后首次加载时，把历史遗留（跟随系统 auto / 旧值）的暗色状态归位为亮色
const THEME_DEFAULT_FLAG = 'theme-default-light-v1';

// 主题版本标记：用于把 localStorage 中上一版主题的陈旧配置迁移到当前默认主题
const THEME_VERSION_KEY = 'app-theme-version';

/**
 * 亮暗模式默认值归一（本项目默认「亮色」）
 * - 第一次进入本版主题功能：一律重置为亮色（清掉历史遗留的 auto/dark）；
 * - 之后：只有本地显式存过 'dark' 才保持暗色，'auto' / 未设置 / 其它值一律归一为 'light'，
 *   避免 vueuse useDark 读到 'auto' 后跟随系统（Windows 深色）偏好而默认变黑；
 * - 想「每次刷新都强制亮色、完全忽略本地选择」时，把下面 const dark 直接改成 false。
 * 需要在 Vue 挂载前调用一次，保证组件内 useDark(useStorage) 读到的是 light。
 */
export const initDefaultColorMode = (): boolean => {
  try {
    const ls = typeof localStorage !== 'undefined' ? localStorage : undefined;
    if (!ls) return false;
    const migrated = ls.getItem(THEME_DEFAULT_FLAG) === '1';
    const stored = ls.getItem('useDarkKey');
    const dark = migrated ? stored === 'dark' : false;
    ls.setItem(THEME_DEFAULT_FLAG, '1');
    ls.setItem('useDarkKey', dark ? 'dark' : 'light');
    document.documentElement.classList.toggle('dark', dark);
    return dark;
  } catch (e) {
    return false;
  }
};

/**
 * 主题版本迁移：把浏览器 localStorage 里「上一版主题」的陈旧配置纠正到当前默认主题。
 *
 * 背景：布局配置用 useStorage('layout-setting', ...) 整体持久化，而 vueuse 的
 * useStorage 默认**不合并默认值**——localStorage 里一旦存过旧的 theme 字段，
 * 就会永久覆盖 src/settings.ts 里的新默认值，导致「改了配色却没生效」。
 *
 * 做法：记录一个主题版本号（THEME_VERSION），版本变化时做一次性迁移：
 *   1. 把「曾经是默认色」的历史取值映射到当前默认色；
 *   2. 补齐新增的默认字段，避免旧配置缺键。
 * 用户主动挑选的其它主题色不受影响。
 *
 * 必须在 Vue 挂载前调用（main.ts 中），保证 store 初始化时读到的是新值。
 */
export const migrateStoredTheme = (): void => {
  try {
    const ls = typeof localStorage !== 'undefined' ? localStorage : undefined;
    if (!ls) return;
    if (ls.getItem(THEME_VERSION_KEY) === THEME_VERSION) return;

    const raw = ls.getItem('layout-setting');
    if (raw) {
      try {
        const parsed = JSON.parse(raw);
        if (parsed && typeof parsed === 'object') {
          const legacy = LEGACY_THEME_MAP[String(parsed.theme || '').toLowerCase()];
          if (legacy) {
            parsed.theme = legacy;
          }
          // 补齐新增的默认字段（保留用户已有取值）
          parsed.tagsView = parsed.tagsView ?? defaultSettings.tagsView;
          parsed.tagsIcon = parsed.tagsIcon ?? defaultSettings.tagsIcon;
          parsed.fixedHeader = parsed.fixedHeader ?? defaultSettings.fixedHeader;
          parsed.sidebarLogo = parsed.sidebarLogo ?? defaultSettings.sidebarLogo;
          parsed.dynamicTitle = parsed.dynamicTitle ?? defaultSettings.dynamicTitle;
          parsed.sideTheme = parsed.sideTheme ?? defaultSettings.sideTheme;
          parsed.navType = parsed.navType ?? defaultSettings.navType;
          parsed.radiusBase = parsed.radiusBase ?? defaultSettings.radiusBase;
          ls.setItem('layout-setting', JSON.stringify(parsed));
        }
      } catch (e) {
        /* localStorage 内容损坏时忽略，走默认值 */
      }
    }
    ls.setItem(THEME_VERSION_KEY, THEME_VERSION);
  } catch (e) {
    /* 隐私模式下 localStorage 不可写时忽略 */
  }
};

/**
 * 应用启动时恢复主题配置：
 * - 亮暗模式归一为默认亮色（已显式选过暗色的除外）
 * - 恢复主题色、页面圆角
 */
export const initAppTheme = (theme?: string, radiusBase?: number) => {
  initDefaultColorMode();
  if (typeof radiusBase === 'number') applyRadiusBase(radiusBase);
  handleThemeStyle(theme || defaultSettings.theme);
};

// hex颜色转rgb颜色
export const hexToRgb = (str: string): string[] => {
  str = str.replace('#', '');
  const hexs = str.match(/../g);
  for (let i = 0; i < 3; i++) {
    if (hexs) {
      hexs[i] = String(parseInt(hexs[i], 16));
    }
  }
  return hexs ? hexs : [];
};

// rgb颜色转Hex颜色
export const rgbToHex = (r: string, g: string, b: string) => {
  const hexs = [Number(r).toString(16), Number(g).toString(16), Number(b).toString(16)];
  for (let i = 0; i < 3; i++) {
    if (hexs[i].length == 1) {
      hexs[i] = `0${hexs[i]}`;
    }
  }
  return `#${hexs.join('')}`;
};

// 变浅颜色值
export const getLightColor = (color: string, level: number) => {
  const rgb = hexToRgb(color);
  for (let i = 0; i < 3; i++) {
    const s = (255 - Number(rgb[i])) * level + Number(rgb[i]);
    rgb[i] = String(Math.floor(s));
  }
  return rgbToHex(rgb[0], rgb[1], rgb[2]);
};

// 变深颜色值
export const getDarkColor = (color: string, level: number) => {
  const rgb = hexToRgb(color);
  for (let i = 0; i < 3; i++) {
    rgb[i] = String(Math.floor(Number(rgb[i]) * (1 - level)));
  }
  return rgbToHex(rgb[0], rgb[1], rgb[2]);
};

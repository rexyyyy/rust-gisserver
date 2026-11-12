/**
 * GeoServer 经典主题（Wicket 默认主题）取色 —— 全系统配色唯一真源
 *
 * 所有色值均由 GeoServer 管理界面截图逐像素采样得到（采样脚本见 .tmp_refs）：
 *   - Logo 文字 / "Logged in as admin." / 链接：rgb(0,146,200) = #0092C8
 *   - 顶部蓝色导航带：自上而下 rgb(0,118,161) → rgb(0,145,198)（向下渐浅）
 *   - 白色页眉与蓝色导航带之间的 2px 分隔线：rgb(198,224,155) = #C6E09B
 *   - Logout 按钮：底色 rgb(198,224,155)，深绿描边，深灰文字
 *   - 侧栏分组标题：rgb(0,132,182) ≈ #0082B6
 *   - 侧栏分组分隔线：rgb(163,171,152) = #A3AB98（过渡 #D7E1C9 / #F4FDE9）
 *   - 表单分节横线：rgb(242,248,232) = #F2F8E8 → rgb(206,228,168) = #CEE4A8
 *   - 输入框 / 表格边框：#DDDDDD
 *
 * 修改这里即可整体换肤，勿在组件 / SCSS 中散落硬编码色值。
 */
export const GS_THEME = {
  /** 主色：Element Plus primary、链接、Logo 文字 */
  primary: '#0092C8',
  /** 顶部蓝色导航带渐变（上沿，较深） */
  navbarTop: '#0076A1',
  /** 顶部蓝色导航带渐变（下沿，较浅） */
  navbarBottom: '#0091C6',
  /** 顶栏上的文字色 */
  navbarText: '#FFFFFF',
  /** 侧栏菜单链接色 */
  link: '#0076A1',
  /** 页面主标题色（form-header / panel-title） */
  title: '#0082B6',
  /** 侧栏分组标题色 */
  groupTitle: '#0082B6',
  /** GeoServer 绿：顶栏分隔线、主按钮底色 */
  accent: '#C6E09B',
  /** 绿按钮描边 */
  accentBorder: '#9CC45C',
  /** 绿按钮文字色 */
  accentText: '#3F5E1E',
  /** 绿按钮 hover 底色 */
  accentHover: '#D2E8AE',
  /** 绿按钮 active 底色 */
  accentActive: '#B8D78C',
  /** 表单分节横线（浅层） */
  accentFaint: '#F2F8E8',
  /** 表单分节横线（深层） */
  accentLine: '#CEE4A8',
  /** 输入框 / 表格边框 */
  border: '#DDDDDD',
  /** 侧栏与区块分隔线 */
  divider: '#DADADA',
  /** 侧栏分组之间的灰绿分隔线 */
  groupDivider: '#A3AB98',
  /** 灰绿分隔线的过渡色 */
  groupDividerSoft: '#D7E1C9',
  /** 侧栏底色 */
  sidebarBg: '#FFFFFF',
  /** 侧栏 hover 底色 */
  sidebarHover: '#E4F1F8'
} as const;

/**
 * 主题版本号：任何影响默认配色的改动都要 +1。
 *
 * 浏览器 localStorage 里存的是上一版主题配置（useStorage 不会自动合并默认值），
 * 版本号用于在启动时触发一次性迁移，把陈旧配置纠正到当前默认主题。
 */
export const THEME_VERSION = 'v2-geoserver';

/**
 * 历史主题色 → 当前默认主题色（键为小写，匹配时统一转小写）。
 * 只迁移「曾经是默认色」的取值；用户主动挑选的其它主题色保持不变。
 */
export const LEGACY_THEME_MAP: Record<string, string> = {
  '#1d9cc9': GS_THEME.primary
};

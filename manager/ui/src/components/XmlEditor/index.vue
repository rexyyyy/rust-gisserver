<template>
  <div class="xml-editor" :style="{ height: props.height }">
    <div ref="hostRef" class="xml-editor__host" />
    <div v-if="syntaxError" class="xml-editor__error">
      <el-icon><WarningFilled /></el-icon>
      <span>{{ syntaxError }}</span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
/**
 * CodeMirror 6 XML 编辑器。
 *
 * 面向「服务配置文件（XML）」的原地编辑场景：
 * - 语法高亮 + 标签自动闭合 + 括号匹配 + 历史回退 + 搜索替换（basicSetup）
 * - v-model 双向绑定，外部可整体替换内容
 * - 跟随站点亮暗主题（localStorage key: useDarkKey，与顶栏开关共用同一份状态）
 * - 内置 XML 语法自检（浏览器原生 DOMParser），错误在编辑器底部红条提示
 */
import { EditorState, Compartment } from '@codemirror/state';
import { EditorView, keymap, placeholder as cmPlaceholder } from '@codemirror/view';
import { basicSetup } from 'codemirror';
import { xml } from '@codemirror/lang-xml';
import { indentWithTab } from '@codemirror/commands';
import { useDark } from '@vueuse/core';

const props = withDefaults(
  defineProps<{
    /** 绑定值（XML 原文） */
    modelValue?: string;
    /** 只读 */
    readonly?: boolean;
    /** 编辑区高度 */
    height?: string;
    /** 占位提示 */
    placeholder?: string;
  }>(),
  {
    modelValue: '',
    readonly: false,
    height: '420px',
    placeholder: ''
  }
);

const emit = defineEmits<{
  (e: 'update:modelValue', value: string): void;
  (e: 'change', value: string): void;
}>();

const hostRef = ref<HTMLDivElement | null>(null);
const isDark = useDark({ storageKey: 'useDarkKey' });
const syntaxError = ref('');

let view: EditorView | null = null;
const themeCompartment = new Compartment();
const readOnlyCompartment = new Compartment();
const placeholderCompartment = new Compartment();

/** 亮色主题：与 Element Plus 默认浅色面板对齐 */
const lightTheme = EditorView.theme({
  '&': { color: '#303133', backgroundColor: '#ffffff' },
  '.cm-content': { caretColor: '#303133' },
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: '#303133' },
  '.cm-gutters': { backgroundColor: '#f5f7fa', color: '#909399', border: 'none' },
  '.cm-activeLine': { backgroundColor: '#f5f7fa' },
  '.cm-activeLineGutter': { backgroundColor: '#e9edf2', color: '#606266' },
  '.cm-selectionBackground, &.cm-focused .cm-selectionBackground': { backgroundColor: '#c6e2ff' }
});

/** 暗色主题：与站点 html.dark 下的面板配色对齐 */
const darkTheme = EditorView.theme(
  {
    '&': { color: '#d4d4d4', backgroundColor: '#1e1e1e' },
    '.cm-content': { caretColor: '#d4d4d4' },
    '.cm-cursor, .cm-dropCursor': { borderLeftColor: '#d4d4d4' },
    '.cm-gutters': { backgroundColor: '#1e1e1e', color: '#858585', border: 'none' },
    '.cm-activeLine': { backgroundColor: '#2a2d2e' },
    '.cm-activeLineGutter': { backgroundColor: '#2a2d2e', color: '#c6c6c6' },
    '.cm-selectionBackground, &.cm-focused .cm-selectionBackground': { backgroundColor: '#264f78' }
  },
  { dark: true }
);

const themeExtension = (dark: boolean) => (dark ? darkTheme : lightTheme);

const readOnlyExtension = (ro: boolean) => [
  EditorState.readOnly.of(ro),
  EditorView.editable.of(!ro)
];

const placeholderExtension = (text: string) => (text ? cmPlaceholder(text) : []);

const buildExtensions = (dark: boolean) => [
  basicSetup,
  xml(),
  keymap.of([indentWithTab]),
  themeCompartment.of(themeExtension(dark)),
  readOnlyCompartment.of(readOnlyExtension(props.readonly)),
  placeholderCompartment.of(placeholderExtension(props.placeholder)),
  EditorView.updateListener.of((u) => {
    if (!u.docChanged) return;
    const text = u.state.doc.toString();
    emit('update:modelValue', text);
    emit('change', text);
    validate(text);
  })
];

onMounted(() => {
  if (!hostRef.value) return;
  view = new EditorView({
    parent: hostRef.value,
    state: EditorState.create({
      doc: props.modelValue || '',
      extensions: buildExtensions(isDark.value)
    })
  });
  validate(view.state.doc.toString());
});

onBeforeUnmount(() => {
  view?.destroy();
  view = null;
});

// 外部整体替换内容时同步进编辑器（避免与用户输入产生回环）
watch(
  () => props.modelValue,
  (val) => {
    if (!view) return;
    const cur = view.state.doc.toString();
    if (cur === (val || '')) return;
    view.dispatch({
      changes: { from: 0, to: cur.length, insert: val || '' }
    });
  }
);

// 亮暗主题切换：仅重配置主题 compartment，不重建编辑器（保留光标与撤销栈）
watch(isDark, (dark) => {
  if (!view) return;
  view.dispatch({
    effects: themeCompartment.reconfigure(themeExtension(dark))
  });
});

// 只读切换
watch(
  () => props.readonly,
  (ro) => {
    if (!view) return;
    view.dispatch({
      effects: readOnlyCompartment.reconfigure(readOnlyExtension(ro))
    });
  }
);

watch(
  () => props.placeholder,
  (text) => {
    if (!view) return;
    view.dispatch({
      effects: placeholderCompartment.reconfigure(placeholderExtension(text))
    });
  }
);

/**
 * XML 语法自检（浏览器原生 DOMParser，不引入额外依赖）。
 * 返回 null 表示通过，否则返回可读的错误描述。
 */
const checkXml = (text: string): string | null => {
  if (!text || !text.trim()) return t('misc.auto.emptyContent');
  try {
    const doc = new DOMParser().parseFromString(text, 'application/xml');
    const err = doc.querySelector('parsererror');
    if (err) {
      const msg = (err.textContent || '').replace(/\s+/g, ' ').trim();
      // Chrome 的 parsererror 文本里带行号信息，截取首句更易读
      const first = msg.split('. ')[0] || msg;
      return first.length > 200 ? first.slice(0, 200) + '…' : first;
    }
    // DOMParser 对良构 XML 也会在根节点下插入 parsererror，需再确认根元素存在
    if (!doc.documentElement || doc.documentElement.nodeName === 'parsererror') {
      return t('misc.auto.xmlIsIncompleteOrHasUnclosedTags');
    }
    return null;
  } catch (e: any) {
    return t('misc.auto.xmlParseFailed') + (e?.message || String(e));
  }
};

/** 触发一次自检并更新底部提示条 */
const validate = (text?: string) => {
  const content = text ?? view?.state.doc.toString() ?? '';
  syntaxError.value = checkXml(content) || '';
  return !syntaxError.value;
};

defineExpose({
  /** 取当前编辑内容 */
  getContent: () => view?.state.doc.toString() ?? props.modelValue ?? '',
  /** 整体替换编辑内容 */
  setContent: (text: string) => {
    if (!view) return;
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } });
  },
  /** 执行 XML 语法自检，返回是否通过 */
  validate: () => validate(),
  /** 聚焦编辑器 */
  focus: () => view?.focus()
});
</script>

<style scoped lang="scss">
.xml-editor {
  position: relative;
  border: 1px solid var(--el-border-color);
  border-radius: 4px;
  overflow: hidden;
  background: var(--el-bg-color);

  &__host {
    height: 100%;
    overflow: auto;

    :deep(.cm-editor) {
      height: 100%;
      font-size: 13px;
    }

    :deep(.cm-editor.cm-focused) {
      outline: none;
    }

    :deep(.cm-scroller) {
      font-family: 'Cascadia Code', Consolas, 'Courier New', monospace;
      line-height: 1.6;
    }
  }

  &__error {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    font-size: 12px;
    line-height: 1.5;
    color: var(--el-color-danger);
    background-color: var(--el-color-danger-light-9);
    border-top: 1px solid var(--el-color-danger-light-7);
    word-break: break-all;
  }
}
</style>

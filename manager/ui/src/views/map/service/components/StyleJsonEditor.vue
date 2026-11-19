<!--
  MVT 矢量瓦片前端样式 JSON 编辑器（左右布局）。
  左：表单（按 geometryType 过滤）  右：实时 JSON 预览/编辑
-->
<template>
  <el-dialog
    v-model="visible"
    :title="`${t('map.auto.mvtStyle')}${title}`"
    width="960px"
    append-to-body
    destroy-on-close
    @open="load"
  >
    <div class="sje-layout">
      <!-- ========= LEFT: FORM ========= -->
      <div class="sje-left">
        <el-form label-width="72px" size="default">
          <!-- POINT -->
          <el-card v-if="showSection('point')" shadow="never" class="sje-card">
            <template #header><b>{{ $t('map.auto.point') }}</b></template>
            <el-form-item :label="$t('map.auto.radius')">
              <el-input-number v-model="point.radius" :min="1" :max="24" />
            </el-form-item>
            <el-form-item :label="$t('map.auto.fill')">
              <el-color-picker v-model="point.fill.color" show-alpha />
            </el-form-item>
            <el-form-item :label="$t('map.auto.stroke')">
              <div class="flex gap-2 w-full">
                <el-color-picker v-model="point.stroke.color" show-alpha />
                <el-input-number v-model="point.stroke.width" :min="0" :max="6" :step="0.5" class="flex-1" />
              </div>
            </el-form-item>
          </el-card>

          <!-- LINE -->
          <el-card v-if="showSection('line')" shadow="never" class="sje-card">
            <template #header><b>{{ $t('map.auto.line') }}</b></template>
            <el-form-item :label="$t('map.auto.stroke')">
              <div class="flex gap-2 w-full">
                <el-color-picker v-model="line.stroke.color" show-alpha />
                <el-input-number v-model="line.stroke.width" :min="0" :max="10" :step="0.5" class="flex-1" />
              </div>
            </el-form-item>
          </el-card>

          <!-- POLYGON -->
          <el-card v-if="showSection('polygon')" shadow="never" class="sje-card">
            <template #header><b>{{ $t('map.auto.polygon') }}</b></template>
            <el-form-item :label="$t('map.auto.fill')">
              <div class="flex gap-2 w-full">
                <el-color-picker v-model="polygon.fill.color" show-alpha />
                <el-input-number v-model="polygon.fill.opacity" :min="0" :max="1" :step="0.1" class="flex-1" />
              </div>
            </el-form-item>
            <el-form-item :label="$t('map.auto.stroke')">
              <div class="flex gap-2 w-full">
                <el-color-picker v-model="polygon.stroke.color" show-alpha />
                <el-input-number v-model="polygon.stroke.width" :min="0" :max="6" :step="0.5" class="flex-1" />
              </div>
            </el-form-item>
          </el-card>

          <!-- LABEL (always shown) -->
          <el-card shadow="never" class="sje-card">
            <template #header>
              <div class="flex items-center justify-between">
                <b>{{ $t('map.auto.label') }}</b>
                <el-switch v-model="label.enabled" size="small" />
              </div>
            </template>
            <template v-if="label.enabled">
              <el-form-item :label="$t('map.auto.attributeField')">
                <el-input v-model="label.field" :placeholder="$t('map.auto.eGNameNAMEId')" />
              </el-form-item>
              <el-form-item :label="$t('map.auto.font')">
                <el-input v-model="label.font" placeholder="13px Arial / 14px Microsoft YaHei" />
              </el-form-item>
              <el-form-item :label="$t('map.auto.textColor')">
                <el-color-picker v-model="label.fill.color" show-alpha />
              </el-form-item>
              <el-form-item :label="$t('map.auto.stroke')">
                <div class="flex gap-2 w-full">
                  <el-color-picker v-model="label.stroke.color" show-alpha />
                  <el-input-number v-model="label.stroke.width" :min="0" :max="6" :step="0.5" class="flex-1" />
                </div>
              </el-form-item>
              <el-form-item :label="$t('map.auto.verticalOffset')">
                <el-input-number v-model="label.offsetY" :min="-40" :max="40" />
              </el-form-item>
            </template>
          </el-card>
        </el-form>
      </div>

      <!-- ========= RIGHT: JSON ========= -->
      <div class="sje-right">
        <div class="sje-right-head">
          <span class="text-xs text-gray-500">{{ $t('map.auto.liveJSONSyncsWithFormChanges') }}</span>
          <div class="flex gap-1">
            <el-button size="small" plain @click="formatJson">{{ $t('map.auto.format') }}</el-button>
            <el-button size="small" plain @click="syncFromJson">{{ $t('map.auto.applyToForm') }}</el-button>
            <el-button size="small" plain type="warning" @click="resetDefault">{{ $t('map.auto.default10') }}</el-button>
          </div>
        </div>
        <el-input
          v-model="jsonText"
          type="textarea"
          :autosize="{ minRows: 22, maxRows: 30 }"
          resize="none"
          class="sje-json font-mono text-xs"
          @change="onJsonChange"
        />
        <div class="sje-right-foot text-xs text-gray-400">
          geometryType = <b>{{ geometryType || 'mixed (show all)' }}</b>
        </div>
      </div>
    </div>

    <template #footer>
      <div class="flex justify-between">
        <el-button plain type="info" size="small" @click="preview">
          <el-icon><View /></el-icon> {{ $t('map.auto.preview4') }}
        </el-button>
        <div>
          <el-button @click="visible = false">{{ $t('map.auto.cancel') }}</el-button>
          <el-button type="primary" :loading="saving" @click="save">{{ $t('map.auto.save') }}</el-button>
        </div>
      </div>
    </template>
  </el-dialog>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
import { getMvtStyle, putMvtStyle, type MvtStyleJson } from '@/api/map/service/style';

type GeoType = 'point' | 'line' | 'polygon' | 'mixed';

const emit = defineEmits<{ saved: [] }>();

const visible = ref(false);
const saving = ref(false);
const kind = ref('');
const name = ref('');
const title = ref('');
const geometryType = ref<GeoType>('mixed');

const showSection = (sec: GeoType) => {
  if (!geometryType.value || geometryType.value === 'mixed') return true;
  return geometryType.value === sec;
};

const label = reactive({
  enabled: false,
  field: '',
  font: 'bold 12px Arial',
  fill: { color: '#000000' },
  stroke: { color: '#ffffff', width: 2 },
  offsetY: -10
});
const point = reactive({
  radius: 4,
  fill: { color: 'rgba(214,60,50,0.85)' },
  stroke: { color: '#ffffff', width: 1 }
});
const line = reactive({
  stroke: { color: '#1e6fd9', width: 1.4 }
});
const polygon = reactive({
  fill: { color: 'rgba(30,111,217,0.18)', opacity: 0.18 },
  stroke: { color: '#1e6fd9', width: 1.4 }
});

const jsonText = ref('');

const open = (cfg: { kind: string; name: string; title?: string; geometryType?: GeoType }) => {
  kind.value = cfg.kind;
  name.value = cfg.name;
  title.value = cfg.title || cfg.name;
  geometryType.value = (cfg.geometryType as GeoType) || 'mixed';
  visible.value = true;
};
defineExpose({ open });

// ---- defaults ----
const defaultStyle = (): MvtStyleJson => ({
  version: 1,
  point: { radius: 4, fill: { color: 'rgba(214,60,50,0.85)' }, stroke: { color: '#ffffff', width: 1 } },
  line:  { stroke: { color: '#1e6fd9', width: 1.4 } },
  polygon: { fill: { color: 'rgba(30,111,217,0.18)' }, stroke: { color: '#1e6fd9', width: 1.4 } },
  label: null
});

const resetDefault = () => {
  const d = defaultStyle();
  applyFromJson(d);
  jsonText.value = JSON.stringify(d, null, 2);
};

// ---- JSON <-> form ----
function applyFromJson(style: any) {
  if (!style) return;
  if (style.point) Object.assign(point, style.point);
  if (style.line) Object.assign(line, style.line);
  if (style.polygon) {
    Object.assign(polygon, style.polygon);
    const c = polygon.fill.color as string;
    const m = c?.match(/rgba?\([^,]+,[^,]+,[^,]+,?\s*([\d.]+)?\)/);
    if (m && m[1]) polygon.fill.opacity = parseFloat(m[1]);
  }
  if (style.label) {
    label.enabled = true;
    Object.assign(label, style.label);
  } else {
    label.enabled = false;
  }
}

function buildJson(): MvtStyleJson {
  const p = { ...point };
  const l = { ...line };
  let poly = { ...polygon };
  if (poly.fill.color.startsWith('#') && poly.fill.opacity !== undefined) {
    const hex = poly.fill.color;
    const r = parseInt(hex.slice(1, 3), 16);
    const g = parseInt(hex.slice(3, 5), 16);
    const b = parseInt(hex.slice(5, 7), 16);
    poly.fill.color = `rgba(${r},${g},${b},${poly.fill.opacity})`;
  }
  return {
    version: 1,
    point: p,
    line: l,
    polygon: poly,
    label: label.enabled ? {
      field: label.field, font: label.font,
      fill: label.fill, stroke: label.stroke, offsetY: label.offsetY
    } : null
  };
}

// watch form → JSON
watch([point, line, polygon, label], () => {
  jsonText.value = JSON.stringify(buildJson(), null, 2);
}, { deep: true });

const syncFromJson = () => {
  try {
    const obj = JSON.parse(jsonText.value);
    applyFromJson(obj);
  } catch { ElMessage.error(t('map.auto.jsonParseFailed')); }
};
const onJsonChange = () => {
  try { JSON.parse(jsonText.value); }
  catch { /* 允许中间态，只在 save 时校验 */ }
};
const formatJson = () => {
  try { jsonText.value = JSON.stringify(JSON.parse(jsonText.value), null, 2); }
  catch { ElMessage.error(t('map.auto.jsonParseFailed')); }
};

// ---- load / save ----
const load = async () => {
  try {
    const res = await getMvtStyle(kind.value, name.value);
    applyFromJson(res.data.style);
    jsonText.value = JSON.stringify(buildJson(), null, 2);
  } catch {
    ElMessage.error(t('map.auto.failedToLoadStyle'));
    resetDefault();
  }
};

const save = async () => {
  let style = buildJson();
  // 若用户在 JSON 面板手动改过有效内容，优先用 JSON 面板
  try {
    const j = JSON.parse(jsonText.value);
    if (j && typeof j === 'object') style = j;
  } catch {}

  saving.value = true;
  try {
    await putMvtStyle(kind.value, name.value, style);
    ElMessage.success(t('map.auto.styleSaved'));
    emit('saved');
    visible.value = false;
  } catch (e: any) {
    ElMessage.error(e?.data?.msg || t('map.auto.saveFailed'));
  } finally {
    saving.value = false;
  }
};

const preview = () => {
  // Derive map-api base from the current manager host so preview works
  // everywhere (localhost dev, Docker/WSL, remote server, prod domain).
  // Manager (6180) and map-api (8080) share the same host; just swap the port.
  const base = `${window.location.protocol}//${window.location.hostname}:8080`;
  const url = `/preview/viewer_mvt.html?layer=${name.value}&kind=${kind.value}&base=${base}`;
  window.open(url, '_blank');
};
</script>

<style scoped>
.sje-layout {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  gap: 16px;
  min-height: 440px;
}
.sje-left {
  overflow-y: auto;
  max-height: 70vh;
  padding-right: 4px;
}
.sje-right {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 6px;
  background: #1e1e1e;
  color: #d4d4d4;
  overflow: hidden;
}
.sje-right-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 8px 10px;
  background: #2d2d2d;
  border-bottom: 1px solid #3c3c3c;
}
.sje-right-foot {
  padding: 4px 10px;
  background: #252526;
  border-top: 1px solid #3c3c3c;
  text-align: right;
}
:deep(.sje-json textarea) {
  background: #1e1e1e !important;
  color: #d4d4d4 !important;
  border: none !important;
  font-family: Consolas, Monaco, monospace !important;
  font-size: 12px !important;
  line-height: 1.6 !important;
  padding: 10px 12px !important;
  resize: none !important;
}
:deep(.sje-json .el-textarea__inner:focus) {
  box-shadow: none !important;
}
.sje-card { margin-bottom: 12px; }
.sje-card:last-child { margin-bottom: 0; }
</style>

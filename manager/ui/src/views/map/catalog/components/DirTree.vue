<template>
  <div class="dir-tree">
    <div v-if="!dataRoot" class="text-xs text-gray-400 p-2">{{ $t('map.auto.text25') }}</div>
    <template v-else>
      <div class="dir-tree__root text-xs text-gray-400 px-1 pb-1" :title="dataRoot">{{ dataRoot }}</div>
      <el-tree
        ref="treeRef"
        :key="`${dataRoot}#${renderKey}`"
        :load="loadNode"
        lazy
        :props="treeProps"
        node-key="path"
        highlight-current
        :expand-on-click-node="false"
        @node-click="handleNodeClick"
        class="dir-tree__tree"
      >
        <template #default="{ data }">
          <div class="dir-tree__node">
            <el-icon v-if="data.isLeaf" class="text-xs text-gray-400 mr-1">
              <Document />
            </el-icon>
            <el-icon v-else class="text-xs text-gray-400 mr-1">
              <Folder />
            </el-icon>
            <span class="dir-tree__label">{{ data.name }}</span>
            <el-tag
              v-if="data.detect"
              size="small"
              :type="data.detect.storeType ? 'success' : 'warning'"
              class="ml-2"
            >
              {{ data.detect.label }}
            </el-tag>
          </div>
        </template>
      </el-tree>
    </template>
  </div>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
// Folder / Document 由全局注册的 Element Plus Icons 提供
import { listWorkspaceDirs } from '@/api/map/catalog';
import { DirEntry } from '@/api/map/catalog/types';

/** 目录接口返回的一层条目 */
const props = defineProps<{
  dataRoot: string;
  workspaceId: number;
}>();

interface DirSelectPayload {
  path: string;
  storeType: string;
  label: string;
  entries: DirEntry[];
}

const emit = defineEmits<{
  (e: 'select', payload: DirSelectPayload): void;
}>();

const treeRef = ref<any>(null);
const renderKey = ref(0);

interface DetectResult {
  /** 展示名，如 3D Tiles / ArcGIS 缓存 / MVT 瓦片 */
  label: string;
  /** 能登记的数据源类型；空串表示引擎暂无对应类型，只识别不登记 */
  storeType: string;
}

interface TreeNode {
  name: string;
  path: string;
  is_dir: boolean;
  isLeaf: boolean;
  detect?: DetectResult;
  /** 点击时已拉取过的条目，展开子节点时复用，避免重复请求 */
  cachedEntries?: DirEntry[];
  children?: TreeNode[];
}

const treeProps = {
  label: 'name',
  children: 'children',
  isLeaf: 'isLeaf'
};

/** 拼接目录路径：统一用 /，避免 dataRoot 末尾已有分隔符时出现 // */
const joinPath = (base: string, name: string): string => {
  const b = (base || '').replace(/[\\/]+$/, '');
  return b ? `${b}/${name}` : name;
};

const lower = (s: string) => s.toLowerCase();

/** 列出工作空间数据根目录下的一层条目（只读元信息） */
const listDir = async (path?: string): Promise<DirEntry[]> => {
  if (!props.workspaceId) return [];
  try {
    const res = await listWorkspaceDirs({ workspaceId: props.workspaceId, path });
    return res.data.entries || [];
  } catch {
    return [];
  }
};

/** 同步识别：只看当前目录这一层的条目（.pbf / ArcGIS 容器另有异步探测） */
const detectByEntries = (entries: DirEntry[]): DetectResult | null => {
  const files = entries.filter((e) => !e.is_dir).map((e) => lower(e.name));
  const dirs = entries.filter((e) => e.is_dir).map((e) => lower(e.name));

  // 3D Tiles：目录内存在 tileset.json（必须早于 json 判定）
  if (files.includes('tileset.json')) return { label: '3D Tiles', storeType: '3dtiles_dir' };

  // ArcGIS 缓存：_alllayers 目录 + Conf.xml / conf.cdi
  if (dirs.includes('_alllayers') && (files.includes('conf.xml') || files.includes('conf.cdi'))) {
    return { label: t('map.auto.arcgisCache'), storeType: 'arcgis_dir' };
  }

  // 成片的 MVT 瓦片（z/x/y.pbf）
  if (files.some((f) => f.endsWith('.pbf'))) return { label: t('map.auto.mvt'), storeType: 'mvt_dir' };

  if (files.some((f) => f.endsWith('.mbtiles'))) return { label: 'MBTiles', storeType: 'mbtiles_dir' };
  if (files.some((f) => f.endsWith('.pmtiles'))) return { label: 'PMTiles', storeType: 'pmtiles_dir' };
  if (files.some((f) => f.endsWith('.shp'))) return { label: 'Shapefile', storeType: 'shapefile_dir' };
  if (files.some((f) => /\.tiff?$/.test(f))) return { label: t('map.auto.cog'), storeType: 'cog_dir' };
  if (files.some((f) => /\.(geojson|json)$/.test(f))) return { label: 'GeoJSON', storeType: 'geojson_dir' };

  return null;
};

/** 异步补判：ArcGIS 的 Layers 容器目录、MVT 的 z/x/y 瓦片金字塔 */
const detectDeep = async (path: string, entries: DirEntry[]): Promise<DetectResult | null> => {
  const dirs = entries.filter((e) => e.is_dir);
  // ArcGIS：父目录下只有 Layers/图层 子目录时，深入一层看是否是缓存本体
  const layersDir = dirs.find((e) => ['layers', t('map.auto.layer2')].includes(lower(e.name)));
  if (layersDir) {
    const sub = await listDir(joinPath(path, layersDir.name));
    const hit = detectByEntries(sub);
    if (hit?.label === t('map.auto.arcgisCache')) return hit;
  }

  // MVT：z（纯数字）→ x（纯数字）→ *.pbf
  const zoomDirs = dirs.filter((e) => /^\d+$/.test(e.name));
  if (!zoomDirs.length) return null;
  const zPath = joinPath(path, zoomDirs[0].name);
  const zEntries = await listDir(zPath);
  if (zEntries.some((e) => !e.is_dir && lower(e.name).endsWith('.pbf'))) {
    return { label: t('map.auto.mvt'), storeType: 'mvt_dir' };
  }
  const xDir = zEntries.find((e) => e.is_dir && /^\d+$/.test(e.name));
  if (!xDir) return null;
  const xEntries = await listDir(joinPath(zPath, xDir.name));
  if (xEntries.some((e) => !e.is_dir && lower(e.name).endsWith('.pbf'))) {
    return { label: t('map.auto.mvt'), storeType: 'mvt_dir' };
  }
  return null;
};

const dirsOf = (path: string, entries: DirEntry[]): TreeNode[] =>
  entries
    .filter((e) => e.is_dir)
    .map((e) => ({
      name: e.name,
      path: joinPath(path, e.name),
      is_dir: true,
      isLeaf: false
    }));

const detectDir = async (path: string, entries: DirEntry[]): Promise<DetectResult | null> =>
  detectByEntries(entries) || (await detectDeep(path, entries));

const loadNode = async (node: any, resolve: (nodes: TreeNode[]) => void) => {
  const path = node.level === 0 ? props.dataRoot : (node.data as TreeNode).path;
  if (!path) {
    resolve([]);
    return;
  }
  const cached = node.level === 0 ? undefined : (node.data as TreeNode).cachedEntries;
  const entries = cached ?? (await listDir(path));
  // 展开箭头也会走到这里：识别为已知图层数据时直接收成叶子，不再展开下一级
  if (node.level > 0 && !cached) {
    const detect = await detectDir(path, entries);
    if (detect) {
      const data = node.data as TreeNode;
      data.detect = detect;
      data.isLeaf = true;
      data.cachedEntries = undefined;
      // 点「展开箭头」也当作选中：回填目录与类型，并收起重展开动作
      emit('select', { path: data.path, storeType: detect.storeType, label: detect.label, entries: [] });
      resolve([]);
      node.collapse();
      return;
    }
  }
  resolve(dirsOf(path, entries));
};

const handleNodeClick = async (data: TreeNode, node: any) => {
  if (data.isLeaf && data.detect) {
    // 已识别的数据目录：只回填，不展开
    emit('select', { path: data.path, storeType: data.detect.storeType, label: data.detect.label, entries: [] });
    return;
  }
  const entries = await listDir(data.path);
  const detect = await detectDir(data.path, entries);
  if (detect) {
    // 已知图层数据：停止展开，回填目录与类型
    data.detect = detect;
    data.isLeaf = true;
    data.cachedEntries = undefined;
    emit('select', { path: data.path, storeType: detect.storeType, label: detect.label, entries });
    return;
  }
  // 普通目录：回填目录，并展开下一级供继续选择
  data.cachedEntries = entries;
  emit('select', { path: data.path, storeType: '', label: '', entries });
  if (!node.expanded) node.expand();
};

/** 工作空间切换 / 重新打开对话框时重建整棵树 */
const refresh = () => {
  renderKey.value += 1;
};

defineExpose({ refresh });
</script>

<style scoped>
.dir-tree {
  height: 100%;
  overflow: auto;
  border: 1px solid var(--el-border-color-light);
  border-radius: 4px;
  padding: 8px;
}

.dir-tree__root {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  border-bottom: 1px dashed var(--el-border-color-lighter);
  margin-bottom: 4px;
}

.dir-tree__tree {
  height: 100%;
}

.dir-tree__node {
  display: flex;
  align-items: center;
  font-size: 13px;
  width: 100%;
  overflow: hidden;
}

.dir-tree__label {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dir-detect-hint {
  color: var(--el-text-color-secondary);
}

.dir-detect-warn {
  color: var(--el-color-warning);
}
</style>

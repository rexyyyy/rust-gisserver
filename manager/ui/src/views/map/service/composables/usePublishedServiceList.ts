import { useI18n } from 'vue-i18n';
import { listPublished, setPublishedState } from '@/api/map/catalog';
import type { PublishedVO } from '@/api/map/catalog/types';
import { useCaps } from '@/views/map/catalog/composables/useCatalog';
import PublishDialog from '@/views/map/catalog/components/PublishDialog.vue';
import PreviewDialog from '@/views/map/catalog/components/PreviewDialog.vue';
import StyleJsonEditor from '@/views/map/service/components/StyleJsonEditor.vue';
import { canEditStyle, geomTypeOf } from './serviceMeta';

export interface ServiceListOptions {
  /** Whitelist of mapserverKind. Empty or omitted means all kinds. */
  kinds?: string[];
  /** Page-specific extra predicate, applied after the kind filter. */
  extraFilter?: (row: PublishedVO) => boolean;
}

/**
 * Shared data and behaviour for a published-service page.
 *
 * Each service type page owns its own template; this layer only supplies the
 * common ledger query, filtering, pagination, lifecycle actions and dialog
 * handles. Page-specific columns, buttons and filters belong in the page.
 */
export function usePublishedServiceList(options: ServiceListOptions = {}) {
  const { t } = useI18n();
  const router = useRouter();
  const proxy = getCurrentInstance()?.proxy;
  const { caps, loadCaps } = useCaps();

  const loading = ref(false);
  const rows = ref<PublishedVO[]>([]);
  const queryName = ref('');
  const queryState = ref('');
  const page = ref(1);
  const pageSize = ref(20);

  const pubRef = ref<InstanceType<typeof PublishDialog>>();
  const pvRef = ref<InstanceType<typeof PreviewDialog>>();
  const styleRef = ref<InstanceType<typeof StyleJsonEditor>>();

  const kinds = options.kinds ?? [];

  const filteredRows = computed(() => {
    let list = rows.value;
    if (kinds.length) {
      list = list.filter((r) => kinds.includes(r.mapserverKind));
    }
    if (options.extraFilter) {
      list = list.filter(options.extraFilter);
    }
    const q = queryName.value.trim().toLowerCase();
    if (q) {
      list = list.filter((r) => r.layerName.toLowerCase().includes(q) || (r.title || '').toLowerCase().includes(q));
    }
    if (queryState.value) {
      list = list.filter((r) => r.state === queryState.value);
    }
    return list;
  });

  const pagedRows = computed(() => {
    const start = (page.value - 1) * pageSize.value;
    return filteredRows.value.slice(start, start + pageSize.value);
  });

  watch([queryName, queryState], () => {
    page.value = 1;
  });

  const load = async () => {
    loading.value = true;
    try {
      const res = await listPublished({ includeDeleted: 1 });
      rows.value = (res.data.published || []).filter((p) => p.state !== 'deleted');
    } finally {
      loading.value = false;
    }
  };

  const goPublish = () => {
    router.push('/data/resource');
  };

  const openPreview = (row: PublishedVO) => pvRef.value?.open(row);

  const openEdit = (row: PublishedVO) => pubRef.value?.openEdit(row);

  const openStyle = (row: PublishedVO) => {
    if (!canEditStyle(row)) return;
    styleRef.value?.open({
      kind: row.mapserverKind,
      name: row.layerName,
      title: row.title || row.layerName,
      geometryType: geomTypeOf(row)
    });
  };

  const handleState = async (row: PublishedVO, state: string) => {
    const tips: Record<string, string> = {
      active: `${t('map.auto.confirmRepublish')}${row.layerName}${t('map.auto.theEngineWillPickUpThisServiceAgain')}`,
      stopped: `${t('map.auto.confirmUnpublish')}${row.layerName}${t('map.auto.theServiceWillStopBeingServedButItsConfigu')}`,
      deleted: `${t('map.auto.confirmDeleteService')}${row.layerName}${t('map.auto.itWillBeMovedToTheTrashConfigurationIsNotP')}`
    };
    await proxy?.$modal.confirm(tips[state] || t('map.auto.confirmOperation'));
    await setPublishedState(row.id, state);
    proxy?.$modal.msgSuccess(t('map.auto.updated2'));
    await load();
  };

  onMounted(async () => {
    await Promise.all([loadCaps(), load()]);
  });

  return {
    loading,
    rows,
    filteredRows,
    pagedRows,
    page,
    pageSize,
    queryName,
    queryState,
    caps,
    pubRef,
    pvRef,
    styleRef,
    load,
    goPublish,
    openPreview,
    openEdit,
    openStyle,
    handleState
  };
}

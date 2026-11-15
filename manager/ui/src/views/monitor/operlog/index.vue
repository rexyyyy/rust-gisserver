<template>
  <div class="p-2">
    <transition :enter-active-class="proxy?.animate.searchAnimate.enter" :leave-active-class="proxy?.animate.searchAnimate.leave">
      <div v-show="showSearch" class="mb-[10px]">
        <el-card shadow="hover">
          <el-form ref="queryFormRef" :model="queryParams" :inline="true">
            <el-form-item :label="$t('monitor.auto.operationIP')" prop="operIp">
              <el-input v-model="queryParams.operIp" :placeholder="$t('monitor.auto.pleaseEnterOperationIP')" clearable @keyup.enter="handleQuery" />
            </el-form-item>
            <el-form-item :label="$t('monitor.auto.systemModule')" prop="title">
              <el-input v-model="queryParams.title" :placeholder="$t('monitor.auto.pleaseEnterSystemModule')" clearable @keyup.enter="handleQuery" />
            </el-form-item>
            <el-form-item :label="$t('monitor.auto.operator')" prop="operName">
              <el-input v-model="queryParams.operName" :placeholder="$t('monitor.auto.pleaseEnterOperator')" clearable @keyup.enter="handleQuery" />
            </el-form-item>
            <el-form-item :label="$t('monitor.auto.type')" prop="businessType">
              <el-select v-model="queryParams.businessType" :placeholder="$t('monitor.auto.operationType')" clearable>
                <el-option v-for="dict in sys_oper_type" :key="dict.value" :label="dict.label" :value="dict.value" />
              </el-select>
            </el-form-item>
            <el-form-item :label="$t('monitor.auto.status')" prop="status">
              <el-select v-model="queryParams.status" :placeholder="$t('monitor.auto.operationStatus')" clearable>
                <el-option v-for="dict in sys_common_status" :key="dict.value" :label="dict.label" :value="dict.value" />
              </el-select>
            </el-form-item>
            <el-form-item :label="$t('monitor.auto.operationTime')" style="width: 308px">
              <el-date-picker
                v-model="dateRange"
                value-format="YYYY-MM-DD HH:mm:ss"
                type="daterange"
                range-separator="-"
                :start-placeholder="$t('monitor.auto.startDate')"
                :end-placeholder="$t('monitor.auto.endDate')"
                :default-time="[new Date(2000, 1, 1, 0, 0, 0), new Date(2000, 1, 1, 23, 59, 59)]"
              ></el-date-picker>
            </el-form-item>
            <el-form-item>
              <el-button type="primary" icon="Search" @click="handleQuery">{{ $t('monitor.auto.search') }}</el-button>
              <el-button icon="Refresh" @click="resetQuery">{{ $t('monitor.auto.reset') }}</el-button>
            </el-form-item>
          </el-form>
        </el-card>
      </div>
    </transition>

    <el-card shadow="hover">
      <template #header>
        <el-row :gutter="10" class="mb8">
          <el-col :span="1.5">
            <el-button v-hasPermi="['monitor:operlog:remove']" type="danger" plain icon="Delete" :disabled="multiple" @click="handleDelete()">
              {{ $t('monitor.auto.delete') }}
            </el-button>
          </el-col>
          <el-col :span="1.5">
            <el-button v-hasPermi="['monitor:operlog:remove']" type="danger" plain icon="WarnTriangleFilled" @click="handleClean">{{ $t('monitor.auto.clear') }}</el-button>
          </el-col>
          <el-col :span="1.5">
            <el-button v-hasPermi="['monitor:operlog:export']" type="warning" plain icon="Download" @click="handleExport">{{ $t('monitor.auto.export') }}</el-button>
          </el-col>
          <right-toolbar v-model:show-search="showSearch" @query-table="getList"></right-toolbar>
        </el-row>
      </template>

      <el-table
        ref="operLogTableRef"
        v-loading="loading"
        :data="operlogList"
        border
        :default-sort="defaultSort"
        @selection-change="handleSelectionChange"
        @sort-change="handleSortChange"
      >
        <el-table-column type="selection" width="50" align="center" />
        <el-table-column :label="$t('monitor.auto.logID')" align="center" prop="operId" />
        <el-table-column :label="$t('monitor.auto.systemModule')" align="center" prop="title" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('monitor.auto.operationType')" align="center" prop="businessType">
          <template #default="scope">
            <dict-tag :options="sys_oper_type" :value="scope.row.businessType" />
          </template>
        </el-table-column>
        <el-table-column
          :label="$t('monitor.auto.operator')"
          align="center"
          width="110"
          prop="operName"
          :show-overflow-tooltip="true"
          sortable="custom"
          :sort-orders="['descending', 'ascending']"
        />
        <el-table-column :label="$t('monitor.auto.department2')" align="center" prop="deptName" width="130" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('monitor.auto.operationIP')" align="center" prop="operIp" width="130" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('monitor.auto.operationStatus')" align="center" prop="status">
          <template #default="scope">
            <dict-tag :options="sys_common_status" :value="scope.row.status" />
          </template>
        </el-table-column>
        <el-table-column :label="$t('monitor.auto.operationDate')" align="center" prop="operTime" width="180" sortable="custom" :sort-orders="['descending', 'ascending']">
          <template #default="scope">
            <span>{{ proxy.parseTime(scope.row.operTime) }}</span>
          </template>
        </el-table-column>
        <el-table-column
          :label="$t('monitor.auto.elapsed')"
          align="center"
          prop="costTime"
          width="110"
          :show-overflow-tooltip="true"
          sortable="custom"
          :sort-orders="['descending', 'ascending']"
        >
          <template #default="scope">
            <span>{{ scope.row.costTime }}{{ $t('monitor.auto.second') }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="$t('monitor.auto.actions')" fixed="right" align="center" class-name="small-padding fixed-width">
          <template #default="scope">
            <el-tooltip :content="$t('monitor.auto.detail')" placement="top">
              <el-button v-hasPermi="['monitor:operlog:query']" link type="primary" icon="View" @click="handleView(scope.row)"> </el-button>
            </el-tooltip>
          </template>
        </el-table-column>
      </el-table>

      <pagination v-show="total > 0" v-model:page="queryParams.pageNum" v-model:limit="queryParams.pageSize" :total="total" @pagination="getList" />
    </el-card>
    <!-- 操作日志详细 -->
    <OperInfoDialog ref="operInfoDialogRef" />
  </div>
</template>

<script setup name="Operlog" lang="ts">
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
import { list, delOperlog, cleanOperlog } from '@/api/monitor/operlog';
import { OperLogForm, OperLogQuery, OperLogVO } from '@/api/monitor/operlog/types';
import OperInfoDialog from './oper-info-dialog.vue';

const { proxy } = getCurrentInstance() as ComponentInternalInstance;
const { sys_oper_type, sys_common_status } = toRefs<any>(proxy?.useDict('sys_oper_type', 'sys_common_status'));

const operlogList = ref<OperLogVO[]>([]);
const loading = ref(true);
const showSearch = ref(true);
const ids = ref<Array<number | string>>([]);
const multiple = ref(true);
const total = ref(0);
const dateRange = ref<any>(['', '']);
const defaultSort = ref<any>({ prop: 'operTime', order: 'descending' });

const operLogTableRef = ref<ElTableInstance>();
const queryFormRef = ref<ElFormInstance>();

const data = reactive<PageData<OperLogForm, OperLogQuery>>({
  form: {
    operId: undefined,
    tenantId: undefined,
    title: '',
    businessType: 0,
    businessTypes: undefined,
    method: '',
    requestMethod: '',
    operatorType: 0,
    operName: '',
    deptName: '',
    operUrl: '',
    operIp: '',
    operLocation: '',
    operParam: '',
    jsonResult: '',
    status: 0,
    errorMsg: '',
    operTime: '',
    costTime: 0
  },
  queryParams: {
    pageNum: 1,
    pageSize: 10,
    operIp: '',
    title: '',
    operName: '',
    businessType: '',
    status: '',
    orderByColumn: defaultSort.value.prop,
    isAsc: defaultSort.value.order
  },
  rules: {}
});

const { queryParams, form } = toRefs(data);

/** 查询登录日志 */
const getList = async () => {
  loading.value = true;
  const res = await list(proxy?.addDateRange(queryParams.value, dateRange.value));
  operlogList.value = res.rows;
  total.value = res.total;
  loading.value = false;
};
/** 操作日志类型字典翻译 */
const typeFormat = (row: OperLogForm) => {
  return proxy?.selectDictLabel(sys_oper_type.value, row.businessType);
};
/** 搜索按钮操作 */
const handleQuery = () => {
  queryParams.value.pageNum = 1;
  getList();
};
/** 重置按钮操作 */
const resetQuery = () => {
  dateRange.value = ['', ''];
  queryFormRef.value?.resetFields();
  queryParams.value.pageNum = 1;
  operLogTableRef.value?.sort(defaultSort.value.prop, defaultSort.value.order);
};
/** 多选框选中数据 */
const handleSelectionChange = (selection: OperLogVO[]) => {
  ids.value = selection.map((item) => item.operId);
  multiple.value = !selection.length;
};
/** 排序触发事件 */
const handleSortChange = (column: any) => {
  queryParams.value.orderByColumn = column.prop;
  queryParams.value.isAsc = column.order;
  getList();
};

const operInfoDialogRef = ref<InstanceType<typeof OperInfoDialog>>();
/** 详细按钮操作 */
const handleView = (row: OperLogVO) => {
  operInfoDialogRef.value.openDialog(row);
};

/** 删除按钮操作 */
const handleDelete = async (row?: OperLogVO) => {
  const operIds = row?.operId || ids.value;
  await proxy?.$modal.confirm(t('monitor.auto.areYouSureToDeleteLogID') + operIds + t('monitor.auto.text'));
  await delOperlog(operIds);
  await getList();
  proxy?.$modal.msgSuccess(t('monitor.auto.deletedSuccessfully'));
};

/** 清空按钮操作 */
const handleClean = async () => {
  await proxy?.$modal.confirm(t('monitor.auto.areYouSureToClearAllOperationLogs'));
  await cleanOperlog();
  await getList();
  proxy?.$modal.msgSuccess(t('monitor.auto.clearedSuccessfully'));
};

/** 导出按钮操作 */
const handleExport = () => {
  proxy?.download(
    'monitor/operlog/export',
    {
      ...queryParams.value
    },
    `config_${new Date().getTime()}.xlsx`
  );
};
onMounted(() => {
  getList();
});
</script>

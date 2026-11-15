<template>
  <div class="p-2">
    <transition :enter-active-class="proxy?.animate.searchAnimate.enter" :leave-active-class="proxy?.animate.searchAnimate.leave">
      <div v-show="showSearch" class="mb-[10px]">
        <el-card shadow="hover">
          <el-form ref="queryFormRef" :model="queryParams" :inline="true">
            <el-form-item :label="$t('monitor.auto.loginIP')" prop="ipaddr">
              <el-input v-model="queryParams.ipaddr" :placeholder="$t('monitor.auto.pleaseEnterLoginIP')" clearable @keyup.enter="handleQuery" />
            </el-form-item>
            <el-form-item :label="$t('monitor.auto.userName')" prop="userName">
              <el-input v-model="queryParams.userName" :placeholder="$t('monitor.auto.pleaseEnterUserName')" clearable @keyup.enter="handleQuery" />
            </el-form-item>
            <el-form-item :label="$t('monitor.auto.status')" prop="status">
              <el-select v-model="queryParams.status" :placeholder="$t('monitor.auto.loginStatus')" clearable>
                <el-option v-for="dict in sys_common_status" :key="dict.value" :label="dict.label" :value="dict.value" />
              </el-select>
            </el-form-item>
            <el-form-item :label="$t('monitor.auto.loginTime')" style="width: 308px">
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
            <el-button v-hasPermi="['monitor:logininfor:remove']" type="danger" plain icon="Delete" :disabled="multiple" @click="handleDelete()">
              {{ $t('monitor.auto.delete') }}
            </el-button>
          </el-col>
          <el-col :span="1.5">
            <el-button v-hasPermi="['monitor:logininfor:remove']" type="danger" plain icon="Delete" @click="handleClean">{{ $t('monitor.auto.clear') }}</el-button>
          </el-col>
          <el-col :span="1.5">
            <el-button v-hasPermi="['monitor:logininfor:unlock']" type="primary" plain icon="Unlock" :disabled="single" @click="handleUnlock">
              {{ $t('monitor.auto.text4') }}
            </el-button>
          </el-col>
          <el-col :span="1.5">
            <el-button v-hasPermi="['monitor:logininfor:export']" type="warning" plain icon="Download" @click="handleExport">{{ $t('monitor.auto.export') }}</el-button>
          </el-col>
          <right-toolbar v-model:show-search="showSearch" @query-table="getList"></right-toolbar>
        </el-row>
      </template>

      <el-table
        ref="loginInfoTableRef"
        v-loading="loading"
        :data="loginInfoList"
        :default-sort="defaultSort"
        border
        @selection-change="handleSelectionChange"
        @sort-change="handleSortChange"
      >
        <el-table-column type="selection" width="55" align="center" />
        <el-table-column :label="$t('monitor.auto.visitID')" align="center" prop="infoId" />
        <el-table-column
          :label="$t('monitor.auto.userName')"
          align="center"
          prop="userName"
          :show-overflow-tooltip="true"
          sortable="custom"
          :sort-orders="['descending', 'ascending']"
        />
        <el-table-column :label="$t('monitor.auto.client')" align="center" prop="clientKey" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('monitor.auto.deviceType')" align="center">
          <template #default="scope">
            <dict-tag :options="sys_device_type" :value="scope.row.deviceType" />
          </template>
        </el-table-column>
        <el-table-column :label="$t('monitor.auto.address')" align="center" prop="ipaddr" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('monitor.auto.loginLocation')" align="center" prop="loginLocation" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('monitor.auto.os')" align="center" prop="os" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('monitor.auto.browser')" align="center" prop="browser" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('monitor.auto.loginStatus')" align="center" prop="status">
          <template #default="scope">
            <dict-tag :options="sys_common_status" :value="scope.row.status" />
          </template>
        </el-table-column>
        <el-table-column :label="$t('monitor.auto.description')" align="center" prop="msg" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('monitor.auto.visitTime')" align="center" prop="loginTime" sortable="custom" :sort-orders="['descending', 'ascending']" width="180">
          <template #default="scope">
            <span>{{ proxy.parseTime(scope.row.loginTime) }}</span>
          </template>
        </el-table-column>
      </el-table>

      <pagination v-show="total > 0" v-model:page="queryParams.pageNum" v-model:limit="queryParams.pageSize" :total="total" @pagination="getList" />
    </el-card>
  </div>
</template>

<script setup name="Logininfor" lang="ts">
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
import { list, delLoginInfo, cleanLoginInfo, unlockLoginInfo } from '@/api/monitor/loginInfo';
import { LoginInfoQuery, LoginInfoVO } from '@/api/monitor/loginInfo/types';

const { proxy } = getCurrentInstance() as ComponentInternalInstance;
const { sys_device_type } = toRefs<any>(proxy?.useDict('sys_device_type'));
const { sys_common_status } = toRefs<any>(proxy?.useDict('sys_common_status'));

const loginInfoList = ref<LoginInfoVO[]>([]);
const loading = ref(true);
const showSearch = ref(true);
const ids = ref<Array<number | string>>([]);
const single = ref(true);
const multiple = ref(true);
const selectName = ref<Array<string>>([]);
const total = ref(0);
const dateRange = ref<any>(['', '']);
const defaultSort = ref<any>({ prop: 'loginTime', order: 'descending' });

const queryFormRef = ref<ElFormInstance>();
const loginInfoTableRef = ref<ElTableInstance>();
// 查询参数
const queryParams = ref<LoginInfoQuery>({
  pageNum: 1,
  pageSize: 10,
  ipaddr: '',
  userName: '',
  status: '',
  orderByColumn: defaultSort.value.prop,
  isAsc: defaultSort.value.order
});

/** 查询登录日志列表 */
const getList = async () => {
  loading.value = true;
  const res = await list(proxy?.addDateRange(queryParams.value, dateRange.value));
  loginInfoList.value = res.rows;
  total.value = res.total;
  loading.value = false;
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
  loginInfoTableRef.value?.sort(defaultSort.value.prop, defaultSort.value.order);
};
/** 多选框选中数据 */
const handleSelectionChange = (selection: LoginInfoVO[]) => {
  ids.value = selection.map((item) => item.infoId);
  multiple.value = !selection.length;
  single.value = selection.length != 1;
  selectName.value = selection.map((item) => item.userName);
};
/** 排序触发事件 */
const handleSortChange = (column: any) => {
  queryParams.value.orderByColumn = column.prop;
  queryParams.value.isAsc = column.order;
  getList();
};
/** 删除按钮操作 */
const handleDelete = async (row?: LoginInfoVO) => {
  const infoIds = row?.infoId || ids.value;
  await proxy?.$modal.confirm(t('monitor.auto.areYouSureToDeleteVisitID') + infoIds + t('monitor.auto.text'));
  await delLoginInfo(infoIds);
  await getList();
  proxy?.$modal.msgSuccess(t('monitor.auto.deletedSuccessfully'));
};
/** 清空按钮操作 */
const handleClean = async () => {
  await proxy?.$modal.confirm(t('monitor.auto.areYouSureToClearAllLoginLogs'));
  await cleanLoginInfo();
  await getList();
  proxy?.$modal.msgSuccess(t('monitor.auto.clearedSuccessfully'));
};
/** 解锁按钮操作 */
const handleUnlock = async () => {
  const username = selectName.value;
  await proxy?.$modal.confirm(t('monitor.auto.areYouSureToUnlockUser') + username + t('monitor.auto.text2'));
  await unlockLoginInfo(username);
  proxy?.$modal.msgSuccess(t('monitor.auto.user') + username + t('monitor.auto.unlockedSuccessfully'));
};
/** 导出按钮操作 */
const handleExport = () => {
  proxy?.download(
    'monitor/logininfor/export',
    {
      ...queryParams.value
    },
    `logininfor_${new Date().getTime()}.xlsx`
  );
};

onMounted(() => {
  getList();
});
</script>

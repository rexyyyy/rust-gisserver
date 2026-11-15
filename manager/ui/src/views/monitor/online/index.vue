<template>
  <div class="p-2">
    <div class="mb-[10px]">
      <el-card shadow="hover">
        <el-form ref="queryFormRef" :model="queryParams" :inline="true">
          <el-form-item :label="$t('monitor.auto.loginIP')" prop="ipaddr">
            <el-input v-model="queryParams.ipaddr" :placeholder="$t('monitor.auto.pleaseEnterLoginIP')" clearable @keyup.enter="handleQuery" />
          </el-form-item>
          <el-form-item :label="$t('monitor.auto.userName')" prop="userName">
            <el-input v-model="queryParams.userName" :placeholder="$t('monitor.auto.pleaseEnterUserName')" clearable @keyup.enter="handleQuery" />
          </el-form-item>
          <el-form-item>
            <el-button type="primary" icon="Search" @click="handleQuery">{{ $t('monitor.auto.search') }}</el-button>
            <el-button icon="Refresh" @click="resetQuery">{{ $t('monitor.auto.reset') }}</el-button>
          </el-form-item>
        </el-form>
      </el-card>
    </div>
    <el-card shadow="hover">
      <el-table
        v-loading="loading"
        border
        :data="onlineList.slice((queryParams.pageNum - 1) * queryParams.pageSize, queryParams.pageNum * queryParams.pageSize)"
        style="width: 100%"
      >
        <el-table-column :label="$t('monitor.auto.no')" width="50" type="index" align="center">
          <template #default="scope">
            <span>{{ (queryParams.pageNum - 1) * queryParams.pageSize + scope.$index + 1 }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="$t('monitor.auto.sessionID')" align="center" prop="tokenId" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('monitor.auto.loginName')" align="center" prop="userName" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('monitor.auto.client')" align="center" prop="clientKey" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('monitor.auto.deviceType')" align="center">
          <template #default="scope">
            <dict-tag :options="sys_device_type" :value="scope.row.deviceType" />
          </template>
        </el-table-column>
        <el-table-column :label="$t('monitor.auto.department')" align="center" prop="deptName" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('monitor.auto.host')" align="center" prop="ipaddr" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('monitor.auto.loginLocation')" align="center" prop="loginLocation" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('monitor.auto.os')" align="center" prop="os" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('monitor.auto.browser')" align="center" prop="browser" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('monitor.auto.loginTime')" align="center" prop="loginTime" width="180">
          <template #default="scope">
            <span>{{ proxy.parseTime(scope.row.loginTime) }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="$t('monitor.auto.actions')" align="center" class-name="small-padding fixed-width">
          <template #default="scope">
            <el-tooltip :content="$t('monitor.auto.forceLogout')" placement="top">
              <el-button v-hasPermi="['monitor:online:forceLogout']" link type="primary" icon="Delete" @click="handleForceLogout(scope.row)">
              </el-button>
            </el-tooltip>
          </template>
        </el-table-column>
      </el-table>

      <pagination v-show="total > 0" v-model:page="queryParams.pageNum" v-model:limit="queryParams.pageSize" :total="total" />
    </el-card>
  </div>
</template>

<script setup name="Online" lang="ts">
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
import { forceLogout, list as initData } from '@/api/monitor/online';
import { OnlineQuery, OnlineVO } from '@/api/monitor/online/types';
import api from '@/api/system/user';
import { to } from 'await-to-js';

const { proxy } = getCurrentInstance() as ComponentInternalInstance;
const { sys_device_type } = toRefs<any>(proxy?.useDict('sys_device_type'));

const onlineList = ref<OnlineVO[]>([]);
const loading = ref(true);
const total = ref(0);

const queryFormRef = ref<ElFormInstance>();

const queryParams = ref<OnlineQuery>({
  pageNum: 1,
  pageSize: 10,
  ipaddr: '',
  userName: ''
});

/** 查询登录日志列表 */
const getList = async () => {
  loading.value = true;
  const res = await initData(queryParams.value);
  onlineList.value = res.rows;
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
  queryFormRef.value?.resetFields();
  handleQuery();
};
/** 强退按钮操作 */
const handleForceLogout = async (row: OnlineVO) => {
  const [err] = await to(proxy?.$modal.confirm(t('monitor.auto.areYouSureToForceLogout') + row.userName + t('monitor.auto.text3')) as any);
  if (!err) {
    await forceLogout(row.tokenId);
    await getList();
    proxy?.$modal.msgSuccess(t('monitor.auto.deletedSuccessfully'));
  }
};

onMounted(() => {
  getList();
});
</script>

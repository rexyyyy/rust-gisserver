<template>
  <div class="p-2">
    <transition :enter-active-class="proxy?.animate.searchAnimate.enter" :leave-active-class="proxy?.animate.searchAnimate.leave">
      <div v-show="showSearch" class="mb-[10px]">
        <el-card shadow="hover">
          <el-form ref="queryFormRef" :model="queryParams" :inline="true">
            <el-form-item :label="$t('system.auto.roleName')" prop="roleName">
              <el-input v-model="queryParams.roleName" :placeholder="$t('system.auto.pleaseEnterRoleName')" clearable @keyup.enter="handleQuery" />
            </el-form-item>
            <el-form-item :label="$t('system.auto.permissionKey2')" prop="roleKey">
              <el-input v-model="queryParams.roleKey" :placeholder="$t('system.auto.pleaseEnterPermissionKey2')" clearable @keyup.enter="handleQuery" />
            </el-form-item>
            <el-form-item :label="$t('system.auto.status')" prop="status">
              <el-select v-model="queryParams.status" :placeholder="$t('system.auto.roleStatus')" clearable>
                <el-option v-for="dict in sys_normal_disable" :key="dict.value" :label="dict.label" :value="dict.value" />
              </el-select>
            </el-form-item>
            <el-form-item :label="$t('system.auto.createdAt')" style="width: 308px">
              <el-date-picker
                v-model="dateRange"
                value-format="YYYY-MM-DD HH:mm:ss"
                type="daterange"
                range-separator="-"
                :start-placeholder="$t('system.auto.startDate')"
                :end-placeholder="$t('system.auto.endDate')"
                :default-time="[new Date(2000, 1, 1, 0, 0, 0), new Date(2000, 1, 1, 23, 59, 59)]"
              ></el-date-picker>
            </el-form-item>

            <el-form-item>
              <el-button type="primary" icon="Search" @click="handleQuery">{{ $t('system.auto.search') }}</el-button>
              <el-button icon="Refresh" @click="resetQuery">{{ $t('system.auto.reset') }}</el-button>
            </el-form-item>
          </el-form>
        </el-card>
      </div>
    </transition>

    <el-card shadow="hover">
      <template #header>
        <el-row :gutter="10">
          <el-col :span="1.5">
            <el-button v-hasPermi="['system:role:add']" type="primary" plain icon="Plus" @click="handleAdd()">{{ $t('system.auto.add') }}</el-button>
          </el-col>
          <el-col :span="1.5">
            <el-button v-hasPermi="['system:role:edit']" type="success" plain :disabled="single" icon="Edit" @click="handleUpdate()">{{ $t('system.auto.edit') }}</el-button>
          </el-col>
          <el-col :span="1.5">
            <el-button v-hasPermi="['system:role:remove']" type="danger" plain :disabled="ids.length === 0" @click="handleDelete()">{{ $t('system.auto.delete') }}</el-button>
          </el-col>
          <el-col :span="1.5">
            <el-button v-hasPermi="['system:role:export']" type="warning" plain icon="Download" @click="handleExport">{{ $t('system.auto.export') }}</el-button>
          </el-col>
          <right-toolbar v-model:show-search="showSearch" @query-table="getList"></right-toolbar>
        </el-row>
      </template>

      <el-table ref="roleTableRef" border v-loading="loading" :data="roleList" @selection-change="handleSelectionChange">
        <el-table-column type="selection" width="55" align="center" />
        <el-table-column v-if="false" :label="$t('system.auto.roleID')" prop="roleId" width="120" />
        <el-table-column :label="$t('system.auto.roleName')" prop="roleName" :show-overflow-tooltip="true" width="150" />
        <el-table-column :label="$t('system.auto.permissionKey2')" prop="roleKey" :show-overflow-tooltip="true" width="200" />
        <el-table-column :label="$t('system.auto.displayOrder2')" prop="roleSort" width="100" />
        <el-table-column :label="$t('system.auto.status')" align="center" width="100">
          <template #default="scope">
            <el-switch v-model="scope.row.status" active-value="0" inactive-value="1" @change="handleStatusChange(scope.row)"></el-switch>
          </template>
        </el-table-column>
        <el-table-column :label="$t('system.auto.createdAt')" align="center" prop="createTime">
          <template #default="scope">
            <span>{{ proxy.parseTime(scope.row.createTime) }}</span>
          </template>
        </el-table-column>

        <el-table-column fixed="right" :label="$t('system.auto.actions')" width="180">
          <template #default="scope">
            <el-tooltip v-if="scope.row.roleId !== 1" :content="$t('system.auto.edit')" placement="top">
              <el-button v-hasPermi="['system:role:edit']" link type="primary" icon="Edit" @click="handleUpdate(scope.row)"></el-button>
            </el-tooltip>
            <el-tooltip v-if="scope.row.roleId !== 1" :content="$t('system.auto.delete')" placement="top">
              <el-button v-hasPermi="['system:role:remove']" link type="primary" icon="Delete" @click="handleDelete(scope.row)"></el-button>
            </el-tooltip>
            <el-tooltip v-if="scope.row.roleId !== 1" :content="$t('system.auto.dataPermission')" placement="top">
              <el-button v-hasPermi="['system:role:edit']" link type="primary" icon="CircleCheck" @click="handleDataScope(scope.row)"></el-button>
            </el-tooltip>
            <el-tooltip v-if="scope.row.roleId !== 1" :content="$t('system.auto.assignUsers')" placement="top">
              <el-button v-hasPermi="['system:role:edit']" link type="primary" icon="User" @click="handleAuthUser(scope.row)"></el-button>
            </el-tooltip>
          </template>
        </el-table-column>
      </el-table>

      <pagination
        v-if="total > 0"
        v-model:total="total"
        v-model:page="queryParams.pageNum"
        v-model:limit="queryParams.pageSize"
        @pagination="getList"
      />
    </el-card>

    <el-dialog v-model="dialog.visible" :title="dialog.title" width="500px" append-to-body>
      <el-form ref="roleFormRef" :model="form" :rules="rules" label-width="100px">
        <el-form-item :label="$t('system.auto.roleName')" prop="roleName">
          <el-input v-model="form.roleName" :placeholder="$t('system.auto.pleaseEnterRoleName')" />
        </el-form-item>
        <el-form-item prop="roleKey">
          <template #label>
            <span>
              <el-tooltip :content="$t('system.auto.roleKeyDefinedInTheController')" placement="top">
                <el-icon><question-filled /></el-icon>
              </el-tooltip>
              {{ $t('system.auto.permissionKey2') }}
            </span>
          </template>
          <el-input v-model="form.roleKey" :placeholder="$t('system.auto.pleaseEnterPermissionKey2')" />
        </el-form-item>
        <el-form-item :label="$t('system.auto.roleOrder')" prop="roleSort">
          <el-input-number v-model="form.roleSort" controls-position="right" :min="0" />
        </el-form-item>
        <el-form-item :label="$t('system.auto.status')">
          <el-radio-group v-model="form.status">
            <el-radio v-for="dict in sys_normal_disable" :key="dict.value" :value="dict.value">{{ dict.label }}</el-radio>
          </el-radio-group>
        </el-form-item>
        <el-form-item :label="$t('system.auto.menuPermission')">
          <el-checkbox v-model="menuExpand" @change="handleCheckedTreeExpand($event as any, 'menu')">{{ $t('system.auto.expandCollapse') }}</el-checkbox>
          <el-checkbox v-model="menuNodeAll" @change="handleCheckedTreeNodeAll($event as any, 'menu')">{{ $t('system.auto.selectAllNone') }}</el-checkbox>
          <el-checkbox v-model="form.menuCheckStrictly" @change="handleCheckedTreeConnect($event as any, 'menu')">{{ $t('system.auto.parentChildLink') }}</el-checkbox>
          <el-tree
            ref="menuRef"
            class="tree-border"
            :data="menuOptions"
            show-checkbox
            node-key="id"
            :check-strictly="!form.menuCheckStrictly"
            :empty-text="$t('system.auto.loadingPleaseWait')"
            :props="{ label: 'label', children: 'children' } as any"
          ></el-tree>
        </el-form-item>
        <el-form-item :label="$t('system.auto.remark')">
          <el-input v-model="form.remark" type="textarea" :placeholder="$t('system.auto.pleaseEnterAValue')"></el-input>
        </el-form-item>
      </el-form>
      <template #footer>
        <div class="dialog-footer">
          <el-button type="primary" @click="submitForm">{{ $t('system.auto.ok') }}</el-button>
          <el-button @click="cancel">{{ $t('system.auto.cancel') }}</el-button>
        </div>
      </template>
    </el-dialog>

    <!-- 分配角色数据权限对话框 -->
    <el-dialog v-model="openDataScope" :title="dialog.title" width="500px" append-to-body>
      <el-form ref="dataScopeRef" :model="form" label-width="80px">
        <el-form-item :label="$t('system.auto.roleName')">
          <el-input v-model="form.roleName" :disabled="true" />
        </el-form-item>
        <el-form-item :label="$t('system.auto.permissionKey2')">
          <el-input v-model="form.roleKey" :disabled="true" />
        </el-form-item>
        <el-form-item :label="$t('system.auto.permissionScope')">
          <el-select v-model="form.dataScope" @change="dataScopeSelectChange">
            <el-option v-for="item in dataScopeOptions" :key="item.value" :label="item.label" :value="item.value"></el-option>
          </el-select>
        </el-form-item>
        <el-form-item v-show="form.dataScope === '2'" :label="$t('system.auto.dataPermission')">
          <el-checkbox v-model="deptExpand" @change="handleCheckedTreeExpand($event as any, 'dept')">{{ $t('system.auto.expandCollapse') }}</el-checkbox>
          <el-checkbox v-model="deptNodeAll" @change="handleCheckedTreeNodeAll($event as any, 'dept')">{{ $t('system.auto.selectAllNone') }}</el-checkbox>
          <el-checkbox v-model="form.deptCheckStrictly" @change="handleCheckedTreeConnect($event as any, 'dept')">{{ $t('system.auto.parentChildLink') }}</el-checkbox>
          <el-tree
            ref="deptRef"
            class="tree-border"
            :data="deptOptions"
            show-checkbox
            default-expand-all
            node-key="id"
            :check-strictly="!form.deptCheckStrictly"
            :empty-text="$t('system.auto.loadingPleaseWait')"
            :props="{ label: 'label', children: 'children' } as any"
          ></el-tree>
        </el-form-item>
      </el-form>
      <template #footer>
        <div class="dialog-footer">
          <el-button type="primary" @click="submitDataScope">{{ $t('system.auto.ok') }}</el-button>
          <el-button @click="cancelDataScope">{{ $t('system.auto.cancel') }}</el-button>
        </div>
      </template>
    </el-dialog>
  </div>
</template>

<script setup name="Role" lang="ts">
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
import { addRole, changeRoleStatus, dataScope, delRole, getRole, listRole, updateRole, deptTreeSelect } from '@/api/system/role';
import { roleMenuTreeselect, treeselect as menuTreeselect } from '@/api/system/menu/index';
import { RoleVO, RoleForm, RoleQuery, DeptTreeOption } from '@/api/system/role/types';
import { MenuTreeOption, RoleMenuTree } from '@/api/system/menu/types';

const router = useRouter();
const { proxy } = getCurrentInstance() as ComponentInternalInstance;
const { sys_normal_disable } = toRefs<any>(proxy?.useDict('sys_normal_disable'));

const roleList = ref<RoleVO[]>();
const loading = ref(true);
const showSearch = ref(true);
const ids = ref<Array<string | number>>([]);
const single = ref(true);
const multiple = ref(true);
const total = ref(0);
const dateRange = ref<any>(['', '']);
const menuOptions = ref<MenuTreeOption[]>([]);
const menuExpand = ref(false);
const menuNodeAll = ref(false);
const deptExpand = ref(true);
const deptNodeAll = ref(false);
const deptOptions = ref<DeptTreeOption[]>([]);
const openDataScope = ref(false);

/** 数据范围选项*/
const dataScopeOptions = ref([
  { value: '1', label: t('system.auto.allData') },
  { value: '2', label: t('system.auto.customData') },
  { value: '3', label: t('system.auto.departmentData') },
  { value: '4', label: t('system.auto.departmentAndBelow') },
  { value: '5', label: t('system.auto.ownDataOnly') },
  { value: '6', label: t('system.auto.departmentBelowOrOwn') }
]);

const queryFormRef = ref<ElFormInstance>();
const roleFormRef = ref<ElFormInstance>();
const dataScopeRef = ref<ElFormInstance>();
const menuRef = ref<ElTreeInstance>();
const deptRef = ref<ElTreeInstance>();

const initForm: RoleForm = {
  roleId: undefined,
  roleSort: 1,
  status: '0',
  roleName: '',
  roleKey: '',
  menuCheckStrictly: true,
  deptCheckStrictly: true,
  remark: '',
  dataScope: '1',
  menuIds: [],
  deptIds: []
};

const data = reactive<PageData<RoleForm, RoleQuery>>({
  form: { ...initForm },
  queryParams: {
    pageNum: 1,
    pageSize: 10,
    roleName: '',
    roleKey: '',
    status: ''
  },
  rules: {
    roleName: [{ required: true, message: t('system.auto.roleNameisRequired'), trigger: 'blur' }],
    roleKey: [{ required: true, message: t('system.auto.permissionKeyisRequired'), trigger: 'blur' }],
    roleSort: [{ required: true, message: t('system.auto.roleOrderisRequired'), trigger: 'blur' }]
  }
});
const { form, queryParams, rules } = toRefs(data);

const dialog = reactive<DialogOption>({
  visible: false,
  title: ''
});

/**
 * 查询角色列表
 */
const getList = () => {
  loading.value = true;
  listRole(proxy?.addDateRange(queryParams.value, dateRange.value)).then((res) => {
    roleList.value = res.rows;
    total.value = res.total;
    loading.value = false;
  });
};

/**
 * 搜索按钮操作
 */
const handleQuery = () => {
  queryParams.value.pageNum = 1;
  getList();
};

/** 重置 */
const resetQuery = () => {
  dateRange.value = ['', ''];
  queryFormRef.value?.resetFields();
  handleQuery();
};
/**删除按钮操作 */
const handleDelete = async (row?: RoleVO) => {
  const roleids = row?.roleId || ids.value;
  await proxy?.$modal.confirm(t('system.auto.areYouSureToDeleteRoleID') + roleids + t('system.auto.dataItem'));
  await delRole(roleids);
  getList();
  proxy?.$modal.msgSuccess(t('system.auto.deletedSuccessfully'));
};

/** 导出按钮操作 */
const handleExport = () => {
  proxy?.download(
    'system/role/export',
    {
      ...queryParams.value
    },
    `role_${new Date().getTime()}.xlsx`
  );
};
/** 多选框选中数据 */
const handleSelectionChange = (selection: RoleVO[]) => {
  ids.value = selection.map((item: RoleVO) => item.roleId);
  single.value = selection.length != 1;
  multiple.value = !selection.length;
};

/** 角色状态修改 */
const handleStatusChange = async (row: RoleVO) => {
  const text = row.status === '0' ? t('system.auto.enable') : t('system.auto.disable');
  try {
    await proxy?.$modal.confirm(t('system.auto.areYouSureTo') + text + '""' + row.roleName + t('system.auto.text4'));
    await changeRoleStatus(row.roleId, row.status);
    proxy?.$modal.msgSuccess(text + t('system.auto.success'));
  } catch {
    row.status = row.status === '0' ? '1' : '0';
  }
};

/** 分配用户 */
const handleAuthUser = (row: RoleVO) => {
  router.push('/system/role-auth/user/' + row.roleId);
};

/** 查询菜单树结构 */
const getMenuTreeselect = async () => {
  const res = await menuTreeselect();
  menuOptions.value = res.data;
};
/** 所有部门节点数据 */
const getDeptAllCheckedKeys = (): any => {
  // 目前被选中的部门节点
  const checkedKeys = deptRef.value?.getCheckedKeys();
  // 半选中的部门节点
  const halfCheckedKeys = deptRef.value?.getHalfCheckedKeys();
  if (halfCheckedKeys) {
    checkedKeys?.unshift(...halfCheckedKeys);
  }
  return checkedKeys;
};
/** 重置新增的表单以及其他数据  */
const reset = () => {
  menuRef.value?.setCheckedKeys([]);
  menuExpand.value = false;
  menuNodeAll.value = false;
  deptExpand.value = true;
  deptNodeAll.value = false;
  form.value = { ...initForm };
  roleFormRef.value?.resetFields();
};

/** 添加角色 */
const handleAdd = () => {
  reset();
  getMenuTreeselect();
  dialog.visible = true;
  dialog.title = t('system.auto.addRole');
};
/** 修改角色 */
const handleUpdate = async (row?: RoleVO) => {
  reset();
  const roleId = row?.roleId || ids.value[0];
  const { data } = await getRole(roleId);
  Object.assign(form.value, data);
  form.value.roleSort = Number(form.value.roleSort);
  const res = await getRoleMenuTreeselect(roleId);
  dialog.title = t('system.auto.editRole');
  dialog.visible = true;
  res.checkedKeys.forEach((v) => {
    nextTick(() => {
      menuRef.value?.setChecked(v, true, false);
    });
  });
};
/** 根据角色ID查询菜单树结构 */
const getRoleMenuTreeselect = (roleId: string | number) => {
  return roleMenuTreeselect(roleId).then((res): RoleMenuTree => {
    menuOptions.value = res.data.menus;
    return res.data;
  });
};
/** 根据角色ID查询部门树结构 */
const getRoleDeptTreeSelect = async (roleId: string | number) => {
  const res = await deptTreeSelect(roleId);
  deptOptions.value = res.data.depts;
  return res.data;
};
/** 树权限（展开/折叠）*/
const handleCheckedTreeExpand = (value: boolean, type: string) => {
  if (type == 'menu') {
    const treeList = menuOptions.value;
    for (let i = 0; i < treeList.length; i++) {
      if (menuRef.value) {
        menuRef.value.store.nodesMap[treeList[i].id].expanded = value;
      }
    }
  } else if (type == 'dept') {
    const treeList = deptOptions.value;
    for (let i = 0; i < treeList.length; i++) {
      if (deptRef.value) {
        deptRef.value.store.nodesMap[treeList[i].id].expanded = value;
      }
    }
  }
};
/** 树权限（全选/全不选） */
const handleCheckedTreeNodeAll = (value: any, type: string) => {
  if (type == 'menu') {
    menuRef.value?.setCheckedNodes(value ? (menuOptions.value as any) : []);
  } else if (type == 'dept') {
    deptRef.value?.setCheckedNodes(value ? (deptOptions.value as any) : []);
  }
};
/** 树权限（父子联动） */
const handleCheckedTreeConnect = (value: any, type: string) => {
  if (type == 'menu') {
    form.value.menuCheckStrictly = value;
  } else if (type == 'dept') {
    form.value.deptCheckStrictly = value;
  }
};
/** 所有菜单节点数据 */
const getMenuAllCheckedKeys = (): any => {
  // 目前被选中的菜单节点
  const checkedKeys = menuRef.value?.getCheckedKeys();
  // 半选中的菜单节点
  const halfCheckedKeys = menuRef.value?.getHalfCheckedKeys();
  if (halfCheckedKeys) {
    checkedKeys?.unshift(...halfCheckedKeys);
  }
  return checkedKeys;
};
/** 提交按钮 */
const submitForm = () => {
  roleFormRef.value?.validate(async (valid: boolean) => {
    if (valid) {
      form.value.menuIds = getMenuAllCheckedKeys();
      form.value.roleId ? await updateRole(form.value) : await addRole(form.value);
      proxy?.$modal.msgSuccess(t('system.auto.operationSuccessful'));
      dialog.visible = false;
      getList();
    }
  });
};
/** 取消按钮 */
const cancel = () => {
  reset();
  dialog.visible = false;
};
/** 选择角色权限范围触发 */
const dataScopeSelectChange = (value: string) => {
  if (value !== '2') {
    deptRef.value?.setCheckedKeys([]);
  }
};
/** 分配数据权限操作 */
const handleDataScope = async (row: RoleVO) => {
  const response = await getRole(row.roleId);
  Object.assign(form.value, response.data);
  const res = await getRoleDeptTreeSelect(row.roleId);
  openDataScope.value = true;
  dialog.title = t('system.auto.assignDataPermissions');
  await nextTick(() => {
    deptRef.value?.setCheckedKeys(res.checkedKeys);
  });
};
/** 提交按钮（数据权限） */
const submitDataScope = async () => {
  if (form.value.roleId) {
    form.value.deptIds = getDeptAllCheckedKeys();
    await dataScope(form.value);
    proxy?.$modal.msgSuccess(t('system.auto.updatedSuccessfully'));
    openDataScope.value = false;
    getList();
  }
};
/** 取消按钮（数据权限）*/
const cancelDataScope = () => {
  dataScopeRef.value?.resetFields();
  form.value = { ...initForm };
  openDataScope.value = false;
};

onMounted(() => {
  getList();
});
</script>

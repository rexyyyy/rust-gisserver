<template>
  <div class="p-2">
    <el-row :gutter="20">
      <!-- 部门树 -->
      <el-col :lg="4" :xs="24" style="">
        <el-card shadow="hover">
          <el-input v-model="deptName" :placeholder="$t('system.auto.pleaseEnterDepartmentName')" prefix-icon="Search" clearable />
          <el-tree
            ref="deptTreeRef"
            class="mt-2"
            node-key="id"
            :data="deptOptions"
            :props="{ label: 'label', children: 'children' } as any"
            :expand-on-click-node="false"
            :filter-node-method="filterNode"
            highlight-current
            default-expand-all
            @node-click="handleNodeClick"
          />
        </el-card>
      </el-col>
      <el-col :lg="20" :xs="24">
        <transition :enter-active-class="proxy?.animate.searchAnimate.enter" :leave-active-class="proxy?.animate.searchAnimate.leave">
          <div v-show="showSearch" class="mb-[10px]">
            <el-card shadow="hover">
              <el-form ref="queryFormRef" :model="queryParams" :inline="true">
                <el-form-item :label="$t('system.auto.postCode')" prop="postCode">
                  <el-input v-model="queryParams.postCode" :placeholder="$t('system.auto.pleaseEnterPostCode')" clearable @keyup.enter="handleQuery" />
                </el-form-item>
                <el-form-item :label="$t('system.auto.categoryCode')" prop="postCategory">
                  <el-input
                    v-model="queryParams.postCategory"
                    :placeholder="$t('system.auto.pleaseEnterCategoryCode')"
                    clearable
                    style="width: 200px"
                    @keyup.enter="handleQuery"
                  />
                </el-form-item>
                <el-form-item :label="$t('system.auto.postName')" prop="postName">
                  <el-input v-model="queryParams.postName" :placeholder="$t('system.auto.pleaseEnterPostName')" clearable @keyup.enter="handleQuery" />
                </el-form-item>
                <el-form-item :label="$t('system.auto.department')" prop="deptId">
                  <el-tree-select
                    v-model="queryParams.deptId"
                    :data="deptOptions"
                    :props="{ value: 'id', label: 'label', children: 'children' } as any"
                    value-key="id"
                    :placeholder="$t('system.auto.pleaseSelectADepartment')"
                    check-strictly
                  />
                </el-form-item>
                <el-form-item :label="$t('system.auto.status')" prop="status">
                  <el-select v-model="queryParams.status" :placeholder="$t('system.auto.postStatus')" clearable>
                    <el-option v-for="dict in sys_normal_disable" :key="dict.value" :label="dict.label" :value="dict.value" />
                  </el-select>
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
            <el-row :gutter="10" class="mb8">
              <el-col :span="1.5">
                <el-button v-hasPermi="['system:post:add']" type="primary" plain icon="Plus" @click="handleAdd">{{ $t('system.auto.add') }}</el-button>
              </el-col>
              <el-col :span="1.5">
                <el-button v-hasPermi="['system:post:edit']" type="success" plain icon="Edit" :disabled="single" @click="handleUpdate()"
                  >{{ $t('system.auto.edit') }}</el-button
                >
              </el-col>
              <el-col :span="1.5">
                <el-button v-hasPermi="['system:post:remove']" type="danger" plain icon="Delete" :disabled="multiple" @click="handleDelete()">
                  {{ $t('system.auto.delete') }}
                </el-button>
              </el-col>
              <el-col :span="1.5">
                <el-button v-hasPermi="['system:post:export']" type="warning" plain icon="Download" @click="handleExport">{{ $t('system.auto.export') }}</el-button>
              </el-col>
              <right-toolbar v-model:show-search="showSearch" @query-table="getList"></right-toolbar>
            </el-row>
          </template>
          <el-table v-loading="loading" border :data="postList" @selection-change="handleSelectionChange">
            <el-table-column type="selection" width="55" align="center" />
            <el-table-column v-if="false" :label="$t('system.auto.postID')" align="center" prop="postId" />
            <el-table-column :label="$t('system.auto.postCode')" align="center" prop="postCode" />
            <el-table-column :label="$t('system.auto.categoryCode')" align="center" prop="postCategory" />
            <el-table-column :label="$t('system.auto.postName')" align="center" prop="postName" />
            <el-table-column :label="$t('system.auto.department')" align="center" prop="deptName" />
            <el-table-column :label="$t('system.auto.order')" align="center" prop="postSort" />
            <el-table-column :label="$t('system.auto.status')" align="center" prop="status">
              <template #default="scope">
                <dict-tag :options="sys_normal_disable" :value="scope.row.status" />
              </template>
            </el-table-column>
            <el-table-column :label="$t('system.auto.createdAt')" align="center" prop="createTime" width="180">
              <template #default="scope">
                <span>{{ proxy.parseTime(scope.row.createTime) }}</span>
              </template>
            </el-table-column>
            <el-table-column :label="$t('system.auto.actions')" width="180" align="center" class-name="small-padding fixed-width">
              <template #default="scope">
                <el-tooltip :content="$t('system.auto.edit')" placement="top">
                  <el-button v-hasPermi="['system:post:edit']" link type="primary" icon="Edit" @click="handleUpdate(scope.row)"></el-button>
                </el-tooltip>
                <el-tooltip :content="$t('system.auto.delete')" placement="top">
                  <el-button v-hasPermi="['system:post:remove']" link type="primary" icon="Delete" @click="handleDelete(scope.row)"></el-button>
                </el-tooltip>
              </template>
            </el-table-column>
          </el-table>

          <pagination
            v-show="total > 0"
            v-model:page="queryParams.pageNum"
            v-model:limit="queryParams.pageSize"
            :total="total"
            @pagination="getList"
          />
        </el-card>

        <!-- 添加或修改岗位对话框 -->
        <el-dialog v-model="dialog.visible" :title="dialog.title" width="500px" append-to-body>
          <el-form ref="postFormRef" :model="form" :rules="rules" label-width="80px">
            <el-form-item :label="$t('system.auto.postName')" prop="postName">
              <el-input v-model="form.postName" :placeholder="$t('system.auto.pleaseEnterPostName')" />
            </el-form-item>
            <el-form-item :label="$t('system.auto.department')" prop="deptId">
              <el-tree-select
                v-model="form.deptId"
                :data="deptOptions"
                :props="{ value: 'id', label: 'label', children: 'children' } as any"
                value-key="id"
                :placeholder="$t('system.auto.pleaseSelectADepartment')"
                check-strictly
              />
            </el-form-item>
            <el-form-item :label="$t('system.auto.postCode')" prop="postCode">
              <el-input v-model="form.postCode" :placeholder="$t('system.auto.pleaseEnterCodeName')" />
            </el-form-item>
            <el-form-item :label="$t('system.auto.categoryCode')" prop="postCategory">
              <el-input v-model="form.postCategory" :placeholder="$t('system.auto.pleaseEnterCategoryCode')" />
            </el-form-item>
            <el-form-item :label="$t('system.auto.postOrder')" prop="postSort">
              <el-input-number v-model="form.postSort" controls-position="right" :min="0" />
            </el-form-item>
            <el-form-item :label="$t('system.auto.postStatus')" prop="status">
              <el-radio-group v-model="form.status">
                <el-radio v-for="dict in sys_normal_disable" :key="dict.value" :value="dict.value">{{ dict.label }}</el-radio>
              </el-radio-group>
            </el-form-item>
            <el-form-item :label="$t('system.auto.remark')" prop="remark">
              <el-input v-model="form.remark" type="textarea" :placeholder="$t('system.auto.pleaseEnterAValue')" />
            </el-form-item>
          </el-form>
          <template #footer>
            <div class="dialog-footer">
              <el-button type="primary" @click="submitForm">{{ $t('system.auto.ok') }}</el-button>
              <el-button @click="cancel">{{ $t('system.auto.cancel') }}</el-button>
            </div>
          </template>
        </el-dialog>
      </el-col>
    </el-row>
  </div>
</template>

<script setup name="Post" lang="ts">
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
import { listPost, addPost, delPost, getPost, updatePost, deptTreeSelect } from '@/api/system/post';
import { PostForm, PostQuery, PostVO } from '@/api/system/post/types';
import { DeptTreeVO, DeptVO } from '@/api/system/dept/types';

const { proxy } = getCurrentInstance() as ComponentInternalInstance;
const { sys_normal_disable } = toRefs<any>(proxy?.useDict('sys_normal_disable'));

const postList = ref<PostVO[]>([]);
const loading = ref(true);
const showSearch = ref(true);
const ids = ref<Array<number | string>>([]);
const single = ref(true);
const multiple = ref(true);
const total = ref(0);
const deptName = ref('');
const deptOptions = ref<DeptTreeVO[]>([]);
const deptTreeRef = ref<ElTreeInstance>();
const postFormRef = ref<ElFormInstance>();
const queryFormRef = ref<ElFormInstance>();

const dialog = reactive<DialogOption>({
  visible: false,
  title: ''
});

const initFormData: PostForm = {
  postId: undefined,
  deptId: undefined,
  postCode: '',
  postName: '',
  postCategory: '',
  postSort: 0,
  status: '0',
  remark: ''
};

const data = reactive<PageData<PostForm, PostQuery>>({
  form: { ...initFormData },
  queryParams: {
    pageNum: 1,
    pageSize: 10,
    deptId: undefined,
    belongDeptId: undefined,
    postCode: '',
    postName: '',
    postCategory: '',
    status: ''
  },
  rules: {
    postName: [{ required: true, message: t('system.auto.postNameisRequired'), trigger: 'blur' }],
    postCode: [{ required: true, message: t('system.auto.postCodeisRequired'), trigger: 'blur' }],
    deptId: [{ required: true, message: t('system.auto.departmentisRequired'), trigger: 'blur' }],
    postSort: [{ required: true, message: t('system.auto.postOrderisRequired'), trigger: 'blur' }]
  }
});

const { queryParams, form, rules } = toRefs<PageData<PostForm, PostQuery>>(data);

/** 通过条件过滤节点  */
const filterNode = (value: string, data: any) => {
  if (!value) return true;
  return data.label.indexOf(value) !== -1;
};

/** 根据名称筛选部门树 */
watchEffect(
  () => {
    deptTreeRef.value?.filter(deptName.value);
  },
  {
    flush: 'post' // watchEffect会在DOM挂载或者更新之前就会触发，此属性控制在DOM元素更新后运行
  }
);

/** 查询部门下拉树结构 */
const getTreeSelect = async () => {
  const res = await deptTreeSelect();
  deptOptions.value = res.data;
};

/** 节点单击事件 */
const handleNodeClick = (data: DeptVO) => {
  queryParams.value.belongDeptId = data.id;
  queryParams.value.deptId = undefined;
  handleQuery();
};

/** 查询岗位列表 */
const getList = async () => {
  loading.value = true;
  const res = await listPost(queryParams.value);
  postList.value = res.rows;
  total.value = res.total;
  loading.value = false;
};

/** 取消按钮 */
const cancel = () => {
  reset();
  dialog.visible = false;
};

/** 表单重置 */
const reset = () => {
  form.value = { ...initFormData };
  postFormRef.value?.resetFields();
};

/** 搜索按钮操作 */
const handleQuery = () => {
  queryParams.value.pageNum = 1;
  if (queryParams.value.deptId) {
    queryParams.value.belongDeptId = undefined;
  }
  getList();
};

/** 重置按钮操作 */
const resetQuery = () => {
  queryFormRef.value?.resetFields();
  queryParams.value.pageNum = 1;
  queryParams.value.deptId = undefined;
  deptTreeRef.value?.setCurrentKey(undefined);
  /** 清空左边部门树选中值 */
  queryParams.value.belongDeptId = undefined;
  handleQuery();
};

/** 多选框选中数据 */
const handleSelectionChange = (selection: PostVO[]) => {
  ids.value = selection.map((item) => item.postId);
  single.value = selection.length != 1;
  multiple.value = !selection.length;
};

/** 新增按钮操作 */
const handleAdd = () => {
  reset();
  dialog.visible = true;
  dialog.title = t('system.auto.addPost');
};

/** 修改按钮操作 */
const handleUpdate = async (row?: PostVO) => {
  reset();
  const postId = row?.postId || ids.value[0];
  const res = await getPost(postId);
  Object.assign(form.value, res.data);
  dialog.visible = true;
  dialog.title = t('system.auto.editPost');
};

/** 提交按钮 */
const submitForm = () => {
  postFormRef.value?.validate(async (valid: boolean) => {
    if (valid) {
      form.value.postId ? await updatePost(form.value) : await addPost(form.value);
      proxy?.$modal.msgSuccess(t('system.auto.operationSuccessful'));
      dialog.visible = false;
      await getList();
    }
  });
};

/** 删除按钮操作 */
const handleDelete = async (row?: PostVO) => {
  const postIds = row?.postId || ids.value;
  await proxy?.$modal.confirm(t('system.auto.areYouSureToDeletePostID') + postIds + t('system.auto.text'));
  await delPost(postIds);
  await getList();
  proxy?.$modal.msgSuccess(t('system.auto.deletedSuccessfully'));
};

/** 导出按钮操作 */
const handleExport = () => {
  proxy?.download(
    'system/post/export',
    {
      ...queryParams.value
    },
    `post_${new Date().getTime()}.xlsx`
  );
};

onMounted(() => {
  getTreeSelect(); // 初始化部门数据
  getList();
});
</script>

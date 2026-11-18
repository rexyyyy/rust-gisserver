<template>
  <el-form ref="userRef" :model="userForm" :rules="rules" label-width="80px">
    <el-form-item :label="$t('system.auto.nickname')" prop="nickName">
      <el-input v-model="userForm.nickName" maxlength="64" />
    </el-form-item>
    <el-form-item :label="$t('system.auto.phoneNumber')" prop="phonenumber">
      <el-input v-model="userForm.phonenumber" maxlength="11" />
    </el-form-item>
    <el-form-item :label="$t('system.auto.email')" prop="email">
      <el-input v-model="userForm.email" maxlength="50" />
    </el-form-item>
    <el-form-item :label="$t('system.auto.gender2')">
      <el-radio-group v-model="userForm.sex">
        <el-radio value="0">{{ $t('system.auto.male') }}</el-radio>
        <el-radio value="1">{{ $t('system.auto.female') }}</el-radio>
      </el-radio-group>
    </el-form-item>
    <el-form-item>
      <el-button type="primary" @click="submit">{{ $t('system.auto.save') }}</el-button>
      <el-button type="danger" @click="close">{{ $t('system.auto.close') }}</el-button>
    </el-form-item>
  </el-form>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
import { updateUserProfile } from '@/api/system/user';
import { propTypes } from '@/utils/propTypes';

const props = defineProps({
  user: propTypes.any.isRequired
});
const userForm = computed(() => props.user);
const { proxy } = getCurrentInstance() as ComponentInternalInstance;
const userRef = ref<ElFormInstance>();
const rule: ElFormRules = {
  nickName: [{ required: true, message: t('system.auto.nicknameisRequired'), trigger: 'blur' }],
  email: [
    { required: true, message: t('system.auto.emailaddressisRequired'), trigger: 'blur' },
    {
      type: 'email',
      message: t('system.auto.pleaseEnterAValidEmailAddress'),
      trigger: ['blur', 'change']
    }
  ],
  phonenumber: [
    {
      required: true,
      message: t('system.auto.phoneNumberisRequired'),
      trigger: 'blur'
    },
    { pattern: /^1[3456789][0-9]\d{8}$/, message: t('system.auto.pleaseEnterAValidPhoneNumber'), trigger: 'blur' }
  ]
};
const rules = ref<ElFormRules>(rule);

/** 提交按钮 */
const submit = () => {
  userRef.value?.validate(async (valid: boolean) => {
    if (valid) {
      await updateUserProfile(props.user);
      proxy?.$modal.msgSuccess(t('system.auto.updatedSuccessfully'));
    }
  });
};
/** 关闭按钮 */
const close = () => {
  proxy?.$tab.closePage();
};
</script>

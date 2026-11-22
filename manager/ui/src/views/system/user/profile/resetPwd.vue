<template>
  <el-form ref="pwdRef" :model="user" :rules="rules" label-width="80px">
    <el-form-item :label="$t('system.auto.oldPassword')" prop="oldPassword">
      <el-input v-model="user.oldPassword" :placeholder="$t('system.auto.pleaseEnterOldPassword')" type="password" show-password />
    </el-form-item>
    <el-form-item :label="$t('system.auto.newPassword')" prop="newPassword">
      <el-input v-model="user.newPassword" :placeholder="$t('system.auto.pleaseEnterNewPassword')" type="password" show-password />
    </el-form-item>
    <el-form-item :label="$t('system.auto.confirmPassword')" prop="confirmPassword">
      <el-input v-model="user.confirmPassword" :placeholder="$t('system.auto.pleaseConfirmNewPassword')" type="password" show-password />
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
import { updateUserPwd } from '@/api/system/user';
import type { ResetPwdForm } from '@/api/system/user/types';

const { proxy } = getCurrentInstance() as ComponentInternalInstance;
const pwdRef = ref<ElFormInstance>();
const user = ref<ResetPwdForm>({
  oldPassword: '',
  newPassword: '',
  confirmPassword: ''
});

const equalToPassword = (rule: any, value: string, callback: any) => {
  if (user.value.newPassword !== value) {
    callback(new Error(t('system.auto.theTwoPasswordsDoNotMatch')));
  } else {
    callback();
  }
};
const rules = ref({
  oldPassword: [{ required: true, message: t('system.auto.oldPasswordisRequired'), trigger: 'blur' }],
  newPassword: [
    { required: true, message: t('system.auto.newPasswordisRequired'), trigger: 'blur' },
    {
      min: 6,
      max: 20,
      message: t('system.auto.n620Characters'),
      trigger: 'blur'
    },
    { pattern: /^[^<>"'|\\]+$/, message: t('system.auto.illegalCharsTip'), trigger: 'blur' }
  ],
  confirmPassword: [
    { required: true, message: t('system.auto.confirmPasswordisRequired'), trigger: 'blur' },
    {
      required: true,
      validator: equalToPassword,
      trigger: 'blur'
    }
  ]
});

/** 提交按钮 */
const submit = () => {
  pwdRef.value?.validate(async (valid: boolean) => {
    if (valid) {
      await updateUserPwd(user.value.oldPassword, user.value.newPassword);
      proxy?.$modal.msgSuccess(t('system.auto.updatedSuccessfully'));
    }
  });
};
/** 关闭按钮 */
const close = () => {
  proxy?.$tab.closePage();
};
</script>

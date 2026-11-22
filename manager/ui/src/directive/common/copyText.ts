/**
 * v-copyText：点击元素时复制绑定的文本。
 * 用法：v-copyText="text" 或 v-copyText:callback="fn"
 */
import { DirectiveBinding } from 'vue';

function writeClipboard(text: string): boolean {
  const area = document.createElement('textarea');
  area.value = text;
  area.setAttribute('readonly', '');
  area.style.position = 'fixed';
  area.style.left = '-9999px';
  document.body.appendChild(area);
  area.select();
  area.setSelectionRange(0, text.length);

  let ok = false;
  try {
    ok = document.execCommand('copy');
  } catch (err) {
    console.error(err);
  }
  document.body.removeChild(area);
  return ok;
}

export default {
  beforeMount(el: any, { value, arg }: DirectiveBinding) {
    if (arg === 'callback') {
      el.$copyCallback = value;
      return;
    }
    el.$copyValue = value;
    const handler = () => {
      writeClipboard(el.$copyValue);
      if (el.$copyCallback) {
        el.$copyCallback(el.$copyValue);
      }
    };
    el.addEventListener('click', handler);
    el.$destroyCopy = () => el.removeEventListener('click', handler);
  }
};

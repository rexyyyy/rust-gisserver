<template>
  <div
    class="sidebar-logo-container"
    :class="{ collapse: collapse }"
  >
    <canvas ref="dotCanvas" class="logo-dot-canvas"></canvas>
    <transition :enter-active-class="proxy?.animate.logoAnimate.enter" mode="out-in">
      <router-link v-if="collapse" key="collapse" class="sidebar-logo-link" to="/">
        <img v-if="logo" :src="logo" class="sidebar-logo" />
        <h1 v-else class="sidebar-title">
          {{ title }}
        </h1>
      </router-link>
      <router-link v-else key="expand" class="sidebar-logo-link" to="/">
        <img v-if="logo" :src="logo" class="sidebar-logo" />
        <h1 class="sidebar-title">
          {{ title }}
        </h1>
      </router-link>
    </transition>
  </div>
</template>

<script setup lang="ts">
import variables from '@/assets/styles/variables.module.scss';
import logo from '@/assets/logo/map_logo.svg';
import { useSettingsStore } from '@/store/modules/settings';
const { proxy } = getCurrentInstance() as ComponentInternalInstance;
import { NavTypeEnum } from '@/enums/NavTypeEnum';

defineProps({
  collapse: {
    type: Boolean,
    required: true
  }
});

const title = import.meta.env.VITE_APP_LOGO_TITLE;
const settingsStore = useSettingsStore();
const sideTheme = computed(() => settingsStore.sideTheme);
const dotCanvas = ref<HTMLCanvasElement | null>(null);

// 获取Logo背景色
const getLogoBackground = computed(() => {
  if (settingsStore.isDark) {
    return 'var(--sidebar-bg)'
  }
  if (settingsStore.navType == NavTypeEnum.TOP) {
    return variables.menuLightBackground
  }
  // menuBackground / menuLightBackground 均为 var(--menuBg)，由主题预设运行时切换
  return sideTheme.value === 'theme-dark' ? variables.menuBackground : variables.menuLightBackground
})

// 获取Logo文字颜色
const getLogoTextColor = computed(() => {
  if (settingsStore.isDark) {
    return 'var(--sidebar-text)'
  }
  if (settingsStore.navType == NavTypeEnum.TOP) {
    return variables.logoLightTitleColor
  }
  return sideTheme.value === 'theme-dark' ? variables.logoTitleColor : variables.logoLightTitleColor
})

// 随机颜色圆点动画
const DOT_COUNT = 12;
const dots: Array<{ x: number; y: number; vx: number; vy: number; r: number; color: string }> = [];

const randomColor = () => {
  const colors = ['#4CAF50', '#2196F3', '#FF9800', '#E91E63', '#9C27B0', '#00BCD4', '#FFEB3B', '#795548'];
  return colors[Math.floor(Math.random() * colors.length)];
};

const initDots = (w: number, h: number) => {
  dots.length = 0;
  for (let i = 0; i < DOT_COUNT; i++) {
    dots.push({
      x: Math.random() * w,
      y: Math.random() * h,
      vx: (Math.random() - 0.5) * 0.4,
      vy: (Math.random() - 0.5) * 0.4,
      r: 1.5 + Math.random() * 2,
      color: randomColor()
    });
  }
};

let animId = 0;
const animate = () => {
  const canvas = dotCanvas.value;
  if (!canvas) return;
  const ctx = canvas.getContext('2d');
  if (!ctx) return;
  const w = canvas.width;
  const h = canvas.height;
  ctx.clearRect(0, 0, w, h);
  for (const d of dots) {
    d.x += d.vx;
    d.y += d.vy;
    if (d.x < 0 || d.x > w) d.vx *= -1;
    if (d.y < 0 || d.y > h) d.vy *= -1;
    ctx.beginPath();
    ctx.arc(d.x, d.y, d.r, 0, Math.PI * 2);
    ctx.fillStyle = d.color;
    ctx.globalAlpha = 0.35;
    ctx.fill();
  }
  ctx.globalAlpha = 1;
  animId = requestAnimationFrame(animate);
};

onMounted(() => {
  const canvas = dotCanvas.value;
  if (!canvas) return;
  const rect = canvas.parentElement?.getBoundingClientRect();
  const w = rect?.width || 200;
  const h = rect?.height || 50;
  canvas.width = w;
  canvas.height = h;
  initDots(w, h);
  animate();
});

onBeforeUnmount(() => {
  cancelAnimationFrame(animId);
});
</script>

<style lang="scss" scoped>
.sidebarLogoFade-enter-active {
  transition: opacity 1.5s;
}

.sidebarLogoFade-enter,
.sidebarLogoFade-leave-to {
  opacity: 0;
}

.logo-dot-canvas {
  position: absolute;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  pointer-events: none;
  z-index: 0;
}

.sidebar-logo-container {
  position: relative;
  height: 50px;
  line-height: 50px;
  background: v-bind(getLogoBackground);
  text-align: center;
  overflow: hidden;
  // Logo 区块下方的 2px 绿色分隔线（用内阴影避免撑高布局）
  box-shadow: inset 0 -2px 0 0 var(--gs-accent);

  & .sidebar-logo-link {
    position: relative;
    z-index: 1;
    height: 100%;
    width: 100%;

    & .sidebar-logo {
      width: 32px;
      height: 32px;
      vertical-align: middle;
      margin-right: 12px;
      margin-left: 12px;
    }

    & .sidebar-title {
      display: inline-block;
      margin: 0;
      color: v-bind(getLogoTextColor);
      font-weight: 600;
      line-height: 50px;
      font-size: 14px;
      font-family: Avenir, Helvetica Neue, Arial, Helvetica, sans-serif;
      vertical-align: middle;
    }
  }

  &.collapse {
    .sidebar-logo {
      margin-right: 0px;
    }
  }
}
</style>

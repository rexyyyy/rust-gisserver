# manager-ui

rust-gisserver 管理控制台前端，基于 Vue 3 + TypeScript + Element Plus + Vite。

## 功能

- 用户管理：用户、角色、菜单、部门、岗位
- 数据服务：数据目录、工作空间、数据源、数据图层、发布记录
- 地图服务：服务管理、SLD 样式、MVT 样式、瓦片、缓存、引擎、API Key
- 系统监控：在线用户、操作日志、登录日志
- 中英文界面实时切换，地图预览基于 OpenLayers

## 开发

```bash
# 安装依赖（使用国内镜像）
npm install --registry=https://registry.npmmirror.com

# 启动开发服务
npm run dev

# 构建生产环境
npm run build:prod
```

构建产物输出到 `dist/`，由安装脚本复制到运行目录的 `web/`。

## 目录结构

```text
src/
├── api/         接口封装
├── assets/      静态资源与样式
├── components/  通用组件
├── layout/      布局与导航
├── lang/        中英文语言包
├── router/      路由
├── store/       Pinia 状态
├── utils/       工具函数
└── views/       业务页面
```

## 许可

MIT，详见仓库根目录 `LICENSE`。

# GitWave

为开发者打造的桌面 Git 客户端。原生 Tauri 2 体验、三栏工作流、可视化提交图与 AI 辅助提交——本地 `git` CLI 驱动，不嵌入 libgit2，你的仓库始终在你手中。

> 产品落地页见 [`website/`](website/)，架构说明见 [`docs/architecture.html`](docs/architecture.html)。

## 特性

### 三栏布局

Splitpanes 可拖拽三栏：**Sidebar** · **工作区 / 历史** · **Diff**，一屏掌控全局。

- **Sidebar**：分支切换、ahead/behind、Stash、Tag、Subtree、push/pull/fetch、最近仓库与 pinned branches
- **工作区**：Unstaged / Staged 文件列表，统一提交区（可拖拽调整高度）
- **Diff**：选中文件或 commit 即看 unified diff

### 可视化提交图

`CommitGraphView` 渲染分支 lane 与 merge 节点，hover 高亮关联路径。分页加载 commit log，支持当前分支与全仓库过滤。

### 精确 Stage

DiffPanel 支持 hunk 级与**行级** stage / revert，不再被迫全文件 stage。二进制图片可预览 staged / unstaged 两侧。

### AI Commit

基于 staged diff 上下文，通过可配置的 AI Provider 流式生成 commit message。支持 OpenAI 兼容端点，自定义 prompt 模板。

### 本地优先

- 所有 Git 读写通过 spawn 本地 `git` 命令完成
- 远程同步经标准 push / pull / fetch / clone
- Settings、recent repos 等持久化到 `app_data_dir`
- AI 请求经 Tauri `plugin-http` 发出，其余逻辑均在本地

## 技术栈

| 层 | 技术 |
|---|---|
| 桌面运行时 | [Tauri 2](https://v2.tauri.app/) |
| 前端 | Vue 3 · TypeScript · Tailwind CSS · Splitpanes |
| 后端 | Rust（命令无状态，`AppState` 持有 `repo_path`） |
| Git 引擎 | 系统 `git` CLI |
| 构建 | Vite → `dist/` → Tauri bundle |

**Bundle 目标**：macOS `.dmg` · Windows NSIS `.exe`

## 环境要求

开发与本机运行需要：

- [Node.js](https://nodejs.org/) 22+
- [pnpm](https://pnpm.io/) 9+
- [Rust](https://www.rust-lang.org/) stable
- 系统已安装 **Git**（运行时必需）

macOS 额外需要 Xcode Command Line Tools；Windows 需要 WebView2（Win10/11 通常已自带）。

## 快速开始

```bash
# 安装依赖
pnpm install

# 开发模式（启动 Vite + Tauri 窗口）
pnpm tauri dev
```

## 常用命令

```bash
# 前端类型检查 + 生产构建
pnpm build

# 打包桌面应用（产物在 src-tauri/target/release/bundle/）
pnpm tauri build

# 单元测试
pnpm test:unit

# 重新生成 Tauri 图标
pnpm icons
```

## 项目结构

```
gitwave/
├── src/                    # Vue 3 前端
│   ├── App.vue             # 三栏主布局
│   └── components/         # Sidebar / Workspace / Diff / History / Settings …
├── src-tauri/              # Rust 后端（Tauri commands）
│   └── src/lib.rs          # git 命令封装、patch stage、subtree、AI diff 上下文
├── website/                # 产品落地页（静态 HTML + Vite 预览）
├── docs/                   # 架构图等文档
└── tests/                  # Vitest 单元测试
```

## 落地页开发

`website/` 是独立的前端静态站点，用于产品介绍与下载引导：

```bash
cd website
pnpm install
pnpm dev        # 本地预览
pnpm preview    # 预览构建结果
```

设计规范见 [`website/DESIGN.md`](website/DESIGN.md)。

## CI / 发布

- **PR / push 构建验证**：`.github/workflows/tauri-build.yml`（Windows artifact）
- **Release 发布**：推送 `v*` tag 触发 `.github/workflows/tauri-release.yml`，自动构建 Windows NSIS 安装包并挂到 GitHub Release

本地打包 macOS：

```bash
pnpm tauri build --bundles dmg
```

## 架构概览

```
┌─────────────────────────────────────────────────────────┐
│  Vue 3 前端                                              │
│  Splitpanes · invoke() · plugin-http (AI)               │
├─────────────────────────────────────────────────────────┤
│  Tauri 2 + Rust                                         │
│  spawn git CLI · patch stage/revert · subtree · 图片 diff │
├─────────────────────────────────────────────────────────┤
│  本地 git · app_data_dir · 远程 push/pull/fetch         │
└─────────────────────────────────────────────────────────┘
```

详细交互与模块边界见 [`docs/architecture.html`](docs/architecture.html)（浏览器直接打开）。

## 相关链接

- 源码：[github.com/ginuim/gitwave](https://github.com/ginuim/gitwave)
- 标识符：`com.reaidea.gitwave`
- 当前版本：`0.1.0`

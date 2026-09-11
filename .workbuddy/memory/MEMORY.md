# 项目长期备忘 · my-todolist

## 项目性质

「我的任务台」——个人任务/客户跟进管理桌面应用（Tauri 2 + Rust + 本地 SQLite，无云端）。
产品名 `todo-list`，工程目录 `my-task-desktop/`。

## 关键事实

- **版本控制已于 2026-09-11 建立**：仓库根为顶层目录，远程 `https://github.com/VizZ-space/mytodolist`（public，`main` 分支）。
  - 本地 git 身份为**仓库级**配置：`VizZ` / `221272360+VizZ-space@users.noreply.github.com`（全局 user.name/email 为空，未设置；**用户明确选择用 GitHub noreply 邮箱而非真实 Gmail**，出于隐私考虑，后续不要擅自改成真实邮箱）
  - `.gitignore` 排除：`node_modules/`、`dist/`、`src-tauri/target/`、`src-tauri/gen/schemas/`、`windows-installer/*.exe|*.msi`
  - 已提交 73 个文件（源码 / 配置 / 图标 / 4 份 md / `.workbuddy/memory`）；安装包刻意未入库，如需分发应走 GitHub Releases
  - 本机**未安装 `gh` CLI**，凭据由 Git Credential Manager 管理
- **前端是单文件**：全部 HTML/CSS/JS 都在 `my-task-desktop/src/index.html`（约 4400 行 / 318 KB），改 UI 只动这一个文件，不涉及 Rust。
- **后端**：`src-tauri/src/main.rs`（约 1852 行），SQLite 建表在 `init_schema`，表有 projects / tasks / subtasks / clients / tags / smart_lists / logs / notes / note_cats / proj_stages。
- **数据模型关键点：客户与项目是「平级实体」**。`projects` 表**没有 `client_id` 列**（`projects(id,name,color,summary,goal,sort_idx)`，main.rs L1381），`clients(id,name,color,category,sort_idx)` 独立存在；两者都通过 `tasks.client_id` / `tasks.project_id` 直接挂在任务上。
  → 因此**不存在"由项目推导客户"的可能**。任何需要该能力的需求，都必须先给 `projects` 加 `client_id` 列（可走 `ensure_col` 兼容追加）。
  → 前端 `renderSide` 里"客户分组（项目的上层归类）"那句注释**与真实数据模型不符**，不要被它误导。
- **前端模块作用域**：`<script type="module">`，所有顶层 `var` **挂在模块作用域而非 window**。浏览器里无法用 `Runtime.evaluate` 直接读 `ui` / `IC` / `NAV_*`，做自动化验证时只能断言 DOM。
- **构建**：`npm run build:front` 产出 `dist/`（`tauri.conf.json` 的 `beforeBuildCommand`），`npm run build` 走 Tauri 打包出 msi/nsis。
- **交付物归档**：打包结果放在顶层 `windows-installer/`（当前 0.1.0，msi + exe）。
- **文档与代码有代差**：`overview.md` 描述的是更早的「Mac 单文件 HTML + localStorage」版本，与当前 Tauri + SQLite 实现不符，读它时要注意时效。

## 产品定位与用户偏好（重要）

- **「笔记」是高频使用的核心模块，用户明确表示后续要作为「个人知识库」使用。**
  - 因此在任何导航/信息架构调整中，笔记必须**保持一级常驻**，不可降级进折叠区或二级入口。
  - 笔记模块的演进方向是「知识库」而非「随手记」：多标签、双链/反链、全文搜索、与任务闭环，都是合理需求。
- 用户的诉求是**降低操作复杂度**，不是砍功能。改造时要"把功能放对位置"，而不是删掉。
- 现有笔记实现已经是三栏工作区且有正文搜索，**属于合理结构，改造时以增强为主，不要推翻重做**。

## 规划类文档（都在顶层）

- `todo-app-design-research.md` — 对标 Things 3 / Todoist / TickTick / OmniFocus / Apple Reminders / MS To Do / Linear 的设计调研
- `enhancement-plan.md` — 六阶段功能增强计划（客户分类 → 标签/智能列表 → 自然语言日期 → 视图 → 回顾/通知 → 打磨），状态为「待拍板」
- `ux-simplification-plan.md` — 交互简化优化方案（2026-09-11 产出），4 个改造包

## 本机环境注意

- Bash 工具偶尔丢 PATH，命令前加 `export PATH="/usr/bin:/bin:$PATH"`
- 删除大量文件需先授权批量删除守卫（与 `rm` 同一次调用），且前台 2 分钟硬超时会中断 → 用后台执行
- 详见用户级 skill `windows-disk-cleanup`
- **Windows 版 git 不认 MSYS 路径**：`git -C /e/workspace/...` 会报 `cannot change to`。必须用原生路径 `git -C "E:/workspace/my-todolist"`，且**不要**设 `MSYS_NO_PATHCONV=1`（它会阻止路径自动转换）
- **本机 git fetch/push 不会自动写 `refs/remotes/origin/*`**：`git fetch` 会打印 `[new branch] main -> origin/main` 但引用实际未落盘，`git status` 因此显示 `[gone]`、`origin/main` 无法解析。**绕过办法**：手动写入引用文件
  `mkdir -p .git/refs/remotes/origin && echo <sha> > .git/refs/remotes/origin/main`
  （推送本身是成功的，远程内容正确，仅本地引用记账有问题）

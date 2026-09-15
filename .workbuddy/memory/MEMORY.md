# 项目长期备忘 · my-todolist

「我的任务台」——个人任务/客户跟进管理桌面应用（Tauri 2 + Rust + 本地 SQLite，无云端）。产品名 `todo-list`，工程目录 `my-task-desktop/`。

**顶层文档分工**（改代码前先读）：
- `ui-spec.md` — **界面规范**（标准本身）：弹窗尺寸/结构/按钮/危险色 + 入口与筛选原则。
- `usability-review.md` — 易用性评审 16 条与实施落点。
- `ux-simplification-plan.md` — 交互简化方案 + 逐轮用户反馈记录。
- `enhancement-plan.md` / `todo-app-design-research.md` — 功能规划与竞品调研。
- `overview.md` — 描述的是更早的 Mac 单文件 HTML 版，**已过时**，别照它改。

## 仓库与构建

- 仓库根 = 顶层目录；远程 `https://github.com/VizZ-space/mytodolist`（public，`main`）。**未装 `gh` CLI**，凭据走 Git Credential Manager。
- git 身份是**仓库级**配置：`VizZ` / `221272360+VizZ-space@users.noreply.github.com`。**用户明确要求用 noreply 邮箱，别改成真实 Gmail。**
- `.gitignore`：`node_modules/`、`dist/`、`src-tauri/target/`、`gen/schemas/`、`windows-installer/*.exe|*.msi`、**`my-task-desktop/.smoke/`**（本地测试脚本一律不入库）。
- **前端是单文件**：全部 HTML/CSS/JS 在 `my-task-desktop/src/index.html`（约 5200 行），改 UI 只动它。`<script type="module">` → 顶层 `var` 挂模块作用域，自动化**只能断言 DOM**，读不到 `ui`/`db`。
- **后端** `src-tauri/src/main.rs`（约 2100 行）：建表在 `init_schema`；表有 projects / tasks / subtasks / clients / tags / smart_lists / logs / notes / note_cats / proj_stages。
- **构建**：`npm run build:front` → `dist/`；`npm run build` → Tauri 打包（msi/nsis），产物归档到顶层 `windows-installer/`（当前 0.1.0）。

## 数据模型与后端约定

- 客户/项目是**从属**关系：`projects.client_id`（由 `ensure_col` 追加），`tasks.client_id` 是覆盖值。
- **加列一律走 `main.rs` 的 `ensure_col(conn, 表, 列, 类型)`**——老库无损、老数据为 NULL、无需迁移脚本。这是本项目新增字段的唯一正确姿势。
- ⚠️ **客户推导必须「项目优先、任务原值兜底」**：`effClient(pid,stored) = projClientId(pid) || stored || ""`。三处必须一致：`openTask()` 的 `#mcView`、`handleChg("tproj")`、`saveTask()` 的 `mcv`。历史 bug：只用 `projClientId()` → 一个"有客户但项目没指定客户"的任务，打开再保存客户就被**静默清空**（弹窗显示"未归类"、卡片却还有客户名）。冒烟 IA7 守着（含"必须真选到无项目芯片的卡"前置断言，防用例空转）。
- ⚠️ **前端给对象加字段，必须同步 Rust 四处：SELECT（读可能两处）/ INSERT / `*_sig` / `CREATE TABLE`**，否则一存一读就丢数据（`notes.tags` 曾漏）。
- **import 规范化**：Rust 顶部统一 `use`，别写内联全路径。macOS 专用符号走 `#[cfg(target_os="macos")]` 门控 —— **Windows 上它们的 unused 告警是误报，删了会破坏 macOS 构建**。
- **存储通道 `storeRead/storeWrite`**：桌面走 Rust `save_store/load_store`，非 Tauri（双击 `dist/index.html`）退回 localStorage。**新代码别直接 `invoke("save_store")`**，否则浏览器预览会弹红色「数据保存失败」横幅（它曾出现在所有截图里，极易误判成自己改坏了）。

## 交互与架构（要点，细节见 `ui-spec.md`）

- **Esc 只有一个权威处理器**（`document.addEventListener("keydown")`），分支顺序 = 浮层层级：`#dp` 日期面板 → `#uselpop` 下拉 → `#palette` 命令面板 → 子任务行内编辑 → `closeModal()`。**不要再给单个浮层挂 document keydown**：`#dp`/`#uselpop` 曾各挂一个、注册更早，会先把状态清 null，导致一次 Esc 同时关掉面板**和**弹窗。
  - **遗留未修**：从任务弹窗里点「＋新建项目/客户/关联笔记」开的**嵌套弹窗**，Esc/取消走 `closeModal()` → 整个 `#modal` 清空、半写任务一起丢。可用机制已存在（`snapshotTaskDraft()` + `openTask(taskId)` 读回），但笔记选择器有独立 `noteReturnTo` 回退语义，**不能直接改 `closeModal()`**，需单独区分。
- **导航**：一级 = 今天/日历/笔记/任务/项目；「更多视图」= 四象限/历史/总览/报告；快捷键 1–9。看板已并入任务页（`ui.taskMode` = list/board），`viewBoard()` 已删。
- **「今天」只做时间焦点**（今天要处理/逾期治理/近 3 天/久未跟进/已完成入口），不再铺全量列表。
- **右侧面板已整体删除**：点任务卡直接 `openTask(id)`；卡片根 `.task` 不带动作，热区只留标题 `.ttl-link`。
- **KPI 卡**（`KPI_VIEWS` 六个视图顶部，空库也显示）：父子版式（待办 = 总量，逾期/进行中 = 子集）；带 `data-act="kpi"` + `.kpi-n`，计数 0 时置灰且不带状态色。
- **侧栏两段式**：`.side-scroll`（内容导航）+ `.side-foot`（钉底，目前只有「设置」）。**应用级入口别塞回顶栏**；**设置入口有桌面/移动两份，改一处必须同步另一处**（≤768px 侧栏整条隐藏，顶栏 `.tbtn.mo-only` 兜底，`responsive.mjs` 守这条）。
- **筛选**：只有 `filterBar()` 一条，列表与看板共用；项目筛选统一 `ui.pid`；**顺序 = 维度控件在前、可移除芯片在后**（芯片宽度会变，放行首会让整排抖动）。
- **弹窗规范**：5 档宽度（480/560/720/880/1160，不写类 = 560）；每个 `.sheet` 必须拼 `sheetX()`（右上角 ×）；`.sheet-act` 页脚最右必须实心色；危险色只给"会丢数据"的动作。⚠️ **`btn-primary` 这个类名不存在**，弹窗按钮只用 `btn-ghost / btn-pri / btn-danger`。
- **点遮罩关不关**：统一规则 —— `#modal` 捕获阶段登记 `input`/`change` → `_maskDirty`，动过就不关并抖一下（复用已有 `.sheet.nudge`）。**必须用事件标记而非值快照**（弹窗会因选标签/选笔记重渲染而冲掉快照）。

## 产品定位与用户偏好（重要）

- **「笔记」是核心高频模块，用户要把它当「个人知识库」用**：必须一级常驻，不可降级进折叠区；演进方向是多标签、双链/反链、全文搜索、与任务闭环。现有三栏工作区 + 正文搜索结构合理，**以增强为主，不要推翻重做**。
- 用户诉求是**降低操作复杂度，不是砍功能** —— 要"把功能放对位置"。
- ⚠️ **「同屏重复」是用户反复（已连续七轮）指出的主题。做任何 UI 改动先自查：这个信息/入口在同一屏里是否已经存在？** 已删掉的重复：①今天/任务/看板渲染同一份列表 ②右侧「今日概览」vs 顶部 KPI ③空态按钮 vs 页顶快速添加行 ④快速添加行 vs 右上「新建」⑤侧栏「＋新建项目」vs 标题右侧 ＋ ⑥「全部任务」「未归类」条目 ⑦右侧详情面板 vs 任务卡片本身。**判断"能不能删"的最快办法：把界面上的内容逐项列出来，看另一处是不是已经全有了。**
- **实体列表只放实体**：判断标准 —— 点它是"去看某个具体东西"，还是"把列表筛掉一半"？后者是筛选维度，不能混进实体列表。**删筛选入口前必须确认数据还有别的路可达**（未归类任务在任务页默认范围里本就在，多个下拉仍有该选项）。
- **一个动作一个入口**：任务新增 = 右上「新建」→ 弹窗；侧栏四个业务分组（项目/客户/标签/智能列表）标题右侧的 ＋ 是各自唯一的新建入口；分组为空必须给"还没有 X，点标题右侧的 ＋ 新建"空态，与其它组同形。
- **空状态原则**：回答 ①这里放什么 ②为什么有用 ③现在能做什么；③**优先指向页面上已有的入口**（一句话"点右上角「新建」"），只有该动作别处确实没入口时才放按钮，否则会造出空转按钮。

## 本地验证（改完 UI 必跑）

`cd my-task-desktop && npm run build:front`（脚本测的是 `dist/`，必须先构建），然后：

| 脚本 | 作用 | 现状 |
|---|---|---|
| `.smoke/smoke.mjs` | 功能冒烟（自带静态服务器 + headless Chrome + CDP），采集 0 条 JS 错误 | **44/44** |
| `.smoke/modal-audit.mjs` | 真打开 21 个弹窗量计算样式，逐条对弹窗规范 | 全过 |
| `.smoke/modal-inventory.mjs` | 静态扫 18 个模板（补齐浏览器不可达的「关于」） | — |
| `.smoke/entry-audit.mjs` | 入口唯一性 + 筛选可见/可退出 | 13/13 |
| `.smoke/responsive.mjs` | 1440/420 两端断点可见性回归 | 7/7 |
| `.smoke/audit.mjs` | 死入口/零引用函数/未用变量/未用图标/未用 CSS | 五类全 0 |
| `.smoke/shot.mjs` | 出图核对 `.smoke/shots/*.png`（窗口 1440x900 → PNG **1418x802**，量尺寸按原生像素） | 20 张 |

- **断言全部走用户可见入口**（点按钮/敲回车），不读模块内部变量 —— 顺带保证"入口真的可用"。
- **测键盘行为**用 `document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape'}))`；`click()` 只发点击。
- 有状态前提的断言（首启空态）**必须排在造数据步骤之前**。
- 任务卡只在「任务」页渲染 → 断言 `.task` 前先 `goView(c,'all')`。
- 驱动弹窗：`data-act` 是 document 级委托，往 body 插一个带 `data-act` 的按钮再 `.click()` 即可打开任意弹窗。
- ⚠️ **量尺寸前必须中和 CSS 动画**：`.sheet` 入场是 `scale(.985)→1`，没播完就量会得到 709/552/867（误报"宽度不在阶梯上"）。修法 `s.getAnimations({subtree:true}).forEach(a=>a.finish())`，**不是把 sleep 调大**。
- 断点相关改动往 `responsive.mjs` 加断言（smoke 固定桌面宽度，覆盖不到"同一功能两入口靠断点切换"的写法）。
- **Rust 测试**：`src-tauri/` 下 `cargo test --no-default-features`（10/10）。首次编译依赖 5–10 分钟。

### 写断言的纪律（踩过坑）

- 断言前先确认目标 DOM 由哪个函数渲染（曾以为"还没有项目"空态在项目页，实际在 `viewDashboard()`）。
- ⚠️ **同文件多处修改必须串行发 Edit**：同一条消息里并行发多个 Edit，后写会覆盖先写，**工具仍报 success** 但改动静默丢失。改完用 `grep -c '<新字面串>' dist/assets/*.js` 校验真的进包（Vite 把 JS 拆到 `dist/assets/index-*.js`，`dist/index.html` 里只有 CSS/HTML）。
- 十几处字面串批量替换 → 用**临时 Node 脚本 + 每条替换断言命中次数**（不满足就整体不写盘），比 15 次串行 Edit 更快也更安全。两条坑：① **新串必须完整保留被匹配串的上下文**（曾漏前导逗号生成 `""importYes"` 的语法错误）；② **匹配锚点要够独特**（`"delYes")` 会同时命中 `confirmBox(...)` 与 `if(a==="delYes")`，要带前导逗号）。
- 脚本跑完立刻 `npm run build:front` 验语法，再跑运行时测试。
- **评审意见必须先找到对应代码行验证再实施**：本轮 1 条 XSS Critical 经 `.smoke/xss-probe.mjs` 实测为误报（`inlineMd` 首行已 `esc()`）。属性注入类断言必须用真实解析器验证，不能靠读码推理。

## 本机环境注意

- **WebView2 崩溃（"Error launching CrashSender.exe"）已定位并修复**：根因是腾讯 WeType 输入法与 WorkBuddy 终端链的 `tsbx.dll` 向 WebView2 注入 DLL，与 Chromium 沙箱冲突。修法：`tauri.conf.json` 窗口配置加 `"additionalBrowserArgs": "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --no-sandbox"`（**它会替换 Tauri 默认参数，默认那三个 disable-features 要自己带上**）。崩溃转储在 `%LOCALAPPDATA%\com.zhuanz.mytask\EBWebView\Crashpad\reports\`，`.smoke/dump-parse.cjs` 可解析。
- ⚠️ **PowerShell 工具在本机完全不可用**（任何命令只返回 exit 0、无 stdout，连 `Set-Content` 也不落盘），从 Bash 调 `powershell.exe` 也被安全策略拦截。进程信息一律走 `tasklist /fo csv | iconv -f GBK -t UTF-8`（中文终端是 GBK，直接 grep 会报 `Binary file matches`）。
- Bash 工具偶尔丢 PATH，命令前加 `export PATH="/usr/bin:/bin:$PATH"`；node 用 `C:/Users/67470/.workbuddy/binaries/node/versions/22.22.2-3/node.exe`。
- **Windows 版 git 不认 MSYS 路径**：用原生路径 `git -C "E:/workspace/my-todolist"`，且**不要**设 `MSYS_NO_PATHCONV=1`（会阻止路径自动转换）。
- **本机 git 引用记账有毛病**：`fetch/push` 不写 `refs/remotes/origin/*`（`git status` 显示 `[gone]`）；还遇到过一次 `.git/refs/heads/*` 与 `.git/objects` 丢失（**工作区文件完好**）。恢复：`git fetch origin main` → 手写 `.git/refs/heads/main` 与 `.git/refs/remotes/origin/main`（先 `mkdir -p`）。**本仓库禁用 `git stash`**（改用手工 `cp` 备份）；任何 git 操作前先 `git rev-parse HEAD` 确认引用还在。
- **启动/重启 dev 会话**：上一轮若未退出，`tauri dev` 全链（vite 占 1420 + cargo + app exe）还挂着；**直接再跑 `npm run dev` 会踩坑** —— vite 自动换到 1421 而 `devUrl` 写死 1420 → 前端连不上。正确顺序：`netstat -ano | grep ":1420"` → 找 app PID（`tasklist /v /fo csv | iconv`）→ `taskkill /F /T /PID <app_pid>` → 再 `npm run dev`（后台）。**别 kill 全部 node.exe**（WorkBuddy 自身跑在 node 上）。
  - 健康标志：日志出现 `Running target\debug\my-task-desktop.exe` + 1420 同时有 **LISTENING 与 ESTABLISHED**（只有 LISTENING = 窗口没起来）。窗口标题恒为「暂缺」是正常的（`hiddenTitle: true`，标题栏前端自绘）。`HotKey already registered (SUPER+N)` 警告无害。
- 删除大量文件需先授权批量删除守卫（与 `rm` 同一次调用），前台 2 分钟硬超时会中断 → 用后台执行。详见用户级 skill `windows-disk-cleanup`。

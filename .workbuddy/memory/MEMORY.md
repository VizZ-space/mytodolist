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
- **前端是单文件**：全部 HTML/CSS/JS 都在 `my-task-desktop/src/index.html`（约 5197 行），改 UI 只动这一个文件。
- **Esc = 逐层退，且只有一个权威处理器**：所有 Esc 分支都写在 `document.addEventListener("keydown", ...)`（约 4948 行那个）里，**顺序 = 浮层层级**：`#dp` 日期面板（z-index 110）→ `#uselpop` 下拉（108）→ 命令面板 `#palette` → 子任务行内编辑 → `closeModal()`。
  - ⚠️ **不要再给某个浮层单独挂 `document` keydown 监听器**。`#dp` 和 `#uselpop` 原先各自挂了一个，它们注册更早、会**先**把 `DP`/`USOPEN` 清成 null，于是全局处理器里的「浮层还开着吗」判断恒为假 → 一次 Esc 把日期面板**和**整个弹窗一起关掉（用户实测反馈）。同一元素上的多个监听器互不阻塞，`return` 只结束自己那一个。
  - **已知遗留（同类问题，尚未修）**：从任务弹窗里点「＋新建项目 / 新建客户 / 关联笔记」开的**嵌套弹窗**，Esc/取消走的是 `closeModal()` → 直接把整个 `#modal` 清空，写到一半的任务一起丢。可用机制已存在：`snapshotTaskDraft()` 存草稿、`openTask(taskId)` 读回草稿（保存路径就是这么回到任务弹窗的）。不能简单改 `closeModal()` —— 笔记选择器有自己的 `noteReturnTo`/`renderAfterNoteChange` 回退语义，会被带坏，需要单独区分。
- **弹窗有正式规范了：顶层 `ui-spec.md`**（2026-09-15 制定）。要点：
  - **尺寸阶梯只有 5 档，全部命名**，禁止再往 `.sheet` 上写裸 `max-width`：`:root` 里 `--sheet-sm/md/lg/xl/xxl` = 480/560/720/880/1160，类 `.sheet.sm/.lg/.xl/.xxl`（**不写类 = md = 560，默认档**）。判定规则是「一行能并排放几个 240px 字段列」（可用宽 = 档位宽 − 52），560 之所以是默认就是因为它正好两列。
  - **`.sheet-act` 页脚 + `<h3>` 标题是每个弹窗的必需件**（设置面板例外，用 `.set-foot` 多键工具条）。
  - **按钮只有 3 个变体** `btn-ghost / btn-pri / btn-danger`，且**几何（38px / padding 0 16px / radius var(--r-sm) / 14px / 600）定义在变体上**。历史坑：几何原先只写在 `.sheet-act button` 里 → `.set-foot` 的按钮和正文里的「删除…」按钮一用变体就退化成**没有内边距的裸按钮**，只好各自 inline 抄尺寸，抄出来的还互不相同。正文整行操作用 `.btn-wide`（100% + 43px），页脚 `.sheet-act` 用 `flex:1` + 43px。
  - **页脚顺序**：表单页脚 `[取消(ghost)] [主操作(pri)]`；收尾页脚 `[动作(ghost)] [关闭/完成(pri)]`。**共同规则：最右那个必须是实心色**，`btn-ghost` 永远在它左边。
  - **危险色 = 会丢数据的动作，不是「确认」这个动作本身**。`confirmBox(msg,act,opt)` 的 `opt.danger` 才用 `btn-danger`，`opt.ok` 换动词文案。历史坑：早前无条件画红，连「载入示例数据」都是红按钮。
  - ⚠️ **`btn-primary` 这个类名不存在**（只有 `.insp-acts .btn-primary` 这一个作用域内定义过），弹窗里用它 = 按钮完全没样式。已全部改为 `btn-pri`。新增按钮请只从 3 个变体里选。
  - `.two-col.auto-right`（`1fr auto`）= 左栏自适应 + 右栏按内容宽度，用在左右需求不对称处（项目弹窗的「客户 | 色板」：8 色色板固定要 287px，均分半栏只有 245px 会折成两行）。
  - **改弹窗宽度后必须检查子元素有没有因此换行**（色板、芯片行、并列按钮）。
  - **改弹窗后跑 `node .smoke/modal-audit.mjs`**：它真打开每个弹窗量计算样式，把上面每条都写成断言了。
- **信息架构（2026-09-15 重构后）**：导航收敛为「常用 5 项（今天 / 日历 / 笔记 / 任务 / 项目）+ 更多视图 4 项（四象限 / 历史 / 总览 / 报告）」，快捷键 1–9 顺延对应。
  - **看板不再是独立视图**：它是任务数据的「按状态分列」呈现，现已并入「任务」页，由页面最上方的 `ui.taskMode`（`list`/`board`）切换器控制，入口由 `taskModeSwitch()` 渲染。`viewBoard()` 已删除。
  - **「今天」只负责时间焦点**：今天要处理 / 逾期批量治理 / 近 3 天 / 久未跟进 / 已完成入口；**不再**渲染优先级筛选条与「进行中的任务」全量列表（那是任务页的内容），只留一行 `.today-more` 引导去任务页。改动前「今天」和「任务」是同一份列表渲染两遍。
  - 优先级筛选芯片统一由 `priChips()` 产出，`filterBar()`（列表）与 `boardFilterBar()`（看板，多一个项目下拉）共用，保证配色一致。
  - 右侧面板**已于 2026-09-15 整体删除**（用户："任务详情不用侧边展示，改用弹窗样式"）。删掉的东西：`#insp` 元素、`.inspector` 等 71 行 CSS、`inspTask()`/`inspProject()`/`inspHasContent()`/`renderInsp()`/`projTaskItem()`、`ui.selId` 与 `.sel` 选中态、顶栏「收起 / 展开面板」按钮、`⌘\` 快捷键、`editModal` 这个重复动作名。
    - **删它的依据是"它是一层纯重复"**：面板里的内容（客户 / 项目 / 优先级 / 截止 / 子任务进度 / 最近跟进）**任务卡片本身就已经全有了**，而它唯一的出口「编辑」指向的编辑弹窗里也有。它没有提供任何独有信息，只是"点卡片 → 看一眼 → 再点编辑"的中间态。
    - **现在点任务卡（`data-act="edit"`，由 `a==="edit"` 派发）直接 `openTask(id)`**。所以卡片上任何"没有自己 `data-act` 的空白处"点击都会打开弹窗 —— 但**输入类元素有守卫**（派发器开头 `if(e.target.closest("select,textarea,input"))return`），所以点「＋添加子任务」的输入框、跟进文本框都不会误触弹窗。
    - 双击打开编辑已删（单击就打开，弹窗遮罩会吃掉第二次点击，留着是死代码）；右键菜单里的「在右侧面板查看」删掉、「打开完整编辑…」正名为「编辑任务…」并改用 `edit`。
    - 项目详情面板（摘要 / 核心目标 / 进行中 / 待办）随之消失，**不是功能损失**：摘要与核心目标在「编辑项目弹窗」里（可改），两个任务清单就是"点项目名筛出来的任务列表"。
  - **KPI 卡片**：原来的「今日概览」已改成主内容区**最顶部**的一行 4 张卡片（`kpiStrip()`，`KPI_VIEWS` 白名单，**空库也显示**，计数 0 时置灰不可点且不带状态色）。
  - **KPI 卡片同时是任务入口**：点一下钻到「任务」页并按 `ui.kf`（`all`/`todo`/`od`/`doing`/`done`）过滤，再点一次取消；任务页筛选条里会多一枚可关闭的钻取芯片（`kfChip()`，`filterBar()` 与 `boardFilterBar()` 共用）。`ui.kf` **只作用于「任务」页**（`kpiFiltered()` 叠加在 `filtered()` 之上，列表与看板共用），不动其它视图 —— 避免"看不见的条件把页面筛空"。
- **侧栏是两段式（2026-09-15 第七轮改造）**：`.sidebar` 为 `display:flex;flex-direction:column;overflow:hidden`，内含
  - `.side-scroll`（`flex:1;min-height:0;overflow:auto`）= **内容导航**（常用 / 更多视图 / 项目 / 客户 / 标签 / 智能列表），`renderSide()` 的 `h` 全部装在这里面；
  - `.side-foot`（`flex:none`，钉底）= **应用级入口**，目前只有「设置」（`data-act="settings"`，复用原派发，带 `⌘,` 快捷键提示）。
  - **分层原则**：内容导航随视图/数据变化，应用级操作恒定。**不要把「设置 / 账号 / 帮助」这一类塞回顶栏 `.ch-acts`** —— 顶栏只留「导出 / 导入 / ＋新建」（数据操作 + 当前视图主操作）。
  - ⚠️ **设置入口有桌面/移动两份，改一处必须同步另一处**：≤768px 时 `.sidebar{display:none}`，所以顶栏保留一个 `.tbtn.mo-only`（桌面 `display:none` / 移动 `inline-flex`）齿轮按钮兜底。两处都藏起来 = 功能彻底消失，`.smoke/responsive.mjs` 专门守这条。
  - 图标用 `IC.gear`（Feather 齿轮）：原顶栏那个「设置」图标其实是**太阳形状**（`circle r=3` + 八条射线，语义像亮度调节），已换掉。
- **后端**：`src-tauri/src/main.rs`（2092 行），SQLite 建表在 `init_schema`，表有 projects / tasks / subtasks / clients / tags / smart_lists / logs / notes / note_cats / proj_stages。
- **Rust 已完成 import 规范化（2026-09-11 第二轮）**：文件顶部统一 `use` 导入（`serde_json::{json,Value}` / `rusqlite::params` / `std::fs` 等，共 181 处内联全路径改为 use）；macOS 专用的 `Menu/MenuItem/PredefinedMenuItem/TrayIconBuilder/TrayIconId/escape_osascript` 走 `#[cfg(target_os = "macos")]` 门控导入。**保持规范：新增依赖也走 use，别写内联全路径。**
  - **大坑：Windows 构建的「unused import/function」告警对 macOS 专用代码是误报** —— 只被 `#[cfg(target_os="macos")]` 分支使用的符号，在 Windows 上报 unused，正确处理是按平台门控导入/定义，**删了会破坏 macOS 构建**。
- **数据模型：客户与项目原为「平级实体」，现已建立从属关系。**
  - 历史事实：`projects` 原本**没有 `client_id` 列**，`clients(id,name,color,category,sort_idx)` 独立存在，两者都通过 `tasks.client_id` / `tasks.project_id` 直接挂在任务上，因此当时**无法"由项目推导客户"**。
  - **2026-09-11 变更**：已给 `projects` 加 `client_id TEXT` 列（`CREATE TABLE` + `ensure_col` 兼容追加），`proj_sig` / `read_full` / 项目 upsert 三处同步扩列，`tasks.client_id` 保留作为「覆盖值」。于是「客户由项目推导」成立：前端 `projClientId(pid)`，任务弹窗里只做「可见回显」（`.derived-row` / `#mcView`），要覆盖时点「手动指定」展开 `#mc`。
  - ⚠️ **推导必须是"项目优先、任务原值兜底"，不能只看项目**（2026-09-15 修的静默丢数据 bug）：生效值一律走 `effClient(projectId, stored)` = `projClientId(pid) || stored || ""`。三处必须一致 —— `openTask()` 的 `#mcView`、`handleChg("tproj")`（弹窗里换项目）、`saveTask()` 的 `mcv`。**历史行为**：三处都只用 `projClientId()`，于是一个"有客户、但项目没指定客户"的任务被打开再保存，客户就被静默清空（弹窗里显示"未归类"、卡片上却还是客户名，两边自相矛盾）。以前要点两下才踩到，现在单击卡片就打开弹窗，误触代价太高 —— 已由冒烟 IA7 守住（含"必须真选到无项目芯片的卡"的前置断言，防止用例退化成空转）。
  - **加列一律走 `main.rs` 的 `ensure_col(conn, "表名", "列名", "类型")`**：老库无损、老数据为 NULL、不需要迁移脚本。这是本项目新增字段的唯一正确姿势。
  - 前端 `renderSide` 里"客户分组（项目的上层归类）"那句注释历史上与数据模型不符，现已随 `client_id` 落地而成立。
- **E3 笔记多标签也需要 Rust 落库（曾漏掉，已补）**：`notes` 表的读（两处）/ 写 / `note_sig` / `CREATE TABLE` 都必须带 `tags`（存 JSON 数组字符串，与 `tasks.tags` 同格式）。**教训：前端给对象加了新字段，务必同步检查 Rust 侧 SELECT / INSERT / `*_sig` / `CREATE TABLE` 四处，否则一存一读就丢数据。**
- **前端模块作用域**：`<script type="module">`，所有顶层 `var` **挂在模块作用域而非 window**。浏览器里无法用 `Runtime.evaluate` 直接读 `ui` / `IC` / `NAV_*`，做自动化验证时只能断言 DOM。
- **构建**：`npm run build:front` 产出 `dist/`（`tauri.conf.json` 的 `beforeBuildCommand`），`npm run build` 走 Tauri 打包出 msi/nsis。
- **交付物归档**：打包结果放在顶层 `windows-installer/`（当前 0.1.0，msi + exe）。
- **文档与代码有代差**：`overview.md` 描述的是更早的「Mac 单文件 HTML + localStorage」版本，与当前 Tauri + SQLite 实现不符，读它时要注意时效。

## 产品定位与用户偏好（重要）

- **「笔记」是高频使用的核心模块，用户明确表示后续要作为「个人知识库」使用。**
  - 因此在任何导航/信息架构调整中，笔记必须**保持一级常驻**，不可降级进折叠区或二级入口。
  - 笔记模块的演进方向是「知识库」而非「随手记」：多标签、双链/反链、全文搜索、与任务闭环，都是合理需求。
- 用户的诉求是**降低操作复杂度**，不是砍功能。改造时要"把功能放对位置"，而不是删掉。
- **用户对「同屏重复」非常敏感，去重是本项目反复出现的主题**（已连续七轮由用户指出）：①「今天/任务/看板」三个入口渲染同一份任务库 ②右侧「今日概览」面板 vs 顶部 KPI 卡片 ③空状态里的「加一条任务」按钮 vs 页顶快速添加行 ④整行「快速添加行」vs 右上角「新建」按钮 ⑤侧栏项目组里「＋新建项目」与标题右侧的 ＋ ⑥侧栏项目组里的「全部任务」「未归类」⑦**右侧任务详情面板 vs 任务卡片本身**（卡片已经显示了面板里的每一项，面板是纯重复的中间态）。做任何 UI 改动时先自查：**这个信息/入口在同一屏里是否已经存在？** 判断"能不能删"的最快办法：**把这个界面上的内容逐项列出来，看另一处是不是已经全有了**。
- **实体列表只放实体（2026-09-15 第九、十轮定）**：侧栏「项目」组里只有项目。判断标准 —— **点它是"去看某个具体东西"，还是"把列表筛掉一半"？** 后者是筛选维度，不能混进实体列表。按此删掉两条：`「全部任务」`（与导航「任务」是同一状态）和 `「未归类」`（描述"任务没有项目归属"，无对应实体；项目组空着时它还在，读起来像个叫未归类的项目）。
  - **删筛选入口前必须确认数据还有别的路可达**：未归类任务在任务页默认范围（全部任务）里本就在，看板 / 报告的项目下拉仍有「未归类」选项，新建任务时项目留空即未归类。所以删的是"侧栏那一个入口"，不是这个筛选维度。
  - 删掉「未归类」后 `ui.pid` 的取值空间收窄为 `"all"` 或真实项目 id，三处 `ui.pid==="none"` 的死分支已一并清除。
  - 侧栏分组为空时必须给空态提示（"还没有项目，点标题右侧的 ＋ 新建"），与客户/标签/智能列表三组同形，否则标题下空白一片像坏了。
- **任务的新增入口只有一个：右上角「新建」→ 弹窗（2026-09-15）**。原来的「快速添加行」（`quickAdd()` / `#qt` / `addTask()`）已整体删除。`openTask()` 现在同时承担新建与编辑：传 `"__new__"` 走 `draftTask()` 造的虚拟任务，`saveTask` 里判断 `ui.editing==="__new__"` 再 `unshift`。新建时隐藏「关联笔记 / 子任务进度 / 跟进记录 / 删除按钮」四块（都依赖已存在的 `t.id`）。内联语法 `!高`/`@标签`/`明天`/`*重要` 由 `parseTitle()` 在**新建提交时**解析，载体是弹窗标题框（⌘K 命令面板也仍支持）。
  - **给任务对象加字段时注意**：`draftTask()` 需要同步补齐该字段的默认值，否则新建弹窗渲染会读到 `undefined`。
- **空状态原则（2026-09-15 修正）**：空状态回答「①这里放什么 ②为什么有用 ③现在能做什么」；但③**优先指向页面上已有的入口**（一句话说明"点右上角「新建」"），**只有当该动作在页面别处没有入口时才放按钮**。早前"每个空状态都必须有一个主行动按钮"的口径过粗，会造出空转按钮（`newTask` 只是把光标送回已有输入框）。
- 现有笔记实现已经是三栏工作区且有正文搜索，**属于合理结构，改造时以增强为主，不要推翻重做**。
- **信息架构现状（截至 2026-09-15）**：一级导航 = 今天 / 日历 / 笔记 / 任务 / 项目；「更多视图」= 四象限 / 历史 / 总览 / 报告（看板已并入任务页，不再是导航项）。「今天 / 任务 / 日历 / 四象限 / 历史 / 项目」六页顶部有 KPI 卡片（`KPI_VIEWS`）。**「设置」在侧栏底部常驻**（不在顶栏，见上文「侧栏是两段式」）。**主区右侧不再有任何常驻/滑出面板** —— 布局就是「侧栏 + 主区」两栏，`.main` 拿全宽。
- **侧栏四个业务分组（项目 / 客户 / 标签 / 智能列表）形态统一**：标题一律 `.side-hd`（= 名称 + 右侧唯一的「新建」`.side-add`），空态提示统一为"还没有 X，点标题右侧的 ＋ 新建"（`.empty`）。**同一分组的「新建」只允许一个入口**，不要再往列表里加第二条。
- **筛选的可见性与退出（2026-09-15 第九轮）**：侧栏那四个筛选维度平铺到「任务」页顶部（`sideFilterChips()`），每个芯片的 × 只取消自己那一维（`clearDim`）；有任一筛选（含搜索框内容）时恒常驻一枚「清空筛选」（`clearChip()`，**虚线边框 + 灰字**，与"生效中的条件"刻意区分）；点侧栏「任务」= 回到无筛选的全部任务。以前进了列表页完全看不出"正在筛什么"，结果为空时用户会以为数据丢了。

## 写自动化断言的纪律（踩过坑）

- **断言前先确认目标 DOM 由哪个函数渲染**：曾以为「还没有项目」空状态在项目页，实际在 `viewDashboard()`（总览页）；项目页的「新建项目」是 `.pbar` 常驻按钮。
- **同文件多处修改必须串行发 Edit**（并行会让后写覆盖先写，且工具仍回 success）；改完用 `grep -c '<新字面串>' dist/assets/*.js` 校验真的进包。
- **但「十几处字面串批量替换」有更安全的做法：临时 Node 脚本 + 每条替换断言命中次数**（不满足就整体不写盘）。这比 15 次串行 Edit 快得多，也比同消息并行 Edit 安全。两条实测坑：
  - **「新串」必须完整保留「被匹配串」的上下文**。实例：把 `,"importYes")` 换成 `"importYes",{danger:1})`（**漏了开头的逗号**），替换后生成 `确定恢复吗？""importYes"` —— 语法错误。凡是替换串以标点结尾/开头，都要把那个标点同时写进「新串」。
  - **匹配锚点要够独特**。用 `"delYes")` 去匹配会同时命中 `confirmBox(...,"delYes")` 和派发分支 `if(a==="delYes")`（各 1 处 → 误报 2 处）。带上前导逗号 `,"delYes")` 才唯一。**断言命中次数就是为了逼出这类问题。**
  - **脚本跑完立刻 `npm run build:front` 验语法**，再跑运行时测试 —— Vite 会在 1 秒内指出错误行。
- 涉及"首启空态"之类有状态前提的断言，**必须排在造数据步骤之前**。

## 规划类文档（都在顶层）

- `todo-app-design-research.md` — 对标 Things 3 / Todoist / TickTick / OmniFocus / Apple Reminders / MS To Do / Linear 的设计调研
- `enhancement-plan.md` — 六阶段功能增强计划（客户分类 → 标签/智能列表 → 自然语言日期 → 视图 → 回顾/通知 → 打磨），状态为「待拍板」
- `ux-simplification-plan.md` — 交互简化优化方案（2026-09-11 产出），改造包 A/B（2 个）+ C/D/E（3 个）；**§9 实施记录已记录全部落地情况、偏差与顺带修掉的 3 个 bug**；§10~§17 是 2026-09-15 的用户反馈轮次（KPI 可见性 / 空状态去重 / 快速添加行删除 / 弹窗宽度 / Esc 分层 / 设置入口分层 / 侧栏项目组瘦身 / 弹窗样式与尺寸全项目规范 / 右侧任务详情面板改弹窗）
- `ui-spec.md` — **界面规范（标准本身，不是决策过程）**，2026-09-15 制定，覆盖两块：**① 弹窗**（尺寸阶梯 / 结构 / 排版 / 按钮变体与页脚顺序 / 危险色规则 / 18 个弹窗的档位分配 / 检查清单，§1–§7）；**② 入口与筛选**（一个动作一个入口 / 实体列表只放实体 / 筛选可见可退出 / 应用级入口不排内容导航 / **详情不占侧栏、一律弹窗**，§8–§9）。**改弹窗或改侧栏入口前先读它。** 别的模块（列表、看板）将来要立规范也放这里，别另开文件

## 本地验证手段（改完 UI 后必跑）

- **冒烟测试**：`my-task-desktop/.smoke/smoke.mjs`，用法
  `cd my-task-desktop && node .smoke/smoke.mjs`
  - 自带静态服务器（serve `dist/`）+ 拉起本机 Chrome `--headless=new` + 走 CDP 驱动；采集 `Runtime.exceptionThrown` + `Log.entryAdded` 断言 **0 条 JS 错误**。当前 **39/39 断言**。
  - **测键盘行为要用 `document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',...}))`**：`click(c, sel)` 只发点击；Esc 这类全局快捷键必须直接派发到 document（`e.target` 会是 document，与真实按键在焦点元素上略有差异，但覆盖本项目所有全局分支）。
  - **`shot.mjs`（出图核对）**：同套 CDP 驱动，窗口 `--window-size=1440,900`，产出 PNG 是 **1418x802**（真实像素，不是显示时的缩放值）。**量尺寸要按 PNG 原生像素算**，直接看对话里渲染出来的图会被缩放误导。
  - 涉及"首启空态"之类有状态前提的断言，**必须排在造数据步骤之前**。
  - **改了默认视图的内容，务必同步改冒烟测试**：任务卡片现在只在「任务」页渲染，任何断言 `.task` 的用例都要先 `goView(c,'all')`（新增 helper，点侧栏入口，走用户真实路径）。
  - **改完 `src/index.html` 必须先 `vite build` 再跑**（它测的是 `dist/`，不是源码）。
  - 断言全部走**用户可见入口**（点按钮 / 敲回车），不读模块内部变量 —— 因为源码是 `type="module"`，`Runtime.evaluate` 拿不到 `db` / `ui`。这也顺带保证了"入口真的可用"。
  - 已 gitignore，不入库。
  - **写这类测试的坑**：① 每次断言前确认处于预期视图（切换视图会重建 DOM）；② 判断"默认可见字段数"要排除 `<details>` 内部、包着 `<details>` 的容器、以及 `offsetParent===null`（`display:none`）的隐藏块；③ 涉及状态的断言（如"首启空态"）必须放在最前面，否则会被前面步骤造出的数据污染。
  - **静态审计工具**：`.smoke/audit.mjs`（死入口/零引用函数/未用变量/未用图标/未用 CSS 全项审计，当前五类全 0）；`xss-probe.mjs`（真实浏览器属性注入探针）。属性注入类断言必须用真实解析器验证，不能靠读码推理 —— HTML tokenizer 只在遇到**字面**引号时才结束属性值，实体编码的 `&quot;` 不会终止属性。
  - **断点回归探针 `.smoke/responsive.mjs`（2026-09-15 新增，7/7）**：`node .smoke/responsive.mjs`。smoke.mjs 固定跑桌面宽度，覆盖不到「同一功能两入口、靠断点切换」的写法 —— 两边都藏起来就等于功能消失，任一单宽度测试都发现不了。该探针用 `Emulation.setDeviceMetricsOverride` 在 1440 / 420 两端各查一次可见性，并在移动端点一次确认真的能打开。**以后凡是"某个入口只在某断点出现"的改动，都往这里加断言。**
  - **入口去重审计 `.smoke/entry-audit.mjs`（2026-09-15 新增，13 项全过）**：`node .smoke/entry-audit.mjs`。守「同一动作多入口」与「筛选可见/可退出」两类问题 —— 这类问题**不报错、不让任何测试挂掉**，只是让用户站在那儿想"这俩有什么区别"，所以必须变成断言。含 E1 四个分组的「新建」各 1 个、E2 侧栏项目组只列项目本身（不含「全部任务」「未归类」，且无项目时有空态提示）、E5–E9 筛选芯片与「清空筛选」的出现/消失/单维取消、E10 点侧栏「任务」= 无筛选全部任务。
  - **弹窗规范审计 `.smoke/modal-audit.mjs`（2026-09-15 新增，21 个弹窗全过）**：`node .smoke/modal-audit.mjs`。**要用它才发现弹窗问题的正确姿势**：`data-act` 是 document 级事件委托，所以往 body 插一个带 `data-act` 的按钮再 `.click()` 就能从外部打开任何弹窗（源码是 `type="module"`，读不到 `ui`/`db`，这是唯一可行的驱动方式）。它逐项断言宽度在阶梯内 / 有 h3 / 有页脚 / 页脚按钮只用 3 变体 / 等高（sheet-act 还等宽）/ 无 inline 几何 / 最右是实心色 / 危险色语义。
    - ⚠️ **别用 `getBoundingClientRect()` 直接断言尺寸 —— 必须先中和 CSS 动画**。弹窗入场动画是 `.sheet` 从 `scale(.985)` 放大到 1（`.2s`），动画没播完就量会得到 720/560/880 的 0.985 倍 —— **709 / 552 / 867**，报"宽度不在阶梯上"。修法是测量前把动画推到终点 `s.getAnimations({subtree:true}).forEach(a=>a.finish())`，**不是把 sleep 调大**（调大只降低概率）。凡是 `transform` / `opacity` 有入场动画的元素都有这个坑，而且**只在慢机器上偶发** —— 最容易被当成"上次还好好的"。
    - 配套 `.smoke/modal-inventory.mjs` 是**静态**版：扫全部 18 个模板，补齐浏览器不可达的（「关于」面板只能由原生菜单 `routeMenu("about")` 打开）。
    - 取任务 id 的坑：**任务卡片只在「任务」页渲染**（「今天」改版后只做时间焦点，不再铺全量列表），所以要先 `view/all` 才能拿到 `[data-act="edit"][data-id]`。
    - 审计工具的坑：原先用 `\b` 做单词边界，而 `$`（`$("#side")` 那个选择器函数）不是单词字符，导致它**永远**被误报成零引用函数。已改用 `(^|[^\w$])…($|[^\w$])`。另：动态拼接的类名（如 `" p-"+f[0]`）审计扫不到，宁可写成字面量数组，既消误报又更清晰。
- **Rust 测试**：`cargo test --no-default-features`（`src-tauri/` 下 `mod tests`，10/10：save/load 往返 + `note_tags`/`project_client_id` 回归 + 老表自动升级两则）。首次编译依赖约 5-10 分钟。
- **评审纪律（receiving-code-review 实证教训）**：评审给的 Critical 必须先在代码里找到对应行验证再实施；本轮 1 条 XSS Critical 经 `.smoke/xss-probe.mjs` 实测为误报（`inlineMd` 首行已 `esc()`，转义发生在 `[[ ]]` 提取之前）。
- **改 UI 后再加一道「产物校验」**：`.smoke/shot.mjs` 用同一套 headless Chrome 出图（`.smoke/shots/*.png`），可人工核对版式。**重要教训（2026-09-15）**：同一条消息里对**同一个文件**并行发多个 Edit，后写会覆盖先写，前面的改动会**静默丢失**（工具仍报 success）—— `filterBar()` 的改动就这样被吞掉，直到 grep 构建产物才发现。所以：**同文件的多处修改必须串行发**；改完用 `grep -c '<新代码里的字面串>' dist/assets/*.js` 确认真的进包（注意 Vite 会把 JS 拆到 `dist/assets/index-*.js`，`dist/index.html` 里只有 CSS/HTML）。

## 本机环境注意

- **WebView2 崩溃（"Error launching CrashSender.exe" 弹窗）已定位并修复（2026-09-11）**：
  - 根因：腾讯 WeType 输入法（`wetype_tip_core.dll`）与 WorkBuddy 终端链的 `tsbx.dll` 向 WebView2 进程注入 DLL，与 Chromium 沙箱冲突 → `msedge.dll` 内 `0x80000003`（CHECK 断点）崩溃，启动后约 17 秒内必崩
  - 次要因素：本机 WebView2 运行时（152.0.4191.66）目录里**没有 CrashSender.exe**，任何崩溃都会弹「Error launching CrashSender.exe」对话框（弹窗只是症状不是原因）
  - 修复：`tauri.conf.json` 的窗口配置加 `"additionalBrowserArgs": "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --no-sandbox"`（已验证：加参后稳定，仅 `--no-sandbox` 即可、无需禁 GPU；注意 additionalBrowserArgs 会**替换** Tauri 默认参数，默认那三个 disable-features 要自己带上）
  - 排查工具：崩溃转储在 `%LOCALAPPDATA%\com.zhuanz.mytask\EBWebView\Crashpad\reports\`，`.smoke/dump-parse.cjs`（零依赖 Node 脚本）可解析 minidump 的异常代码/出错模块/注入 DLL 清单
- Bash 工具偶尔丢 PATH，命令前加 `export PATH="/usr/bin:/bin:$PATH"`
- 删除大量文件需先授权批量删除守卫（与 `rm` 同一次调用），且前台 2 分钟硬超时会中断 → 用后台执行
- 详见用户级 skill `windows-disk-cleanup`
- **Windows 版 git 不认 MSYS 路径**：`git -C /e/workspace/...` 会报 `cannot change to`。必须用原生路径 `git -C "E:/workspace/my-todolist"`，且**不要**设 `MSYS_NO_PATHCONV=1`（它会阻止路径自动转换）
- **本机 git fetch/push 不会自动写 `refs/remotes/origin/*`**：`git fetch` 会打印 `[new branch] main -> origin/main` 但引用实际未落盘，`git status` 因此显示 `[gone]`、`origin/main` 无法解析。**绕过办法**：手动写入引用文件
  `mkdir -p .git/refs/remotes/origin && echo <sha> > .git/refs/remotes/origin/main`
  （推送本身是成功的，远程内容正确，仅本地引用记账有问题）
- **⚠️ 本机还会丢 `.git/refs/heads/*` 与 `.git/objects`（2026-09-15 实测遇到一次）**：表现为 `git rev-parse HEAD` 报 `unknown revision`、`git status` 把所有文件显示成 `A `（像未出生的分支）、`.git` 只有 151K。**根因未确认**（怀疑与 `git stash`／某清理进程有关，`.git/objects/pack` 也不存在）。
  - **恢复办法（工作区文件一直是完好的，不要慌）**：
    1. `git fetch origin main` —— 远程是完整的，会把对象拉回来（`git cat-file -t <sha>` 能验证）
    2. 手动写两个引用：`printf '%s\n' <sha> > .git/refs/heads/main` 与 `.git/refs/remotes/origin/main`（要 `mkdir -p .git/refs/heads .git/refs/remotes/origin`）
    3. `git log` / `git status` 恢复正常，未提交的工作区改动会重新显示为 ` M`
  - **教训**：在这个仓库里**不要用 `git stash`**（改用 `cp src/index.html /tmp/x.html` 这种手工备份），且执行任何 git 操作前先 `git rev-parse HEAD` 确认引用还在；`git log --oneline -1` 要真的看到 SHA 才算正常。

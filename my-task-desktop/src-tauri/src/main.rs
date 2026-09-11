#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use rusqlite::{params, Connection};
use serde_json::{json, Value};
use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Manager, State};
// 托盘与托盘菜单项只在 macOS 编译；导入一并按平台门控，Windows 构建才不会出现「未使用导入」告警
#[cfg(target_os = "macos")]
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
#[cfg(target_os = "macos")]
use tauri::tray::{TrayIconBuilder, TrayIconId};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

struct DbState(Mutex<Connection>);
/// 应用数据目录：~/Documents/我的任务台
struct Paths {
    app_dir: PathBuf,
}

/// 保留最近 KEEP 份每日快照
const KEEP_SNAPSHOTS: usize = 7;

/// 主数据键（必须与前端 KEY 一致）。其余 key（如 AI 配置）仍走 store 表。
const MAIN_KEY: &str = "wb_taskdeck_data_v1";

/// 列出快照文件，按文件名（tasks-YYYY-MM-DD.db，字典序即时间序）升序
fn list_snapshots(snap_dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = match fs::read_dir(snap_dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("tasks-") && n.ends_with(".db"))
                    .unwrap_or(false)
            })
            .collect(),
        Err(_) => return Vec::new(),
    };
    v.sort();
    v
}

/// 每天首次启动自动做一份快照，只保留最近 KEEP_SNAPSHOTS 份。
/// 用 SQLite 官方 backup API 而不是直接复制文件 —— 即使库正在被使用也能拿到一致的副本。
fn daily_snapshot(conn: &Connection, app_dir: &Path) {
    // 空库没有备份价值，跳过（避免首次启动就占掉一个名额）
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))
        .unwrap_or(0);
    if rows == 0 {
        return;
    }

    // 交给 SQLite 算本地日期，省掉一个时区依赖
    let today: String = match conn.query_row("SELECT date('now','localtime')", [], |r| r.get(0)) {
        Ok(d) => d,
        Err(_) => return,
    };

    let snap_dir = app_dir.join("snapshots");
    if fs::create_dir_all(&snap_dir).is_err() {
        return;
    }

    let target = snap_dir.join(format!("tasks-{}.db", today));
    if !target.exists() {
        if conn
            .backup(rusqlite::DatabaseName::Main, &target, None)
            .is_err()
        {
            return;
        }
    }

    // 超出份数就删最旧的
    let files = list_snapshots(&snap_dir);
    if files.len() > KEEP_SNAPSHOTS {
        for old in &files[..files.len() - KEEP_SNAPSHOTS] {
            fs::remove_file(old).ok();
        }
    }
}

/// 给设置页显示：快照目录、份数、最新一份的日期
#[tauri::command]
fn snapshot_info(paths: State<Paths>) -> Value {
    let snap_dir = paths.app_dir.join("snapshots");
    let files = list_snapshots(&snap_dir);
    let latest = files
        .last()
        .and_then(|p| p.file_name().and_then(|n| n.to_str()))
        .map(|n| n.trim_start_matches("tasks-").trim_end_matches(".db").to_string())
        .unwrap_or_default();
    json!({
        "dir": snap_dir.to_string_lossy(),
        "dataDir": paths.app_dir.to_string_lossy(),
        "count": files.len(),
        "keep": KEEP_SNAPSHOTS,
        "latest": latest,
    })
}

/// 在文件管理器里打开数据目录（"which" = data | snapshots）
#[tauri::command]
fn reveal_dir(which: String, paths: State<Paths>) -> Result<(), String> {
    let target = if which == "snapshots" {
        paths.app_dir.join("snapshots")
    } else {
        paths.app_dir.clone()
    };
    fs::create_dir_all(&target).ok();
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&target)
            .spawn()
            .map_err(|e| format!("打开文件夹失败：{}", e))?;
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer.exe")
            .arg(&target)
            .spawn()
            .map_err(|e| format!("打开文件夹失败：{}", e))?;
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Command::new("xdg-open")
            .arg(&target)
            .spawn()
            .map_err(|e| format!("打开文件夹失败：{}", e))?;
    }
    Ok(())
}

/// 配置文件路径：记录自定义数据目录（与 exe 同级的 data_dir.txt）
fn config_file_path() -> PathBuf {
    let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    let exe_dir = exe.parent().unwrap_or_else(|| Path::new("."));
    exe_dir.join("data_dir.txt")
}

/// 读取自定义数据目录：有则用，无则默认 exe 同级 data/
fn resolve_data_dir() -> PathBuf {
    let cfg = config_file_path();
    if let Ok(custom) = fs::read_to_string(&cfg) {
        let custom = custom.trim();
        if !custom.is_empty() && Path::new(custom).is_dir() {
            return PathBuf::from(custom);
        }
    }
    // 默认：exe 同级的 data 目录
    let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    let exe_dir = exe.parent().unwrap_or_else(|| Path::new("."));
    exe_dir.join("data")
}

/// 返回当前数据目录路径
#[tauri::command]
fn get_data_dir(paths: State<Paths>) -> String {
    paths.app_dir.to_string_lossy().to_string()
}

/// 重启应用
#[tauri::command]
fn restart_app(app: AppHandle) {
    let _ = app.restart();
}

/// 选择新数据目录（弹出文件夹选择对话框），迁移数据并重启
#[tauri::command]
async fn set_data_dir(app: AppHandle, paths: State<'_, Paths>) -> Result<String, String> {
    let (tx, rx) = mpsc::channel();
    app.dialog()
        .file()
        .set_title("选择数据存储目录")
        .pick_folder(move |p| {
            let _ = tx.send(p);
        });
    let picked = rx.recv().map_err(|e| format!("对话框异常：{}", e))?;
    let new_dir = match picked {
        Some(fp) => fp.into_path().map_err(|e| format!("路径无效：{}", e))?,
        None => return Ok(String::new()), // 用户取消
    };
    let new_dir_str = new_dir.to_string_lossy().to_string();

    // 创建目标目录
    fs::create_dir_all(&new_dir).map_err(|e| format!("创建目录失败：{}", e))?;

    // 复制数据库和快照
    let old_db = paths.app_dir.join("tasks.db");
    let new_db = new_dir.join("tasks.db");
    if old_db.exists() {
        fs::copy(&old_db, &new_db).map_err(|e| format!("复制数据库失败：{}", e))?;
    }
    let old_snap = paths.app_dir.join("snapshots");
    let new_snap = new_dir.join("snapshots");
    if old_snap.exists() {
        fs::create_dir_all(&new_snap).ok();
        if let Ok(entries) = fs::read_dir(&old_snap) {
            for entry in entries.flatten() {
                let from = entry.path();
                let to = new_snap.join(entry.file_name());
                fs::copy(&from, &to).ok();
            }
        }
    }

    // 记录新路径
    fs::write(config_file_path(), &new_dir_str).map_err(|e| format!("写入配置失败：{}", e))?;

    Ok(new_dir_str)
}

/// E4：原生「保存文件」对话框，把备份 JSON 落到用户选的位置。
/// 返回空字符串表示用户取消。
#[tauri::command]
async fn export_backup(
    app: AppHandle,
    json: String,
    filename: String,
) -> Result<String, String> {
    let (tx, rx) = mpsc::channel();
    app.dialog()
        .file()
        .set_title("导出任务台备份")
        .set_file_name(&filename)
        .add_filter("JSON 备份", &["json"])
        .save_file(move |p| {
            let _ = tx.send(p);
        });
    let picked = rx.recv().map_err(|e| format!("对话框异常：{}", e))?;
    match picked {
        Some(fp) => {
            let path = fp.into_path().map_err(|e| format!("路径无效：{}", e))?;
            fs::write(&path, json).map_err(|e| format!("写入失败：{}", e))?;
            Ok(path.to_string_lossy().to_string())
        }
        None => Ok(String::new()),
    }
}

/// 通用「保存文本文件」对话框（用于导出 Markdown / 纯文本等）。
/// 返回空字符串表示用户取消。
#[tauri::command]
async fn save_text_file(
    app: AppHandle,
    content: String,
    filename: String,
) -> Result<String, String> {
    let (tx, rx) = mpsc::channel();
    app.dialog()
        .file()
        .set_title("导出文件")
        .set_file_name(&filename)
        .add_filter("Markdown", &["md"])
        .add_filter("纯文本", &["txt"])
        .save_file(move |p| {
            let _ = tx.send(p);
        });
    let picked = rx.recv().map_err(|e| format!("对话框异常：{}", e))?;
    match picked {
        Some(fp) => {
            let path = fp.into_path().map_err(|e| format!("路径无效：{}", e))?;
            fs::write(&path, content).map_err(|e| format!("写入失败：{}", e))?;
            Ok(path.to_string_lossy().to_string())
        }
        None => Ok(String::new()),
    }
}

/// E4：原生「打开文件」对话框，读回备份 JSON 文本。
/// 返回空字符串表示用户取消。
#[tauri::command]
async fn import_backup(app: AppHandle) -> Result<String, String> {
    let (tx, rx) = mpsc::channel();
    app.dialog()
        .file()
        .set_title("选择要恢复的备份文件")
        .add_filter("JSON 备份", &["json"])
        .pick_file(move |p| {
            let _ = tx.send(p);
        });
    let picked = rx.recv().map_err(|e| format!("对话框异常：{}", e))?;
    match picked {
        Some(fp) => {
            let path = fp.into_path().map_err(|e| format!("路径无效：{}", e))?;
            let txt = fs::read_to_string(&path).map_err(|e| format!("读取失败：{}", e))?;
            Ok(txt)
        }
        None => Ok(String::new()),
    }
}

/// 直接读 store 表的原始值（用于 AI 配置等非主数据）
fn read_store_raw(conn: &Connection, key: &str) -> Option<String> {
    let mut stmt = conn.prepare("SELECT v FROM store WHERE k = ?").ok()?;
    let mut rows = stmt.query(params![key]).ok()?;
    match rows.next().ok()? {
        Some(r) => r.get(0).ok(),
        None => None,
    }
}

/// 直接写 store 表（用于 AI 配置等非主数据）
fn write_store_raw(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO store(k, v) VALUES(?, ?)",
        params![key, value],
    )?;
    Ok(())
}

/// bool -> 0/1
fn b2i(b: Option<bool>) -> i64 {
    if b.unwrap_or(false) {
        1
    } else {
        0
    }
}

/// meta 值统一转成可存文本（字符串原样，其余 JSON 化）
fn meta_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// 把整包 db JSON 拆进规范化表（事务内整体替换，简单且无孤儿行）
/// 读取规范化表，重建 (projects 数组, tasks 数组)，每个 task 含 subtasks/logs。
/// 结构与 load_main 的读取完全一致，供增量保存时做差异比对，避免每次全量重写。
struct FullData {
    projects: Vec<Value>,
    tasks: Vec<Value>,
    clients: Vec<Value>,
    tags: Vec<Value>,
    notes: Vec<Value>,
    smart_lists: Vec<Value>,
    proj_stages: Vec<Value>,
}

fn read_full(conn: &Connection) -> FullData {
    let mut projects: Vec<Value> = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id,name,color,summary,goal,status,landing,isContract,signDate,landYear,budget,contract,client_id FROM projects ORDER BY sort_idx") {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(r)) = rows.next() {
                projects.push(json!({
                    "id": r.get::<_,String>(0).unwrap_or_default(),
                    "name": r.get::<_,String>(1).unwrap_or_default(),
                    "color": r.get::<_,String>(2).unwrap_or_default(),
                    "summary": r.get::<_,String>(3).unwrap_or_default(),
                    "goal": r.get::<_,String>(4).unwrap_or_default(),
                    "status": r.get::<_,String>(5).unwrap_or_default(),
                    "landing": r.get::<_,String>(6).unwrap_or_default(),
                    "isContract": r.get::<_,i64>(7).unwrap_or(0),
                    "signDate": r.get::<_,String>(8).unwrap_or_default(),
                    "landYear": r.get::<_,i64>(9).unwrap_or(0),
                    "budget": r.get::<_,f64>(10).unwrap_or(0.0),
                    "contract": r.get::<_,f64>(11).unwrap_or(0.0),
                    "clientId": r.get::<_,String>(12).unwrap_or_default(),
                }));
            }
        }
    }

    // 项目阶段（自定义，可增删改）
    let mut proj_stages: Vec<Value> = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id,name,ord,landed FROM proj_stages ORDER BY ord") {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(r)) = rows.next() {
                proj_stages.push(json!({
                    "id": r.get::<_,String>(0).unwrap_or_default(),
                    "name": r.get::<_,String>(1).unwrap_or_default(),
                    "ord": r.get::<_,i64>(2).unwrap_or(0),
                    "landed": r.get::<_,i64>(3).unwrap_or(0),
                }));
            }
        }
    }

    // 客户 / 标签 / 笔记 / 智能列表：供 save_main 增量比对
    let mut clients: Vec<Value> = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id,name,color,category FROM clients ORDER BY sort_idx") {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(r)) = rows.next() {
                clients.push(json!({
                    "id": r.get::<_,String>(0).unwrap_or_default(),
                    "name": r.get::<_,String>(1).unwrap_or_default(),
                    "color": r.get::<_,String>(2).unwrap_or_default(),
                    "category": r.get::<_,String>(3).unwrap_or_default(),
                }));
            }
        }
    }
    let mut tags: Vec<Value> = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id,name,color,category FROM tags ORDER BY sort_idx") {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(r)) = rows.next() {
                tags.push(json!({
                    "id": r.get::<_,String>(0).unwrap_or_default(),
                    "name": r.get::<_,String>(1).unwrap_or_default(),
                    "color": r.get::<_,String>(2).unwrap_or_default(),
                    "category": r.get::<_,String>(3).unwrap_or_default(),
                }));
            }
        }
    }
    let mut notes: Vec<Value> = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id,title,cat,md,created_at,updated_at,pinned,tags FROM notes ORDER BY sort_idx") {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(r)) = rows.next() {
                /* E3：多标签以 JSON 数组字符串落库（与 tasks.tags 同格式） */
                let ntags_raw: String = r.get(7).unwrap_or_default();
                let ntags: Value = serde_json::from_str(&ntags_raw)
                    .unwrap_or(Value::Array(vec![]));
                notes.push(json!({
                    "id": r.get::<_,String>(0).unwrap_or_default(),
                    "title": r.get::<_,String>(1).unwrap_or_default(),
                    "cat": r.get::<_,String>(2).unwrap_or_default(),
                    "md": r.get::<_,String>(3).unwrap_or_default(),
                    "createdAt": r.get::<_,i64>(4).unwrap_or(0),
                    "updatedAt": r.get::<_,i64>(5).unwrap_or(0),
                    "pinned": r.get::<_,i64>(6).unwrap_or(0) == 1,
                    "tags": ntags,
                }));
            }
        }
    }
    let mut smart_lists: Vec<Value> = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id,name,query FROM smart_lists ORDER BY sort_idx") {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(r)) = rows.next() {
                let qraw: String = r.get(2).unwrap_or_default();
                let query: Value = serde_json::from_str(&qraw)
                    .unwrap_or(Value::Null);
                smart_lists.push(json!({
                    "id": r.get::<_,String>(0).unwrap_or_default(),
                    "name": r.get::<_,String>(1).unwrap_or_default(),
                    "query": query,
                }));
            }
        }
    }

    // 批量读取子任务 / 日志，按 task_id 分组，避免每条任务一次嵌套查询
    let mut subs_map: HashMap<String, Vec<Value>> = HashMap::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id,task_id,title,done,done_at,due,priority,note_id,status FROM subtasks ORDER BY task_id,sort_idx") {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(r)) = rows.next() {
                let tid: String = r.get(1).unwrap_or_default();
                subs_map.entry(tid).or_default().push(json!({
                    "id": r.get::<_,String>(0).unwrap_or_default(),
                    "title": r.get::<_,String>(2).unwrap_or_default(),
                    "done": r.get::<_,i64>(3).unwrap_or(0) == 1,
                    "doneAt": r.get::<_,i64>(4).unwrap_or(0),
                    "due": r.get::<_,String>(5).unwrap_or_default(),
                    "priority": r.get::<_,String>(6).unwrap_or_default(),
                    "noteId": r.get::<_,String>(7).unwrap_or_default(),
                    "status": r.get::<_,String>(8).unwrap_or_default(),
                }));
            }
        }
    }
    let mut logs_map: HashMap<String, Vec<Value>> = HashMap::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id,task_id,at,text FROM logs ORDER BY task_id,at") {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(r)) = rows.next() {
                let tid: String = r.get(1).unwrap_or_default();
                logs_map.entry(tid).or_default().push(json!({
                    "id": r.get::<_,String>(0).unwrap_or_default(),
                    "at": r.get::<_,i64>(2).unwrap_or(0),
                    "text": r.get::<_,String>(3).unwrap_or_default(),
                }));
            }
        }
    }

    let mut tasks: Vec<Value> = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id,project_id,client_id,title,priority,due,note,done,done_at,status,created_at,repeat,waiting,waiting_for,waiting_since,planned,tags,important,note_id FROM tasks ORDER BY sort_idx") {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(r)) = rows.next() {
                let id: String = r.get(0).unwrap_or_default();
                let subs = subs_map.remove(&id).unwrap_or_default();
                let logs = logs_map.remove(&id).unwrap_or_default();
                let tags_raw: String = r.get(16).unwrap_or_default();
                let tags: Value = serde_json::from_str(&tags_raw)
                    .unwrap_or(Value::Array(vec![]));
                tasks.push(json!({
                    "id": id,
                    "projectId": r.get::<_,String>(1).unwrap_or_default(),
                    "clientId": r.get::<_,String>(2).unwrap_or_default(),
                    "title": r.get::<_,String>(3).unwrap_or_default(),
                    "priority": r.get::<_,String>(4).unwrap_or_default(),
                    "due": r.get::<_,String>(5).unwrap_or_default(),
                    "note": r.get::<_,String>(6).unwrap_or_default(),
                    "done": r.get::<_,i64>(7).unwrap_or(0) == 1,
                    "doneAt": r.get::<_,i64>(8).unwrap_or(0),
                    "status": r.get::<_,String>(9).unwrap_or_default(),
                    "createdAt": r.get::<_,i64>(10).unwrap_or(0),
                    "repeat": r.get::<_,String>(11).unwrap_or_default(),
                    "waiting": r.get::<_,i64>(12).unwrap_or(0) == 1,
                    "waitingFor": r.get::<_,String>(13).unwrap_or_default(),
                    "waitingSince": r.get::<_,i64>(14).unwrap_or(0),
                    "planned": r.get::<_,i64>(15).unwrap_or(0) == 1,
                    "tags": tags,
                    "important": r.get::<_,i64>(17).unwrap_or(0) == 1,
                    "noteId": r.get::<_,String>(18).unwrap_or_default(),
                    "subtasks": subs,
                    "logs": logs,
                }));
            }
        }
    }
    FullData { projects, tasks, clients, tags, notes, smart_lists, proj_stages }
}

/// 项目签名：用于判断内容是否变化（不含 sort_idx，顺序由数组下标决定）
fn proj_sig(p: &Value) -> String {
    format!("{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        p["id"].as_str().unwrap_or(""),
        p["name"].as_str().unwrap_or(""),
        p["color"].as_str().unwrap_or(""),
        p["summary"].as_str().unwrap_or(""),
        p["goal"].as_str().unwrap_or(""),
        p["status"].as_str().unwrap_or(""),
        p["landing"].as_str().unwrap_or(""),
        (p["isContract"].as_bool().unwrap_or(false) as i64),
        p["signDate"].as_str().unwrap_or(""),
        p["landYear"].as_i64().unwrap_or(0),
        p["budget"].as_f64().unwrap_or(0.0),
        p["contract"].as_f64().unwrap_or(0.0),
        p["clientId"].as_str().unwrap_or(""),
    )
}

/// 客户 / 标签 / 笔记 / 智能列表 签名：与 save_main 写入的字段保持一致
fn client_sig(c: &Value) -> String {
    format!("{}|{}|{}|{}",
        c["id"].as_str().unwrap_or(""),
        c["name"].as_str().unwrap_or(""),
        c["color"].as_str().unwrap_or(""),
        c["category"].as_str().unwrap_or(""))
}
fn tag_sig(t: &Value) -> String {
    format!("{}|{}|{}|{}",
        t["id"].as_str().unwrap_or(""),
        t["name"].as_str().unwrap_or(""),
        t["color"].as_str().unwrap_or(""),
        t["category"].as_str().unwrap_or(""))
}
fn note_sig(n: &Value) -> String {
    /* E3：tags 必须进签名，否则"只改标签"这次改动会被判定为无变化而不落库 */
    format!("{}|{}|{}|{}|{}|{}|{}|{}",
        n["id"].as_str().unwrap_or(""),
        n["title"].as_str().unwrap_or(""),
        n["cat"].as_str().unwrap_or(""),
        n["md"].as_str().unwrap_or(""),
        n["createdAt"].as_i64().unwrap_or(0),
        n["updatedAt"].as_i64().unwrap_or(0),
        b2i(n["pinned"].as_bool()),
        n["tags"].to_string())
}
fn smart_sig(s: &Value) -> String {
    format!("{}|{}|{}",
        s["id"].as_str().unwrap_or(""),
        s["name"].as_str().unwrap_or(""),
        s["query"].to_string())
}

/// 任务签名：涵盖 16 个字段 + 子任务 + 日志，作为行级变化判定依据。
/// idx 为数组下标（影响 sort_idx）；子任务带 j 下标；日志按内容排序（与 ORDER BY at 一致）。
fn task_sig(t: &Value, idx: usize) -> String {
    let mut s = format!("{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        idx,
        t["id"].as_str().unwrap_or(""),
        t["projectId"].as_str().unwrap_or(""),
        t["clientId"].as_str().unwrap_or(""),
        t["title"].as_str().unwrap_or(""),
        t["priority"].as_str().unwrap_or(""),
        t["due"].as_str().unwrap_or(""),
        b2i(t["done"].as_bool()),
        t["doneAt"].as_i64().unwrap_or(0),
        t["status"].as_str().unwrap_or(""),
        t["createdAt"].as_i64().unwrap_or(0),
        t["repeat"].as_str().unwrap_or(""),
        b2i(t["waiting"].as_bool()),
        t["waitingFor"].as_str().unwrap_or(""),
        t["waitingSince"].as_i64().unwrap_or(0),
        b2i(t["planned"].as_bool()),
        t["note"].as_str().unwrap_or(""),
        t["tags"].to_string(),
        b2i(t["important"].as_bool()),
        t["noteId"].as_str().unwrap_or(""),
    );
    if let Some(sa) = t["subtasks"].as_array() {
        for (j, sub) in sa.iter().enumerate() {
            s.push_str(&format!("||S{}|{}|{}|{}|{}|{}|{}|{}|{}",
                j,
                sub["id"].as_str().unwrap_or(""),
                sub["title"].as_str().unwrap_or(""),
                b2i(sub["done"].as_bool()),
                sub["doneAt"].as_i64().unwrap_or(0),
                sub["due"].as_str().unwrap_or(""),
                sub["priority"].as_str().unwrap_or(""),
                sub["noteId"].as_str().unwrap_or(""),
                sub["status"].as_str().unwrap_or("")));
        }
    }
    if let Some(la) = t["logs"].as_array() {
        let mut lv: Vec<String> = la.iter().map(|l| format!("{}|{}|{}",
            l["id"].as_str().unwrap_or(""),
            l["at"].as_i64().unwrap_or(0),
            l["text"].as_str().unwrap_or(""))).collect();
        lv.sort();
        for x in lv { s.push_str(&format!("||L{}", x)); }
    }
    s
}

fn save_main(conn: &mut Connection, value: &str) -> Result<(), String> {
    let v: Value =
        serde_json::from_str(value).map_err(|e| format!("数据解析失败：{}", e))?;
    // 先读取已落盘状态用于差异比对（必须在开启事务之前，避免与 tx 的可变借用冲突）
    let cur = read_full(conn);
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    // ---- 项目：删除已不存在的，再增量 upsert 内容变化的 ----
    let cur_proj_map: HashMap<String, String> = cur.projects.iter()
        .map(|p| (p["id"].as_str().unwrap_or("").to_string(), proj_sig(p)))
        .collect();
    let in_proj_ids: HashSet<String> = v["projects"].as_array()
        .map(|a| a.iter().map(|p| p["id"].as_str().unwrap_or("").to_string()).collect())
        .unwrap_or_default();
    for pid in cur_proj_map.keys() {
        if !in_proj_ids.contains(pid) {
            tx.execute("DELETE FROM projects WHERE id=?", params![pid.to_string()]).map_err(|e| e.to_string())?;
        }
    }
    if let Some(arr) = v["projects"].as_array() {
        for (i, p) in arr.iter().enumerate() {
            let id = p["id"].as_str().unwrap_or("").to_string();
            let changed = cur_proj_map.get(&id).map_or(true, |csig| proj_sig(p) != *csig);
            if changed {
                tx.execute(
                    "INSERT OR REPLACE INTO projects(id,name,color,summary,goal,status,landing,isContract,signDate,landYear,budget,contract,client_id,sort_idx) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                    params![
                        id,
                        p["name"].as_str().unwrap_or(""),
                        p["color"].as_str().unwrap_or(""),
                        p["summary"].as_str().unwrap_or(""),
                        p["goal"].as_str().unwrap_or(""),
                        p["status"].as_str().unwrap_or(""),
                        p["landing"].as_str().unwrap_or(""),
                        p["isContract"].as_bool().unwrap_or(false) as i64,
                        p["signDate"].as_str().unwrap_or(""),
                        p["landYear"].as_i64().unwrap_or(0),
                        p["budget"].as_f64().unwrap_or(0.0),
                        p["contract"].as_f64().unwrap_or(0.0),
                        p["clientId"].as_str().unwrap_or(""),
                        i as i64
                    ],
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }

    // ---- 项目阶段：删除已不存在的，再 upsert ----
    let cur_stage_ids: HashSet<String> = cur.proj_stages.iter()
        .map(|s| s["id"].as_str().unwrap_or("").to_string()).collect();
    let in_stage_ids: HashSet<String> = v["projStages"].as_array()
        .map(|a| a.iter().map(|s| s["id"].as_str().unwrap_or("").to_string()).collect()).unwrap_or_default();
    for sid in &cur_stage_ids {
        if !in_stage_ids.contains(sid) {
            tx.execute("DELETE FROM proj_stages WHERE id=?", params![sid.to_string()]).map_err(|e| e.to_string())?;
        }
    }
    if let Some(arr) = v["projStages"].as_array() {
        for (i, s) in arr.iter().enumerate() {
            let sid = s["id"].as_str().unwrap_or("").to_string();
            tx.execute(
                "INSERT OR REPLACE INTO proj_stages(id,name,ord,landed) VALUES(?,?,?,?)",
                params![
                    sid,
                    s["name"].as_str().unwrap_or(""),
                    i as i64,
                    s["landed"].as_i64().unwrap_or(0),
                ],
            ).map_err(|e| e.to_string())?;
        }
    }

    // ---- 客户：删除已不存在的，再增量 upsert 内容变化的 ----
    let cur_client_map: HashMap<String, String> = cur.clients.iter()
        .map(|c| (c["id"].as_str().unwrap_or("").to_string(), client_sig(c)))
        .collect();
    let in_client_ids: HashSet<String> = v["clients"].as_array()
        .map(|a| a.iter().map(|c| c["id"].as_str().unwrap_or("").to_string()).collect())
        .unwrap_or_default();
    for cid in cur_client_map.keys() {
        if !in_client_ids.contains(cid) {
            tx.execute("DELETE FROM clients WHERE id=?", params![cid.to_string()]).map_err(|e| e.to_string())?;
        }
    }
    if let Some(arr) = v["clients"].as_array() {
        for (i, c) in arr.iter().enumerate() {
            let id = c["id"].as_str().unwrap_or("").to_string();
            let changed = cur_client_map.get(&id).map_or(true, |cs| client_sig(c) != *cs);
            if changed {
                tx.execute(
                    "INSERT OR REPLACE INTO clients(id,name,color,category,sort_idx) VALUES(?,?,?,?,?)",
                    params![
                        id,
                        c["name"].as_str().unwrap_or(""),
                        c["color"].as_str().unwrap_or(""),
                        c["category"].as_str().unwrap_or(""),
                        i as i64
                    ],
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }

    // ---- 标签：删除已不存在的，再增量 upsert 内容变化的 ----
    let cur_tag_map: HashMap<String, String> = cur.tags.iter()
        .map(|t| (t["id"].as_str().unwrap_or("").to_string(), tag_sig(t)))
        .collect();
    let in_tag_ids: HashSet<String> = v["tags"].as_array()
        .map(|a| a.iter().map(|t| t["id"].as_str().unwrap_or("").to_string()).collect())
        .unwrap_or_default();
    for tgid in cur_tag_map.keys() {
        if !in_tag_ids.contains(tgid) {
            tx.execute("DELETE FROM tags WHERE id=?", params![tgid.to_string()]).map_err(|e| e.to_string())?;
        }
    }
    if let Some(arr) = v["tags"].as_array() {
        for (i, t) in arr.iter().enumerate() {
            let id = t["id"].as_str().unwrap_or("").to_string();
            let changed = cur_tag_map.get(&id).map_or(true, |cs| tag_sig(t) != *cs);
            if changed {
                tx.execute(
                    "INSERT OR REPLACE INTO tags(id,name,color,category,sort_idx) VALUES(?,?,?,?,?)",
                    params![
                        id,
                        t["name"].as_str().unwrap_or(""),
                        t["color"].as_str().unwrap_or(""),
                        t["category"].as_str().unwrap_or(""),
                        i as i64
                    ],
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }

    // ---- 笔记：删除已不存在的，再增量 upsert 内容变化的 ----
    let cur_note_map: HashMap<String, String> = cur.notes.iter()
        .map(|n| (n["id"].as_str().unwrap_or("").to_string(), note_sig(n)))
        .collect();
    let in_note_ids: HashSet<String> = v["notes"].as_array()
        .map(|a| a.iter().map(|n| n["id"].as_str().unwrap_or("").to_string()).collect())
        .unwrap_or_default();
    for nid in cur_note_map.keys() {
        if !in_note_ids.contains(nid) {
            tx.execute("DELETE FROM notes WHERE id=?", params![nid.to_string()]).map_err(|e| e.to_string())?;
        }
    }
    if let Some(arr) = v["notes"].as_array() {
        for (i, n) in arr.iter().enumerate() {
            let id = n["id"].as_str().unwrap_or("").to_string();
            let changed = cur_note_map.get(&id).map_or(true, |cs| note_sig(n) != *cs);
            if changed {
                tx.execute(
                    "INSERT OR REPLACE INTO notes(id,title,cat,md,created_at,updated_at,pinned,tags,sort_idx) VALUES(?,?,?,?,?,?,?,?,?)",
                    params![
                        id,
                        n["title"].as_str().unwrap_or(""),
                        n["cat"].as_str().unwrap_or(""),
                        n["md"].as_str().unwrap_or(""),
                        n["createdAt"].as_i64().unwrap_or(0),
                        n["updatedAt"].as_i64().unwrap_or(0),
                        b2i(n["pinned"].as_bool()),
                        n["tags"].to_string(),
                        i as i64
                    ],
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }

    // ---- 智能列表：删除已不存在的，再增量 upsert 内容变化的 ----
    let cur_sl_map: HashMap<String, String> = cur.smart_lists.iter()
        .map(|s| (s["id"].as_str().unwrap_or("").to_string(), smart_sig(s)))
        .collect();
    let in_sl_ids: HashSet<String> = v["smartLists"].as_array()
        .map(|a| a.iter().map(|s| s["id"].as_str().unwrap_or("").to_string()).collect())
        .unwrap_or_default();
    for slid in cur_sl_map.keys() {
        if !in_sl_ids.contains(slid) {
            tx.execute("DELETE FROM smart_lists WHERE id=?", params![slid.to_string()]).map_err(|e| e.to_string())?;
        }
    }
    if let Some(arr) = v["smartLists"].as_array() {
        for (i, s) in arr.iter().enumerate() {
            let id = s["id"].as_str().unwrap_or("").to_string();
            let changed = cur_sl_map.get(&id).map_or(true, |cs| smart_sig(s) != *cs);
            if changed {
                tx.execute(
                    "INSERT OR REPLACE INTO smart_lists(id,name,query,sort_idx) VALUES(?,?,?,?)",
                    params![
                        id,
                        s["name"].as_str().unwrap_or(""),
                        s["query"].to_string(),
                        i as i64
                    ],
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }

    // ---- 笔记分类：全量重写（列表很小，简单可靠） ----
    tx.execute("DELETE FROM note_cats", []).map_err(|e| e.to_string())?;
    if let Some(arr) = v["noteCats"].as_array() {
        for (i, c) in arr.iter().enumerate() {
            if let Some(name) = c.as_str() {
                tx.execute(
                    "INSERT OR REPLACE INTO note_cats(name,sort_idx) VALUES(?,?)",
                    params![name, i as i64],
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }

    // ---- 任务：删除已不存在的（连同其子任务/日志），再增量 upsert 内容变化的 ----
    let cur_task_map: HashMap<String, (usize, String)> = cur.tasks.iter().enumerate()
        .map(|(ci, c)| (c["id"].as_str().unwrap_or("").to_string(), (ci, task_sig(c, ci))))
        .collect();
    let in_task_ids: HashSet<String> = v["tasks"].as_array()
        .map(|a| a.iter().map(|t| t["id"].as_str().unwrap_or("").to_string()).collect())
        .unwrap_or_default();
    for tid in cur_task_map.keys() {
        if !in_task_ids.contains(tid) {
            tx.execute("DELETE FROM tasks WHERE id=?", params![tid.to_string()]).map_err(|e| e.to_string())?;
            tx.execute("DELETE FROM subtasks WHERE task_id=?", params![tid.to_string()]).map_err(|e| e.to_string())?;
            tx.execute("DELETE FROM logs WHERE task_id=?", params![tid.to_string()]).map_err(|e| e.to_string())?;
        }
    }
    if let Some(arr) = v["tasks"].as_array() {
        for (i, t) in arr.iter().enumerate() {
            let tid = t["id"].as_str().unwrap_or("").to_string();
            let changed = match cur_task_map.get(&tid) {
                None => true,
                Some((_ci, csig)) => task_sig(t, i) != *csig,
            };
            if !changed { continue; }
            tx.execute(
                "INSERT OR REPLACE INTO tasks(id,project_id,client_id,title,priority,due,note,done,done_at,status,created_at,repeat,waiting,waiting_for,waiting_since,planned,sort_idx,tags,important,note_id) \
                 VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    tid,
                    t["projectId"].as_str().unwrap_or(""),
                    t["clientId"].as_str().unwrap_or(""),
                    t["title"].as_str().unwrap_or(""),
                    t["priority"].as_str().unwrap_or(""),
                    t["due"].as_str().unwrap_or(""),
                    t["note"].as_str().unwrap_or(""),
                    b2i(t["done"].as_bool()),
                    t["doneAt"].as_i64().unwrap_or(0),
                    t["status"].as_str().unwrap_or(""),
                    t["createdAt"].as_i64().unwrap_or(0),
                    t["repeat"].as_str().unwrap_or(""),
                    b2i(t["waiting"].as_bool()),
                    t["waitingFor"].as_str().unwrap_or(""),
                    t["waitingSince"].as_i64().unwrap_or(0),
                    b2i(t["planned"].as_bool()),
                    i as i64,
                    t["tags"].to_string(),
                    b2i(t["important"].as_bool()),
                    t["noteId"].as_str().unwrap_or("")
                ],
            )
            .map_err(|e| e.to_string())?;

            // 子任务/日志仅在“本任务变化”时才重写，避免全量刷新
            tx.execute("DELETE FROM subtasks WHERE task_id=?", params![tid.clone()]).map_err(|e| e.to_string())?;
            if let Some(sa) = t["subtasks"].as_array() {
                for (j, s) in sa.iter().enumerate() {
                    tx.execute(
                        "INSERT INTO subtasks(id,task_id,title,done,done_at,sort_idx,due,priority,note_id,status) VALUES(?,?,?,?,?,?,?,?,?,?)",
                        params![
                            s["id"].as_str().unwrap_or(""),
                            tid,
                            s["title"].as_str().unwrap_or(""),
                            b2i(s["done"].as_bool()),
                            s["doneAt"].as_i64().unwrap_or(0),
                            j as i64,
                            s["due"].as_str().unwrap_or(""),
                            s["priority"].as_str().unwrap_or(""),
                            s["noteId"].as_str().unwrap_or(""),
                            s["status"].as_str().unwrap_or("")
                        ],
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            tx.execute("DELETE FROM logs WHERE task_id=?", params![tid.clone()]).map_err(|e| e.to_string())?;
            if let Some(la) = t["logs"].as_array() {
                for l in la {
                    tx.execute(
                        "INSERT INTO logs(id,task_id,at,text) VALUES(?,?,?,?)",
                        params![
                            l["id"].as_str().unwrap_or(""),
                            tid,
                            l["at"].as_i64().unwrap_or(0),
                            l["text"].as_str().unwrap_or("")
                        ],
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
        }
    }

    // app_meta：version / seeded / prefs / exportedAt
    if let Some(ver) = v.get("version") {
        let s = meta_str(ver);
        tx.execute(
            "INSERT OR REPLACE INTO app_meta(k,v) VALUES(?,?)",
            params!["version", s],
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(sd) = v.get("seeded") {
        let s = meta_str(sd);
        tx.execute(
            "INSERT OR REPLACE INTO app_meta(k,v) VALUES(?,?)",
            params!["seeded", s],
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(pr) = v.get("prefs") {
        let s = meta_str(pr);
        tx.execute(
            "INSERT OR REPLACE INTO app_meta(k,v) VALUES(?,?)",
            params!["prefs", s],
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(ex) = v.get("exportedAt") {
        let s = meta_str(ex);
        tx.execute(
            "INSERT OR REPLACE INTO app_meta(k,v) VALUES(?,?)",
            params!["exportedAt", s],
        )
        .map_err(|e| e.to_string())?;
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// 从规范化表重建整包 db JSON（空库返回 None，让前端走示例数据）
fn load_main(conn: &Connection) -> Option<String> {
    let mut db: serde_json::Map<String, Value> = serde_json::Map::new();
    db.insert("version".into(), json!(1));
    db.insert("seeded".into(), json!(false));

    if let Ok(mut stmt) = conn.prepare("SELECT k,v FROM app_meta") {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(r)) = rows.next() {
                let k: String = r.get(0).unwrap_or_default();
                let val: String = r.get(1).unwrap_or_default();
                let parsed: Value =
                    serde_json::from_str(&val).unwrap_or(Value::String(val));
                db.insert(k, parsed);
            }
        }
    }

    let mut projects = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id,name,color,summary,goal,status,landing,isContract,signDate,landYear,budget,contract,client_id FROM projects ORDER BY sort_idx") {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(r)) = rows.next() {
                projects.push(json!({
                    "id": r.get::<_,String>(0).unwrap_or_default(),
                    "name": r.get::<_,String>(1).unwrap_or_default(),
                    "color": r.get::<_,String>(2).unwrap_or_default(),
                    "summary": r.get::<_,String>(3).unwrap_or_default(),
                    "goal": r.get::<_,String>(4).unwrap_or_default(),
                    "status": r.get::<_,String>(5).unwrap_or_default(),
                    "landing": r.get::<_,String>(6).unwrap_or_default(),
                    "isContract": r.get::<_,i64>(7).unwrap_or(0),
                    "signDate": r.get::<_,String>(8).unwrap_or_default(),
                    "landYear": r.get::<_,i64>(9).unwrap_or(0),
                    "budget": r.get::<_,f64>(10).unwrap_or(0.0),
                    "contract": r.get::<_,f64>(11).unwrap_or(0.0),
                    "clientId": r.get::<_,String>(12).unwrap_or_default(),
                }));
            }
        }
    }

    // 项目阶段（自定义，可增删改）
    let mut proj_stages: Vec<Value> = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id,name,ord,landed FROM proj_stages ORDER BY ord") {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(r)) = rows.next() {
                proj_stages.push(json!({
                    "id": r.get::<_,String>(0).unwrap_or_default(),
                    "name": r.get::<_,String>(1).unwrap_or_default(),
                    "ord": r.get::<_,i64>(2).unwrap_or(0),
                    "landed": r.get::<_,i64>(3).unwrap_or(0),
                }));
            }
        }
    }
    db.insert("projects".into(), Value::Array(projects.clone()));
    db.insert("projStages".into(), Value::Array(proj_stages.clone()));

    // 客户 / 标签 / 笔记 / 智能列表 / 笔记分类
    let mut clients = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id,name,color,category FROM clients ORDER BY sort_idx") {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(r)) = rows.next() {
                clients.push(json!({
                    "id": r.get::<_,String>(0).unwrap_or_default(),
                    "name": r.get::<_,String>(1).unwrap_or_default(),
                    "color": r.get::<_,String>(2).unwrap_or_default(),
                    "category": r.get::<_,String>(3).unwrap_or_default(),
                }));
            }
        }
    }
    db.insert("clients".into(), Value::Array(clients.clone()));
    let mut tags = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id,name,color,category FROM tags ORDER BY sort_idx") {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(r)) = rows.next() {
                tags.push(json!({
                    "id": r.get::<_,String>(0).unwrap_or_default(),
                    "name": r.get::<_,String>(1).unwrap_or_default(),
                    "color": r.get::<_,String>(2).unwrap_or_default(),
                    "category": r.get::<_,String>(3).unwrap_or_default(),
                }));
            }
        }
    }
    db.insert("tags".into(), Value::Array(tags.clone()));
    let mut notes = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id,title,cat,md,created_at,updated_at,pinned,tags FROM notes ORDER BY sort_idx") {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(r)) = rows.next() {
                /* E3：多标签以 JSON 数组字符串落库（与 tasks.tags 同格式） */
                let ntags_raw: String = r.get(7).unwrap_or_default();
                let ntags: Value = serde_json::from_str(&ntags_raw)
                    .unwrap_or(Value::Array(vec![]));
                notes.push(json!({
                    "id": r.get::<_,String>(0).unwrap_or_default(),
                    "title": r.get::<_,String>(1).unwrap_or_default(),
                    "cat": r.get::<_,String>(2).unwrap_or_default(),
                    "md": r.get::<_,String>(3).unwrap_or_default(),
                    "createdAt": r.get::<_,i64>(4).unwrap_or(0),
                    "updatedAt": r.get::<_,i64>(5).unwrap_or(0),
                    "pinned": r.get::<_,i64>(6).unwrap_or(0) == 1,
                    "tags": ntags,
                }));
            }
        }
    }
    db.insert("notes".into(), Value::Array(notes.clone()));
    let mut smart_lists = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id,name,query FROM smart_lists ORDER BY sort_idx") {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(r)) = rows.next() {
                let qraw: String = r.get(2).unwrap_or_default();
                let query: Value = serde_json::from_str(&qraw)
                    .unwrap_or(Value::Null);
                smart_lists.push(json!({
                    "id": r.get::<_,String>(0).unwrap_or_default(),
                    "name": r.get::<_,String>(1).unwrap_or_default(),
                    "query": query,
                }));
            }
        }
    }
    db.insert("smartLists".into(), Value::Array(smart_lists.clone()));
    let mut note_cats: Vec<Value> = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT name FROM note_cats ORDER BY sort_idx") {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(r)) = rows.next() {
                note_cats.push(json!(r.get::<_,String>(0).unwrap_or_default()));
            }
        }
    }
    db.insert("noteCats".into(), Value::Array(note_cats.clone()));

    let mut tasks = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id,project_id,client_id,title,priority,due,note,done,done_at,status,created_at,repeat,waiting,waiting_for,waiting_since,planned,tags,important,note_id \
         FROM tasks ORDER BY sort_idx",
    ) {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(r)) = rows.next() {
                let id: String = r.get(0).unwrap_or_default();
                let mut subs = Vec::new();
                if let Ok(mut s2) =
                    conn.prepare("SELECT id,title,done,done_at,due,priority,note_id,status FROM subtasks WHERE task_id=? ORDER BY sort_idx")
                {
                    if let Ok(mut sr) = s2.query(params![id.clone()]) {
                        while let Ok(Some(sr2)) = sr.next() {
                            subs.push(json!({
                                "id": sr2.get::<_,String>(0).unwrap_or_default(),
                                "title": sr2.get::<_,String>(1).unwrap_or_default(),
                                "done": sr2.get::<_,i64>(2).unwrap_or(0) == 1,
                                "doneAt": sr2.get::<_,i64>(3).unwrap_or(0),
                                "due": sr2.get::<_,String>(4).unwrap_or_default(),
                                "priority": sr2.get::<_,String>(5).unwrap_or_default(),
                                "noteId": sr2.get::<_,String>(6).unwrap_or_default(),
                                "status": sr2.get::<_,String>(7).unwrap_or_default(),
                            }));
                        }
                    }
                }
                let mut logs = Vec::new();
                if let Ok(mut l2) =
                    conn.prepare("SELECT id,at,text FROM logs WHERE task_id=? ORDER BY at")
                {
                    if let Ok(mut lr) = l2.query(params![id.clone()]) {
                        while let Ok(Some(lr2)) = lr.next() {
                            logs.push(json!({
                                "id": lr2.get::<_,String>(0).unwrap_or_default(),
                                "at": lr2.get::<_,i64>(1).unwrap_or(0),
                                "text": lr2.get::<_,String>(2).unwrap_or_default(),
                            }));
                        }
                    }
                }
                let tags_raw: String = r.get(16).unwrap_or_default();
                let tags: Value = serde_json::from_str(&tags_raw)
                    .unwrap_or(Value::Array(vec![]));
                tasks.push(json!({
                    "id": id,
                    "projectId": r.get::<_,String>(1).unwrap_or_default(),
                    "clientId": r.get::<_,String>(2).unwrap_or_default(),
                    "title": r.get::<_,String>(3).unwrap_or_default(),
                    "priority": r.get::<_,String>(4).unwrap_or_default(),
                    "due": r.get::<_,String>(5).unwrap_or_default(),
                    "note": r.get::<_,String>(6).unwrap_or_default(),
                    "done": r.get::<_,i64>(7).unwrap_or(0) == 1,
                    "doneAt": r.get::<_,i64>(8).unwrap_or(0),
                    "status": r.get::<_,String>(9).unwrap_or_default(),
                    "createdAt": r.get::<_,i64>(10).unwrap_or(0),
                    "repeat": r.get::<_,String>(11).unwrap_or_default(),
                    "waiting": r.get::<_,i64>(12).unwrap_or(0) == 1,
                    "waitingFor": r.get::<_,String>(13).unwrap_or_default(),
                    "waitingSince": r.get::<_,i64>(14).unwrap_or(0),
                    "planned": r.get::<_,i64>(15).unwrap_or(0) == 1,
                    "tags": tags,
                    "important": r.get::<_,i64>(17).unwrap_or(0) == 1,
                    "noteId": r.get::<_,String>(18).unwrap_or_default(),
                    "subtasks": subs,
                    "logs": logs,
                }));
            }
        }
    }
    db.insert("tasks".into(), Value::Array(tasks.clone()));

    if !db.contains_key("prefs") {
        db.insert(
            "prefs".into(),
            json!({ "pid": "", "pri": "P2" }),
        );
    }

    let has_data = !tasks.is_empty()
        || !projects.is_empty()
        || !clients.is_empty()
        || !tags.is_empty()
        || !notes.is_empty()
        || !smart_lists.is_empty();
    if !has_data {
        return None;
    }
    Some(Value::Object(db).to_string())
}

/// 首次启动：若 store 里还有旧的单 blob 主数据，且新表为空，则平滑迁移过去
fn migrate(conn: &mut Connection) {
    let has_old = conn
        .query_row(
            "SELECT 1 FROM store WHERE k = ?1",
            params![MAIN_KEY],
            |_| Ok(true),
        )
        .unwrap_or(false);
    let has_new: i64 = conn
        .query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))
        .unwrap_or(0);
    if has_old && has_new == 0 {
        if let Some(json) = read_store_raw(conn, MAIN_KEY) {
            if save_main(&mut *conn, &json).is_ok() {
                let _ = conn.execute(
                    "DELETE FROM store WHERE k = ?1",
                    params![MAIN_KEY],
                );
            }
        }
    }
}

#[tauri::command]
fn load_store(key: String, db: State<DbState>) -> Result<Option<String>, String> {
    let conn = db.0.lock().unwrap();
    if key == MAIN_KEY {
        Ok(load_main(&conn))
    } else {
        Ok(read_store_raw(&conn, &key))
    }
}

#[tauri::command]
fn save_store(key: String, value: String, db: State<DbState>) -> Result<(), String> {
    let mut conn = db.0.lock().unwrap();
    if key == MAIN_KEY {
        save_main(&mut *conn, &value)
    } else {
        write_store_raw(&conn, &key, &value).map_err(|e| e.to_string())
    }
}

/// 调用 OpenAI 兼容的 /chat/completions 接口。
/// 放在 Rust 端而非前端 fetch，是为了绕开 WebView 的 CSP 限制，
/// 同时避免 API Key 出现在页面上下文里。
#[tauri::command]
async fn ai_chat(
    base_url: String,
    api_key: String,
    model: String,
    system: String,
    user: String,
) -> Result<String, String> {
    let base = base_url.trim().trim_end_matches('/').to_string();
    if base.is_empty() {
        return Err("还没有配置接口地址，请到「设置 → AI 总结」里填写".into());
    }
    if model.trim().is_empty() {
        return Err("还没有填写模型名称，请到「设置 → AI 总结」里填写".into());
    }
    // 用户可能填了带 /chat/completions 的完整地址，做个兼容
    let url = if base.ends_with("/chat/completions") {
        base.clone()
    } else {
        format!("{}/chat/completions", base)
    };

    let body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user",   "content": user }
        ],
        "temperature": 0.5,
        "stream": false
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(240))
        .build()
        .map_err(|e| format!("创建网络客户端失败：{}", e))?;

    let mut req = client.post(&url).json(&body);
    // 本地 Ollama 之类不需要密钥，留空就不带 Authorization
    if !api_key.trim().is_empty() {
        req = req.bearer_auth(api_key.trim());
    }

    let resp = req.send().await.map_err(|e| {
        if e.is_timeout() {
            "请求超时了（4 分钟）。可能是模型太慢或网络不通，可以换个更快的模型再试。".to_string()
        } else if e.is_connect() {
            format!("连不上接口地址：{}。检查一下网址是否正确、是否需要代理。", url)
        } else {
            format!("请求失败：{}", e)
        }
    })?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("读取返回内容失败：{}", e))?;

    if !status.is_success() {
        let hint = match status.as_u16() {
            401 | 403 => "（API Key 不对或没有权限）",
            404 => "（接口地址不对，注意多数服务要带 /v1）",
            429 => "（请求太频繁或余额不足）",
            _ => "",
        };
        let brief: String = text.chars().take(300).collect();
        return Err(format!("接口返回 {} {}：{}", status.as_u16(), hint, brief));
    }

    let v: Value =
        serde_json::from_str(&text).map_err(|_| format!("返回内容不是合法 JSON：{}", text.chars().take(300).collect::<String>()))?;

    // 标准 OpenAI 格式
    if let Some(c) = v["choices"][0]["message"]["content"].as_str() {
        return Ok(c.trim().to_string());
    }
    // 部分服务把推理和正文分开，兜底取 reasoning_content
    if let Some(c) = v["choices"][0]["message"]["reasoning_content"].as_str() {
        return Ok(c.trim().to_string());
    }
    if let Some(msg) = v["error"]["message"].as_str() {
        return Err(format!("接口报错：{}", msg));
    }
    Err(format!(
        "没能从返回里解析出内容：{}",
        text.chars().take(300).collect::<String>()
    ))
}

/// B5：给「关于」面板用的基础信息（版本 / Bundle ID / 数据目录）
#[tauri::command]
fn app_info(app: AppHandle, paths: State<Paths>) -> Value {
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "identifier": app.config().identifier,
        "productName": app.config().product_name.clone().unwrap_or_default(),
        "dataDir": paths.app_dir.to_string_lossy(),
    })
}

/// B3：构建一套中文原生菜单（macOS 风格：第一个子菜单是 App 菜单）
fn build_menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    let about = MenuItemBuilder::with_id("about", "关于 todo-list").build(app)?;
    let settings = MenuItemBuilder::with_id("settings", "设置").accelerator("CmdOrCtrl+,").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "退出 todo-list").accelerator("CmdOrCtrl+Q").build(app)?;
    let app_menu = SubmenuBuilder::new(app, "todo-list")
        .items(&[&about, &settings, &quit])
        .build()?;

    let export = MenuItemBuilder::with_id("export", "导出备份…").build(app)?;
    let import = MenuItemBuilder::with_id("import", "导入备份…").build(app)?;
    let file_menu = SubmenuBuilder::new(app, "文件")
        .items(&[&export, &import])
        .build()?;

    let undo = MenuItemBuilder::with_id("undo", "撤销").accelerator("CmdOrCtrl+Z").build(app)?;
    let edit_menu = SubmenuBuilder::new(app, "编辑").items(&[&undo]).build()?;

    let today = MenuItemBuilder::with_id("today", "回到今天").accelerator("CmdOrCtrl+1").build(app)?;
    let view_menu = SubmenuBuilder::new(app, "视图").items(&[&today]).build()?;

    MenuBuilder::new(app)
        .items(&[&app_menu, &file_menu, &edit_menu, &view_menu])
        .build()
}

/// 旧库平滑补列：SQLite 不支持 `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`（语法错误），
/// 故先用 PRAGMA table_info 探测列是否存在，缺失时才执行 ADD COLUMN。
fn ensure_col(conn: &Connection, table: &str, col: &str, decl: &str) {
    let mut exists = false;
    let pragma = format!("PRAGMA table_info({})", table);
    if let Ok(mut stmt) = conn.prepare(&pragma) {
        if let Ok(mut rows) = stmt.query([]) {
            while let Ok(Some(r)) = rows.next() {
                let name: String = r.get(1).unwrap_or_default();
                if name == col { exists = true; break; }
            }
        }
    }
    if !exists {
        let sql = format!("ALTER TABLE {} ADD COLUMN {} {}", table, col, decl);
        let _ = conn.execute(&sql, []);
    }
}

/// 建好所有表（store 保留给 AI 配置等非主数据；其余为 E2 规范化表）
fn init_schema(conn: &Connection) {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS store(k TEXT PRIMARY KEY, v TEXT)",
        [],
    )
    .ok();
    conn.execute(
        "CREATE TABLE IF NOT EXISTS projects(id TEXT PRIMARY KEY, name TEXT, color TEXT, summary TEXT, goal TEXT, sort_idx INTEGER)",
        [],
    )
    .ok();
    ensure_col(conn, "projects", "summary", "TEXT");
    ensure_col(conn, "projects", "goal", "TEXT");
    ensure_col(conn, "projects", "status", "TEXT");
    ensure_col(conn, "projects", "landing", "TEXT");
    ensure_col(conn, "projects", "isContract", "INTEGER");
    ensure_col(conn, "projects", "signDate", "TEXT");
    ensure_col(conn, "projects", "landYear", "INTEGER");
    ensure_col(conn, "projects", "budget", "REAL");
    ensure_col(conn, "projects", "contract", "REAL");
    // A1/C2：项目归属客户。加了这一列之后，任务的客户可以由所选项目自动推导，用户少做一个决定。
    // 走 ensure_col 兼容追加：老库无损，老数据 client_id 为 NULL，前端按空串处理。
    ensure_col(conn, "projects", "client_id", "TEXT");
    conn.execute(
        "CREATE TABLE IF NOT EXISTS proj_stages(id TEXT PRIMARY KEY, name TEXT, ord INTEGER, landed INTEGER)",
        [],
    ).ok();
    // 首次运行（proj_stages 为空）预置 4 个阶段
    let mut stage_cnt: i64 = 0;
    if let Ok(mut stmt) = conn.prepare("SELECT COUNT(*) FROM proj_stages") {
        if let Ok(mut rows) = stmt.query([]) {
            if let Ok(Some(r)) = rows.next() { stage_cnt = r.get::<_,i64>(0).unwrap_or(0); }
        }
    }
    if stage_cnt == 0 {
        let seeds = [
            ("st_pre", "售前阶段", 1i64, 0i64),
            ("st_sign", "合同签订", 2i64, 1i64),
            ("st_deliver", "交付中", 3i64, 1i64),
            ("st_accept", "已验收", 4i64, 1i64),
        ];
        for (id, name, ord, landed) in seeds.iter() {
            conn.execute("INSERT OR IGNORE INTO proj_stages(id,name,ord,landed) VALUES(?,?,?,?)", params![id, name, ord, landed]).ok();
        }
    }
    conn.execute(
        "CREATE TABLE IF NOT EXISTS tasks(\
            id TEXT PRIMARY KEY, project_id TEXT, client_id TEXT, title TEXT, priority TEXT, due TEXT, note TEXT,\
            done INTEGER, done_at INTEGER, status TEXT, created_at INTEGER, repeat TEXT,\
            waiting INTEGER, waiting_for TEXT, waiting_since INTEGER, planned INTEGER, sort_idx INTEGER,\
            tags TEXT, important INTEGER, note_id TEXT)",
        [],
    )
    .ok();
    conn.execute(
        "CREATE TABLE IF NOT EXISTS subtasks(id TEXT PRIMARY KEY, task_id TEXT, title TEXT, done INTEGER, done_at INTEGER, sort_idx INTEGER, due TEXT, priority TEXT, note_id TEXT, status TEXT)",
        [],
    )
    .ok();
    // 其余集合：客户 / 标签 / 笔记 / 笔记分类 / 智能列表（E2 规范化表）
    conn.execute(
        "CREATE TABLE IF NOT EXISTS clients(id TEXT PRIMARY KEY, name TEXT, color TEXT, category TEXT, sort_idx INTEGER)",
        [],
    )
    .ok();
    conn.execute(
        "CREATE TABLE IF NOT EXISTS tags(id TEXT PRIMARY KEY, name TEXT, color TEXT, category TEXT, sort_idx INTEGER)",
        [],
    )
    .ok();
    conn.execute(
        "CREATE TABLE IF NOT EXISTS notes(id TEXT PRIMARY KEY, title TEXT, cat TEXT, md TEXT, created_at INTEGER, updated_at INTEGER, pinned INTEGER, sort_idx INTEGER, tags TEXT)",
        [],
    )
    .ok();
    conn.execute(
        "CREATE TABLE IF NOT EXISTS note_cats(name TEXT PRIMARY KEY, sort_idx INTEGER)",
        [],
    )
    .ok();
    conn.execute(
        "CREATE TABLE IF NOT EXISTS smart_lists(id TEXT PRIMARY KEY, name TEXT, query TEXT, sort_idx INTEGER)",
        [],
    )
    .ok();
    // 旧库平滑升级：补齐新建任务所需的字段列（CREATE 时已含，这里仅对存量库保险）
    ensure_col(conn, "tasks", "client_id", "TEXT");
    ensure_col(conn, "tasks", "tags", "TEXT");
    ensure_col(conn, "tasks", "important", "INTEGER");
    ensure_col(conn, "tasks", "note_id", "TEXT");
    ensure_col(conn, "subtasks", "due", "TEXT");
    ensure_col(conn, "subtasks", "priority", "TEXT");
    ensure_col(conn, "subtasks", "note_id", "TEXT");
    ensure_col(conn, "subtasks", "status", "TEXT");
    // E3：笔记多标签。老库没有这一列，补齐后前端挂上的标签才能真正落库（原先只存在于内存，一存一读就丢）。
    ensure_col(conn, "notes", "tags", "TEXT");
    conn.execute(
        "CREATE TABLE IF NOT EXISTS logs(id TEXT PRIMARY KEY, task_id TEXT, at INTEGER, text TEXT)",
        [],
    )
    .ok();
    conn.execute(
        "CREATE TABLE IF NOT EXISTS app_meta(k TEXT PRIMARY KEY, v TEXT)",
        [],
    )
    .ok();
}

// 菜单栏托盘「新建任务」的全局快捷键（Tauri accelerator 字符串，如 "CmdOrCtrl+N"）。
// 由设置面板通过 set_new_task_shortcut 命令更新，并触发托盘菜单重建。
struct NewTaskShortcut(pub Mutex<String>);

/// 构建托盘菜单；新建任务的加速键使用传入的 accel。
#[cfg(target_os = "macos")]
fn build_tray_menu(app: &AppHandle, accel: &str) -> tauri::Result<Menu<tauri::Wry>> {
    let show = MenuItem::with_id(app, "show", "显示窗口", true, None::<&str>)?;
    let new = MenuItem::with_id(app, "new", "新建任务", true, Some(accel))?;
    let quit = MenuItem::with_id(app, "quit", "退出 todo-list", true, Some("CmdOrCtrl+Q"))?;
    let sep = PredefinedMenuItem::separator(app)?;
    Menu::with_items(app, &[&show, &new, &sep, &quit])
}

#[cfg(target_os = "macos")]
fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    // 读取当前保存的快捷键，作为托盘菜单「新建任务」的加速键
    let accel = app.state::<NewTaskShortcut>().inner().0.lock().unwrap().clone();
    let menu = build_tray_menu(app, accel.as_str())?;
    // 托盘图标直接内联打包进二进制的 PNG，避免依赖运行时窗口图标缺失导致空白
    let icon_bytes = include_bytes!("../icons/32x32.png");
    let icon = match tauri::image::Image::from_bytes(icon_bytes) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("托盘图标加载失败（不影响主功能）：{}", e);
            return Ok(());
        }
    };
    let _tray = TrayIconBuilder::with_id(TrayIconId::new("tray"))
        .icon(icon)
        .tooltip("todo-list")
        .icon_as_template(false)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "new" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
                let _ = app.emit("tray-new", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

/// 设置面板调用：更新「新建任务」全局快捷键，并立即重建托盘菜单使其生效。
#[tauri::command]
fn set_new_task_shortcut(app: AppHandle, accel: String) -> Result<(), String> {
    *app.state::<NewTaskShortcut>().inner().0.lock().unwrap() = accel.clone();
    #[cfg(target_os = "macos")]
    {
        if let Some(tray) = app.tray_by_id("tray") {
            let menu = build_tray_menu(&app, accel.as_str()).map_err(|e| e.to_string())?;
            tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// 登录时启动：把 App 注册为 macOS LaunchAgent（写 ~/Library/LaunchAgents 下的 plist）。
/// 纯 std 实现，零依赖、离线可用；Program 用当前可执行文件自身路径。
#[cfg(target_os = "macos")]
fn autostart_plist_path() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| "/".to_string());
    PathBuf::from(home).join("Library/LaunchAgents/com.zhuanz.mytask.plist")
}

#[cfg(target_os = "macos")]
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn set_autostart_macos(enabled: bool) -> Result<(), String> {
    let path = autostart_plist_path();
    if enabled {
        let exe = env::current_exe().map_err(|e| e.to_string())?;
        let exe_str = exe
            .to_str()
            .ok_or_else(|| "无法读取可执行文件路径".to_string())?
            .to_string();
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n<dict>\n\
             <key>Label</key><string>com.zhuanz.mytask</string>\n\
             <key>Program</key><string>{}</string>\n\
             <key>RunAtLoad</key><true/>\n\
             <key>ProcessType</key><string>Interactive</string>\n\
             </dict>\n</plist>\n",
            escape_xml(&exe_str)
        );
        fs::write(&path, plist).map_err(|e| e.to_string())?;
        // 立即加载（best effort，失败不影响下次登录生效）
        let _ = Command::new("launchctl")
            .args(["load", path.to_str().unwrap_or("")])
            .output();
    } else if path.exists() {
        let _ = Command::new("launchctl")
            .args(["unload", path.to_str().unwrap_or("")])
            .output();
        fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn autostart_enabled_macos() -> bool {
    autostart_plist_path().exists()
}

/// Windows 开机启动：通过注册表 Run 键实现
#[cfg(target_os = "windows")]
#[tauri::command]
fn set_autostart_windows(enabled: bool) -> Result<(), String> {
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let exe_str = exe.to_str().ok_or("无法读取可执行文件路径")?;
    if enabled {
        let _ = Command::new("reg").args(["add", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run", "/v", "todo-list", "/t", "REG_SZ", "/d", exe_str, "/f"]).output();
    } else {
        let _ = Command::new("reg").args(["delete", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run", "/v", "todo-list", "/f"]).output();
    }
    Ok(())
}

#[cfg(target_os = "windows")]
#[tauri::command]
fn autostart_enabled_windows() -> bool {
    let out = Command::new("reg").args(["query", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run", "/v", "todo-list"]).output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).contains("todo-list"),
        Err(_) => false,
    }
}

/// 统一入口：根据平台调用对应的实现
#[tauri::command]
fn set_autostart(enabled: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    { set_autostart_macos(enabled) }
    #[cfg(target_os = "windows")]
    { set_autostart_windows(enabled) }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    { let _ = enabled; Ok(()) }
}

#[tauri::command]
fn autostart_enabled() -> bool {
    #[cfg(target_os = "macos")]
    { autostart_enabled_macos() }
    #[cfg(target_os = "windows")]
    { autostart_enabled_windows() }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    { false }
}

/// 系统通知：macOS 用 osascript 原生弹出，零额外依赖、离线可用。
/// 只在 macOS 分支被调用，按平台门控以免 Windows 构建报「未使用」。
#[cfg(target_os = "macos")]
fn escape_osascript(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', " ")
}
#[tauri::command]
fn notify(title: String, body: String) {
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            escape_osascript(&body),
            escape_osascript(&title)
        );
        let _ = Command::new("osascript")
            .args(["-e", &script])
            .output();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (title, body);
    }
}

/// 注册 ⌘N 全局快捷键（即使窗口隐藏/应用非活跃也能唤起新建）。
fn register_global_shortcuts(app: &AppHandle) {
    let sc = Shortcut::new(Some(Modifiers::SUPER), Code::KeyN);
    if let Err(e) = app.global_shortcut().register(sc) {
        eprintln!("全局快捷键注册失败（不影响主功能）：{}", e);
    }
}

fn main() {
    let app_dir = resolve_data_dir();
    fs::create_dir_all(&app_dir).ok();
    let path = app_dir.join("tasks.db");
    let mut conn = Connection::open(&path).expect("无法打开数据库");
    init_schema(&conn);

    // E2：首次启动把旧的单 blob 主数据平滑迁移进规范化表
    migrate(&mut conn);

    // 每天首次启动自动留一份快照，误清空时还有得救
    daily_snapshot(&conn, &app_dir);

    tauri::Builder::default()
        // 记住窗口大小和位置，下次打开还原
        .plugin(tauri_plugin_window_state::Builder::default().build())
        // 原生「保存 / 打开文件」对话框
        .plugin(tauri_plugin_dialog::init())
        // 系统剪贴板：复制 / 粘贴 / 剪切（macOS NSPasteboard，双向可靠）
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _sc, _evt| {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                    let _ = app.emit("global-new", ());
                })
                .build(),
        )
        // B3：中文原生菜单
        .menu(build_menu)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref().to_string();
            if id == "quit" {
                app.exit(0);
                return;
            }
            // 其余菜单项把 id 转发出去，由前端路由到对应动作
            let _ = app.emit("menu", id);
        })
        // 点关闭 / ⌘W：仅隐藏窗口到后台，不退出 app；
        // 彻底退出走 ⌘Q（菜单「退出」）或托盘「退出 todo-list」
        .on_window_event(|win, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = win.hide();
                api.prevent_close();
            }
        })
        .manage(DbState(Mutex::new(conn)))
        .manage(Paths { app_dir })
        .manage(NewTaskShortcut(Mutex::new("CmdOrCtrl+N".to_string())))
        .invoke_handler(tauri::generate_handler![
            load_store,
            save_store,
            ai_chat,
            snapshot_info,
            reveal_dir,
            export_backup,
            save_text_file,
            import_backup,
            app_info,
            get_data_dir,
            set_data_dir,
            restart_app,
            set_autostart,
            autostart_enabled,
            set_new_task_shortcut,
            notify
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                if let Err(e) = setup_tray(app.handle()) {
                    eprintln!("托盘初始化失败（不影响主功能）：{}", e);
                }
            }
            register_global_shortcuts(app.handle());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let c = Connection::open(":memory:").unwrap();
        init_schema(&c);
        c
    }

    #[test]
    fn roundtrip_preserves_data() {
        let mut conn = mem();
        let sample = json!({
            "version": 1,
            "seeded": true,
            "projects": [{"id":"p1","name":"项目A","color":"#f00"}],
            "tasks": [
                {"id":"t1","projectId":"p1","title":"任务1","priority":"P0","due":"2026-08-09","note":"备注","done":false,"doneAt":0,"status":"todo","createdAt":1700000000000i64,"repeat":"weekly","waiting":false,"waitingFor":"","waitingSince":0,"planned":true,"subtasks":[{"id":"s1","title":"子1","done":true,"doneAt":123i64}],"logs":[{"id":"l1","at":1700000000000i64,"text":"跟进"}]},
                {"id":"t2","projectId":"","title":"等待中","priority":"P1","due":"","note":"","done":false,"doneAt":0,"status":"todo","createdAt":1i64,"repeat":"","waiting":true,"waitingFor":"小李","waitingSince":999i64,"planned":false,"subtasks":[],"logs":[]}
            ],
            "prefs": {"pid":"p1","pri":"P2"},
            "exportedAt": "2026-08-09T10:00:00.000Z"
        }).to_string();
        save_main(&mut conn, &sample).unwrap();
        let out = load_main(&conn).expect("should reconstruct");
        let v: Value = serde_json::from_str(&out).unwrap();

        assert_eq!(v["version"].as_i64(), Some(1));
        assert_eq!(v["seeded"].as_bool(), Some(true));
        assert_eq!(v["projects"].as_array().map(|a| a.len()), Some(1));
        assert_eq!(v["projects"][0]["name"].as_str(), Some("项目A"));

        let tasks = v["tasks"].as_array().expect("tasks array");
        assert_eq!(tasks.len(), 2);
        let t1 = tasks.iter().find(|t| t["id"].as_str() == Some("t1")).expect("t1");
        assert_eq!(t1["done"].as_bool(), Some(false));
        assert_eq!(t1["repeat"].as_str(), Some("weekly"));
        assert_eq!(t1["planned"].as_bool(), Some(true));
        assert_eq!(t1["subtasks"].as_array().map(|a| a.len()), Some(1));
        assert_eq!(t1["subtasks"][0]["done"].as_bool(), Some(true));
        assert_eq!(t1["logs"].as_array().map(|a| a.len()), Some(1));
        assert_eq!(t1["logs"][0]["text"].as_str(), Some("跟进"));
        assert_eq!(t1["createdAt"].as_i64(), Some(1700000000000));
        assert!(t1["done"].is_boolean(), "bool must survive as bool");

        let t2 = tasks.iter().find(|t| t["id"].as_str() == Some("t2")).expect("t2");
        assert_eq!(t2["waiting"].as_bool(), Some(true));
        assert_eq!(t2["waitingFor"].as_str(), Some("小李"));
        assert_eq!(t2["waitingSince"].as_i64(), Some(999));
        assert_eq!(t2["projectId"].as_str(), Some(""));

        assert_eq!(v["prefs"]["pid"].as_str(), Some("p1"));
        assert_eq!(v["exportedAt"].as_str(), Some("2026-08-09T10:00:00.000Z"));
    }

    #[test]
    fn empty_returns_none() {
        let conn = mem();
        assert!(load_main(&conn).is_none());
    }

    #[test]
    fn migrate_old_blob() {
        let mut conn = mem();
        let blob = json!({
            "version":1,"seeded":true,
            "projects":[{"id":"p1","name":"旧项目","color":"#0f0"}],
            "tasks":[{"id":"t1","projectId":"p1","title":"旧任务","priority":"P2","due":"","note":"","done":true,"doneAt":5i64,"status":"done","createdAt":9i64,"repeat":"","waiting":false,"waitingFor":"","waitingSince":0,"planned":false,"subtasks":[],"logs":[]}],
            "prefs":{"pid":"","pri":"P2"}
        }).to_string();
        conn.execute(
            "INSERT INTO store(k,v) VALUES(?1,?2)",
            params![MAIN_KEY, blob],
        )
        .unwrap();
        migrate(&mut conn);
        let out = load_main(&conn).expect("migrated");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["tasks"].as_array().map(|a| a.len()), Some(1));
        assert_eq!(v["projects"][0]["name"].as_str(), Some("旧项目"));
        let leftover: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM store WHERE k=?1",
                params![MAIN_KEY],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(leftover, 0, "old blob should be removed after migrate");
    }

    #[test]
    fn incremental_save_only_touches_changed() {
        let mut conn = mem();
        let base = json!({
            "version": 1, "seeded": true,
            "projects": [{"id":"p1","name":"项目A","color":"#f00"}],
            "tasks": [
                {"id":"t1","projectId":"p1","title":"任务1","priority":"P0","due":"","note":"","done":false,"doneAt":0,"status":"todo","createdAt":1i64,"repeat":"","waiting":false,"waitingFor":"","waitingSince":0,"planned":false,"subtasks":[{"id":"s1","title":"子1","done":false,"doneAt":0}],"logs":[]},
                {"id":"t2","projectId":"p1","title":"任务2","priority":"P1","due":"","note":"","done":false,"doneAt":0,"status":"todo","createdAt":2i64,"repeat":"","waiting":false,"waitingFor":"","waitingSince":0,"planned":false,"subtasks":[],"logs":[{"id":"l1","at":9i64,"text":"日志"}]},
                {"id":"t3","projectId":"","title":"任务3","priority":"P2","due":"","note":"","done":false,"doneAt":0,"status":"todo","createdAt":3i64,"repeat":"","waiting":false,"waitingFor":"","waitingSince":0,"planned":false,"subtasks":[],"logs":[]}
            ],
            "prefs": {"pid":"","pri":"P2"}
        });
        save_main(&mut conn, &base.to_string()).unwrap();

        // 加载后，未变化的任务签名应与当前落盘状态一致（即不会误判为“变化”而触发全量重写）
        let cur = read_full(&conn);
        let loaded: Value = serde_json::from_str(&load_main(&conn).unwrap()).unwrap();
        for (i, t) in loaded["tasks"].as_array().unwrap().iter().enumerate() {
            let id = t["id"].as_str().unwrap();
            let ci = cur.tasks.iter().position(|c| c["id"].as_str().unwrap() == id).unwrap();
            assert_eq!(task_sig(t, i), task_sig(&cur.tasks[ci], ci), "任务 {} 不应被判定为变化", id);
        }

        // 仅修改 t2 的标题
        let mut changed = base.clone();
        changed["tasks"][1]["title"] = json!("任务2-改");
        save_main(&mut conn, &changed.to_string()).unwrap();

        let v: Value = serde_json::from_str(&load_main(&conn).unwrap()).unwrap();
        let tasks = v["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 3, "任务总数应保持不变");
        assert_eq!(tasks.iter().find(|t| t["id"].as_str() == Some("t2")).unwrap()["title"].as_str(), Some("任务2-改"));
        // t1 / t3 及其子任务、日志完好
        let t1 = tasks.iter().find(|t| t["id"].as_str() == Some("t1")).unwrap();
        assert_eq!(t1["title"].as_str(), Some("任务1"));
        assert_eq!(t1["subtasks"].as_array().map(|a| a.len()), Some(1));
        assert_eq!(tasks.iter().find(|t| t["id"].as_str() == Some("t3")).unwrap()["title"].as_str(), Some("任务3"));

        // 删除 t3（连同其数据），再保存
        let mut without_t3 = changed.clone();
        without_t3["tasks"] = json!([changed["tasks"][0], changed["tasks"][1]]);
        save_main(&mut conn, &without_t3.to_string()).unwrap();
        let v2: Value = serde_json::from_str(&load_main(&conn).unwrap()).unwrap();
        assert_eq!(v2["tasks"].as_array().unwrap().len(), 2, "t3 应被删除");
        let subs_left: i64 = conn.query_row("SELECT COUNT(*) FROM subtasks", [], |r| r.get(0)).unwrap();
        assert_eq!(subs_left, 1, "t1 的 1 个子任务应保留，t3 无子任务");
    }

    #[test]
    fn old_db_upgrade_adds_missing_columns() {
        // 模拟旧库：用旧 schema 建表（无 summary/goal、无 tasks 新列、无 subtasks 新列）
        let mut conn = Connection::open(":memory:").unwrap();
        conn.execute("CREATE TABLE projects(id TEXT PRIMARY KEY, name TEXT, color TEXT, sort_idx INTEGER)", []).unwrap();
        conn.execute("CREATE TABLE tasks(id TEXT PRIMARY KEY, project_id TEXT, title TEXT, priority TEXT, due TEXT, note TEXT, done INTEGER, done_at INTEGER, status TEXT, created_at INTEGER, repeat TEXT, waiting INTEGER, waiting_for TEXT, waiting_since INTEGER, planned INTEGER, sort_idx INTEGER)", []).unwrap();
        conn.execute("CREATE TABLE subtasks(id TEXT PRIMARY KEY, task_id TEXT, title TEXT, done INTEGER, done_at INTEGER, sort_idx INTEGER)", []).unwrap();
        conn.execute("CREATE TABLE store(k TEXT PRIMARY KEY, v TEXT)", []).unwrap();
        conn.execute("INSERT INTO projects(id,name,color,sort_idx) VALUES('p1','旧项目','#f00',0)", []).unwrap();
        // 升级：init_schema 应通过 ensure_col 补齐缺失列
        init_schema(&conn);
        // save_main 引用 summary/goal/tags/important/noteId/clientId，升级后必须成功
        let sample = json!({
            "version":1,"seeded":true,
            "projects":[{"id":"p1","name":"旧项目","color":"#f00","summary":"新摘要","goal":"新目标"}],
            "tasks":[{"id":"t1","projectId":"p1","title":"任务1","priority":"high","due":"","note":"","done":false,"doneAt":0,"status":"todo","createdAt":1,"repeat":"","waiting":false,"waitingFor":"","waitingSince":0,"planned":false,"subtasks":[],"logs":[],"tags":["tg1"],"important":true,"noteId":"n1","clientId":"c1"}],
            "prefs":{"pid":"","pri":"low"}
        }).to_string();
        save_main(&mut conn, &sample).expect("save_main 应在升级后成功");
        let v: Value = serde_json::from_str(&load_main(&conn).unwrap()).unwrap();
        assert_eq!(v["projects"][0]["summary"].as_str(), Some("新摘要"));
        assert_eq!(v["projects"][0]["goal"].as_str(), Some("新目标"));
        assert_eq!(v["tasks"][0]["important"].as_bool(), Some(true));
        assert_eq!(v["tasks"][0]["clientId"].as_str(), Some("c1"));
        assert_eq!(v["tasks"][0]["tags"].as_array().map(|a|a.len()), Some(1));
    }

    #[test]
    fn proj_fields_and_stages_roundtrip() {
        let mut conn = Connection::open(":memory:").unwrap();
        init_schema(&conn);
        let sample = json!({
            "version":1,"seeded":true,
            "projects":[{"id":"p1","name":"客户A","color":"#0a84ff","summary":"s","goal":"g",
                "status":"st_accept","landing":"high","isContract":true,"signDate":"2026-03-15","landYear":2026,
                "budget":500000,"contract":800000}],
            "projStages":[
                {"id":"st_pre","name":"售前阶段","ord":1,"landed":0},
                {"id":"st_sign","name":"合同签订","ord":2,"landed":1},
                {"id":"st_deliver","name":"交付中","ord":3,"landed":1},
                {"id":"st_accept","name":"已验收","ord":4,"landed":1}
            ],
            "prefs":{"pid":"","pri":"low"}
        }).to_string();
        save_main(&mut conn, &sample).expect("save_main 成功");
        let v: Value = serde_json::from_str(&load_main(&conn).unwrap()).unwrap();
        assert_eq!(v["projects"][0]["status"].as_str(), Some("st_accept"));
        assert_eq!(v["projects"][0]["landing"].as_str(), Some("high"));
        assert_eq!(v["projects"][0]["isContract"].as_i64(), Some(1));
        assert_eq!(v["projects"][0]["signDate"].as_str(), Some("2026-03-15"));
        assert_eq!(v["projects"][0]["landYear"].as_i64(), Some(2026));
        assert_eq!(v["projects"][0]["budget"].as_f64(), Some(500000.0));
        assert_eq!(v["projects"][0]["contract"].as_f64(), Some(800000.0));
        assert_eq!(v["projStages"].as_array().map(|a| a.len()), Some(4));
        assert_eq!(v["projStages"][3]["name"].as_str(), Some("已验收"));
    }

    /// E3 回归：笔记的多标签必须真正落库（修复前 notes 表的读/写/签名都不含 tags，标签一存一读就丢）
    #[test]
    fn note_tags_roundtrip() {
        let mut conn = Connection::open(":memory:").unwrap();
        init_schema(&conn);
        let sample = json!({
            "version":1,"seeded":true,
            "notes":[
                {"id":"n1","title":"会议记录","cat":"工作","md":"# 会议记录","createdAt":1,"updatedAt":2,"pinned":false,"tags":["tg1","tg2"]},
                {"id":"n2","title":"无标签","cat":"","md":"","createdAt":3,"updatedAt":4,"pinned":true,"tags":[]}
            ],
            "prefs":{"pid":"","pri":"low"}
        });
        save_main(&mut conn, &sample.to_string()).expect("save_main 成功");
        let v: Value = serde_json::from_str(&load_main(&conn).unwrap()).unwrap();
        let notes = v["notes"].as_array().unwrap();
        assert_eq!(notes.len(), 2);
        let n1 = notes.iter().find(|n| n["id"].as_str() == Some("n1")).unwrap();
        assert_eq!(n1["tags"].as_array().map(|a| a.len()), Some(2), "n1 的 2 个标签应完整保留");
        assert_eq!(n1["tags"][0].as_str(), Some("tg1"));
        assert_eq!(n1["title"].as_str(), Some("会议记录"));
        let n2 = notes.iter().find(|n| n["id"].as_str() == Some("n2")).unwrap();
        assert_eq!(n2["tags"].as_array().map(|a| a.len()), Some(0));

        // 只改标签（内容不变）也必须被判定为"有变化"从而落库 —— 依赖 note_sig 含 tags
        let mut only_tags = sample.clone();
        only_tags["notes"][1]["tags"] = json!(["tg3"]);
        save_main(&mut conn, &only_tags.to_string()).unwrap();
        let v2: Value = serde_json::from_str(&load_main(&conn).unwrap()).unwrap();
        let n2b = v2["notes"].as_array().unwrap().iter().find(|n| n["id"].as_str() == Some("n2")).unwrap().clone();
        assert_eq!(n2b["tags"].as_array().map(|a| a.len()), Some(1), "仅改标签也应落库");
        assert_eq!(n2b["tags"][0].as_str(), Some("tg3"));
    }

    /// E3 回归：老库的 notes 表没有 tags 列，init_schema 必须靠 ensure_col 平滑补上
    #[test]
    fn old_notes_table_upgrade_adds_tags() {
        let mut conn = Connection::open(":memory:").unwrap();
        conn.execute("CREATE TABLE store(k TEXT PRIMARY KEY, v TEXT)", []).unwrap();
        // 旧 schema：无 tags 列
        conn.execute("CREATE TABLE notes(id TEXT PRIMARY KEY, title TEXT, cat TEXT, md TEXT, created_at INTEGER, updated_at INTEGER, pinned INTEGER, sort_idx INTEGER)", []).unwrap();
        conn.execute("INSERT INTO notes(id,title,cat,md,created_at,updated_at,pinned,sort_idx) VALUES('old','老笔记','','body',1,2,0,0)", []).unwrap();
        init_schema(&conn);
        let sample = json!({
            "version":1,"seeded":true,
            "notes":[{"id":"old","title":"老笔记","cat":"","md":"body","createdAt":1,"updatedAt":2,"pinned":false,"tags":["tg9"]}],
            "prefs":{"pid":"","pri":"low"}
        }).to_string();
        save_main(&mut conn, &sample).expect("升级后 save_main 必须成功（tags 列已补）");
        let v: Value = serde_json::from_str(&load_main(&conn).unwrap()).unwrap();
        assert_eq!(v["notes"][0]["tags"][0].as_str(), Some("tg9"));
    }

    /// A1/C2 回归：项目的 client_id 必须真的落库（任务的客户由它推导）
    /// —— 与 note_tags_roundtrip 同型：读 / 写 / 签名 三处任一漏掉，客户归属都会静默丢失
    #[test]
    fn project_client_id_roundtrip() {
        let mut conn = Connection::open(":memory:").unwrap();
        init_schema(&conn);
        let sample = json!({
            "version":1,"seeded":true,
            "projects":[
                {"id":"p1","name":"客户 A 交付","color":"#0071e3","clientId":"c1"},
                {"id":"p2","name":"内部事务","color":"#e5484d","clientId":""}
            ],
            "prefs":{"pid":"","pri":"low"}
        });
        save_main(&mut conn, &sample.to_string()).expect("save_main 成功");
        let v: Value = serde_json::from_str(&load_main(&conn).unwrap()).unwrap();
        let projects = v["projects"].as_array().unwrap();
        assert_eq!(projects.len(), 2);
        let p1 = projects.iter().find(|p| p["id"].as_str() == Some("p1")).unwrap();
        assert_eq!(p1["clientId"].as_str(), Some("c1"), "项目的 clientId 应完整保留");
        let p2 = projects.iter().find(|p| p["id"].as_str() == Some("p2")).unwrap();
        assert_eq!(p2["clientId"].as_str(), Some(""));

        // 只改 clientId（其余字段不变）也必须被判定为"有变化"从而落库 —— 依赖 proj_sig 含 clientId
        let mut only_client = sample.clone();
        only_client["projects"][1]["clientId"] = json!("c2");
        save_main(&mut conn, &only_client.to_string()).unwrap();
        let v2: Value = serde_json::from_str(&load_main(&conn).unwrap()).unwrap();
        let p2b = v2["projects"].as_array().unwrap().iter()
            .find(|p| p["id"].as_str() == Some("p2")).unwrap().clone();
        assert_eq!(p2b["clientId"].as_str(), Some("c2"), "仅改 clientId 也应落库");
    }

    /// A1 回归：老库的 projects 表没有 client_id 列，init_schema 必须靠 ensure_col 平滑补上
    #[test]
    fn old_projects_table_upgrade_adds_client_id() {
        let mut conn = Connection::open(":memory:").unwrap();
        conn.execute("CREATE TABLE store(k TEXT PRIMARY KEY, v TEXT)", []).unwrap();
        // 旧 schema：无 client_id 列
        conn.execute("CREATE TABLE projects(id TEXT PRIMARY KEY, name TEXT, color TEXT, summary TEXT, goal TEXT, sort_idx INTEGER)", []).unwrap();
        conn.execute("INSERT INTO projects(id,name,color,sort_idx) VALUES('old','老项目','#0071e3',0)", []).unwrap();
        init_schema(&conn);
        let sample = json!({
            "version":1,"seeded":true,
            "projects":[{"id":"old","name":"老项目","color":"#0071e3","clientId":"c9"}],
            "prefs":{"pid":"","pri":"low"}
        }).to_string();
        save_main(&mut conn, &sample).expect("升级后 save_main 必须成功（client_id 列已补）");
        let v: Value = serde_json::from_str(&load_main(&conn).unwrap()).unwrap();
        assert_eq!(v["projects"][0]["clientId"].as_str(), Some("c9"));
    }
}

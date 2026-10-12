use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use tauri::{Emitter, State};
use std::sync::Mutex;

#[derive(Default)]
pub struct PipelineState {
    pub running: Mutex<bool>,
}

fn emit_log(app: &tauri::AppHandle, line: &str) {
    let _ = app.emit("tool-log", line.to_string());
}

fn run_step(
    app: &tauri::AppHandle,
    program: &str,
    args: &[&str],
    cwd: &str,
) -> Result<String, String> {
    emit_log(app, &format!("> {} {}", program, args.join(" ")));

    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn failed: {}", e))?;

    let out = child.stdout.take().unwrap();
    let err = child.stdout.take();
    let _ = err;
    let (tx, rx) = mpsc::channel();
    let tx2 = tx.clone();
    thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        let reader = BufReader::new(out);
        for line in reader.lines().map_while(|l| l.ok()) {
            let _ = tx.send(line);
        }
    });
    // stderr in same thread style
    if let Some(err_pipe) = child.stderr.take() {
        let app2 = app.clone();
        thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(err_pipe);
            for line in reader.lines().map_while(|l| l.ok()) {
                emit_log(&app2, &line);
            }
        });
    }
    let app2 = app.clone();
    thread::spawn(move || {
        for line in rx {
            emit_log(&app2, &line);
        }
    });

    let status = child.wait().map_err(|e| format!("wait failed: {}", e))?;
    drop(tx2);
    if status.success() {
        Ok("ok".into())
    } else {
        Err(format!("exit code {:?}", status.code()))
    }
}

pub const ZN_DIR: &str = "D:\\ZoneNova";
pub const ADB: &str = "C:\\Program Files\\BlueStacks_nxt\\HD-Adb.exe";
pub const DEVICE: &str = "emulator-5554";
pub const REMOTE: &str =
    "/sdcard/Android/data/com.yro.novazone.erolab/files/AssetBundles/Source/Android/";
pub const WIKI: &str = "C:\\Users\\Borin\\ZCodeProject\\gacha-wiki";
pub const BLOB_IN: &str = "D:\\ZoneNova\\input\\device_data_fresh\\AssetBundles\\Source\\Android\\DE8EE416E0F0AFCA1C2DB0E781BC702D.ab";
pub const PULLED_DIR: &str = "D:\\ZoneNova\\input\\device_data_fresh\\AssetBundles\\Source\\Android";
pub const EXTRACTOR: &str = "C:\\Users\\Borin\\ZCodeProject\\zone-nova-extractor";
pub const DEC_OUT: &str = "D:\\ZoneNova\\output\\masterdata\\live_master_data_decrypted.bin";
pub const TABLES_OUT: &str = "D:\\ZoneNova\\output\\masterdata\\tables_live";

/// Find the live master data blob by its AES magic (b5 61 2a 39 2a 41 b8 09).
/// The hash filename changes between game versions, so scanning is the only
/// reliable way. Newest matching file wins.
fn find_master_blob(app: &tauri::AppHandle) -> Result<String, String> {
    const MAGIC: [u8; 8] = [0xb5, 0x61, 0x2a, 0x39, 0x2a, 0x41, 0xb8, 0x09];
    let mut best: Option<(std::time::SystemTime, String)> = None;
    let entries = std::fs::read_dir(PULLED_DIR)
        .map_err(|e| format!("cannot read pulled dir {}: {}", PULLED_DIR, e))?;
    for e in entries.flatten() {
        let path = e.path();
        let mut head = [0u8; 8];
        let ok = std::fs::File::open(&path)
            .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
            .is_ok();
        if !ok || head != MAGIC {
            continue;
        }
        let mtime = e
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let name = path.to_string_lossy().to_string();
        emit_log(app, &format!("master data candidate: {} ({:?} bytes)", name, e.metadata().map(|m| m.len()).unwrap_or(0)));
        if best.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true) {
            best = Some((mtime, name));
        }
    }
    best.map(|(_, p)| p).ok_or_else(|| {
        format!(
            "no master data blob (magic b5 61 2a 39 ...) found in {} — run the pull step first",
            PULLED_DIR
        )
    })
}

#[tauri::command]
fn pull_master(app: tauri::AppHandle, state: State<PipelineState>) -> Result<String, String> {
    {
        let mut r = state.running.lock().unwrap();
        if *r {
            return Err("a step is already running".into());
        }
        *r = true;
    }
    let result = (|| {
        // list device
        run_step(&app, ADB, &["devices"], ".")?;
        // pull bundles dir
        run_step(
            &app,
            ADB,
            &["-s", DEVICE, "pull", REMOTE, "D:\\ZoneNova\\input\\device_data_fresh\\"],
            ".",
        )?;
        emit_log(&app, "pull done — step 2 auto-finds the master blob by magic b5 61 2a 39 ... and decrypts it");
        Ok("pull complete".into())
    })();
    *state.running.lock().unwrap() = false;
    result
}

#[tauri::command]
fn decrypt_dump(app: tauri::AppHandle, state: State<PipelineState>) -> Result<String, String> {
    {
        let mut r = state.running.lock().unwrap();
        if *r {
            return Err("a step is already running".into());
        }
        *r = true;
    }
    let result = (|| {
        // locate the live blob by magic (hash name changes per game version)
        let blob = match find_master_blob(&app) {
            Ok(p) => p,
            Err(e) => {
                if std::path::Path::new(BLOB_IN).exists() {
                    emit_log(&app, &format!("scan failed ({}), falling back to last known blob", e));
                    BLOB_IN.to_string()
                } else {
                    return Err(e);
                }
            }
        };
        // pure-Python decryptor from the zone-nova-extractor repo
        // (verified key/IV + PasswordDeriveBytes port, no PowerShell needed)
        run_step(
            &app,
            "python",
            &[
                "decrypt.py",
                &blob,
                "-o",
                DEC_OUT,
            ],
            EXTRACTOR,
        )?;
        run_step(
            &app,
            "python",
            &["dump_tables.py", DEC_OUT, TABLES_OUT],
            EXTRACTOR,
        )?;
        emit_log(&app, "decrypt + dump complete → output\\masterdata\\tables_live\\");
        Ok("decrypt+dump complete".into())
    })();
    *state.running.lock().unwrap() = false;
    result
}

#[tauri::command]
fn run_script(
    app: tauri::AppHandle,
    state: State<PipelineState>,
    script: String,
) -> Result<String, String> {
    {
        let mut r = state.running.lock().unwrap();
        if *r {
            return Err("a step is already running".into());
        }
        *r = true;
    }
    let result = (|| {
        let in_wiki = script.starts_with("wiki:");
        let (dir, file) = if in_wiki {
            (WIKI.to_string(), script[5..].to_string())
        } else {
            (ZN_DIR.to_string(), script.clone())
        };
        run_step(&app, "python", &["-u", &file], &dir)?;
        emit_log(&app, &format!("{} finished", file));
        Ok(format!("{} finished", file))
    })();
    *state.running.lock().unwrap() = false;
    result
}

#[tauri::command]
fn table_counts() -> Result<serde_json::Value, String> {
    let dir = format!("{}\\output\\masterdata\\tables_live", ZN_DIR);
    let mut out = serde_json::Map::new();
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("read dir failed: {}", e))?;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.ends_with(".json") || name.starts_with('_') {
            continue;
        }
        let path = e.path();
        let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        let count = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.as_array().map(|a| a.len()))
            .unwrap_or(0);
        out.insert(
            name.trim_end_matches(".json").to_string(),
            serde_json::json!({ "rows": count, "bytes": meta.len() }),
        );
    }
    Ok(serde_json::Value::Object(out))
}

#[tauri::command]
fn read_table(name: String, offset: usize, limit: usize) -> Result<serde_json::Value, String> {
    let path = format!("{}\\output\\masterdata\\tables_live\\{}.json", ZN_DIR, name);
    let body = std::fs::read_to_string(&path).map_err(|e| format!("read failed: {}", e))?;
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("parse failed: {}", e))?;
    match &v {
        serde_json::Value::Array(arr) => Ok(serde_json::json!({
            "total": arr.len(),
            "rows": arr.iter().skip(offset).take(limit).cloned().collect::<Vec<_>>(),
        })),
        _ => Ok(serde_json::json!({ "total": 1, "rows": [v] })),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(PipelineState::default())
        .invoke_handler(tauri::generate_handler![
            pull_master,
            decrypt_dump,
            run_script,
            table_counts,
            read_table
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

//! `cadcraft-cli mcp --automation-read-root/--automation-write-root`: the headless session reads
//! and writes inside the folders it was given and nowhere else, whichever tool names the file.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

fn mcp(args: &[&str], calls: &[(&str, Value)]) -> Output {
    let mut input = String::from(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#);
    input.push('\n');
    for (i, (name, arguments)) in calls.iter().enumerate() {
        let req = json!({"jsonrpc": "2.0", "id": i + 1, "method": "tools/call", "params": {"name": name, "arguments": arguments}});
        input.push_str(&format!("{req}\n"));
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_cadcraft-cli"))
        .arg("mcp")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cadcraft-cli");
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    child.wait_with_output().expect("wait for cadcraft-cli")
}

/// The text of each tool result by call number, and whether it was an error.
fn results(out: &Output) -> Vec<(bool, String)> {
    let replies: Vec<Value> = String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    let mut r: Vec<(u64, bool, String)> = replies
        .iter()
        .filter(|v| v["id"].as_u64().is_some_and(|id| id > 0))
        .map(|v| {
            let res = &v["result"];
            let text = res["content"].as_array().into_iter().flatten().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n");
            (v["id"].as_u64().unwrap_or(0), res["isError"].as_bool().unwrap_or(false) || v.get("error").is_some(), text)
        })
        .collect();
    r.sort_by_key(|x| x.0);
    r.into_iter().map(|(_, e, t)| (e, t)).collect()
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn reads_and_writes_stay_inside_the_roots() {
    let dir = std::env::temp_dir().join(format!("cadcraft-cli-mcp-roots-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (input, output, other) = (dir.join("in"), dir.join("out"), dir.join("other"));
    for d in [&input, &output, &other] {
        std::fs::create_dir_all(d).unwrap();
    }
    let secret = other.join("secret.dxf");
    std::fs::write(&secret, b"0\nEOF\n").unwrap();
    let calls = [
        ("execute", json!({"command": "circle", "params": {"center": [0, 0], "radius": 5}})),
        ("save", json!({"path": s(&output.join("plan.dxf"))})),
        ("save", json!({"path": s(&other.join("plan.dxf"))})),
        ("save", json!({"path": s(&output.join("../other/escape.dxf"))})),
        ("render", json!({"path": s(&other.join("shot.png"))})),
        ("execute", json!({"command": "export", "params": {"path": s(&input.join("plan.svg"))}})),
        ("open", json!({"path": s(&secret)})),
        ("command_line", json!({"text": format!("(setq f (open \"{}\" \"w\"))", s(&other.join("lisp.txt")))})),
        ("open", json!({"path": s(&output.join("plan.dxf"))})),
    ];
    let out = mcp(&["--automation-read-root", &s(&input), "--automation-write-root", &s(&output)], &calls);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let r = results(&out);
    assert_eq!(r.len(), calls.len(), "{r:?}");
    assert!(!r[0].0 && !r[1].0, "{r:?}");
    for i in [2, 3, 4, 5, 6] {
        assert!(r[i].0 && r[i].1.contains("while file access is confined"), "call {i}: {:?}", r[i]);
    }
    assert!(!r[8].0, "{:?}", r[8]);
    assert!(output.join("plan.dxf").is_file());
    let outside: Vec<_> = std::fs::read_dir(&other).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(outside, [std::ffi::OsString::from("secret.dxf")], "nothing may be written outside the write root");
    assert!(!input.join("plan.svg").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_root_that_is_not_a_folder_stops_startup() {
    let out = mcp(&["--automation-write-root", "/nonexistent/cadcraft-root"], &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("automation root"), "{}", String::from_utf8_lossy(&out.stderr));
}

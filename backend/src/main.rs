//! Debug CLI: `mp-backend picker`, `manager`, `protons`, `settings`, `logs`,
//! `games`, `session <id>`, `raw '<json>'`.  The real UI calls the library.

use mp_backend::{handle, Ctx};
use serde_json::{json, Value};

fn request(args: &[String]) -> Value {
    let command = args.first().map(String::as_str).unwrap_or("picker");
    let mut value = json!({ "cmd": command });
    match command {
        "picker" | "manager" => {
            if let Some(appid) = args.get(1) {
                value["appid"] = json!(appid);
            }
        }
        "session" | "stop_session" => {
            if let Some(id) = args.get(1).and_then(|s| s.parse::<u64>().ok()) {
                value["id"] = json!(id);
            }
        }
        "icon" => {
            if let Some(path) = args.get(1) {
                value["path"] = json!(path);
            }
        }
        "launch" => {
            if let Some(appid) = args.get(1) {
                value["appid"] = json!(appid);
            }
            if let Some(exe) = args.get(2) {
                value["exe"] = json!(exe);
            }
            value["remember"] = json!(false);
        }
        "raw" => {
            if let Some(raw) = args.get(1) {
                if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
                    return parsed;
                }
            }
        }
        _ => {}
    }
    value
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let req = request(&args);
    let mut ctx = Ctx::new();
    let reply = handle(&mut ctx, &req);
    println!("{}", serde_json::to_string_pretty(&reply).unwrap());
    if req["cmd"] == "launch" && reply["ok"].as_bool().unwrap_or(false) {
        let id = reply["id"].as_u64().unwrap_or(0);
        loop {
            std::thread::sleep(std::time::Duration::from_millis(400));
            let state = handle(&mut ctx, &json!({"cmd": "session", "id": id}));
            let done = state["done"].as_bool().unwrap_or(false);
            if done {
                println!("{}", serde_json::to_string_pretty(&state).unwrap());
                break;
            }
        }
    }
}

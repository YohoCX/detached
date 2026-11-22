//! A scripted `pg-dev` for the tests; its databases are `dbs.json` beside its executable:
//! `list --json` prints them, `drop [--] <name>` drops one, `args …` prints its arguments as JSON,
//! `env` the names in its env, and `sleep` runs until killed.

use std::path::Path;
use std::process::exit;

use serde_json::{Value, json};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let db = std::env::current_exe().unwrap().with_file_name("dbs.json");
    match args.first().map(String::as_str) {
        Some("list") if args[1..] == ["--json"] => print!("{}", read(&db)),
        Some("drop") => {
            let name = args
                .iter()
                .skip(1)
                .find(|a| *a != "--")
                .cloned()
                .unwrap_or_default();
            let mut dbs = read(&db);
            let all = dbs.as_array_mut().unwrap();
            let before = all.len();
            all.retain(|d| d["name"] != name.as_str());
            if all.len() == before {
                eprintln!("pg-dev: no database {name}");
                exit(1);
            }
            std::fs::write(&db, dbs.to_string()).unwrap();
        }
        Some("args") => print!("{}", json!(args[1..])),
        Some("env") => print!(
            "{}",
            json!(
                std::env::vars_os()
                    .map(|(k, _)| k.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            )
        ),
        Some("sleep") => loop {
            std::thread::park();
        },
        _ => {
            eprintln!("pg-dev: unknown command {args:?}");
            exit(2);
        }
    }
}

fn read(db: &Path) -> Value {
    std::fs::read(db).map_or(json!([]), |b| serde_json::from_slice(&b).unwrap())
}

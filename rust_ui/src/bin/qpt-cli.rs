//! Port of cli.js — standalone CLI: `qpt-cli "<command>"`.
//! Works without the server: opens the store directly, executes, prints.

use qpt::{cli_exec, store::Store, Paths};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: qpt-cli \"<command>\"  (try: qpt-cli \"help\")");
        std::process::exit(1);
    }
    let line = args.join(" ");
    let paths = Paths::cwd();
    let mut store = Store::open(&paths);
    let r = cli_exec::exec_command(&line, &mut store, &paths);
    println!("{}", r.output);
    let code = if r.ok { 0 } else { 1 };
    drop(store); // drain the background flusher before exiting
    std::process::exit(code);
}

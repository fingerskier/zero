//! ZeroDB CLI — local MVP + process/machine sync helpers.
//!
//! Multi-process: `export` / `import` or `sync --peer <other.db>`
//! Multi-machine: `serve` + `pull --from host:port`

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use zerodb_storage::{ExportBundle, LocalStore, WireOp};

#[derive(Parser)]
#[command(
    name = "zerodb",
    about = "ZeroDB local MVP CLI (M1 slice + peer exchange)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a new local datastore file.
    Init {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
    },
    /// Show peer id and datastore id.
    Info {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
    },
    /// Create a node; prints node id (hex).
    CreateNode {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long, default_value = "Todo")]
        label: String,
    },
    /// Set an LWW text property on a node.
    Set {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long)]
        node: String,
        #[arg(long)]
        key: String,
        #[arg(long)]
        value: String,
    },
    /// Get an LWW text property.
    Get {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long)]
        node: String,
        #[arg(long)]
        key: String,
    },
    /// List nodes.
    Nodes {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
    },
    /// Export all ops to a JSON bundle (file exchange / multi-process).
    Export {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Import ops from a JSON bundle (merge / multi-process).
    Import {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long)]
        file: PathBuf,
    },
    /// Two-way file sync with another local database (same machine, two processes' DBs).
    Sync {
        #[arg(long)]
        path: PathBuf,
        #[arg(long)]
        peer: PathBuf,
    },
    /// Serve ops over TCP for multi-machine pull.
    Serve {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long, default_value = "127.0.0.1:7700")]
        listen: String,
    },
    /// Pull missing ops from a remote `serve` peer and merge.
    Pull {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long)]
        from: String,
    },
}

#[derive(Serialize, Deserialize)]
struct Hello {
    v: u32,
    datastore_id: String,
    peer: String,
    op_ids: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct HelloOk {
    v: u32,
    datastore_id: String,
    peer: String,
    need: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct OpsMsg {
    ops: Vec<WireOp>,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Init { path } => {
            let store = LocalStore::init(&path)?;
            println!("initialized {}", path.display());
            println!("datastore {}", store.datastore_id_hex());
            println!("peer      {}", store.author_hex());
        }
        Cmd::Info { path } => {
            let store = LocalStore::open(&path)?;
            println!("path      {}", path.display());
            println!("datastore {}", store.datastore_id_hex());
            println!("peer      {}", store.author_hex());
            println!("ops       {}", store.op_count()?);
        }
        Cmd::CreateNode { path, label } => {
            let mut store = LocalStore::open(&path)?;
            let id = store.create_node(&label)?;
            println!("{id}");
        }
        Cmd::Set {
            path,
            node,
            key,
            value,
        } => {
            let mut store = LocalStore::open(&path)?;
            let op = store.set_lww(&node, &key, &value)?;
            println!("ok op {op}");
        }
        Cmd::Get { path, node, key } => {
            let store = LocalStore::open(&path)?;
            match store.get_lww(&node, &key)? {
                Some(v) => println!("{v}"),
                None => {
                    eprintln!("(null)");
                    std::process::exit(2);
                }
            }
        }
        Cmd::Nodes { path } => {
            let store = LocalStore::open(&path)?;
            for (id, label) in store.list_nodes()? {
                println!("{id}\t{label}");
            }
        }
        Cmd::Export { path, out } => {
            let store = LocalStore::open(&path)?;
            let bundle = store.export_all()?;
            std::fs::write(&out, serde_json::to_string_pretty(&bundle)?)?;
            println!("exported {} ops -> {}", bundle.ops.len(), out.display());
        }
        Cmd::Import { path, file } => {
            let mut store = LocalStore::open(&path)?;
            let raw = std::fs::read_to_string(&file)?;
            let bundle: ExportBundle = serde_json::from_str(&raw)?;
            let (a, s) = store.import_bundle(&bundle)?;
            println!("imported accepted={a} skipped={s}");
        }
        Cmd::Sync { path, peer } => {
            // Two-way merge via in-memory export (no temp file required).
            let mut a = LocalStore::open(&path)?;
            let mut b = LocalStore::open(&peer)?;
            let from_a = a.export_all()?;
            let from_b = b.export_all()?;
            let (ba, bs) = b.import_bundle(&from_a)?;
            let (aa, as_) = a.import_bundle(&from_b)?;
            println!(
                "sync done: {} <- peer accepted={ba} skipped={bs}; peer <- {} accepted={aa} skipped={as_}",
                path.display(),
                path.display()
            );
            println!(
                "ops now: {}={}  {}={}",
                path.display(),
                a.op_count()?,
                peer.display(),
                b.op_count()?
            );
        }
        Cmd::Serve { path, listen } => {
            let store = LocalStore::open(&path)?;
            let listener = TcpListener::bind(&listen)?;
            println!(
                "serving {} (ds {}) on {listen}",
                path.display(),
                store.datastore_id_hex()
            );
            for stream in listener.incoming() {
                let stream = stream?;
                if let Err(e) = handle_peer(&path, stream) {
                    eprintln!("peer error: {e}");
                }
            }
        }
        Cmd::Pull { path, from } => {
            let mut store = LocalStore::open(&path)?;
            let mut stream = TcpStream::connect(&from)?;
            stream.set_read_timeout(Some(Duration::from_secs(30)))?;
            stream.set_write_timeout(Some(Duration::from_secs(30)))?;

            let local_ids = store
                .list_op_ids()?
                .into_iter()
                .map(hex::encode)
                .collect::<Vec<_>>();
            let hello = Hello {
                v: 1,
                datastore_id: store.datastore_id_hex(),
                peer: store.author_hex(),
                op_ids: local_ids,
            };
            write_msg(&mut stream, &hello)?;
            let hello_ok: HelloOk = read_msg(&mut stream)?;
            if hello_ok.v != 1 {
                return Err("protocol version".into());
            }
            // If empty local, adopt remote ds via first ops batch
            let need: Vec<[u8; 32]> = hello_ok
                .need
                .iter()
                .filter_map(|h| hex::decode(h).ok()?.try_into().ok())
                .collect();
            // Request is implicit: server sends ops we need based on inverse...
            // Protocol: client sends its ids; server replies with ops client is missing
            // (server computes client_missing = server - client). Client's `need` field
            // is unused in this direction; server ignores need from client HelloOk.
            let ops_msg: OpsMsg = read_msg(&mut stream)?;
            let bundle = ExportBundle {
                format: 1,
                datastore_id: hello_ok.datastore_id,
                ops: ops_msg.ops,
            };
            let (a, s) = store.import_bundle(&bundle)?;
            println!(
                "pull from {from}: accepted={a} skipped={s}; local ops={}",
                store.op_count()?
            );
            let _ = need;
        }
    }
    Ok(())
}

fn handle_peer(path: &Path, mut stream: TcpStream) -> Result<(), Box<dyn std::error::Error>> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let store = LocalStore::open(path)?;
    let hello: Hello = read_msg(&mut stream)?;
    if hello.v != 1 {
        return Err("bad hello version".into());
    }
    let local: std::collections::BTreeSet<[u8; 32]> = store.list_op_ids()?.into_iter().collect();
    let remote: std::collections::BTreeSet<[u8; 32]> = hello
        .op_ids
        .iter()
        .filter_map(|h| hex::decode(h).ok()?.try_into().ok())
        .collect();
    // Ops remote is missing
    let missing: Vec<[u8; 32]> = local.difference(&remote).copied().collect();
    // Ops we might want from them (not sent in this one-way pull protocol)
    let need: Vec<String> = remote.difference(&local).map(hex::encode).collect();
    write_msg(
        &mut stream,
        &HelloOk {
            v: 1,
            datastore_id: store.datastore_id_hex(),
            peer: store.author_hex(),
            need,
        },
    )?;
    let ops = store.export_ops_by_id(&missing)?;
    write_msg(&mut stream, &OpsMsg { ops })?;
    println!(
        "served peer {} — sent {} ops",
        &hello.peer[..8.min(hello.peer.len())],
        missing.len()
    );
    Ok(())
}

fn write_msg<T: Serialize>(
    stream: &mut TcpStream,
    msg: &T,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = serde_json::to_vec(msg)?;
    let len = (bytes.len() as u32).to_be_bytes();
    stream.write_all(&len)?;
    stream.write_all(&bytes)?;
    stream.flush()?;
    Ok(())
}

fn read_msg<T: for<'de> Deserialize<'de>>(
    stream: &mut TcpStream,
) -> Result<T, Box<dyn std::error::Error>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 64 * 1024 * 1024 {
        return Err("message too large".into());
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    Ok(serde_json::from_slice(&buf)?)
}

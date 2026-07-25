//! ZeroDB CLI — M1 local durable core + process/machine sync.
//!
//! Multi-process: `export` / `import` or `sync --peer <other.db>`
//! Multi-machine: `serve` + `pull --from host:port`

use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Parser, Subcommand};
use zerodb_storage::{sync, ExportBundle, LocalStore};

#[derive(Parser)]
#[command(
    name = "zerodb",
    about = "ZeroDB M1 CLI — local SQLite + CRDT props + peer exchange"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Init {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
    },
    Info {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
    },
    Inspect {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
    },
    CreateNode {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long, default_value = "Todo")]
        label: String,
    },
    DeleteNode {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long)]
        node: String,
    },
    /// Set LWW text (default) or --crdt for typed write helpers.
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
    Get {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long)]
        node: String,
        #[arg(long)]
        key: String,
    },
    Inc {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long)]
        node: String,
        #[arg(long)]
        key: String,
        #[arg(long, default_value_t = 1)]
        n: u64,
        #[arg(long, default_value = "pn")]
        kind: String, // pn | g
    },
    Dec {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long)]
        node: String,
        #[arg(long)]
        key: String,
        #[arg(long, default_value_t = 1)]
        n: u64,
    },
    SetAdd {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long)]
        node: String,
        #[arg(long)]
        key: String,
        #[arg(long)]
        value: String,
    },
    SetRemove {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long)]
        node: String,
        #[arg(long)]
        key: String,
        #[arg(long)]
        value: String,
    },
    FlagOn {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long)]
        node: String,
        #[arg(long)]
        key: String,
    },
    FlagOff {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long)]
        node: String,
        #[arg(long)]
        key: String,
    },
    Nodes {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
    },
    CreateEdge {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long)]
        label: String,
        #[arg(long)]
        src: String,
        #[arg(long)]
        dst: String,
    },
    Edges {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
    },
    /// Apply simplified schema pin JSON: { "nodes": { "Todo": { "props": { "title": "lww" } } } }
    SchemaApply {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long)]
        file: PathBuf,
    },
    /// O3 minimal query (MATCH/WHERE/RETURN/ORDER BY/LIMIT).
    Query {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long)]
        q: String,
    },
    /// Rebuild all props from oplog (E1 fresh replay).
    Replay {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
    },
    Export {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    Import {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long)]
        file: PathBuf,
    },
    Sync {
        #[arg(long)]
        path: PathBuf,
        #[arg(long)]
        peer: PathBuf,
    },
    Serve {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long, default_value = "127.0.0.1:7700")]
        listen: String,
        /// Allow binding to a non-loopback address (plaintext, unauthenticated).
        #[arg(long, default_value_t = false)]
        allow_insecure_lan: bool,
    },
    Pull {
        #[arg(long, default_value = "./zerodb.sqlite")]
        path: PathBuf,
        #[arg(long)]
        from: String,
    },
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
        Cmd::Inspect { path } => {
            let store = LocalStore::open(&path)?;
            let rep = store.inspect(&path)?;
            println!("{}", serde_json::to_string_pretty(&rep)?);
        }
        Cmd::CreateNode { path, label } => {
            let mut store = LocalStore::open(&path)?;
            println!("{}", store.create_node(&label)?);
        }
        Cmd::DeleteNode { path, node } => {
            let mut store = LocalStore::open(&path)?;
            println!("ok op {}", store.delete_node(&node)?);
        }
        Cmd::Set {
            path,
            node,
            key,
            value,
        } => {
            let mut store = LocalStore::open(&path)?;
            println!("ok op {}", store.set_lww(&node, &key, &value)?);
        }
        Cmd::Get { path, node, key } => {
            let store = LocalStore::open(&path)?;
            match store.get_prop(&node, &key)? {
                Some(serde_json::Value::String(s)) => println!("{s}"),
                Some(v) => println!("{}", serde_json::to_string(&v)?),
                None => {
                    eprintln!("(null)");
                    std::process::exit(2);
                }
            }
        }
        Cmd::Inc {
            path,
            node,
            key,
            n,
            kind,
        } => {
            let mut store = LocalStore::open(&path)?;
            let op = if kind == "g" {
                store.gcounter_inc(&node, &key, n)?
            } else {
                store.counter_inc(&node, &key, n)?
            };
            println!("ok op {op}");
        }
        Cmd::Dec { path, node, key, n } => {
            let mut store = LocalStore::open(&path)?;
            println!("ok op {}", store.counter_dec(&node, &key, n)?);
        }
        Cmd::SetAdd {
            path,
            node,
            key,
            value,
        } => {
            let mut store = LocalStore::open(&path)?;
            println!("ok op {}", store.set_add(&node, &key, &value)?);
        }
        Cmd::SetRemove {
            path,
            node,
            key,
            value,
        } => {
            let mut store = LocalStore::open(&path)?;
            println!("ok op {}", store.set_remove(&node, &key, &value)?);
        }
        Cmd::FlagOn { path, node, key } => {
            let mut store = LocalStore::open(&path)?;
            println!("ok op {}", store.flag_enable(&node, &key)?);
        }
        Cmd::FlagOff { path, node, key } => {
            let mut store = LocalStore::open(&path)?;
            println!("ok op {}", store.flag_disable(&node, &key)?);
        }
        Cmd::Nodes { path } => {
            let store = LocalStore::open(&path)?;
            for (id, label, deleted) in store.list_nodes()? {
                let flag = if deleted { "deleted" } else { "live" };
                println!("{id}\t{label}\t{flag}");
            }
        }
        Cmd::CreateEdge {
            path,
            label,
            src,
            dst,
        } => {
            let mut store = LocalStore::open(&path)?;
            println!("ok edge {}", store.create_edge(&label, &src, &dst)?);
        }
        Cmd::Edges { path } => {
            let store = LocalStore::open(&path)?;
            for (id, label, src, dst) in store.list_edges_visible()? {
                println!("{id}\t{label}\t{src}\t{dst}");
            }
        }
        Cmd::SchemaApply { path, file } => {
            let mut store = LocalStore::open(&path)?;
            let raw = std::fs::read_to_string(&file)?;
            store.apply_schema_json(&raw)?;
            println!("schema applied from {}", file.display());
        }
        Cmd::Query { path, q } => {
            let store = LocalStore::open(&path)?;
            let rows = store.query(&q)?;
            println!("{}", serde_json::to_string_pretty(&rows)?);
        }
        Cmd::Replay { path } => {
            let mut store = LocalStore::open(&path)?;
            store.replay_all()?;
            println!("replayed {} ops", store.op_count()?);
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
            let mut a = LocalStore::open(&path)?;
            let mut b = LocalStore::open(&peer)?;
            let mut ba = 0;
            let mut bs = 0;
            let mut aa = 0;
            let mut as_ = 0;

            if a.datastore_id_hex() != b.datastore_id_hex() {
                match (a.op_count()?, b.op_count()?) {
                    (0, 0) => {
                        return Err(
                            "cannot choose a datastore when syncing two empty databases".into()
                        );
                    }
                    (0, _) => {
                        let from_b = b.export_all()?;
                        let (accepted, skipped) = a.import_bundle(&from_b)?;
                        aa += accepted;
                        as_ += skipped;
                    }
                    (_, 0) => {
                        let from_a = a.export_all()?;
                        let (accepted, skipped) = b.import_bundle(&from_a)?;
                        ba += accepted;
                        bs += skipped;
                    }
                    _ => {
                        return Err(format!(
                            "datastore mismatch: local {} peer {}",
                            a.datastore_id_hex(),
                            b.datastore_id_hex()
                        )
                        .into());
                    }
                }
            }

            // Export only after any empty-peer adoption so neither side sees a
            // stale pre-adoption datastore ID.
            let from_a = a.export_all()?;
            let from_b = b.export_all()?;
            let (accepted, skipped) = b.import_bundle(&from_a)?;
            ba += accepted;
            bs += skipped;
            let (accepted, skipped) = a.import_bundle(&from_b)?;
            aa += accepted;
            as_ += skipped;
            println!(
                "sync done: {} <- peer accepted={aa} skipped={as_}; peer <- local accepted={ba} skipped={bs}",
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
        Cmd::Serve {
            path,
            listen,
            allow_insecure_lan,
        } => {
            assert_serve_bind_allowed(&listen, allow_insecure_lan)?;
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
            let summary = sync::pull(&mut store, &mut stream)?;
            println!(
                "pull from {from}: accepted={} skipped={} sent={} remote_accepted={}; local ops={}",
                summary.accepted,
                summary.skipped,
                summary.sent,
                summary.remote_accepted,
                store.op_count()?
            );
        }
    }
    Ok(())
}

fn assert_serve_bind_allowed(
    listen: &str,
    allow_insecure_lan: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if allow_insecure_lan {
        return Ok(());
    }
    let host = listen
        .rsplit_once(':')
        .map(|(h, _)| h.trim_matches(|c| c == '[' || c == ']'))
        .unwrap_or(listen);
    let loopback = matches!(host, "127.0.0.1" | "localhost" | "::1" | "0:0:0:0:0:0:0:1");
    if !loopback {
        return Err(format!(
            "serve refuses non-loopback bind {listen:?} without --allow-insecure-lan \
             (plaintext, unauthenticated; use loopback or pass the flag for disposable LAN tests only)"
        )
        .into());
    }
    Ok(())
}

fn handle_peer(path: &Path, mut stream: TcpStream) -> Result<(), Box<dyn std::error::Error>> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut store = LocalStore::open(path)?;
    let summary = sync::serve(&mut store, &mut stream)?;
    println!(
        "served peer {} — sent {} ops, accepted {} skipped {}",
        &summary.peer[..8.min(summary.peer.len())],
        summary.sent,
        summary.accepted,
        summary.skipped
    );
    Ok(())
}

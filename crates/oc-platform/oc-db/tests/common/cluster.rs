//! A throwaway Postgres cluster (port of packages/db/src/testCluster.ts) for tests that create cluster-wide objects
//! (roles), so they never touch the shared development server: initdb into a temp dir, start on a free port (TCP on
//! 127.0.0.1 only, no Unix socket), trust auth for the superuser `oc_admin`; dropping it stops the server and removes
//! the directory. Binaries: OC_TEST_PG_BIN (a directory with initdb and pg_ctl), else the newest Postgres in the usual
//! install locations, else PATH. `TestCluster::start()` returns None when none is found, so callers can skip.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub const SUPERUSER: &str = "oc_admin";

pub struct TestCluster {
    pub port: u16,
    dir: PathBuf,
    pg_ctl: PathBuf,
}

fn find_bin() -> Option<PathBuf> {
    let ok = |d: &Path| d.join("initdb").exists() && d.join("pg_ctl").exists();
    if let Some(d) = std::env::var_os("OC_TEST_PG_BIN") {
        let d = PathBuf::from(d);
        return ok(&d).then_some(d);
    }
    let mut c: Vec<PathBuf> = Vec::new();
    for v in [18, 17, 16, 15, 14] {
        for p in [format!("/opt/homebrew/opt/postgresql@{v}/bin"), format!("/usr/local/opt/postgresql@{v}/bin"), format!("/usr/lib/postgresql/{v}/bin"), format!("/usr/pgsql-{v}/bin")] {
            c.push(p.into());
        }
    }
    c.push("/Applications/Postgres.app/Contents/Versions/latest/bin".into());
    if let Some(hit) = c.into_iter().find(|d| ok(d)) {
        return Some(hit);
    }
    let out = Command::new("sh").args(["-c", "command -v initdb"]).output().ok()?;
    let p = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!p.is_empty()).then(|| PathBuf::from(p.trim_end_matches("/initdb")))
}

fn run(cmd: &mut Command) {
    let out = cmd.env("LC_ALL", "C").env("LANG", "C").output().expect("spawn");
    assert!(out.status.success(), "{cmd:?} failed: {}{}", String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout));
}

impl TestCluster {
    pub fn start() -> Option<TestCluster> {
        let bin = find_bin()?;
        let dir = std::env::temp_dir().join(format!("oc-pg-rust-{}", uuid::Uuid::new_v4().simple()));
        let data = dir.join("data");
        std::fs::create_dir_all(&dir).unwrap();
        run(Command::new(bin.join("initdb")).arg("-D").arg(&data).args(["-U", SUPERUSER, "--auth=trust", "--no-sync", "-E", "UTF8", "--locale=C"]));
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let conf = format!(
            "\nport = '{port}'\nlisten_addresses = '127.0.0.1'\nunix_socket_directories = ''\nfsync = 'off'\nsynchronous_commit = 'off'\nfull_page_writes = 'off'\nmax_connections = '40'\n"
        );
        let mut f = std::fs::OpenOptions::new().append(true).open(data.join("postgresql.conf")).unwrap();
        std::io::Write::write_all(&mut f, conf.as_bytes()).unwrap();
        let pg_ctl = bin.join("pg_ctl");
        run(Command::new(&pg_ctl).arg("-D").arg(&data).arg("-l").arg(dir.join("server.log")).args(["-w", "-t", "30", "start"]));
        Some(TestCluster { port, dir, pg_ctl })
    }

    pub fn url(&self, database: &str, user: &str) -> String {
        format!("postgres://{user}@127.0.0.1:{}/{database}", self.port)
    }
}

impl Drop for TestCluster {
    fn drop(&mut self) {
        let _ = Command::new(&self.pg_ctl).arg("-D").arg(self.dir.join("data")).args(["-m", "immediate", "-w", "stop"]).env("LC_ALL", "C").output();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

//! `--exit-on-stdin-eof` : studiod s'arrête quand son app parente ferme le tube.

use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn spawn(exit_on_eof: bool) -> std::process::Child {
    let dir = std::env::temp_dir().join(format!("studiod-parent-watch-{}", std::process::id()));
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_studiod"));
    cmd.args(["--listen", &format!("127.0.0.1:{}", free_port())])
        .arg("--recordings")
        .arg(&dir)
        .args(["--width", "64", "--height", "36", "--fps", "5"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if exit_on_eof {
        cmd.arg("--exit-on-stdin-eof");
    }
    cmd.spawn().expect("lancement de studiod")
}

fn wait_exit(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Option<std::process::ExitStatus> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

#[test]
fn exits_when_stdin_closes() {
    let mut child = spawn(true);
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        child.try_wait().unwrap().is_none(),
        "studiod doit tourner tant que le tube est ouvert"
    );
    drop(child.stdin.take());
    let status = wait_exit(&mut child, Duration::from_secs(10))
        .expect("studiod doit s'arrêter à la fermeture du tube");
    assert!(status.success(), "arrêt propre attendu, obtenu {status:?}");
}

#[test]
fn keeps_running_without_flag() {
    let mut child = spawn(false);
    drop(child.stdin.take());
    let exited = wait_exit(&mut child, Duration::from_millis(800));
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        exited.is_none(),
        "sans l'option, studiod ignore son entrée standard"
    );
}

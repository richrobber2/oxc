use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let serial = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir()
            .join(format!("oxc-parser-tools-{}-{nonce}-{serial}", std::process::id()));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path).unwrap();
        Self(path)
    }
    fn run(&self, mode: &str, name: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_oxc-parser-tools"))
            .args([mode])
            .arg(self.0.join(name))
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn formats_stdout_without_changing_input_and_rechecks_idempotently() {
    let fixture = Fixture::new();
    let source = "// 雪\nexport const item: { name: string } = { name: '雪' }";
    fs::write(fixture.0.join("valid.ts"), source).unwrap();
    let checked = fixture.run("check", "valid.ts");
    assert!(checked.status.success(), "{checked:?}");
    let metadata: serde_json::Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(metadata["syntaxValid"], true);
    let first = fixture.run("format", "valid.ts");
    assert!(first.status.success(), "{first:?}");
    assert_eq!(first.stderr, [] as [u8; 0]);
    assert_eq!(fs::read_to_string(fixture.0.join("valid.ts")).unwrap(), source);
    fs::write(fixture.0.join("formatted.ts"), &first.stdout).unwrap();
    let second = fixture.run("format", "formatted.ts");
    assert!(second.status.success());
    assert_eq!(first.stdout, second.stdout);
}

#[test]
fn invalid_input_fails_without_stdout_or_file_changes() {
    let fixture = Fixture::new();
    for (name, bytes) in [
        ("malformed.js", b"const item = ;".to_vec()),
        ("duplicate.js", b"let same; let same;".to_vec()),
        ("invalid.js", vec![0xff]),
        ("large.js", vec![b' '; oxc_parser_tools::MAX_SOURCE_BYTES + 1]),
        ("unsupported.txt", b"const item = 1;".to_vec()),
    ] {
        fs::write(fixture.0.join(name), &bytes).unwrap();
        for mode in ["check", "format"] {
            let result = fixture.run(mode, name);
            assert!(!result.status.success(), "{mode} {name}");
            assert!(result.stdout.is_empty(), "{mode} {name}");
            assert!(!result.stderr.is_empty(), "{mode} {name}");
        }
        assert_eq!(fs::read(fixture.0.join(name)).unwrap(), bytes);
    }
}

#[cfg(feature = "heap-profile")]
#[test]
fn dhat_guards_borrowed_diagnostic_allocation_cost() {
    let fixture = Fixture::new();
    let profile = |case: &str| {
        let output = Command::new(env!("CARGO_BIN_EXE_oxc-parser-tools-heap"))
            .args([case, "3"])
            .current_dir(&fixture.0)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    let owned = profile("diagnostics-owned");
    let borrowed = profile("diagnostics-borrowed");
    assert_eq!(owned["checksum"], borrowed["checksum"]);
    for metric in ["totalBytes", "peakBytes"] {
        assert!(borrowed[metric].as_u64().unwrap() < owned[metric].as_u64().unwrap() / 10);
    }
    assert_eq!(owned["endBytes"], 0);
    assert_eq!(borrowed["endBytes"], 0);
}

#[cfg(unix)]
#[test]
fn rejects_fifo_without_waiting_for_a_writer() {
    use std::{
        ffi::CString,
        os::unix::ffi::OsStrExt,
        process::Stdio,
        time::{Duration, Instant},
    };
    let fixture = Fixture::new();
    let path = fixture.0.join("pipe.js");
    let name = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: name is a valid NUL-terminated path in our private temporary directory.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let mut child = Command::new(env!("CARGO_BIN_EXE_oxc-parser-tools"))
        .arg("check")
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > Duration::from_secs(2) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("FIFO open blocked waiting for a writer");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let result = child.wait_with_output().unwrap();
    assert!(!result.status.success());
    assert_eq!(result.stdout, [] as [u8; 0]);
    assert!(String::from_utf8(result.stderr).unwrap().contains("regular file"));
}

//! HV-SCRIPT-11: showing a badge never runs a program (Q4, R-M4b-10, `alg/28` §0.4, §0.8, U-02).
//!
//! Every test here re-runs itself as a child process of this test binary, with a hostile
//! environment (`HOME`, `XDG_CONFIG_HOME`, `PATH`, `GIT_CONFIG_*`, `GIT_DIR`…) set on the child
//! only (no test calls `set_var`). The child calls the real [`hacivat_git::status`] and writes
//! what it got to a result file. Where the OS offers one, the child runs under a spawn
//! detector:
//! - macOS: `sandbox-exec` with `(deny process-fork (with send-signal SIGKILL))`, so any spawn
//!   attempt kills the child (exit 137);
//! - Linux: `strace -f -e trace=execve` when `strace` is installed (one `execve`, the child's
//!   own, is expected);
//! - Windows: a Job Object (created by a PowerShell helper) that the child starts in,
//!   suspended; after it exits the job's `TotalProcesses` must be 1 (U-02).
//!
//! Each detector has a positive control (a child that does spawn). Every program the hostile
//! configuration names writes a marker file; no marker may appear. The git CLI runs only to
//! build fixtures in this test's own temp directories (`NoGitSubprocess` exempts `tests/`).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use hacivat_git::{FileStatus, GitLimits, GitState, status};

/// Env var naming the library a child run reads (set only on the child).
const CHILD_LIB: &str = "HV_T07_CHILD_LIB";
/// Env var naming the file the child writes its result to.
const CHILD_OUT: &str = "HV_T07_CHILD_OUT";
/// Env var listing FIFO detectors (`;`-separated) the child checks after the status.
const CHILD_FIFOS: &str = "HV_T07_CHILD_FIFOS";
/// Env var asking the child to spawn a program (the detectors' positive control).
const CHILD_SPAWN: &str = "HV_T07_CHILD_SPAWN";

fn fact(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(format!("HV-FACT {line}\n").as_bytes());
    let _ = out.flush();
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).expect("canonical path")
}

// ---------------------------------------------------------------------------------------------
// The child side.
// ---------------------------------------------------------------------------------------------

/// The status as one line: `Ready path=badge;…`, or the state's `Debug`.
fn describe(st: &GitState) -> String {
    match st {
        GitState::Ready(s) => {
            let parts: Vec<String> = s
                .by_path
                .iter()
                .map(|(k, v)| format!("{}={v:?}", k.as_str()))
                .collect();
            format!("Ready {}", parts.join(";"))
        }
        other => format!("{other:?}"),
    }
}

/// When this process is a child run: do the child's work, write the result, and answer true.
fn child_run() -> bool {
    let (Some(lib), Some(out)) = (std::env::var_os(CHILD_LIB), std::env::var_os(CHILD_OUT)) else {
        return false;
    };
    if std::env::var_os(CHILD_SPAWN).is_some() {
        // Positive control: a spawn the detector must see.
        #[cfg(windows)]
        let _ = Command::new("cmd").args(["/C", "exit", "0"]).status();
        #[cfg(not(windows))]
        let _ = Command::new("/bin/sh").args(["-c", "exit 0"]).status();
    }
    let st = status(Path::new(&lib), &GitLimits::default());
    let mut text = describe(&st);
    if let Some(fifos) = std::env::var_os(CHILD_FIFOS) {
        let opened: Vec<String> = fifos
            .to_string_lossy()
            .split(';')
            .filter(|f| !f.is_empty())
            .filter(|f| fifo_has_reader(Path::new(f)))
            .map(str::to_owned)
            .collect();
        text.push_str(&format!("\nfifo_opened={}", opened.join(";")));
    }
    std::fs::write(out, text).expect("write the child result");
    true
}

/// Whether some process holds `fifo` open for reading: a non-blocking open for writing
/// succeeds only then (ENXIO otherwise). The FIFO detectors of `alg/28` §0.8.
#[cfg(unix)]
fn fifo_has_reader(fifo: &Path) -> bool {
    use std::os::unix::fs::OpenOptionsExt as _;
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
    const O_NONBLOCK: i32 = 0x0004;
    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "freebsd")))]
    const O_NONBLOCK: i32 = 0o4000;
    std::fs::File::options()
        .write(true)
        .custom_flags(O_NONBLOCK)
        .open(fifo)
        .is_ok()
}

#[cfg(not(unix))]
fn fifo_has_reader(_fifo: &Path) -> bool {
    false
}

// ---------------------------------------------------------------------------------------------
// The parent side: fixtures, the hostile environment, the detectors.
// ---------------------------------------------------------------------------------------------

/// A fixture superproject (with a submodule when asked) plus a hostile environment directory.
struct Hostile {
    repo: tempfile::TempDir,
    env: tempfile::TempDir,
}

impl Hostile {
    fn root(&self) -> PathBuf {
        canonical(self.repo.path())
    }
    fn envdir(&self) -> PathBuf {
        canonical(self.env.path())
    }
    fn marker(&self, name: &str) -> PathBuf {
        self.envdir().join("markers").join(name)
    }
    fn markers(&self) -> Vec<String> {
        std::fs::read_dir(self.envdir().join("markers"))
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The CLI for building fixtures, with a clean environment of its own.
fn git(cwd: &Path, args: &[&str]) {
    let mut c = Command::new("git");
    c.current_dir(cwd).args(args);
    for k in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_COUNT",
    ] {
        c.env_remove(k);
    }
    c.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", no_global_config(cwd))
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .env("GIT_TERMINAL_PROMPT", "0");
    let out = c.output().expect("run the git CLI");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `GIT_CONFIG_GLOBAL` while building fixtures: a path that never exists (the CLI reads a
/// missing global file as empty), so no shared temp directory has to outlive a test.
fn no_global_config(cwd: &Path) -> PathBuf {
    cwd.join("hv-t07-no-global-config")
}

fn write(p: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

/// A shell command that writes the marker `name` (valid for `sh -c` and `cmd /C` alike).
fn touch_cmd(h: &Hostile, name: &str) -> String {
    format!("echo hv > \"{}\"", h.marker(name).display())
}

/// A committed repository with `a.sql`, `b.sql`, and (when `with_submodule`) a submodule at
/// `mods/sub`. Nothing hostile yet: the CLI still runs on it.
fn base(with_submodule: bool) -> Hostile {
    let h = Hostile {
        repo: tempfile::tempdir().unwrap(),
        env: tempfile::tempdir().unwrap(),
    };
    std::fs::create_dir_all(h.envdir().join("markers")).unwrap();
    let root = h.root();
    git(&root, &["init", "-q", "-b", "main"]);
    write(&root.join("a.sql"), b"select 1;\n");
    write(&root.join("b.sql"), b"select 2;\n");
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "--no-verify", "-m", "base"]);
    if with_submodule {
        let sub = h.envdir().join("subsrc");
        std::fs::create_dir_all(&sub).unwrap();
        git(&sub, &["init", "-q", "-b", "main"]);
        write(&sub.join("s.sql"), b"select 3;\n");
        git(&sub, &["add", "-A"]);
        git(&sub, &["commit", "-q", "--no-verify", "-m", "sub"]);
        git(
            &root,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-q",
                sub.to_str().unwrap(),
                "mods/sub",
            ],
        );
        git(&root, &["commit", "-q", "--no-verify", "-m", "add submodule"]);
        // A change inside the submodule's worktree, so a real submodule status would look.
        write(&root.join("mods/sub/s.sql"), b"select 'changed';\n");
    }
    h
}

/// Sets `key = value` in the config file `file` with the CLI (`git config --file` runs no
/// configured program).
fn set(file: &Path, key: &str, value: &str) {
    let dir = file.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["config", "--file", file.to_str().unwrap(), key, value]);
}

/// Every program-running key of `alg/28` §0.4 in `file`, each writing its own marker.
fn hostile_config(h: &Hostile, file: &Path, tag: &str) {
    let m = |k: &str| touch_cmd(h, &format!("{tag}-{k}"));
    for (k, v) in [
        ("core.fsmonitor", m("fsmonitor")),
        ("filter.x.clean", m("filter-clean")),
        ("filter.x.smudge", m("filter-smudge")),
        ("filter.x.process", m("filter-process")),
        ("filter.x.required", "true".to_owned()),
        ("filter.lfs.clean", m("lfs-clean")),
        ("filter.lfs.process", m("lfs-process")),
        ("diff.external", m("diff-external")),
        ("diff.x.textconv", m("textconv")),
        ("diff.x.command", m("diff-command")),
        ("merge.x.driver", m("merge-driver")),
        ("core.sshCommand", m("ssh")),
        ("credential.helper", format!("!{}", m("credential"))),
        ("core.pager", m("pager")),
        ("pager.status", m("pager-status")),
        ("core.editor", m("editor")),
        ("sequence.editor", m("sequence-editor")),
        ("gpg.program", m("gpg")),
        ("core.askPass", m("askpass")),
        ("core.gitProxy", m("proxy")),
        ("uploadpack.packObjectsHook", m("pack-hook")),
        ("safe.directory", "*".to_owned()),
    ] {
        set(file, k, &v);
    }
}

/// A hooks directory whose `post-index-change` (and friends) write markers.
fn hooks_dir(h: &Hostile, dir: &Path, tag: &str) {
    std::fs::create_dir_all(dir).unwrap();
    for hook in ["post-index-change", "reference-transaction", "pre-commit", "post-checkout"] {
        let p = dir.join(hook);
        write(
            &p,
            format!("#!/bin/sh\n{}\n", touch_cmd(h, &format!("{tag}-hook-{hook}"))).as_bytes(),
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

/// Turns the committed fixture hostile (repository side): every program key, includes,
/// hooks, `core.worktree`, attributes naming the drivers, a changed `a.sql`, and (with a
/// submodule) a hostile submodule config.
fn arm_repository(h: &Hostile) {
    let root = h.root();
    let cfg = root.join(".git/config");
    hostile_config(h, &cfg, "repo");
    let inc = h.envdir().join("include.cfg");
    set(&inc, "core.fsmonitor", &touch_cmd(h, "include-fsmonitor"));
    set(&cfg, "include.path", inc.to_str().unwrap());
    set(&cfg, "includeIf.gitdir:/.path", inc.to_str().unwrap());
    let hooks = h.envdir().join("hooks");
    hooks_dir(h, &hooks, "hookspath");
    set(&cfg, "core.hooksPath", hooks.to_str().unwrap());
    hooks_dir(h, &root.join(".git/hooks"), "dotgit");
    let elsewhere = h.envdir().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    write(&elsewhere.join("decoy-worktree.sql"), b"select 0;\n");
    set(&cfg, "core.worktree", elsewhere.to_str().unwrap());
    write(
        &root.join(".git/info/attributes"),
        b"*.sql filter=x diff=x merge=x\n",
    );
    write(&root.join(".gitattributes"), b"*.sql filter=x diff=x merge=x\n");
    // Changed content of another length: status must look at it.
    write(&root.join("a.sql"), b"select 'changed on disk';\n");
    let sub_cfg = root.join(".git/modules/mods/sub/config");
    if sub_cfg.exists() {
        hostile_config(h, &sub_cfg, "submodule");
        write(
            &root.join(".git/modules/mods/sub/info/attributes"),
            b"* filter=x\n",
        );
    }
}

/// A decoy repository that `GIT_DIR`/`GIT_WORK_TREE`/`GIT_INDEX_FILE` point at.
fn decoy(h: &Hostile) -> PathBuf {
    let d = h.envdir().join("decoy");
    std::fs::create_dir_all(&d).unwrap();
    git(&d, &["init", "-q", "-b", "main"]);
    write(&d.join("decoy-only.sql"), b"select 0;\n");
    d
}

/// The hostile environment of `alg/28` §0.4 plus `GIT_CONFIG_PARAMETERS` (m4): user and XDG
/// config with program keys and `safe.directory = *`, global attributes naming a filter,
/// `GIT_CONFIG_*` overrides, `GIT_DIR` at a decoy, and (Unix) a fake program named like the
/// git CLI first on `PATH`.
fn hostile_env(h: &Hostile) -> Vec<(String, String)> {
    let e = h.envdir();
    let home = e.join("home");
    let xdg = e.join("xdg");
    hostile_config(h, &home.join(".gitconfig"), "home");
    hostile_config(h, &xdg.join("git/config"), "xdg");
    write(&xdg.join("git/attributes"), b"* filter=x\n");
    hostile_config(h, &e.join("global.cfg"), "global");
    hostile_config(h, &e.join("system.cfg"), "system");
    let d = decoy(h);
    let mut env = vec![
        ("HOME".to_owned(), home.display().to_string()),
        ("USERPROFILE".to_owned(), home.display().to_string()),
        ("XDG_CONFIG_HOME".to_owned(), xdg.display().to_string()),
        ("GIT_CONFIG_GLOBAL".to_owned(), e.join("global.cfg").display().to_string()),
        ("GIT_CONFIG_SYSTEM".to_owned(), e.join("system.cfg").display().to_string()),
        ("GIT_CONFIG_COUNT".to_owned(), "1".to_owned()),
        ("GIT_CONFIG_KEY_0".to_owned(), "core.fsmonitor".to_owned()),
        ("GIT_CONFIG_VALUE_0".to_owned(), touch_cmd(h, "env-count")),
        (
            "GIT_CONFIG_PARAMETERS".to_owned(),
            format!("'core.fsmonitor'='{}'", touch_cmd(h, "env-parameters")),
        ),
        ("GIT_EXTERNAL_DIFF".to_owned(), touch_cmd(h, "env-external-diff")),
        ("GIT_SSH_COMMAND".to_owned(), touch_cmd(h, "env-ssh")),
        ("GIT_DIR".to_owned(), d.join(".git").display().to_string()),
        ("GIT_WORK_TREE".to_owned(), d.display().to_string()),
        ("GIT_INDEX_FILE".to_owned(), d.join(".git/index").display().to_string()),
    ];
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let bin = e.join("bin");
        let fake = bin.join("git");
        write(
            &fake,
            format!("#!/bin/sh\n{}\nexit 0\n", touch_cmd(h, "path-fake-cli")).as_bytes(),
        );
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::var("PATH").unwrap_or_default();
        env.push(("PATH".to_owned(), format!("{}:{path}", bin.display())));
    }
    env
}

/// What a child run saw, and what the spawn detector says.
struct ChildRun {
    result: String,
    detector: &'static str,
    /// `Some(n)`: the detector counted `n` spawns beyond the child itself.
    spawned: Option<u64>,
}

/// Re-runs the test `name` of this binary as a child with `envs`, under the OS's spawn
/// detector, and returns what it wrote.
fn run_child(name: &str, lib: &Path, envs: &[(String, String)], out_dir: &Path) -> ChildRun {
    let exe = std::env::current_exe().unwrap();
    let out = out_dir.join(format!("child-{name}.txt"));
    let _ = std::fs::remove_file(&out);
    let args = [name, "--exact", "--test-threads=1", "--quiet"];
    let mut all: Vec<(String, String)> = envs.to_vec();
    all.push((CHILD_LIB.to_owned(), lib.display().to_string()));
    all.push((CHILD_OUT.to_owned(), out.display().to_string()));
    let (detector, spawned, status_ok) = detect(&exe, &args, &all, out_dir);
    let result = std::fs::read_to_string(&out).unwrap_or_default();
    assert!(
        status_ok || spawned.is_some_and(|n| n > 0),
        "the child run failed without a result: {result:?}"
    );
    ChildRun {
        result,
        detector,
        spawned,
    }
}

#[cfg(target_os = "macos")]
fn detect(
    exe: &Path,
    args: &[&str],
    envs: &[(String, String)],
    _dir: &Path,
) -> (&'static str, Option<u64>, bool) {
    let sandbox = Path::new("/usr/bin/sandbox-exec");
    if !sandbox.exists() {
        let st = Command::new(exe).args(args).envs(envs.iter().cloned()).status().unwrap();
        return ("markers", None, st.success());
    }
    let st = Command::new(sandbox)
        .args([
            "-p",
            "(version 1)(allow default)(deny process-fork (with send-signal SIGKILL))",
        ])
        .arg(exe)
        .args(args)
        .envs(envs.iter().cloned())
        .status()
        .unwrap();
    use std::os::unix::process::ExitStatusExt as _;
    let killed = st.signal() == Some(9) || st.code() == Some(137);
    ("sandbox_exec", Some(u64::from(killed)), st.success())
}

#[cfg(target_os = "linux")]
fn detect(
    exe: &Path,
    args: &[&str],
    envs: &[(String, String)],
    dir: &Path,
) -> (&'static str, Option<u64>, bool) {
    let has_strace = Command::new("strace")
        .arg("-V")
        .output()
        .is_ok_and(|o| o.status.success());
    if !has_strace {
        let st = Command::new(exe).args(args).envs(envs.iter().cloned()).status().unwrap();
        return ("markers", None, st.success());
    }
    let log = dir.join("strace.log");
    let st = Command::new("strace")
        .args(["-f", "-qq", "-e", "trace=execve,execveat", "-o"])
        .arg(&log)
        .arg(exe)
        .args(args)
        .envs(envs.iter().cloned())
        .status()
        .unwrap();
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    let execs = text
        .lines()
        .filter(|l| l.contains("execve(") || l.contains("execveat("))
        .count() as u64;
    ("strace", Some(execs.saturating_sub(1)), st.success())
}

/// Windows: the child starts suspended inside a fresh Job Object, created by a PowerShell
/// helper (no `unsafe` in Rust tests); after it exits the job's `TotalProcesses` says how many
/// processes ever ran in it. 1 is the child alone (U-02).
#[cfg(windows)]
fn detect(
    exe: &Path,
    args: &[&str],
    envs: &[(String, String)],
    dir: &Path,
) -> (&'static str, Option<u64>, bool) {
    let script = dir.join("hv-job.ps1");
    std::fs::write(&script, JOB_PS1).unwrap();
    let mut cmdline = format!("\"{}\"", exe.display());
    for a in args {
        cmdline.push(' ');
        cmdline.push_str(a);
    }
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .env("HV_T07_JOB_CMD", &cmdline)
        .envs(envs.iter().cloned())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let line = text
        .lines()
        .find(|l| l.starts_with("HVJOB "))
        .unwrap_or_else(|| {
            panic!(
                "the Job Object helper failed: {text} {}",
                String::from_utf8_lossy(&out.stderr)
            )
        });
    let field = |k: &str| -> u64 {
        line.split_whitespace()
            .find_map(|f| f.strip_prefix(k))
            .and_then(|v| v.parse().ok())
            .unwrap_or(u64::MAX)
    };
    let total = field("total=");
    let code = field("exit=");
    ("job_object", Some(total.saturating_sub(1)), code == 0)
}

/// Creates a Job Object, starts `HV_T07_JOB_CMD` suspended, assigns it, resumes it, waits, and
/// prints `HVJOB total=<TotalProcesses> exit=<code>`.
#[cfg(windows)]
const JOB_PS1: &str = r#"
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class HvJob {
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
  public struct STARTUPINFO { public int cb; public string lpReserved; public string lpDesktop; public string lpTitle;
    public int dwX; public int dwY; public int dwXSize; public int dwYSize; public int dwXCountChars; public int dwYCountChars;
    public int dwFillAttribute; public int dwFlags; public short wShowWindow; public short cbReserved2; public IntPtr lpReserved2;
    public IntPtr hStdInput; public IntPtr hStdOutput; public IntPtr hStdError; }
  [StructLayout(LayoutKind.Sequential)]
  public struct PROCESS_INFORMATION { public IntPtr hProcess; public IntPtr hThread; public int dwProcessId; public int dwThreadId; }
  [StructLayout(LayoutKind.Sequential)]
  public struct ACCOUNTING { public long TotalUserTime; public long TotalKernelTime; public long ThisPeriodTotalUserTime;
    public long ThisPeriodTotalKernelTime; public uint TotalPageFaultCount; public uint TotalProcesses; public uint ActiveProcesses;
    public uint TotalTerminatedProcesses; }
  [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
  static extern IntPtr CreateJobObjectW(IntPtr attrs, string name);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool QueryInformationJobObject(IntPtr job, int cls, out ACCOUNTING info, int len, IntPtr ret);
  [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
  static extern bool CreateProcessW(string app, System.Text.StringBuilder cmd, IntPtr pa, IntPtr ta, bool inherit,
    uint flags, IntPtr env, string cwd, ref STARTUPINFO si, out PROCESS_INFORMATION pi);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern uint ResumeThread(IntPtr thread);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern uint WaitForSingleObject(IntPtr h, uint ms);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool GetExitCodeProcess(IntPtr h, out uint code);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool TerminateProcess(IntPtr h, uint code);
  public static string Run(string cmdline) {
    IntPtr job = CreateJobObjectW(IntPtr.Zero, null);
    if (job == IntPtr.Zero) { return "error=CreateJobObject:" + Marshal.GetLastWin32Error(); }
    STARTUPINFO si = new STARTUPINFO(); si.cb = Marshal.SizeOf(typeof(STARTUPINFO));
    PROCESS_INFORMATION pi;
    const uint CREATE_SUSPENDED = 0x4;
    if (!CreateProcessW(null, new System.Text.StringBuilder(cmdline), IntPtr.Zero, IntPtr.Zero, true, CREATE_SUSPENDED,
        IntPtr.Zero, null, ref si, out pi)) { return "error=CreateProcess:" + Marshal.GetLastWin32Error(); }
    if (!AssignProcessToJobObject(job, pi.hProcess)) {
      int err = Marshal.GetLastWin32Error(); TerminateProcess(pi.hProcess, 1); return "error=AssignProcessToJobObject:" + err; }
    ResumeThread(pi.hThread);
    WaitForSingleObject(pi.hProcess, 0xFFFFFFFF);
    uint code; GetExitCodeProcess(pi.hProcess, out code);
    ACCOUNTING acc;
    if (!QueryInformationJobObject(job, 1, out acc, Marshal.SizeOf(typeof(ACCOUNTING)), IntPtr.Zero)) {
      return "error=QueryInformationJobObject:" + Marshal.GetLastWin32Error(); }
    return "total=" + acc.TotalProcesses + " exit=" + code;
  }
}
"@
$r = [HvJob]::Run($env:HV_T07_JOB_CMD)
Write-Output ("HVJOB " + $r)
"#;

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn detect(
    exe: &Path,
    args: &[&str],
    envs: &[(String, String)],
    _dir: &Path,
) -> (&'static str, Option<u64>, bool) {
    let st = Command::new(exe).args(args).envs(envs.iter().cloned()).status().unwrap();
    ("markers", None, st.success())
}

fn badge<'a>(result: &'a str, path: &str) -> Option<&'a str> {
    let first = result.lines().next()?;
    let list = first.strip_prefix("Ready ")?;
    list.split(';')
        .find_map(|kv| kv.strip_prefix(&format!("{path}=")))
}

// ---------------------------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------------------------

/// The detector's positive control: a child that does start a program is seen.
#[test]
fn spawn_detector_sees_a_spawn() {
    if child_run() {
        return;
    }
    let h = base(false);
    let mut env = vec![(CHILD_SPAWN.to_owned(), "1".to_owned())];
    env.push(("HOME".to_owned(), h.envdir().display().to_string()));
    let run = run_child("spawn_detector_sees_a_spawn", &h.root(), &env, &h.envdir());
    fact(&format!(
        "t07 spawn_detector_control detector={} spawned={:?}",
        run.detector, run.spawned
    ));
    if let Some(n) = run.spawned {
        assert!(n >= 1, "{}: a spawn went unseen", run.detector);
    }
}

#[test]
fn hostile_repo_runs_no_program() {
    if child_run() {
        return;
    }
    let h = base(true);
    arm_repository(&h);
    let env = hostile_env(&h);
    assert!(h.markers().is_empty(), "a marker before the status: {:?}", h.markers());
    let before = std::fs::read(h.root().join(".git/index")).unwrap();
    let run = run_child("hostile_repo_runs_no_program", &h.root(), &env, &h.envdir());
    let markers = h.markers();
    fact(&format!(
        "t07 hostile os={} detector={} spawned={:?} markers={}",
        std::env::consts::OS,
        run.detector,
        run.spawned,
        markers.len()
    ));
    assert!(markers.is_empty(), "a repository program ran: {markers:?}");
    assert_eq!(run.spawned.unwrap_or(0), 0, "{}: a spawn", run.detector);
    assert_eq!(
        std::fs::read(h.root().join(".git/index")).unwrap(),
        before,
        "the index changed"
    );
    assert_eq!(
        badge(&run.result, "a.sql"),
        Some("Unknown(Filtered)"),
        "{}",
        run.result
    );
    assert_eq!(
        badge(&run.result, "mods/sub"),
        Some("Unknown(Submodule)"),
        "{}",
        run.result
    );
    // `core.worktree` was ignored: the walk listed the real worktree's untracked file (a
    // fully trusted open walks the decoy worktree instead, where no filtered file is hashed).
    assert_eq!(
        badge(&run.result, ".gitattributes"),
        Some("New"),
        "{}",
        run.result
    );
    assert!(!run.result.contains("decoy"), "{}", run.result);
}

/// `GIT_CONFIG_GLOBAL`, `HOME/.gitconfig`, `XDG_CONFIG_HOME/git/config` and the global
/// attributes are FIFOs: opening one blocks until a writer comes, and the child checks after
/// the status whether any of them has a reader (`alg/28` §0.8). A submodule entry is present,
/// so the built-in handler's nested default-options open would open them (C1, C12).
#[cfg(unix)]
#[test]
fn submodule_entry_reads_no_global_config_and_spawns_nothing() {
    if child_run() {
        return;
    }
    let h = base(true);
    let fifos = fifo_env(&h);
    let run = run_child(
        "submodule_entry_reads_no_global_config_and_spawns_nothing",
        &h.root(),
        &fifos.0,
        &h.envdir(),
    );
    fact(&format!(
        "t07 submodule_fifo detector={} spawned={:?} result={}",
        run.detector,
        run.spawned,
        run.result.replace('\n', " ")
    ));
    assert!(run.result.ends_with("\nfifo_opened="), "{}", run.result);
    assert_eq!(badge(&run.result, "mods/sub"), Some("Unknown(Submodule)"), "{}", run.result);
    assert_eq!(run.spawned.unwrap_or(0), 0);
    release(&fifos.1);
}

#[cfg(unix)]
#[test]
fn global_and_system_config_are_not_read() {
    if child_run() {
        return;
    }
    let h = base(false);
    write(&h.root().join("a.sql"), b"select 'changed';\n");
    let fifos = fifo_env(&h);
    let run = run_child("global_and_system_config_are_not_read", &h.root(), &fifos.0, &h.envdir());
    assert!(run.result.ends_with("\nfifo_opened="), "{}", run.result);
    assert_eq!(badge(&run.result, "a.sql"), Some("Modified"), "{}", run.result);
    release(&fifos.1);
}

/// The FIFO environment: every user, XDG, global and system config and attribute or ignore
/// file a non-isolated open would read is a FIFO.
#[cfg(unix)]
fn fifo_env(h: &Hostile) -> (Vec<(String, String)>, Vec<PathBuf>) {
    let e = h.envdir();
    let home = e.join("home");
    let xdg = e.join("xdg");
    let paths = vec![
        home.join(".gitconfig"),
        xdg.join("git/config"),
        xdg.join("git/attributes"),
        xdg.join("git/ignore"),
        e.join("global.cfg"),
        e.join("system.cfg"),
    ];
    for p in &paths {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let st = Command::new("mkfifo").arg(p).status().unwrap();
        assert!(st.success());
    }
    let list: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    let env = vec![
        ("HOME".to_owned(), home.display().to_string()),
        ("XDG_CONFIG_HOME".to_owned(), xdg.display().to_string()),
        ("GIT_CONFIG_GLOBAL".to_owned(), e.join("global.cfg").display().to_string()),
        ("GIT_CONFIG_SYSTEM".to_owned(), e.join("system.cfg").display().to_string()),
        (CHILD_FIFOS.to_owned(), list.join(";")),
    ];
    (env, paths)
}

/// Unblocks any reader still stuck in a FIFO (so no thread outlives the test).
#[cfg(unix)]
fn release(fifos: &[PathBuf]) {
    for f in fifos {
        if fifo_has_reader(f) {
            let _ = std::fs::File::options().write(true).open(f);
        }
    }
}

#[test]
fn git_dir_env_is_ignored() {
    if child_run() {
        return;
    }
    let h = base(false);
    write(&h.root().join("a.sql"), b"select 'changed';\n");
    let d = decoy(&h);
    let env = vec![
        ("GIT_DIR".to_owned(), d.join(".git").display().to_string()),
        ("GIT_WORK_TREE".to_owned(), d.display().to_string()),
        ("GIT_INDEX_FILE".to_owned(), d.join(".git/index").display().to_string()),
        ("GIT_COMMON_DIR".to_owned(), d.join(".git").display().to_string()),
    ];
    let run = run_child("git_dir_env_is_ignored", &h.root(), &env, &h.envdir());
    assert_eq!(badge(&run.result, "a.sql"), Some("Modified"), "{}", run.result);
    assert!(!run.result.contains("decoy-only"), "{}", run.result);
}

/// A user config with `safe.directory = *` (and one naming only the `.git` path) cannot promote
/// the repository: the filter driver in its config never runs and the path is `Unknown`.
#[test]
fn trust_is_reduced_even_with_global_safe_directory_star() {
    if child_run() {
        return;
    }
    for (case, value) in [("star", None), ("dot-git-only", Some(()))] {
        let h = base(false);
        let root = h.root();
        let cfg = root.join(".git/config");
        set(&cfg, "filter.x.clean", &touch_cmd(&h, &format!("{case}-clean")));
        set(&cfg, "filter.x.process", &touch_cmd(&h, &format!("{case}-process")));
        write(&root.join(".git/info/attributes"), b"*.sql filter=x\n");
        // Same length as committed, so the content must be hashed (racy stat).
        write(&root.join("a.sql"), b"select 9;\n");
        let home = h.envdir().join("home");
        let safe = match value {
            None => "*".to_owned(),
            Some(()) => root.join(".git").display().to_string(),
        };
        set(&home.join(".gitconfig"), "safe.directory", &safe);
        let env = vec![
            ("HOME".to_owned(), home.display().to_string()),
            ("USERPROFILE".to_owned(), home.display().to_string()),
            ("GIT_CONFIG_GLOBAL".to_owned(), home.join(".gitconfig").display().to_string()),
        ];
        let run = run_child(
            "trust_is_reduced_even_with_global_safe_directory_star",
            &root,
            &env,
            &h.envdir(),
        );
        assert!(h.markers().is_empty(), "{case}: {:?}", h.markers());
        assert_eq!(
            run.spawned.unwrap_or(0),
            0,
            "{case}: {} saw a spawn",
            run.detector
        );
        assert_eq!(
            badge(&run.result, "a.sql"),
            Some("Unknown(Filtered)"),
            "{case}: {}",
            run.result
        );
    }
}

/// A `.git` owned by another user: `Untrusted(ForeignOwner)`, nothing read (U-03). Only root
/// can make such a fixture (`chown`); elsewhere the test says so on a visible line and the
/// owner check is covered by `trust::tests::foreign_owner_*` with an injected owner.
#[cfg(unix)]
#[test]
fn foreign_owner_repo_is_untrusted() {
    use std::os::unix::fs::MetadataExt as _;
    let h = base(false);
    let probe = h.envdir().join("uid-probe");
    write(&probe, b"");
    let uid = std::fs::metadata(&probe).unwrap().uid();
    if uid != 0 {
        fact(&format!(
            "t07 foreign_owner_repo_is_untrusted skipped uid={uid} (needs root to chown); unit tests cover the check"
        ));
        return;
    }
    let dotgit = h.root().join(".git");
    std::os::unix::fs::chown(&dotgit, Some(65534), Some(65534)).unwrap();
    let st = status(&h.root(), &GitLimits::default());
    assert!(
        matches!(st, GitState::Untrusted(hacivat_git::UntrustReason::ForeignOwner)),
        "{st:?}"
    );
    fact("t07 foreign_owner_repo_is_untrusted ran as root");
}

/// Badges come back for a plain repository too (the control of the tests above).
#[test]
fn plain_repository_has_badges() {
    let h = base(false);
    write(&h.root().join("a.sql"), b"select 'changed';\n");
    let st = status(&h.root(), &GitLimits::default());
    let GitState::Ready(s) = &st else {
        panic!("{st:?}")
    };
    assert_eq!(s.by_path.get("a.sql"), Some(&FileStatus::Modified));
}

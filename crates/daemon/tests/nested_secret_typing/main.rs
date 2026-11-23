#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Secret typing end to end in a nested crownpositor. The real crownos-secretsd keeps a vault in
//! the test root; this test acts as the computer-use agent (the daemon's debug typing caller) and
//! as the admin. A GTK 4 window with a focused `Gtk.PasswordEntry` reports only a hash of what it
//! gets. crown-contextd watches the session too, as `/usr/bin/crown-contextd`.
//!
//! Run the tests through the namespace runner of crownos-visual-context-engine, with:
//! - `VCGE_NESTED_COMPOSITOR`: a debug crownpositor;
//! - `CROWNOS_SECRETSD`: a debug crownos-secretsd;
//! - `CROWNOS_PORTAL_DIALOG`: a build of crownos-portal-dialog, for the dialog test.
//!
//! ```sh
//! .../crown-contextd/tests/nested_capture/run-in-namespace.sh \
//!     target/debug/deps/nested_secret_typing-<hash> target/debug/crown-contextd
//! ```
//!
//! No person can click "Allow" in a test, so the typing test names the consent dialog stub
//! (`examples/consent_dialog_stub.rs`) in `CROWN_CONSENT_DIALOG`. The dialog test runs the real
//! dialog and checks only that its process hides itself while it asks.

mod compositor_watch;
mod session;

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crownos_ipc::Pending;
use secrets_core::{Binding, Confirm, SecretKind, SecretName, Target, UseOutcome};
use secrets_protocol::secrets::{self, Client, Event};
use secrets_protocol::{TypingTarget, transfer};

use compositor_watch::{toplevel_identifier, watch_flags};
use session::{Process, Session, wait_until};

const TIMEOUT: Option<Duration> = Some(Duration::from_secs(30));
const TYPING_TIMEOUT: Option<Duration> = Some(Duration::from_secs(150));
const STARTUP: Duration = Duration::from_secs(60);
const SECRET_NAME: &str = "e2e_login_password";
/// A test value only, made for this test.
const SECRET_VALUE: &str = "W8-e2e-Tr0ub4dor-7c1f9a";
const MASTER_PASSWORD: &[u8] = b"w8-e2e-master";
const APP_ID: &str = "org.crownos.VcgeSecretProbe";
const ENGINE: &str = "/usr/bin/crown-contextd";

/// The GTK window: a focused password entry. It prints when its text first changed and the
/// SHA-256 of the text each time, never the text. It is a native program with a system desktop
/// entry, because crownos-secretsd types only into installed apps that are no script.
const GTK_PROBE: &str = r#"
#include <gtk/gtk.h>
#include <stdio.h>

static gboolean first = TRUE;

static void changed(GObject *entry, GParamSpec *spec, gpointer data) {
    const char *text = gtk_editable_get_text(GTK_EDITABLE(entry));
    if (first) {
        first = FALSE;
        printf("first change %.6f\n", (double)g_get_real_time() / 1e6);
    }
    char *hash = g_compute_checksum_for_string(G_CHECKSUM_SHA256, text, -1);
    printf("hash %s\n", hash);
    fflush(stdout);
    g_free(hash);
}

static void activate(GtkApplication *app, gpointer data) {
    GtkWidget *window = gtk_application_window_new(app);
    gtk_window_set_title(GTK_WINDOW(window), "Secret probe");
    GtkWidget *entry = gtk_password_entry_new();
    g_signal_connect(entry, "notify::text", G_CALLBACK(changed), NULL);
    gtk_window_set_child(GTK_WINDOW(window), entry);
    gtk_window_present(GTK_WINDOW(window));
    gtk_widget_grab_focus(entry);
}

int main(int argc, char **argv) {
    GtkApplication *app = gtk_application_new(PROBE_APP_ID, G_APPLICATION_DEFAULT_FLAGS);
    g_signal_connect(app, "activate", G_CALLBACK(activate), NULL);
    int status = g_application_run(G_APPLICATION(app), 1, argv);
    g_object_unref(app);
    return status;
}
"#;

/// Compiles the probe into the test root and installs its desktop entry in the namespace's
/// `/usr/share/applications`.
fn install_gtk_probe(root: &Path) -> PathBuf {
    let source = root.join("secret-probe.c");
    let program = root.join("secret-probe");
    fs::write(&source, GTK_PROBE).unwrap();
    let compiled = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "gcc -O1 -DPROBE_APP_ID='\"{APP_ID}\"' -o {} {} $(pkg-config --cflags --libs gtk4)",
            program.display(),
            source.display()
        ))
        .status()
        .unwrap();
    assert!(compiled.success(), "cannot compile the GTK probe");
    fs::write(
        Path::new("/usr/share/applications").join(format!("{APP_ID}.desktop")),
        format!(
            "[Desktop Entry]\nType=Application\nName=Secret probe\nExec={}\n",
            program.display()
        ),
    )
    .expect("run through run-in-namespace.sh, which overlays /usr/share/applications");
    program
}

fn this_test() -> PathBuf {
    std::env::current_exe().unwrap()
}

fn sibling_binary(name: &str) -> PathBuf {
    this_test()
        .parent()
        .and_then(Path::parent)
        .map(|directory| directory.join(name))
        .unwrap()
}

/// `agents.ron` names the engine, so it gets its migrated grant; the test gets the metadata
/// scope for its context watch.
fn write_config(config: &Path) {
    fs::write(
        config.join("agents.ron"),
        format!("(trusted_executables: [{ENGINE:?}], ask_for_app_access: false, app_access: {{}})"),
    )
    .unwrap();
    fs::write(config.join("visual_context.ron"), "(enabled: true)").unwrap();
    fs::write(
        config.join("context_access.ron"),
        format!(
            "(ask_on_first_use: false, host_default: Deny, migrated_agents: false, \
             grants: {{{:?}: (scopes: [Metadata], expires: Never)}})\n",
            format!("exe:{}", this_test().display())
        ),
    )
    .unwrap();
}

fn sha256_hex(text: &str) -> String {
    let mut python = Command::new("python3")
        .args([
            "-I",
            "-c",
            "import hashlib,sys;print(hashlib.sha256(sys.stdin.buffer.read()).hexdigest())",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    python
        .stdin
        .take()
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();
    String::from_utf8(python.wait_with_output().unwrap().stdout)
        .unwrap()
        .trim()
        .to_owned()
}

/// The admin and agent side of crownos-secretsd, with every reply and event kept as text.
struct Secrets {
    client: Client,
    seen: String,
}

impl Secrets {
    fn connect(session: &Session) -> Self {
        let directory = session.runtime.join("crownos");
        let socket = directory.join(format!("{}.sock", secrets::SERVICE));
        wait_until("the secrets socket", STARTUP, || socket.exists());
        let mut client = Client::connect_in(&directory).unwrap();
        client.subscribe_blocking::<secrets::Used>(TIMEOUT).unwrap();
        Self {
            client,
            seen: String::new(),
        }
    }

    fn call<R: serde::de::DeserializeOwned + std::fmt::Debug>(
        &mut self,
        timeout: Option<Duration>,
        start: impl FnOnce(&mut Client) -> Result<Pending<R>, crownos_ipc::Error>,
    ) -> R {
        let pending = start(&mut self.client).unwrap();
        let reply = self.client.wait(pending, timeout).unwrap();
        self.seen.push_str(&format!("{reply:?}\n"));
        while let Some(Event::Used(event)) = self.client.next_event() {
            self.seen.push_str(&format!("{event:?}\n"));
        }
        reply
    }

    /// Makes the vault and a password that only this test may type, into the probe app only.
    fn plant_secret(&mut self) {
        let name = || SecretName::parse(SECRET_NAME).unwrap();
        let password = || transfer::value_pipe(MASTER_PASSWORD).unwrap();
        self.call(TIMEOUT, |client| client.init(password()));
        self.call(TIMEOUT, |client| {
            client.add(
                name(),
                SecretKind::Password,
                password(),
                transfer::value_pipe(SECRET_VALUE.as_bytes()).unwrap(),
            )
        });
        let binding = Binding {
            agents: vec![format!("exe:{}", this_test().display())],
            targets: vec![Target::AppId(APP_ID.to_owned())],
            require_password_field: true,
            confirm: Some(Confirm::Never),
            http: Vec::new(),
        };
        self.call(TIMEOUT, |client| client.bind(name(), binding, password()));
    }

    fn type_secret(&mut self, window: String) -> UseOutcome {
        let target = TypingTarget {
            window,
            app_id: APP_ID.to_owned(),
            origin: None,
        };
        self.call(TYPING_TIMEOUT, |client| {
            client.type_into(SecretName::parse(SECRET_NAME).unwrap(), target)
        })
    }
}

/// The nested session with crown-contextd, crownos-secretsd and the GTK window.
struct SecretSession {
    session: Session,
    contextd: Process,
    secretsd: Process,
    gtk: Process,
}

impl SecretSession {
    fn start(dialog: &Path) -> Self {
        let secretsd =
            PathBuf::from(std::env::var_os("CROWNOS_SECRETSD").expect("set CROWNOS_SECRETSD"));
        let session = Session::start(
            &[
                ("CROWN_SECRETS_DAEMON", &secretsd),
                ("CROWN_CONSENT_DIALOG", dialog),
            ],
            write_config,
        );
        let state = session.root.join("state");
        fs::create_dir_all(&state).unwrap();
        let contextd = Process::start(
            "crown-contextd",
            session
                .client(ENGINE)
                .arg("--state-dir")
                .arg(&state)
                .arg("--socket-dir")
                .arg(session.runtime.join("crownos")),
            &session.root.join("contextd.log"),
        );
        let secretsd = Process::start(
            "crownos-secretsd",
            session
                .client(&secretsd)
                .env("CROWNOS_SECRETS_DEBUG_ADMIN_EXECUTABLES", this_test())
                .env("CROWNOS_SECRETS_DEBUG_TYPING_CALLERS", this_test())
                .env("RUST_LOG", "info"),
            &session.root.join("secretsd.log"),
        );
        let probe = install_gtk_probe(&session.root);
        let gtk = Process::start(
            "gtk4 password probe",
            &mut session.client(probe),
            &session.root.join("gtk.log"),
        );
        Self {
            session,
            contextd,
            secretsd,
            gtk,
        }
    }

    fn gtk_pid(&self) -> u32 {
        self.gtk.pid()
    }

    fn stop(&mut self) {
        self.gtk.interrupt_then_wait(Duration::from_secs(2));
        self.secretsd.interrupt_then_wait(Duration::from_secs(5));
        self.contextd.interrupt_then_wait(Duration::from_secs(15));
        self.session
            .compositor
            .interrupt_then_wait(Duration::from_secs(10));
    }
}

fn first_change(gtk_log: &str) -> Option<SystemTime> {
    let seconds: f64 = gtk_log
        .lines()
        .find_map(|line| line.strip_prefix("first change "))?
        .trim()
        .parse()
        .ok()?;
    Some(UNIX_EPOCH + Duration::from_secs_f64(seconds))
}

/// Every file under `root` and every DB value of contextd, searched for the value.
fn places_with_the_value(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut directories = vec![root.to_owned()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory).into_iter().flatten().flatten() {
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.is_dir() {
                directories.push(path);
            } else if metadata.is_file()
                && fs::read(&path).is_ok_and(|bytes| {
                    bytes
                        .windows(SECRET_VALUE.len())
                        .any(|window| window == SECRET_VALUE.as_bytes())
                })
            {
                found.push(path);
            }
        }
    }
    found
}

#[test]
#[ignore = "run through run-in-namespace.sh with a nested crownpositor and crownos-secretsd"]
fn a_typed_secret_reaches_the_field_and_nothing_else() {
    let stub = sibling_binary("examples/consent_dialog_stub");
    assert!(
        stub.is_file(),
        "build the consent_dialog_stub example first"
    );
    let mut nested = SecretSession::start(&stub);
    let flags = watch_flags(&nested.session.display);
    let window = toplevel_identifier(&nested.session.display, APP_ID, STARTUP);
    let mut secrets = Secrets::connect(&nested.session);
    secrets.plant_secret();
    std::thread::sleep(Duration::from_secs(2));

    let outcome = secrets.type_secret(window);
    eprintln!("type_into answered {outcome:?}");
    std::thread::sleep(Duration::from_secs(2));
    nested.stop();

    let gtk_log = nested.session.log("gtk.log");
    let expected = format!("hash {}", sha256_hex(SECRET_VALUE));
    assert_eq!(
        outcome,
        UseOutcome::Done,
        "see {}",
        nested.session.root.display()
    );
    assert!(
        gtk_log.lines().any(|line| line == expected),
        "the field never held the value"
    );
    let flagged = flags
        .first_flag_of(APP_ID)
        .expect("the window was never flagged");
    let typed = first_change(&gtk_log).expect("the field never changed");
    eprintln!(
        "flagged {:?} before the first change",
        typed.duration_since(flagged).unwrap_or_default()
    );
    assert!(flagged <= typed, "the first key came before the flag");

    assert!(
        !secrets.seen.contains(SECRET_VALUE),
        "a reply or event held the value"
    );
    let leaks = places_with_the_value(&nested.session.root);
    assert!(leaks.is_empty(), "the value is in {leaks:?}");
    let compositor_log = nested.session.log("compositor.log");
    assert!(compositor_log.contains("asking the user to allow typing"));
    let _ = fs::remove_dir_all(&nested.session.root);
}

#[test]
#[ignore = "run through run-in-namespace.sh with a nested crownpositor, crownos-secretsd and the real dialog"]
fn the_real_dialog_hides_its_process_while_it_asks() {
    let dialog = PathBuf::from(
        std::env::var_os("CROWNOS_PORTAL_DIALOG").expect("set CROWNOS_PORTAL_DIALOG"),
    );
    let mut nested = SecretSession::start(&dialog);
    let window = toplevel_identifier(&nested.session.display, APP_ID, STARTUP);
    let mut secrets = Secrets::connect(&nested.session);
    secrets.plant_secret();
    std::thread::sleep(Duration::from_secs(2));

    let agent = std::thread::spawn(move || {
        let outcome = secrets.type_secret(window);
        (outcome, secrets.seen)
    });
    let mut dialog_pid = None;
    wait_until("the dialog process", STARTUP, || {
        dialog_pid = process_running(&dialog);
        dialog_pid.is_some()
    });
    let dialog_pid = dialog_pid.unwrap();
    eprintln!("the dialog runs as PID {dialog_pid}");
    assert!(
        !is_hidden(i32::try_from(nested.gtk_pid()).unwrap()),
        "the check must see a process that did not hide"
    );
    wait_until("the dialog to hide itself", Duration::from_secs(15), || {
        is_hidden(dialog_pid)
    });
    eprintln!("PID {dialog_pid} turned PR_SET_DUMPABLE off: its environ cannot be read");
    if let Some(pid) = rustix::process::Pid::from_raw(dialog_pid) {
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
    }
    let (outcome, seen) = agent.join().unwrap();
    eprintln!("type_into answered {outcome:?}");
    nested.stop();

    let gtk_log = nested.session.log("gtk.log");
    assert_ne!(outcome, UseOutcome::Done);
    assert!(
        first_change(&gtk_log).is_none(),
        "the field changed without an answer"
    );
    assert!(!seen.contains(SECRET_VALUE));
    let _ = fs::remove_dir_all(&nested.session.root);
}

/// The PID of a process started as `executable`, from its command line, which stays readable
/// after the process hides itself.
fn process_running(executable: &Path) -> Option<i32> {
    let program = executable.as_os_str().as_encoded_bytes();
    fs::read_dir("/proc").ok()?.flatten().find_map(|entry| {
        let pid: i32 = entry.file_name().to_str()?.parse().ok()?;
        let command_line = fs::read(entry.path().join("cmdline")).ok()?;
        (command_line.split(|byte| *byte == 0).next() == Some(program)).then_some(pid)
    })
}

/// A process that turned off `PR_SET_DUMPABLE`: the test runs without capabilities, so it can
/// no longer read the environment of such a process of its own user.
fn is_hidden(pid: i32) -> bool {
    let entry = PathBuf::from(format!("/proc/{pid}"));
    entry.exists()
        && matches!(
            fs::read(entry.join("environ")),
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied
        )
}

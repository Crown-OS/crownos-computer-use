//! The nested session: a private runtime directory and session bus, a nested crownpositor
//! without X11, and the processes of the test, each stopped by its own PID.

use std::fs;
use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal};

const NO_X11_BRIDGE: &str = "/nonexistent/xwayland-satellite";
const TEST_ROOT_ENV: &str = "VCGE_TEST_ROOT";

/// A child process stopped by its own PID, never by name.
pub(crate) struct Process {
    name: &'static str,
    child: Child,
}

impl Process {
    pub(crate) fn start(name: &'static str, command: &mut Command, log: &Path) -> Self {
        let log = fs::File::create(log).unwrap();
        let child = command
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap_or_else(|error| panic!("cannot start {name}: {error}"));
        eprintln!("{name} runs as PID {}", child.id());
        Self { name, child }
    }

    pub(crate) fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Returns whether the process stopped within `grace`.
    pub(crate) fn interrupt_then_wait(&mut self, grace: Duration) -> bool {
        if let Some(pid) = Pid::from_raw(i32::try_from(self.child.id()).unwrap()) {
            let _ = rustix::process::kill_process(pid, Signal::INT);
        }
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!(
            "{} did not stop in time; killing PID {}",
            self.name,
            self.child.id()
        );
        let _ = self.child.kill();
        let _ = self.child.wait();
        false
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// The directories, bus and display every process of the test shares.
pub(crate) struct Session {
    pub(crate) root: PathBuf,
    pub(crate) runtime: PathBuf,
    pub(crate) config: PathBuf,
    pub(crate) bus: String,
    pub(crate) display: PathBuf,
    _bus_daemon: Process,
    pub(crate) compositor: Process,
}

impl Session {
    /// Starts the bus and the compositor. `compositor_env` adds variables for the compositor
    /// only, such as the development daemon and dialog it trusts.
    pub(crate) fn start(
        compositor_env: &[(&str, &Path)],
        write_config: impl FnOnce(&Path),
    ) -> Self {
        let compositor_binary = PathBuf::from(
            std::env::var_os("VCGE_NESTED_COMPOSITOR").expect("set VCGE_NESTED_COMPOSITOR"),
        );
        let root = PathBuf::from(
            std::env::var_os(TEST_ROOT_ENV).expect("run through run-in-namespace.sh"),
        );
        let runtime = private_directory(&root.join("run"));
        let config = private_directory(&root.join("config"));
        write_config(&config);
        let (bus_daemon, bus) = private_bus(&runtime);
        let parent_display = parent_display();
        let session_command = |program: &Path| {
            let mut command = Command::new(program);
            command
                .env("XDG_RUNTIME_DIR", &runtime)
                .env("CROWN_CONFIG_DIR", &config)
                .env("DBUS_SESSION_BUS_ADDRESS", &bus)
                .env("GSETTINGS_BACKEND", "memory")
                .env_remove("AT_SPI_BUS_ADDRESS")
                .env_remove("LD_PRELOAD")
                .env_remove("LD_LIBRARY_PATH")
                .env_remove("DISPLAY");
            command
        };
        let mut command = session_command(&compositor_binary);
        command
            .env("CROWN_BACKEND", "winit")
            .env("WORKSPACE", "headless")
            .env("CROWN_XWAYLAND_SATELLITE", NO_X11_BRIDGE)
            .env("WAYLAND_DISPLAY", parent_display);
        for (name, value) in compositor_env {
            command.env(name, value);
        }
        let compositor = Process::start("crownpositor", &mut command, &root.join("compositor.log"));
        let display = wait_for_socket(&runtime, Duration::from_secs(30));
        eprintln!("the nested compositor listens on {}", display.display());
        Self {
            root,
            runtime,
            config,
            bus,
            display,
            _bus_daemon: bus_daemon,
            compositor,
        }
    }

    /// A command for a client of the nested session: its own runtime directory, bus and
    /// display, no preloaded code.
    pub(crate) fn client(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut command = Command::new(program);
        command
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("CROWN_CONFIG_DIR", &self.config)
            .env("DBUS_SESSION_BUS_ADDRESS", &self.bus)
            .env("GSETTINGS_BACKEND", "memory")
            .env("WAYLAND_DISPLAY", &self.display)
            .env_remove("AT_SPI_BUS_ADDRESS")
            .env_remove("LD_PRELOAD")
            .env_remove("LD_LIBRARY_PATH")
            .env_remove("DISPLAY")
            .env_remove("WORKSPACE");
        command
    }

    pub(crate) fn log(&self, name: &str) -> String {
        without_colours(&fs::read_to_string(self.root.join(name)).unwrap_or_default())
    }
}

fn private_directory(path: &Path) -> PathBuf {
    fs::create_dir_all(path).unwrap();
    rustix::fs::chmod(path, rustix::fs::Mode::from_raw_mode(0o700)).unwrap();
    path.to_owned()
}

fn private_bus(runtime: &Path) -> (Process, String) {
    let mut child = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1"])
        .env("XDG_RUNTIME_DIR", runtime)
        .env("GSETTINGS_BACKEND", "memory")
        .env_remove("DISPLAY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    eprintln!("dbus-daemon runs as PID {}", child.id());
    let stdout = child.stdout.take().unwrap();
    let mut address = String::new();
    BufReader::new(stdout).read_line(&mut address).unwrap();
    let address = address.trim().to_owned();
    assert!(!address.contains("/run/user/"), "{address} is a live bus");
    (
        Process {
            name: "dbus-daemon",
            child,
        },
        address,
    )
}

fn parent_display() -> PathBuf {
    let runtime = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR"));
    let display = PathBuf::from(std::env::var_os("WAYLAND_DISPLAY").expect("WAYLAND_DISPLAY"));
    if display.is_absolute() {
        display
    } else {
        runtime.join(display)
    }
}

fn wait_for_socket(runtime: &Path, timeout: Duration) -> PathBuf {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let socket = fs::read_dir(runtime).unwrap().flatten().find_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            (name.starts_with("wayland-") && !name.ends_with(".lock")).then(|| entry.path())
        });
        if let Some(socket) = socket {
            return socket;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!(
        "the nested compositor made no socket in {}",
        runtime.display()
    );
}

pub(crate) fn wait_until(what: &str, timeout: Duration, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if ready() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out waiting for {what}");
}

fn without_colours(log: &str) -> String {
    let mut plain = String::with_capacity(log.len());
    let mut in_escape = false;
    for character in log.chars() {
        match (in_escape, character) {
            (false, '\u{1b}') => in_escape = true,
            (true, 'm') => in_escape = false,
            (false, _) => plain.push(character),
            (true, _) => {}
        }
    }
    plain
}

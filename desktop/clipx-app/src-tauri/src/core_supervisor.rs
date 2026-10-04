use clipx_lib::ipc::non_blocking::Client;
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::Duration,
};
use tauri::{async_runtime::Mutex, menu::MenuItem, AppHandle, Emitter, Manager, Wry};

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CoreStatus {
    Running,
    Stopped,
    Restarting,
    Failed,
}

pub struct CoreSupervisor {
    child: StdMutex<Option<Child>>,
    /// true while the user/app stopped the core on purpose: watchdog must not revive it
    paused: AtomicBool,
    status: StdMutex<CoreStatus>,
    label: StdMutex<Option<MenuItem<Wry>>>,
}

fn core_exe() -> Result<PathBuf, String> {
    let dir = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("no exe directory")?
        .to_path_buf();
    Ok(dir.join(if cfg!(windows) {
        "clipx-core.exe"
    } else {
        "clipx-core"
    }))
}

fn spawn_child() -> Result<Child, String> {
    let mut cmd = Command::new(core_exe()?);
    cmd.stdin(Stdio::null());

    if cfg!(debug_assertions) {
        // dev: core logs print in the `pnpm tauri dev` terminal
        cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    } else {
        // release: silent, the core writes its own log file
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
    }

    cmd.spawn().map_err(|e| format!("could not launch clipx-core: {e}"))
}

async fn connect_with_retry(client: &mut Client, tries: u32) -> Result<(), String> {
    let mut last = String::new();
    for _ in 0..tries {
        let _ = client.shutdown().await; // reset any dead connection state
        match client.start().await {
            Ok(()) => return Ok(()),
            Err(e) => last = e,
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(last)
}

impl CoreSupervisor {
    pub fn new() -> Self {
        Self {
            child: StdMutex::new(None),
            paused: AtomicBool::new(false),
            status: StdMutex::new(CoreStatus::Stopped),
            label: StdMutex::new(None),
        }
    }

    pub fn attach_label(&self, item: MenuItem<Wry>) {
        *self.label.lock().unwrap() = Some(item);
    }

    pub fn status(&self) -> CoreStatus {
        *self.status.lock().unwrap()
    }

    fn set(&self, s: CoreStatus) {
        *self.status.lock().unwrap() = s;
        if let Some(item) = self.label.lock().unwrap().as_ref() {
            let _ = item.set_text(format!("Core: {:?}", s).to_lowercase());
        }
    }

    /// Attach to a running core, or launch one and attach.
    pub async fn ensure_running(&self, ipc: &Mutex<Client>) -> Result<(), String> {
        self.paused.store(false, Ordering::SeqCst);
        let mut c = ipc.lock().await;

        if c.is_connected() || connect_with_retry(&mut c, 1).await.is_ok() {
            self.set(CoreStatus::Running);
            return Ok(());
        }

        // reap a dead child from a previous launch
        if let Some(mut old) = self.child.lock().unwrap().take() {
            let _ = old.kill();
            let _ = old.wait();
        }
        *self.child.lock().unwrap() = Some(spawn_child()?);

        match connect_with_retry(&mut c, 20).await {
            // about 5 seconds
            Ok(()) => {
                self.set(CoreStatus::Running);
                Ok(())
            }
            Err(e) => {
                if let Some(mut ch) = self.child.lock().unwrap().take() {
                    let _ = ch.kill();
                    let _ = ch.wait();
                }
                self.set(CoreStatus::Failed);
                Err(e)
            }
        }
    }
    fn owns_child(&self) -> bool {
        self.child.lock().unwrap().is_some()
    }

    pub async fn stop(&self, ipc: &Mutex<Client>, include_attached: bool) {
        self.paused.store(true, Ordering::SeqCst);
        let owned = self.owns_child();
        {
            let mut c = ipc.lock().await;
            if c.is_connected() && (owned || include_attached) {
                let _ = c.request_shutdown().await;
            }
            let _ = c.shutdown().await;
        }
        let child = self.child.lock().unwrap().take();
        if let Some(mut child) = child {
            for _ in 0..12 {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        self.set(CoreStatus::Stopped);
    }

    pub async fn restart(&self, ipc: &Mutex<Client>) -> Result<(), String> {
        self.stop(ipc, true).await;
        self.ensure_running(ipc).await
    }

    /// Every 2s: if the core vanished and we didn't stop it, bring it back.
    pub fn spawn_watchdog(self: Arc<Self>, app: AppHandle) {
        tauri::async_runtime::spawn(async move {
            let mut failures = 0u32;
            let mut last = self.status();
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                if !self.paused.load(Ordering::SeqCst) {
                    let state = app.state::<crate::AppState>();
                    let alive = state.ipc.lock().await.is_connected();

                    if alive {
                        failures = 0;
                        self.set(CoreStatus::Running);
                    } else if self.owns_child() && failures < 5 {
                        // our own core died: bring it back
                        failures += 1;
                        self.set(CoreStatus::Restarting);
                        let _ = app.emit("core-status", self.status());
                        if self.ensure_running(&state.ipc).await.is_err() {
                            tokio::time::sleep(Duration::from_secs(1u64 << failures.min(4))).await;
                        }
                    } else if !self.owns_child() {
                        // an attached core went away: just report it, don't take over
                        self.set(CoreStatus::Stopped);
                    }
                }
                let now = self.status();
                if now != last {
                    last = now;
                    let _ = app.emit("core-status", now);
                }
            }
        });
    }
}

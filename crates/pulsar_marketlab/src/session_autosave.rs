//! Background JSON/USDA session autosave to survive shutdown and restart.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use pulsar_marketlab_core::schema_sidecar_usda;
use pulsar_marketlab_core::SCHEMA_SIDECAR_FILENAME;

use crate::canvas_compose::compose_pipeline_usda;
use crate::graph_compiler::{NodeConnection, VisualNode};

pub const SESSION_FORMAT_VERSION: u32 = 1;
pub const SESSION_JSON_FILENAME: &str = "session.json";
pub const SESSION_USDA_FILENAME: &str = "session.usda";

const AUTOSAVE_DEBOUNCE: Duration = Duration::from_millis(900);

/// Serializable workstation snapshot written beside the composed stage layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub version: u32,
    pub saved_at_unix_ms: u64,
    pub usd_document_path: Option<String>,
    pub nodes: Vec<VisualNode>,
    pub connections: Vec<NodeConnection>,
    pub selected_node_id: Option<usize>,
    pub pan_offset: [f32; 2],
    pub zoom_scale: f32,
    pub stage_share: f32,
    pub inspector_share: f32,
    pub active_workspace_tab: String,
}

impl SessionSnapshot {
    pub fn new(
        usd_document_path: Option<PathBuf>,
        nodes: Vec<VisualNode>,
        connections: Vec<NodeConnection>,
        selected_node_id: Option<usize>,
        pan_offset: [f32; 2],
        zoom_scale: f32,
        stage_share: f32,
        inspector_share: f32,
        active_workspace_tab: impl Into<String>,
    ) -> Self {
        Self {
            version: SESSION_FORMAT_VERSION,
            saved_at_unix_ms: unix_ms_now(),
            usd_document_path: usd_document_path
                .and_then(|path| path.to_str().map(str::to_string)),
            nodes,
            connections,
            selected_node_id,
            pan_offset,
            zoom_scale,
            stage_share,
            inspector_share,
            active_workspace_tab: active_workspace_tab.into(),
        }
    }
}

struct AutosavePayload {
    dir: PathBuf,
    snapshot: SessionSnapshot,
}

/// Debounced background writer for session JSON (USDA only on explicit flush).
pub struct SessionAutosaveDaemon {
    tx: Sender<AutosavePayload>,
    _worker: JoinHandle<()>,
}

impl SessionAutosaveDaemon {
    pub fn spawn(dir: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("marketlab-session-autosave".into())
            .spawn(move || autosave_worker(dir, rx))
            .expect("session autosave worker thread");
        Self {
            tx,
            _worker: worker,
        }
    }

    pub fn schedule(&self, dir: PathBuf, snapshot: SessionSnapshot) {
        let _ = self.tx.send(AutosavePayload { dir, snapshot });
    }

    pub fn flush_sync(&self, dir: &Path, snapshot: SessionSnapshot, usda: &str) -> io::Result<()> {
        write_session_files(dir, &snapshot, Some(usda))
    }
}

impl Drop for SessionAutosaveDaemon {
    fn drop(&mut self) {
        // Disconnect the sender so the worker drains pending jobs before exiting.
    }
}

fn autosave_worker(default_dir: PathBuf, rx: Receiver<AutosavePayload>) {
    while let Ok(first) = rx.recv() {
        let mut latest = first;
        loop {
            match rx.recv_timeout(AUTOSAVE_DEBOUNCE) {
                Ok(next) => latest = next,
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = write_session_payload(&default_dir, &latest);
                    return;
                }
            }
        }
        let _ = write_session_payload(&default_dir, &latest);
    }
}

/// Directory for autosaved session artifacts.
pub fn session_autosave_dir() -> PathBuf {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(local).join("MarketLab").join("session");
    }
    if let Ok(home) = std::env::var("USERPROFILE") {
        return PathBuf::from(home).join(".marketlab").join("session");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".marketlab").join("session");
    }
    PathBuf::from(".marketlab/session")
}

pub fn session_json_path(dir: &Path) -> PathBuf {
    dir.join(SESSION_JSON_FILENAME)
}

pub fn session_usda_path(dir: &Path) -> PathBuf {
    dir.join(SESSION_USDA_FILENAME)
}

pub fn compose_session_usda(nodes: &[VisualNode], connections: &[NodeConnection]) -> String {
    compose_pipeline_usda(nodes, connections)
}

fn write_session_payload(default_dir: &Path, payload: &AutosavePayload) -> io::Result<()> {
    let dir = if payload.dir.as_os_str().is_empty() {
        default_dir
    } else {
        &payload.dir
    };
    write_session_files(&dir, &payload.snapshot, None)
}

pub fn write_session_files(
    dir: &Path,
    snapshot: &SessionSnapshot,
    usda: Option<&str>,
) -> io::Result<()> {
    fs_create_dir_all(dir)?;
    fs_write_atomic(&session_json_path(dir), &serde_json::to_string_pretty(snapshot)?)?;
    if let Some(usda) = usda {
        fs_write_atomic(&session_usda_path(dir), usda)?;
        fs_write_atomic(&dir.join(SCHEMA_SIDECAR_FILENAME), &schema_sidecar_usda())?;
    }
    Ok(())
}

pub fn load_session_snapshot(dir: &Path) -> io::Result<Option<SessionSnapshot>> {
    let json_path = session_json_path(dir);
    if !json_path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(json_path)?;
    let snapshot: SessionSnapshot = serde_json::from_str(&text)?;
    if snapshot.version != SESSION_FORMAT_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unsupported session format version {} (expected {SESSION_FORMAT_VERSION})",
                snapshot.version
            ),
        ));
    }
    Ok(Some(snapshot))
}

pub fn session_autosave_exists(dir: &Path) -> bool {
    session_json_path(dir).is_file()
}

/// Shared handle used by the workspace to enqueue debounced autosaves.
#[derive(Clone)]
pub struct SessionAutosaveHandle {
    dir: PathBuf,
    daemon: Arc<SessionAutosaveDaemon>,
    last_revision: Arc<Mutex<u64>>,
}

impl SessionAutosaveHandle {
    pub fn new() -> Self {
        let dir = session_autosave_dir();
        Self {
            daemon: Arc::new(SessionAutosaveDaemon::spawn(dir.clone())),
            dir,
            last_revision: Arc::new(Mutex::new(0)),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn schedule(&self, revision: u64, snapshot: SessionSnapshot) {
        if let Ok(mut last) = self.last_revision.lock() {
            if revision <= *last {
                return;
            }
            *last = revision;
        }
        self.daemon.schedule(self.dir.clone(), snapshot);
    }

    pub fn flush_sync(&self, snapshot: SessionSnapshot, usda: &str) -> io::Result<()> {
        self.daemon.flush_sync(&self.dir, snapshot, usda)
    }
}

impl Default for SessionAutosaveHandle {
    fn default() -> Self {
        Self::new()
    }
}

fn unix_ms_now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn fs_create_dir_all(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)
}

fn fs_write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents)?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_compiler::{NodeGradeType, NodeType};

    fn sample_node(id: usize) -> VisualNode {
        VisualNode {
            id,
            stable_prim_leaf: crate::graph_compiler::test_visual_node_fields(id),
            name: format!("Asset_{id}"),
            node_type: NodeType::asset_adaptor(format!("/MarketLab/SPY{id}")),
            grade: NodeGradeType::Scalar,
            portfolio_allocation_id: None,
            dsl_formula: None,
            aov_outputs: Vec::new(),
            asset_source: None,
            x: 100.0 * id as f32,
            y: 80.0,
            collapsed: false,
            inputs: Vec::new(),
            outputs: vec!["out".into()],
        }
    }

    #[test]
    fn session_snapshot_round_trips_json() {
        let snapshot = SessionSnapshot::new(
            None,
            vec![sample_node(1), sample_node(2)],
            Vec::new(),
            Some(1),
            [12.0, 34.0],
            1.25,
            0.22,
            0.30,
            "param_inspector",
        );
        let json = serde_json::to_string_pretty(&snapshot).expect("serialize");
        let restored: SessionSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.nodes.len(), snapshot.nodes.len());
        assert_eq!(restored.zoom_scale, snapshot.zoom_scale);
    }

    #[test]
    fn write_and_load_session_files() {
        let dir = std::env::temp_dir().join(format!(
            "marketlab_session_test_{}",
            unix_ms_now()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let nodes = vec![sample_node(1)];
        let snapshot = SessionSnapshot::new(
            None,
            nodes.clone(),
            Vec::new(),
            None,
            [0.0, 0.0],
            1.0,
            0.22,
            0.30,
            "param_inspector",
        );
        let usda = compose_session_usda(&nodes, &[]);
        write_session_files(&dir, &snapshot, Some(&usda)).expect("write session");
        let loaded = load_session_snapshot(&dir)
            .expect("load session")
            .expect("snapshot present");
        assert_eq!(loaded.nodes.len(), 1);
        assert!(session_usda_path(&dir).is_file());
        let _ = std::fs::remove_dir_all(dir);
    }
}

//! `<userData>/state.json`: the Resume Point and volume. Writes are debounced and atomic.
//!
//! The file format is the Electron app's, byte for byte, so a Resume Point and volume saved by it
//! carry over.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Serialize, Serializer};
use serde_json::Value;
use slopify_spotify::ResumePoint;

const DEFAULT_VOLUME: f64 = 0.5;
const DEBOUNCE: Duration = Duration::from_millis(250);

/// `~/Library/Application Support/slopify/state.json`. Electron put `userData` there because
/// package.json `name` is `slopify`; the path stays so an existing file is picked up.
pub fn default_path() -> Option<PathBuf> {
    std::env::home_dir().map(|home| home.join("Library/Application Support/slopify/state.json"))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct Persisted {
    version: u8,
    resume: Option<ResumePoint>,
    #[serde(serialize_with = "js_number")]
    volume: f64,
}

impl Persisted {
    fn fresh() -> Self {
        Self {
            version: 1,
            resume: None,
            volume: DEFAULT_VOLUME,
        }
    }
}

// JSON.stringify writes 1 where serde_json writes 1.0. Both parse, but keep the file identical.
fn js_number<S: Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        s.serialize_i64(*v as i64)
    } else {
        s.serialize_f64(*v)
    }
}

fn parse(raw: &str) -> Option<Persisted> {
    let json: Value = serde_json::from_str(raw).ok()?;
    let obj = json.as_object()?;
    if obj.get("version").and_then(Value::as_f64) != Some(1.0) {
        return None;
    }
    let volume = obj
        .get("volume")
        .and_then(Value::as_f64)
        .filter(|v| (0.0..=1.0).contains(v))
        .unwrap_or(DEFAULT_VOLUME);
    let resume = obj
        .get("resume")
        .and_then(|r| serde_json::from_value(r.clone()).ok());
    Some(Persisted {
        version: 1,
        resume,
        volume,
    })
}

fn write_atomic(target: &Path, contents: &str) -> io::Result<()> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let mut tmp = target.as_os_str().to_owned();
    tmp.push(format!(".{}.{millis}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp);
    if let Some(dir) = target.parent() {
        fs::create_dir_all(dir)?;
    }
    let result = fs::write(&tmp, contents).and_then(|()| fs::rename(&tmp, target));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

struct Inner {
    data: Persisted,
    dirty: bool,
    /// When the debounced write is due; `None` when nothing is scheduled.
    due: Option<Instant>,
    closing: bool,
}

struct Shared {
    path: PathBuf,
    inner: Mutex<Inner>,
    wake: Condvar,
    /// Held while writing, so writes land in the order they were snapshotted.
    writing: Mutex<()>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Snapshots the data and writes it. Takes the `writing` lock before releasing `inner`, so a
    /// later snapshot can never be overtaken by an earlier one.
    fn write(&self, mut inner: MutexGuard<'_, Inner>) {
        inner.dirty = false;
        inner.due = None;
        let contents = match serde_json::to_string(&inner.data) {
            Ok(c) => c,
            Err(err) => {
                log::warn!("state file write failed {err}");
                return;
            }
        };
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        drop(inner);
        if let Err(err) = write_atomic(&self.path, &contents) {
            log::warn!("state file write failed {err}");
        }
    }

    /// The debounce timer: sleeps until a write is due, then writes.
    fn run(&self) {
        let mut inner = self.lock();
        loop {
            if inner.closing {
                return;
            }
            match inner.due {
                None => {
                    inner = self
                        .wake
                        .wait(inner)
                        .unwrap_or_else(PoisonError::into_inner)
                }
                Some(due) => {
                    let now = Instant::now();
                    if now < due {
                        inner = self
                            .wake
                            .wait_timeout(inner, due - now)
                            .unwrap_or_else(PoisonError::into_inner)
                            .0;
                        continue;
                    }
                    self.write(inner);
                    inner = self.lock();
                }
            }
        }
    }
}

pub struct StateFile {
    shared: Arc<Shared>,
    first_run: bool,
    debounce: Duration,
    timer: Option<JoinHandle<()>>,
}

impl StateFile {
    /// Loads `path` and starts the debounce thread. A missing file is a first run; an unreadable
    /// or unusable one is logged and replaced by defaults on the next save.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self::open_with_debounce(path, DEBOUNCE)
    }

    pub fn open_with_debounce(path: impl Into<PathBuf>, debounce: Duration) -> Self {
        let path = path.into();
        let (data, first_run) = load(&path);
        let shared = Arc::new(Shared {
            path,
            inner: Mutex::new(Inner {
                data,
                dirty: false,
                due: None,
                closing: false,
            }),
            wake: Condvar::new(),
            writing: Mutex::new(()),
        });
        let timer = {
            let shared = shared.clone();
            thread::Builder::new()
                .name("state-file".into())
                .spawn(move || shared.run())
                .expect("spawn state-file thread")
        };
        Self {
            shared,
            first_run,
            debounce,
            timer: Some(timer),
        }
    }

    pub fn first_run(&self) -> bool {
        self.first_run
    }

    pub fn resume_point(&self) -> Option<ResumePoint> {
        self.shared.lock().data.resume.clone()
    }

    pub fn save_resume_point(&self, p: ResumePoint) {
        let mut inner = self.shared.lock();
        inner.data.resume = Some(p);
        self.schedule(inner);
    }

    pub fn clear_resume_point(&self) {
        let mut inner = self.shared.lock();
        inner.data.resume = None;
        self.schedule(inner);
    }

    pub fn volume(&self) -> f64 {
        self.shared.lock().data.volume
    }

    pub fn save_volume(&self, v: f64) {
        let mut inner = self.shared.lock();
        inner.data.volume = v.clamp(0.0, 1.0);
        self.schedule(inner);
    }

    /// Writes anything pending now and waits for it. Call before quit.
    pub fn flush(&self) {
        let inner = self.shared.lock();
        if inner.dirty {
            self.shared.write(inner);
        } else {
            drop(inner);
            // Wait out a write the timer already started.
            drop(
                self.shared
                    .writing
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner),
            );
        }
    }

    fn schedule(&self, mut inner: MutexGuard<'_, Inner>) {
        inner.dirty = true;
        if inner.due.is_none() {
            inner.due = Some(Instant::now() + self.debounce);
            self.shared.wake.notify_one();
        }
    }
}

impl Drop for StateFile {
    /// Flushes, so a save just before the app drops its state is not lost with the timer.
    fn drop(&mut self) {
        self.flush();
        self.shared.lock().closing = true;
        self.shared.wake.notify_one();
        if let Some(timer) = self.timer.take() {
            let _ = timer.join();
        }
    }
}

fn load(path: &Path) -> (Persisted, bool) {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return (Persisted::fresh(), true),
        Err(err) => {
            log::warn!("state file unreadable {err}");
            return (Persisted::fresh(), false);
        }
    };
    match parse(&raw) {
        Some(p) => (p, false),
        None => {
            log::warn!("state file unusable, starting fresh");
            (Persisted::fresh(), false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use slopify_spotify::Source;
    use tempfile::TempDir;

    fn resume() -> ResumePoint {
        ResumePoint {
            source: Source::Playlist {
                id: "abc".into(),
                uri: "spotify:playlist:abc".into(),
                name: "Mix".into(),
                image_url: None,
                pasted: false,
            },
            track_uri: Some("spotify:track:xyz".into()),
            position_ms: 83210,
        }
    }

    struct Dir {
        dir: TempDir,
        file: PathBuf,
    }

    fn dir() -> Dir {
        let dir = tempfile::Builder::new()
            .prefix("slopify-state-")
            .tempdir()
            .unwrap();
        let file = dir.path().join("state.json");
        Dir { dir, file }
    }

    fn entries(d: &Dir) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(d.dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    fn fast(d: &Dir) -> StateFile {
        StateFile::open_with_debounce(&d.file, Duration::from_millis(5))
    }

    #[test]
    fn treats_a_missing_file_as_first_run_with_defaults() {
        let d = dir();
        let s = StateFile::open(&d.file);
        assert!(s.first_run());
        assert_eq!(s.resume_point(), None);
        assert_eq!(s.volume(), 0.5);
    }

    #[test]
    fn round_trips_the_resume_point_and_volume() {
        let d = dir();
        let a = fast(&d);
        a.save_resume_point(resume());
        a.save_volume(0.6);
        a.flush();

        let b = StateFile::open(&d.file);
        assert!(!b.first_run());
        assert_eq!(b.resume_point(), Some(resume()));
        assert_eq!(b.volume(), 0.6);
    }

    #[test]
    fn clears_the_resume_point() {
        let d = dir();
        let a = fast(&d);
        a.save_resume_point(resume());
        a.clear_resume_point();
        a.flush();

        let b = StateFile::open(&d.file);
        assert_eq!(b.resume_point(), None);
    }

    #[test]
    fn writes_atomically_the_target_exists_and_no_temp_file_is_left_behind() {
        let d = dir();
        let s = fast(&d);
        s.save_volume(0.3);
        s.save_resume_point(resume());
        s.flush();

        assert_eq!(entries(&d), ["state.json"]);
        let parsed: Value = serde_json::from_str(&fs::read_to_string(&d.file).unwrap()).unwrap();
        assert_eq!(
            parsed,
            json!({ "version": 1, "resume": serde_json::to_value(resume()).unwrap(), "volume": 0.3 })
        );
    }

    #[test]
    fn flushes_a_pending_write_before_the_debounce_elapses() {
        let d = dir();
        let s = StateFile::open_with_debounce(&d.file, Duration::from_secs(10));
        s.save_volume(0.9);
        s.flush();
        let parsed: Value = serde_json::from_str(&fs::read_to_string(&d.file).unwrap()).unwrap();
        assert_eq!(parsed["volume"], json!(0.9));
    }

    #[test]
    fn starts_fresh_on_an_unknown_version_not_as_first_run() {
        let d = dir();
        let body = json!({ "version": 2, "resume": serde_json::to_value(resume()).unwrap(), "volume": 0.1 });
        fs::write(&d.file, body.to_string()).unwrap();
        let s = StateFile::open(&d.file);
        assert!(!s.first_run());
        assert_eq!(s.resume_point(), None);
        assert_eq!(s.volume(), 0.5);
    }

    #[test]
    fn starts_fresh_on_an_unparsable_file_not_as_first_run() {
        let d = dir();
        fs::write(&d.file, "{not json").unwrap();
        let s = StateFile::open(&d.file);
        assert!(!s.first_run());
        assert_eq!(s.resume_point(), None);
        assert_eq!(s.volume(), 0.5);
    }

    #[test]
    fn flush_is_a_no_op_when_nothing_changed() {
        let d = dir();
        let s = StateFile::open(&d.file);
        s.flush();
        drop(s);
        assert!(entries(&d).is_empty());
    }

    #[test]
    fn reads_a_file_the_electron_app_wrote_and_writes_it_back_identically() {
        // JSON.stringify output from the TypeScript StateFile.
        let electron = r#"{"version":1,"resume":{"source":{"kind":"playlist","id":"abc","uri":"spotify:playlist:abc","name":"Mix","imageUrl":null,"pasted":false},"trackUri":"spotify:track:xyz","positionMs":83210},"volume":1}"#;
        let d = dir();
        fs::write(&d.file, electron).unwrap();
        let s = fast(&d);
        assert_eq!(s.resume_point(), Some(resume()));
        assert_eq!(s.volume(), 1.0);
        s.save_volume(1.0);
        s.flush();
        assert_eq!(fs::read_to_string(&d.file).unwrap(), electron);
    }

    #[test]
    fn keeps_liked_songs_and_a_missing_track() {
        let liked = r#"{"version":1,"resume":{"source":{"kind":"liked"},"trackUri":null,"positionMs":0},"volume":0.25}"#;
        let d = dir();
        fs::write(&d.file, liked).unwrap();
        let s = fast(&d);
        assert_eq!(
            s.resume_point(),
            Some(ResumePoint {
                source: Source::Liked,
                track_uri: None,
                position_ms: 0
            })
        );
        s.save_volume(0.25);
        s.flush();
        assert_eq!(fs::read_to_string(&d.file).unwrap(), liked);
    }

    #[test]
    fn falls_back_to_the_default_volume_when_it_is_out_of_range() {
        let d = dir();
        fs::write(&d.file, r#"{"version":1,"resume":null,"volume":3}"#).unwrap();
        assert_eq!(StateFile::open(&d.file).volume(), 0.5);
    }

    #[test]
    fn clamps_a_saved_volume() {
        let d = dir();
        let s = fast(&d);
        s.save_volume(1.5);
        assert_eq!(s.volume(), 1.0);
        s.save_volume(-1.0);
        assert_eq!(s.volume(), 0.0);
    }

    #[test]
    fn writes_on_its_own_once_the_debounce_elapses() {
        let d = dir();
        let s = fast(&d);
        s.save_volume(0.7);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !d.file.exists() {
            assert!(Instant::now() < deadline, "debounced write never happened");
            thread::sleep(Duration::from_millis(5));
        }
        // The rename is atomic, so once the file exists it is complete.
        let parsed: Value = serde_json::from_str(&fs::read_to_string(&d.file).unwrap()).unwrap();
        assert_eq!(parsed["volume"], json!(0.7));
    }

    #[test]
    fn default_path_is_electrons_user_data() {
        let path = default_path().unwrap();
        assert!(path.ends_with("Library/Application Support/slopify/state.json"));
    }
}

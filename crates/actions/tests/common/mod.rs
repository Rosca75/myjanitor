//! Shared harness: a fixture tree inside a `TempDir`, a test platform whose trash is
//! redirected into that tree (never the user's real trash), fakes for the platform
//! features Linux lacks, and a snapshot that compares paths, sizes, content hashes
//! and file mtimes.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use mj_actions::{ExecContext, ExecOptions, Guard, Journal, ProtectedPaths, Services, TrashRoute};
use mj_core::testutil::TempDir;
use mj_core::{
    Action, ActionItem, Attributes, CancelToken, Error, FileEntry, FsCompressAlgo, JobId, NullProgress, Result, RunId,
    RunKind, RunRecord, Settings, TargetKind, Timestamp, TrashTicket,
};
use mj_platform::action::trash::{restore_freedesktop, trash_with, TrashConfig};
use mj_platform::{ActionPlatform, CloudInfo, DirItem, NativePlatform, ScanPlatform, SyncRoot, VolumeInfo};

/// Real filesystem calls, with the trash redirected and failures injectable.
pub struct TestPlatform {
    pub native: NativePlatform,
    pub trash_cfg: TrashConfig,
    /// Fail the n-th (1-based) trash call.
    pub fail_trash_at: Option<usize>,
    pub trash_calls: AtomicUsize,
    /// Simulated cloud placeholders that are online-only.
    pub online: Mutex<BTreeSet<PathBuf>>,
    /// Simulated filesystem-compressed files.
    pub compressed: Mutex<BTreeSet<PathBuf>>,
    /// Simulate a copy-on-write filesystem (reflink = verified copy).
    pub fake_reflink: bool,
}

impl TestPlatform {
    pub fn new(home_trash: PathBuf) -> Self {
        TestPlatform {
            native: NativePlatform::new(),
            trash_cfg: TrashConfig { home_trash, uid: 0, volume_trash: false },
            fail_trash_at: None,
            trash_calls: AtomicUsize::new(0),
            online: Mutex::new(BTreeSet::new()),
            compressed: Mutex::new(BTreeSet::new()),
            fake_reflink: true,
        }
    }
}

impl ScanPlatform for TestPlatform {
    fn metadata(&self, path: &Path) -> Result<FileEntry> {
        let mut e = self.native.metadata(path)?;
        if self.online.lock().unwrap().contains(path) {
            e.attrs |= Attributes::ONLINE_ONLY;
        }
        Ok(e)
    }
    fn read_dir(&self, dir: &Path) -> Result<Vec<DirItem>> {
        self.native.read_dir(dir)
    }
    fn volumes(&self) -> Result<Vec<VolumeInfo>> {
        self.native.volumes()
    }
    fn volume_of(&self, path: &Path) -> Result<VolumeInfo> {
        self.native.volume_of(path)
    }
    fn fast_enumerate(&self, root: &Path, sink: &mut dyn FnMut(FileEntry), cancel: &CancelToken) -> Result<()> {
        self.native.fast_enumerate(root, sink, cancel)
    }
    fn cloud_info(&self, path: &Path) -> Result<CloudInfo> {
        self.native.cloud_info(path)
    }
}

impl ActionPlatform for TestPlatform {
    fn trash(&self, path: &Path) -> Result<TrashTicket> {
        let n = self.trash_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail_trash_at == Some(n) {
            return Err(Error::Io { path: Some(path.into()), kind: "other", message: "injected failure".into() });
        }
        trash_with(path, &self.trash_cfg, Timestamp::now())
    }
    fn restore_trashed(&self, ticket: &TrashTicket, original: &Path) -> Result<()> {
        restore_freedesktop(ticket, original)
    }
    fn hardlink(&self, existing: &Path, new_link: &Path) -> Result<()> {
        self.native.hardlink(existing, new_link)
    }
    fn reflink(&self, src: &Path, dst: &Path) -> Result<()> {
        if self.fake_reflink {
            mj_actions::fsops::copy_file_verified(src, dst).map(|_| ())
        } else {
            self.native.reflink(src, dst)
        }
    }
    fn dehydrate(&self, path: &Path) -> Result<()> {
        self.online.lock().unwrap().insert(path.to_path_buf());
        Ok(())
    }
    fn hydrate(&self, path: &Path) -> Result<()> {
        self.online.lock().unwrap().remove(path);
        Ok(())
    }
    fn sync_roots(&self) -> Result<Vec<SyncRoot>> {
        Err(Error::not_implemented("test"))
    }
    fn is_elevated(&self) -> bool {
        false
    }
    fn fs_compress(&self, path: &Path, _algo: FsCompressAlgo) -> Result<()> {
        self.compressed.lock().unwrap().insert(path.to_path_buf());
        Ok(())
    }
    fn fs_decompress(&self, path: &Path) -> Result<()> {
        self.compressed.lock().unwrap().remove(path);
        Ok(())
    }
    fn fs_compression(&self, path: &Path) -> Result<Option<FsCompressAlgo>> {
        Ok(self.compressed.lock().unwrap().contains(path).then_some(FsCompressAlgo::BtrfsZstd))
    }
}

/// One fixture: `root/` is the scanned tree (the only allowed root), the trash, the
/// quarantine, the journal and the scratch folder live beside it.
pub struct Fixture {
    pub t: TempDir,
    pub platform: TestPlatform,
    pub guard: Guard,
    pub journal: Journal,
    pub settings: Settings,
    pub cancel: CancelToken,
    pub qdir: PathBuf,
    pub tmp: PathBuf,
}

impl Fixture {
    pub fn new(label: &str) -> Fixture {
        let t = TempDir::new(label).unwrap();
        std::fs::create_dir_all(t.join("root")).unwrap();
        let platform = TestPlatform::new(t.join("xdg/Trash"));
        let guard = Guard::new(vec![t.join("root")], ProtectedPaths::empty().with_app_dirs(&[t.join("q")]));
        let journal = Journal::open(&t.join("journal")).unwrap();
        let (qdir, tmp) = (t.join("q"), t.join("tmp"));
        Fixture { t, platform, guard, journal, settings: Settings::default(), cancel: CancelToken::new(), qdir, tmp }
    }

    pub fn root(&self) -> PathBuf {
        self.t.join("root")
    }

    pub fn write(&self, rel: &str, content: &[u8]) -> PathBuf {
        let p = self.t.write(Path::new("root").join(rel), content).unwrap();
        // A fixed mtime in the past, so "mtime restored" is a real check.
        mj_core::testutil::set_mtime(&p, std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000))
            .unwrap();
        p
    }

    pub fn ctx<'a>(&'a self, run: RunId, services: Services<'a>) -> ExecContext<'a> {
        ExecContext {
            platform: &self.platform,
            guard: &self.guard,
            journal: &self.journal,
            services,
            settings: &self.settings,
            quarantine_dir: &self.qdir,
            temp_dir: &self.tmp,
            run_id: run,
            job_id: JobId(1),
            step_id: None,
            cancel: &self.cancel,
            progress: &NullProgress,
        }
    }

    pub fn new_run(&self, kind: RunKind) -> RunId {
        let id = self.journal.next_run_id(Timestamp::now());
        self.journal
            .put_run(&RunRecord {
                run_id: id,
                kind,
                title: "test".into(),
                started: Timestamp::now(),
                finished: None,
                steps: vec![],
                undo_of: None,
            })
            .unwrap();
        id
    }

    pub fn item(&self, path: &Path, action: Action) -> ActionItem {
        let md = self.platform.metadata(path).unwrap();
        let target = if md.is_dir() { TargetKind::Folder } else { TargetKind::File };
        ActionItem {
            path: path.to_path_buf(),
            target,
            action,
            link_source: None,
            root: self.root(),
            expected: if md.is_dir() { mj_core::FileStamp { size: 0, mtime: Timestamp(0) } } else { md.stamp() },
        }
    }

    /// Apply, then undo every undoable entry of the run as its own undo run.
    pub fn apply(&self, items: &[ActionItem], opts: &ExecOptions) -> (RunId, mj_core::JobOutcome) {
        let run = self.new_run(RunKind::Apply);
        let out = mj_actions::execute_with(&self.ctx(run, no_services()), items, opts).unwrap();
        (run, out)
    }

    pub fn undo_run(&self, run: RunId) -> mj_core::JobOutcome {
        let ids = mj_actions::undo::undoable_entries(&self.journal, run, Timestamp::now());
        let u = mj_actions::undo::start_undo_run(&self.journal, Some(run), "Undo", Timestamp::now()).unwrap();
        mj_actions::undo::undo(&self.ctx(u.run_id, no_services()), &ids).unwrap()
    }
}

pub fn no_services() -> Services<'static> {
    Services { optimisers: &[], archiver: None, tools: &[] }
}

pub fn opts() -> ExecOptions {
    ExecOptions { confirmed: false, trash_route: TrashRoute::Platform, audit_hash_limit: 1 << 30 }
}

pub fn confirmed() -> ExecOptions {
    ExecOptions { confirmed: true, ..opts() }
}

/// Sorted `(relative path, kind, size, sha256, mtime)`; kind: f file, d dir, l link (target).
/// Folder mtimes are not compared (adding and removing entries changes them by design).
pub fn snapshot(root: &Path) -> Vec<(String, char, u64, String, i64)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, char, u64, String, i64)>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
            let ft = e.file_type().unwrap();
            if ft.is_symlink() {
                out.push((rel, 'l', 0, std::fs::read_link(&p).unwrap().to_string_lossy().into_owned(), 0));
            } else if ft.is_dir() {
                out.push((rel, 'd', 0, String::new(), 0));
                walk(root, &p, out);
            } else {
                let data = std::fs::read(&p).unwrap();
                let md = std::fs::symlink_metadata(&p).unwrap();
                let mtime = Timestamp::from_system_time(md.modified().unwrap()).0;
                out.push((rel, 'f', data.len() as u64, mj_core::hash::to_hex(&mj_core::hash::sha256(&data)), mtime));
            }
        }
    }
    let mut v = Vec::new();
    walk(root, root, &mut v);
    v.sort();
    v
}

/// Latest journal states of a run, by entry id.
pub fn states(j: &Journal, run: RunId) -> BTreeMap<u64, mj_core::JournalState> {
    j.entries_of(run).into_iter().map(|e| (e.id.0, e.state)).collect()
}

/// A stored-only ZIP with the given entries (`flags` per entry).
pub fn zip(entries: &[(&str, &[u8], u16)]) -> Vec<u8> {
    let mut data = Vec::new();
    let mut cd = Vec::new();
    for (name, content, flags) in entries {
        let crc = mj_formats::crc32::crc32(content);
        let off = data.len() as u32;
        data.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        data.extend_from_slice(&20u16.to_le_bytes());
        data.extend_from_slice(&flags.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&[0, 0, 0x21, 0]);
        data.extend_from_slice(&crc.to_le_bytes());
        data.extend_from_slice(&(content.len() as u32).to_le_bytes());
        data.extend_from_slice(&(content.len() as u32).to_le_bytes());
        data.extend_from_slice(&(name.len() as u16).to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(name.as_bytes());
        data.extend_from_slice(content);
        cd.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        cd.extend_from_slice(&20u16.to_le_bytes());
        cd.extend_from_slice(&20u16.to_le_bytes());
        cd.extend_from_slice(&flags.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&[0, 0, 0x21, 0]);
        cd.extend_from_slice(&crc.to_le_bytes());
        cd.extend_from_slice(&(content.len() as u32).to_le_bytes());
        cd.extend_from_slice(&(content.len() as u32).to_le_bytes());
        cd.extend_from_slice(&(name.len() as u16).to_le_bytes());
        cd.extend_from_slice(&[0; 12]);
        cd.extend_from_slice(&off.to_le_bytes());
        cd.extend_from_slice(name.as_bytes());
    }
    let cd_off = data.len() as u32;
    let n = entries.len() as u16;
    data.extend_from_slice(&cd);
    data.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    data.extend_from_slice(&[0; 4]);
    data.extend_from_slice(&n.to_le_bytes());
    data.extend_from_slice(&n.to_le_bytes());
    data.extend_from_slice(&(cd.len() as u32).to_le_bytes());
    data.extend_from_slice(&cd_off.to_le_bytes());
    data.extend_from_slice(&[0, 0]);
    data
}

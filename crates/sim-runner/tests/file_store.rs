#[path = "../src/file_store.rs"]
mod file_store;

use std::fs::{self, File};
use std::io::{self, Write};

use file_store::{FileStore, Operation, StoreError, StorePhase};
use sim_core::{RuleSet, Scenario, Simulation, Tick};
use tempfile::tempdir;

fn checkpoints() -> [Vec<u8>; 2] {
    [10, 13].map(|tick| {
        Simulation::new_game(Scenario::empty(Tick::new(tick)), RuleSet::default(), 42)
            .unwrap()
            .save()
            .unwrap()
    })
}

fn assert_checkpoint(actual: &[u8], expected: &[u8]) {
    assert_eq!(actual, expected);
    assert!(Simulation::load(actual).is_ok());
}

#[test]
fn atomic_save_keeps_previous_file_until_replace() {
    let [old, new] = checkpoints();
    for failure in [StorePhase::Write, StorePhase::SyncFile, StorePhase::Replace] {
        let dir = tempdir().unwrap();
        let path = dir.path().join("checkpoint.json");
        let other = dir.path().join("other.json");
        fs::write(&path, &old).unwrap();
        fs::write(&other, b"other file").unwrap();

        let result = FileStore.write_with(&path, &new, |operation| {
            assert_checkpoint(&FileStore.read(&path).unwrap(), &old);
            match operation {
                Operation::Write(file, bytes) if failure == StorePhase::Write => {
                    file.write_all(&bytes[..3]).unwrap();
                    Err(io::Error::other("injected error after partial write"))
                }
                Operation::SyncFile(_) if failure == StorePhase::SyncFile => {
                    Err(io::Error::other("injected file sync error"))
                }
                Operation::Replace(_, _) if failure == StorePhase::Replace => {
                    Err(io::Error::other("injected replace error"))
                }
                operation => operation.perform(),
            }
        });

        assert!(
            matches!(result, Err(StoreError::Io { phase, .. }) if phase == failure),
            "{result:?}"
        );
        assert_checkpoint(&FileStore.read(&path).unwrap(), &old);
        assert_eq!(fs::read(&other).unwrap(), b"other file");
        FileStore.write(&path, &new).unwrap();
        assert_checkpoint(&FileStore.read(&path).unwrap(), &new);
    }
}

struct ShortWriter<'a>(&'a mut dyn Write);

impl Write for ShortWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(&bytes[..bytes.len().min(3)])
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

#[test]
fn atomic_save_retries_short_writes() {
    let [old, new] = checkpoints();
    let dir = tempdir().unwrap();
    let path = dir.path().join("checkpoint.json");
    fs::write(&path, &old).unwrap();

    FileStore
        .write_with(&path, &new, |operation| match operation {
            Operation::Write(file, bytes) => {
                Operation::Write(&mut ShortWriter(file), bytes).perform()
            }
            operation => operation.perform(),
        })
        .unwrap();
    assert_checkpoint(&FileStore.read(&path).unwrap(), &new);
}

#[test]
fn directory_sync_failure_leaves_the_new_checkpoint_readable() {
    let [old, new] = checkpoints();
    let dir = tempdir().unwrap();
    let path = dir.path().join("checkpoint.json");
    fs::write(&path, &old).unwrap();

    let result = FileStore.write_with(&path, &new, |operation| match operation {
        Operation::SyncDirectory(_) => Err(io::Error::other("injected directory sync error")),
        operation => operation.perform(),
    });

    assert!(
        matches!(result, Err(StoreError::DurabilityUnconfirmed { .. })),
        "{result:?}"
    );
    assert_checkpoint(&FileStore.read(&path).unwrap(), &new);
    FileStore.write(&path, &old).unwrap();
    assert_checkpoint(&FileStore.read(&path).unwrap(), &old);
}

#[test]
fn oversized_sparse_file_is_rejected() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("oversized.json");
    File::create(&path)
        .unwrap()
        .set_len(128 * 1024 * 1024 + 1)
        .unwrap();

    let result = FileStore.read(&path);
    assert!(
        matches!(
            result,
            Err(StoreError::TooLarge {
                limit: 134_217_728,
                ..
            })
        ),
        "expected the file to exceed the byte limit"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn filename_can_use_all_255_bytes() {
    let [_, new] = checkpoints();
    let dir = tempdir().unwrap();
    let path = dir.path().join("a".repeat(255));
    FileStore.write(&path, &new).unwrap();
    assert_checkpoint(&FileStore.read(&path).unwrap(), &new);
}

#[cfg(unix)]
#[test]
fn writers_using_a_parent_alias_are_serialized() {
    use std::os::unix::fs::symlink;
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::thread;
    use std::time::Duration;

    let [old, new] = checkpoints();
    let new = new.as_slice();

    let dir = tempdir().unwrap();
    let real = dir.path().join("real");
    fs::create_dir(&real).unwrap();
    let alias = dir.path().join("alias");
    symlink(&real, &alias).unwrap();
    let path = real.join("checkpoint.json");
    let aliased_path = alias.join("checkpoint.json");
    fs::write(&path, &old).unwrap();

    thread::scope(|scope| {
        let (first_ready, first_wait) = mpsc::channel();
        let (first_resume, first_paused) = mpsc::channel();
        let (second_start, second_started) = mpsc::channel();
        let (second_ready, second_wait) = mpsc::channel();
        let first = scope.spawn(move || {
            FileStore.write_with(&path, new, |operation| {
                if let Operation::SyncDirectory(_) = operation {
                    first_ready.send(()).unwrap();
                    first_paused.recv().unwrap();
                }
                operation.perform()
            })
        });
        first_wait.recv_timeout(Duration::from_secs(5)).unwrap();
        let alias = &aliased_path;
        let old = old.as_slice();
        let second = scope.spawn(move || {
            second_start.send(()).unwrap();
            FileStore.write_with(alias, old, |operation| {
                if let Operation::Write(_, _) = operation {
                    second_ready.send(FileStore.read(alias).unwrap()).unwrap();
                }
                operation.perform()
            })
        });
        let started = second_started.recv_timeout(Duration::from_secs(5));
        let blocked = second_wait.recv_timeout(Duration::from_millis(50));
        let visible = FileStore.read(alias);
        first_resume.send(()).unwrap();
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
        assert_eq!(started, Ok(()));
        assert_eq!(blocked, Err(RecvTimeoutError::Timeout));
        assert_checkpoint(&visible.unwrap(), new);
        assert_checkpoint(&second_wait.recv().unwrap(), new);
        assert_checkpoint(&FileStore.read(alias).unwrap(), old);
    });
}

#[test]
fn directory_syntax_does_not_replace_a_regular_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("checkpoint.json");
    fs::write(&path, b"untouched").unwrap();

    assert!(FileStore.write(&path.join("."), b"replacement").is_err());
    assert_eq!(fs::read(&path).unwrap(), b"untouched");
}

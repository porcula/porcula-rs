//check modification time of single file

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

pub struct MtimeChecker {
    path: Arc<Path>,
    mtime: u64,
}

fn get_mtime(path: &Path) -> u64 {
    let mut res = 0;
    if let Ok(m) = fs::metadata(path) {
        if let Ok(t) = m.modified() {
            if let Ok(d) = t.duration_since(UNIX_EPOCH) {
                res = d.as_secs();
            }
        }
    }
    res
}

impl MtimeChecker {
    pub fn new(path: &Path) -> MtimeChecker {
        MtimeChecker {
            path: Arc::from(path),
            mtime: get_mtime(path),
        }
    }
    pub fn is_modified(&mut self) -> bool {
        let prev = self.mtime;
        self.mtime = get_mtime(&self.path);
        self.mtime != prev
    }
}

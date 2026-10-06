use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

/// Serializes all tests that call `set_current_dir`. Shared across modules so
/// tests in config.rs, feature.rs, and release.rs don't race each other.
pub static DIR_LOCK: Mutex<()> = Mutex::new(());

/// A temporary git repository that cleans itself up on drop.
///
/// Setup: `main` and `develop` branches, one committed `.gitflow` with
/// default config, clean working tree.
pub struct TestRepo {
    pub dir: PathBuf,
}

impl TestRepo {
    pub fn new(name: &str) -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir = std::env::temp_dir().join(format!("gust_test_{}_{}", name, nanos));
        std::fs::create_dir_all(&dir).unwrap();

        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .expect("git command failed in TestRepo::new")
        };

        git(&["init"]);
        git(&["config", "user.email", "test@gust.test"]);
        git(&["config", "user.name", "Gust Test"]);
        git(&["commit", "--allow-empty", "-m", "Initial commit"]);
        // Rename current branch to `main` (handles both master and main defaults)
        Command::new("git")
            .args(["branch", "-m", "main"])
            .current_dir(&dir)
            .output()
            .ok();
        git(&["checkout", "-b", "develop"]);

        let config_toml = concat!(
            "[branch]\n",
            "main = \"main\"\n",
            "develop = \"develop\"\n",
            "\n[prefix]\n",
            "feature = \"feature/\"\n",
            "release = \"release/\"\n",
            "hotfix = \"hotfix/\"\n",
            "tag = \"\"\n",
            "\n[pr]\n",
            "enabled = false\n",
        );
        std::fs::write(dir.join(".gitflow"), config_toml).unwrap();
        git(&["add", ".gitflow"]);
        git(&["commit", "-m", "Add gust config"]);

        TestRepo { dir }
    }

    /// Run a git command inside the repo directory.
    pub fn git(&self, args: &[&str]) {
        Command::new("git")
            .args(args)
            .current_dir(&self.dir)
            .output()
            .expect("git command failed in TestRepo::git");
    }

    /// Create an empty commit (useful for making merges non-trivial).
    pub fn empty_commit(&self, message: &str) {
        self.git(&["commit", "--allow-empty", "-m", message]);
    }
}

impl Drop for TestRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// RAII guard that saves the current directory and restores it on drop.
pub struct OrigDir(PathBuf);

impl OrigDir {
    pub fn save_and_switch(new_dir: &Path) -> Self {
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(new_dir).unwrap();
        OrigDir(orig)
    }
}

impl Drop for OrigDir {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.0);
    }
}

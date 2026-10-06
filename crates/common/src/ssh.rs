//! SSH helpers: generating `~/.ssh/config` and connecting to a host.
//! Ported from `net-switch.sh` (config generation) and `bastion.sh` (`ssh $1`).

use crate::error::Result;
use crate::paths;
use crate::proc;
use std::fs;
use std::os::unix::fs::PermissionsExt;

/// Cluster host -> bastion port map, matching `net-switch.sh`.
const CLUSTER_PORTS: &[(&str, u16)] = &[
    ("core", 3434),
    ("portal", 3435),
    ("services", 3436),
    ("mart", 3437),
    ("logs", 3438),
    ("reports", 3439),
    ("work", 3440),
    ("customers", 3441),
    ("document", 3442),
];

/// Render the `~/.ssh/config` contents for a given bastion IP and ssh key name.
pub fn render_config(bastion_ip: &str, mykey: &str) -> String {
    let mut out = String::new();
    out.push_str(
        "Host vhpdev\n\tHostName 192.168.0.120\n\tUser devteam\n\tPort 22\n\tIdentityFile ~/.ssh/devteam.pub\n\n",
    );
    out.push_str(&format!(
        "Host bastion\n\tHostName {bastion_ip}\n\tUser ec2-user\n\tPort 22\n\tIdentityFile ~/.ssh/{mykey}\n\n"
    ));
    for (host, port) in CLUSTER_PORTS {
        out.push_str(&format!(
            "Host {host}\n\tHostName {bastion_ip}\n\tUser ec2-user\n\tPort {port}\n\tIdentityFile ~/.ssh/{mykey}\n\n"
        ));
    }
    out
}

/// Write `~/.ssh/config` (mode 600) for the given bastion IP and key name.
/// When `dry_run`, prints the rendered config instead of writing it.
pub fn write_config(bastion_ip: &str, mykey: &str, dry_run: bool) -> Result<()> {
    let config = render_config(bastion_ip, mykey);
    let path = paths::home()?.join(".ssh/config");
    println!("  write {} ({} bytes)", path.display(), config.len());
    if dry_run {
        println!("{config}");
        return Ok(());
    }
    fs::write(&path, config)?;
    let mut perms = fs::metadata(&path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(&path, perms)?;
    Ok(())
}

/// Open an interactive ssh session to `host` (the `bastion.sh` behavior).
pub fn connect(host: &str) -> Result<()> {
    proc::run("ssh", &[host], false)
}

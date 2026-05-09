use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Windows,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxFamily {
    DebianLike,
    FedoraLike,
    ArchLike,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    AptGet,
    Dnf,
    Pacman,
    Winget,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct OsContext {
    pub platform: Platform,
    pub pretty_name: Option<String>,
    pub linux_family: Option<LinuxFamily>,
    pub package_manager: PackageManager,
    pub os_release_id: Option<String>,
    pub os_release_id_like: Vec<String>,
}

impl OsContext {
    pub fn detect() -> Self {
        if cfg!(target_os = "linux") {
            detect_linux()
        } else if cfg!(target_os = "windows") {
            detect_windows()
        } else {
            OsContext {
                platform: Platform::Unknown,
                pretty_name: None,
                linux_family: None,
                package_manager: PackageManager::Unknown,
                os_release_id: None,
                os_release_id_like: vec![],
            }
        }
    }
}

fn detect_linux() -> OsContext {
    let (id, id_like, pretty) = read_os_release_fields();
    let family = Some(map_linux_family(id.as_deref(), &id_like));
    let pm = if which::which("apt-get").is_ok() {
        PackageManager::AptGet
    } else if which::which("dnf").is_ok() {
        PackageManager::Dnf
    } else if which::which("pacman").is_ok() {
        PackageManager::Pacman
    } else {
        PackageManager::Unknown
    };

    OsContext {
        platform: Platform::Linux,
        pretty_name: pretty,
        linux_family: family,
        package_manager: pm,
        os_release_id: id,
        os_release_id_like: id_like,
    }
}

fn detect_windows() -> OsContext {
    let pm = if which::which("winget").is_ok() {
        PackageManager::Winget
    } else {
        PackageManager::Unknown
    };
    OsContext {
        platform: Platform::Windows,
        pretty_name: Some("Windows".to_string()),
        linux_family: None,
        package_manager: pm,
        os_release_id: None,
        os_release_id_like: vec![],
    }
}

fn map_linux_family(id: Option<&str>, id_like: &[String]) -> LinuxFamily {
    let id = id.unwrap_or("").to_lowercase();
    let likes: Vec<String> = id_like.iter().map(|s| s.to_lowercase()).collect();
    let like_contains = |needle: &str| likes.iter().any(|v| v == needle);

    if id == "ubuntu" || id == "debian" || like_contains("debian") {
        return LinuxFamily::DebianLike;
    }
    if id == "fedora" || id == "rhel" || id == "centos" || like_contains("fedora") || like_contains("rhel") {
        return LinuxFamily::FedoraLike;
    }
    if id == "arch" || id == "manjaro" || like_contains("arch") {
        return LinuxFamily::ArchLike;
    }
    LinuxFamily::Unknown
}

fn read_os_release_fields() -> (Option<String>, Vec<String>, Option<String>) {
    // Prefer /etc/os-release, fall back to /usr/lib/os-release (per os-release spec).
    let etc = Path::new("/etc/os-release");
    let usr = Path::new("/usr/lib/os-release");

    let path = if etc.exists() { etc } else { usr };
    if !path.exists() {
        return (None, vec![], None);
    }

    let file = std::fs::File::open(path);
    let r = match file {
        Ok(f) => etc_os_release::OsRelease::from_reader(f).ok(),
        Err(_) => None,
    };
    match r {
        Some(r) => {
            let id = {
                let v = r.id().to_string();
                if v.is_empty() { None } else { Some(v) }
            };
            let id_like: Vec<String> = r
                .id_like()
                .map(|it| it.map(|v| v.to_string()).collect())
                .unwrap_or_else(Vec::new);
            let pretty = {
                let v = r.pretty_name().to_string();
                if v.is_empty() { None } else { Some(v) }
            };
            (id, id_like, pretty)
        }
        None => (None, vec![], None),
    }
}


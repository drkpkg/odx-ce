use crate::os_context::{LinuxFamily, OsContext, PackageManager, Platform};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    BuildTools,
    PythonDev,
    PythonPip,
    LibPQDev,
    LibXML2Dev,
    LibXSLTDev,
    LibJPEGDev,
    ZlibDev,
    OpenSSLDev,
    LibFFIDev,
    LDAPDev,
    SASLDev,
}

pub fn odoo_full_requirements() -> Vec<Requirement> {
    vec![
        Requirement::BuildTools,
        Requirement::PythonDev,
        Requirement::PythonPip,
        Requirement::LibPQDev,
        Requirement::LibXML2Dev,
        Requirement::LibXSLTDev,
        Requirement::LibJPEGDev,
        Requirement::ZlibDev,
        Requirement::OpenSSLDev,
        Requirement::LibFFIDev,
        Requirement::LDAPDev,
        Requirement::SASLDev,
    ]
}

pub struct InstallGuide {
    pub detected: String,
    pub command: Option<String>,
    pub notes: Vec<String>,
}

pub fn build_install_guide(ctx: &OsContext) -> InstallGuide {
    match ctx.platform {
        Platform::Linux => build_linux_guide(ctx),
        Platform::Windows => build_windows_guide(ctx),
        Platform::Unknown => InstallGuide {
            detected: "Unknown".to_string(),
            command: None,
            notes: vec![
                "Could not detect OS family. Install build tools, Python dev headers and common libraries (libxml2, libxslt, libjpeg, zlib, openssl, libffi, libpq) using your system package manager.".to_string(),
            ],
        },
    }
}

fn build_linux_guide(ctx: &OsContext) -> InstallGuide {
    let pretty = ctx
        .pretty_name
        .clone()
        .or_else(|| ctx.os_release_id.clone())
        .unwrap_or_else(|| "Linux".to_string());

    let family = ctx.linux_family.unwrap_or(LinuxFamily::Unknown);
    let reqs = odoo_full_requirements();
    let pkgs = linux_packages_for(ctx.package_manager, family, &reqs);

    let command = match ctx.package_manager {
        PackageManager::AptGet => Some(format!(
            "sudo apt-get update && sudo apt-get install -y {}",
            pkgs.join(" ")
        )),
        PackageManager::Dnf => Some(format!("sudo dnf install -y {}", pkgs.join(" "))),
        PackageManager::Pacman => Some(format!("sudo pacman -S --needed {}", pkgs.join(" "))),
        _ => None,
    };

    let mut notes = vec![
        "Package names may vary slightly by distro/version.".to_string(),
        "After system deps: run `odx install` to install Python requirements into the project venv.".to_string(),
    ];

    if family == LinuxFamily::Unknown {
        notes.push(
            "Could not determine Linux family from os-release; command may be incomplete."
                .to_string(),
        );
    }
    if command.is_none() {
        notes
            .push("Could not detect a supported package manager (apt-get/dnf/pacman).".to_string());
    }

    InstallGuide {
        detected: pretty,
        command,
        notes,
    }
}

fn build_windows_guide(ctx: &OsContext) -> InstallGuide {
    let detected = ctx
        .pretty_name
        .clone()
        .unwrap_or_else(|| "Windows".to_string());
    let mut notes = vec![
        "Recommended for Odoo development: install WSL2 (Ubuntu/Debian/Fedora) and run odx inside WSL.".to_string(),
        "Native Windows support is best-effort; many Python wheels build more reliably on Linux.".to_string(),
    ];

    let command = match ctx.package_manager {
        PackageManager::Winget => {
            notes.push("If you need to compile Python packages: install Visual Studio Build Tools (C++ workload).".to_string());
            Some(
                "winget install -e --id Git.Git && winget install -e --id Python.Python.3.11"
                    .to_string(),
            )
        }
        _ => {
            notes.push(
                "winget not found. Install Git and Python manually, or use WSL2.".to_string(),
            );
            None
        }
    };

    InstallGuide {
        detected,
        command,
        notes,
    }
}

pub(crate) fn linux_packages_for(
    pm: PackageManager,
    family: LinuxFamily,
    reqs: &[Requirement],
) -> Vec<String> {
    reqs.iter()
        .flat_map(|r| linux_pkg_names(pm, family, *r))
        .collect()
}

pub(crate) fn linux_pkg_names(
    pm: PackageManager,
    family: LinuxFamily,
    r: Requirement,
) -> Vec<String> {
    use LinuxFamily::*;
    use Requirement::*;

    let s = match family {
        DebianLike => match r {
            BuildTools => "build-essential",
            PythonDev => "python3-dev",
            PythonPip => "python3-pip",
            LibPQDev => "libpq-dev",
            LibXML2Dev => "libxml2-dev",
            LibXSLTDev => "libxslt1-dev",
            LibJPEGDev => "libjpeg-dev",
            ZlibDev => "zlib1g-dev",
            OpenSSLDev => "libssl-dev",
            LibFFIDev => "libffi-dev",
            LDAPDev => "libldap2-dev",
            SASLDev => "libsasl2-dev",
        },
        FedoraLike => match r {
            BuildTools => {
                // Prefer explicit packages for predictability vs group installs
                "gcc gcc-c++ make"
            }
            PythonDev => "python3-devel",
            PythonPip => "python3-pip",
            LibPQDev => "postgresql-devel",
            LibXML2Dev => "libxml2-devel",
            LibXSLTDev => "libxslt-devel",
            LibJPEGDev => "libjpeg-turbo-devel",
            ZlibDev => "zlib-devel",
            OpenSSLDev => "openssl-devel",
            LibFFIDev => "libffi-devel",
            LDAPDev => "openldap-devel",
            SASLDev => "cyrus-sasl-devel",
        },
        ArchLike => match r {
            BuildTools => "base-devel",
            PythonDev => "python",
            PythonPip => "python-pip",
            LibPQDev => "postgresql",
            LibXML2Dev => "libxml2",
            LibXSLTDev => "libxslt",
            LibJPEGDev => "libjpeg-turbo",
            ZlibDev => "zlib",
            OpenSSLDev => "openssl",
            LibFFIDev => "libffi",
            LDAPDev => "libldap",
            SASLDev => "cyrus-sasl",
        },
        Unknown => return vec![],
    };

    match pm {
        PackageManager::Dnf => s.split_whitespace().map(|v| v.to_string()).collect(),
        PackageManager::AptGet => vec![s.to_string()],
        PackageManager::Pacman => vec![s.to_string()],
        _ => vec![s.to_string()],
    }
}

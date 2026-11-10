use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AppKind {
    System,
    Flatpak,
    Snap,
    AppImage,
    /// A desktop entry no package manager owns (hand-made launchers, unpacked tarballs).
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    User,
    System,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct App {
    /// `<kind>:<native id>`, stable across scans.
    pub id: String,
    pub name: String,
    pub summary: String,
    pub kind: AppKind,
    pub scope: Scope,
    /// Package name, Flatpak app ID, snap name or AppImage file name.
    pub package: String,
    pub version: String,
    /// `data:` URL, so the webview needs no file access.
    pub icon: Option<String>,
    pub desktop_file: String,
    /// The AppImage file, or the main binary when known.
    pub location: Option<String>,
    /// What the package itself takes on disk, in bytes.
    pub install_size: u64,
    /// Why Deep purge is refused, when it is.
    pub protected: Option<String>,
    /// Something wrong with the install, such as a launcher whose file is gone.
    pub issue: Option<String>,
    /// Best estimate of when the app was last used, Unix seconds, and where it comes from.
    pub last_used: Option<u64>,
    pub last_used_source: Option<String>,
    #[serde(skip)]
    pub identifiers: Vec<String>,
    #[serde(skip)]
    pub desktop_id: String,
    #[serde(skip)]
    pub exec_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Config,
    Data,
    Cache,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Residue {
    pub path: String,
    pub size: u64,
    pub category: Category,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppFootprint {
    pub id: String,
    pub config: u64,
    pub data: u64,
    pub cache: u64,
    pub residue: Vec<Residue>,
    /// Newest change inside the app's own folders, Unix seconds: apps write there when they run.
    pub last_activity: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessUsage {
    pub id: String,
    pub memory: u64,
    pub processes: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    pub distro: String,
    pub family: String,
    pub backend: Option<String>,
    pub desktop: String,
    pub home: String,
    pub has_flatpak: bool,
    pub has_snap: bool,
    pub has_docker: bool,
    pub kernel: String,
}

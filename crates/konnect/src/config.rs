use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Path to the kicad-cli binary
    #[serde(default = "default_kicad_cli")]
    pub kicad_cli: String,

    /// Path to the KiCAD binary (for launching the UI)
    #[serde(default = "default_kicad_binary")]
    pub kicad_binary: String,

    /// Default project directory
    #[serde(default)]
    pub project_dir: Option<PathBuf>,

    /// KiCAD IPC socket path (NNG). Auto-detected from KICAD_API_SOCKET env var if empty.
    #[serde(default = "default_ipc_address")]
    #[serde(alias = "ipc_socket_path")]
    pub ipc_address: String,

    /// MCP server transport mode
    #[serde(default)]
    pub transport: TransportMode,

    /// HTTP server bind address (used when transport includes HTTP)
    #[serde(default = "default_http_address")]
    pub http_address: String,

    /// JLCPCB database cache path
    #[serde(default)]
    pub jlcpcb_db_path: Option<PathBuf>,

    /// Log level (error, warn, info, debug, trace)
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TransportMode {
    #[default]
    Stdio,
    Http,
    Both,
}

fn default_kicad_cli() -> String {
    if let Some(path) = nonempty_env("KICAD_CLI") {
        return path;
    }
    let candidates: &[&str] = if cfg!(target_os = "windows") {
        &[
            r"C:\Program Files\KiCad\10.0\bin\kicad-cli.exe",
            r"C:\Program Files\KiCad\9.0\bin\kicad-cli.exe",
        ]
    } else if cfg!(target_os = "macos") {
        &[
            "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli",
            "/usr/local/bin/kicad-cli",
        ]
    } else {
        &[
            "/usr/bin/kicad-cli",
            "/usr/local/bin/kicad-cli",
            "/snap/bin/kicad-cli",
            "/snap/kicad/current/usr/bin/kicad-cli",
        ]
    };
    candidates
        .iter()
        .find(|candidate| std::path::Path::new(candidate).is_file())
        .map(|candidate| (*candidate).to_string())
        .unwrap_or_else(|| {
            if cfg!(target_os = "windows") {
                "kicad-cli.exe".to_string()
            } else {
                "kicad-cli".to_string()
            }
        })
}

fn default_kicad_binary() -> String {
    if let Some(path) = nonempty_env("KICAD_BINARY") {
        return path;
    }
    let candidates: &[&str] = if cfg!(target_os = "windows") {
        &[
            r"C:\Program Files\KiCad\10.0\bin\kicad.exe",
            r"C:\Program Files\KiCad\9.0\bin\kicad.exe",
        ]
    } else if cfg!(target_os = "macos") {
        &["/Applications/KiCad/KiCad.app/Contents/MacOS/kicad"]
    } else {
        &["/usr/bin/kicad", "/usr/local/bin/kicad", "/snap/bin/kicad"]
    };
    candidates
        .iter()
        .find(|candidate| std::path::Path::new(candidate).is_file())
        .map(|candidate| (*candidate).to_string())
        .unwrap_or_else(|| {
            if cfg!(target_os = "windows") {
                "kicad.exe".to_string()
            } else {
                "kicad".to_string()
            }
        })
}

fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn default_ipc_address() -> String {
    std::env::var("KICAD_API_SOCKET").unwrap_or_else(|_| {
        read_ipc_discovery()
            .filter(|discovery| discovery_address_is_usable(&discovery.socket))
            .map(|discovery| discovery.socket)
            .unwrap_or_default()
    })
}

fn default_http_address() -> String {
    "127.0.0.1:3000".to_string()
}

fn default_log_level() -> String {
    "info".to_string()
}

impl Config {
    /// Load config from the default search path.
    pub fn load() -> Result<Self> {
        let config_paths = [
            PathBuf::from("konnect.toml"),
            PathBuf::from("settings.json"),
            dirs_config_path(),
        ];

        for path in &config_paths {
            if path.exists() {
                return Self::load_from(path);
            }
        }

        // No config file found; use defaults
        Ok(Config::default())
    }

    /// Load config from a specific file path. Auto-detects JSON vs TOML by extension.
    pub fn load_from(path: &std::path::Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let mut config = match ext {
            "json" => {
                let config: Config = serde_json::from_str(&content)?;
                config
            }
            _ => {
                // Default: TOML
                let config: Config = toml::from_str(&content)?;
                config
            }
        };
        // The KiCad settings dialog persists an explicitly empty socket field.
        // Serde defaults only apply to missing fields, so resolve runtime
        // discovery here as well as in Config::default().
        if config.ipc_address.is_empty() {
            config.ipc_address = default_ipc_address();
        }
        if config.kicad_cli.is_empty() {
            config.kicad_cli = default_kicad_cli();
        }
        if config.kicad_binary.is_empty() {
            config.kicad_binary = default_kicad_binary();
        }
        Ok(config)
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            kicad_cli: default_kicad_cli(),
            kicad_binary: default_kicad_binary(),
            project_dir: None,
            ipc_address: default_ipc_address(),
            transport: TransportMode::default(),
            http_address: default_http_address(),
            jlcpcb_db_path: None,
            log_level: default_log_level(),
        }
    }
}

fn dirs_config_path() -> PathBuf {
    // Platform-specific config directory
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").unwrap_or_default();
        PathBuf::from(appdata).join("konnect").join("config.toml")
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("konnect")
            .join("config.toml")
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(dirs::config_dir)
            .unwrap_or_else(|| PathBuf::from(".config"));
        base.join("konnect").join("config.toml")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IpcDiscovery {
    socket: String,
    token: String,
}

fn ipc_discovery_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("konnect")
        .join("kicad-api.json")
}

fn write_ipc_discovery(path: &std::path::Path, discovery: &IpcDiscovery) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    serde_json::to_writer_pretty(&mut file, discovery)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

fn read_ipc_discovery_from(path: &std::path::Path) -> Option<IpcDiscovery> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn read_ipc_discovery() -> Option<IpcDiscovery> {
    read_ipc_discovery_from(&ipc_discovery_path())
}

fn discovery_address_is_usable(address: &str) -> bool {
    if address.is_empty() {
        return false;
    }
    if address.starts_with("tcp://") || cfg!(target_os = "windows") {
        return true;
    }
    let path = address.strip_prefix("ipc://").unwrap_or(address);
    std::path::Path::new(path).exists()
}

/// Persist the KiCAD IPC connection details supplied to an executable plugin.
pub fn register_kicad_instance() -> Result<PathBuf> {
    let socket = std::env::var("KICAD_API_SOCKET")
        .context("KICAD_API_SOCKET is missing; launch this action from KiCAD")?;
    let token = std::env::var("KICAD_API_TOKEN").unwrap_or_default();
    let discovery = IpcDiscovery { socket, token };
    let path = ipc_discovery_path();
    write_ipc_discovery(&path, &discovery)?;
    Ok(path)
}

/// Restore the last KiCAD-launched socket/token into this MCP server process.
pub fn restore_kicad_instance_environment() {
    if std::env::var_os("KICAD_API_SOCKET").is_some() {
        return;
    }
    let Some(discovery) = read_ipc_discovery() else {
        return;
    };
    if !discovery_address_is_usable(&discovery.socket) {
        return;
    }
    std::env::set_var("KICAD_API_SOCKET", discovery.socket);
    if !discovery.token.is_empty() {
        std::env::set_var("KICAD_API_TOKEN", discovery.token);
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(ext: &str, content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(&format!(".{ext}"))
            .tempfile()
            .unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    // Malformed input must produce Err, never a panic (the class of bug
    // PR #9 found in the config *tools*; this pins the server config too).

    #[test]
    fn json_non_object_root_is_err_not_panic() {
        for bad in ["[1, 2, 3]", "42", "\"just a string\"", "null", "true"] {
            let f = write_temp("json", bad);
            assert!(Config::load_from(f.path()).is_err(), "input: {bad}");
        }
    }

    #[test]
    fn json_wrong_field_types_are_err() {
        for bad in [
            r#"{"transport": 42}"#,
            r#"{"transport": "carrier-pigeon"}"#,
            r#"{"kicad_cli": ["a", "b"]}"#,
            r#"{"log_level": {"nested": true}}"#,
        ] {
            let f = write_temp("json", bad);
            assert!(Config::load_from(f.path()).is_err(), "input: {bad}");
        }
    }

    #[test]
    fn toml_garbage_is_err_not_panic() {
        for bad in ["= = =", "[unclosed", "transport = ", "\u{0000}\u{FFFF}"] {
            let f = write_temp("toml", bad);
            assert!(Config::load_from(f.path()).is_err(), "input: {bad:?}");
        }
    }

    #[test]
    fn missing_file_is_err() {
        assert!(Config::load_from(std::path::Path::new("does/not/exist.toml")).is_err());
    }

    #[test]
    fn ipc_discovery_round_trips_without_exposing_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kicad-api.json");
        let expected = IpcDiscovery {
            socket: "ipc:///tmp/kicad/api.sock".into(),
            token: "secret-token".into(),
        };
        write_ipc_discovery(&path, &expected).unwrap();
        let actual = read_ipc_discovery_from(&path).unwrap();
        assert_eq!(actual.socket, expected.socket);
        assert_eq!(actual.token, expected.token);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn empty_discovery_address_is_rejected() {
        assert!(!discovery_address_is_usable(""));
        assert!(discovery_address_is_usable("tcp://127.0.0.1:1234"));
    }

    // Partial configs fill in defaults for everything omitted.

    #[test]
    fn empty_json_object_yields_defaults() {
        let f = write_temp("json", "{}");
        let c = Config::load_from(f.path()).unwrap();
        let d = Config::default();
        assert_eq!(c.kicad_cli, d.kicad_cli);
        assert_eq!(c.http_address, d.http_address);
        assert_eq!(c.log_level, d.log_level);
        assert!(matches!(c.transport, TransportMode::Stdio));
    }

    #[test]
    fn empty_toml_yields_defaults() {
        let f = write_temp("toml", "");
        let c = Config::load_from(f.path()).unwrap();
        assert_eq!(c.log_level, "info");
    }

    #[test]
    fn partial_toml_overrides_only_named_fields() {
        let f = write_temp(
            "toml",
            "transport = \"http\"\nhttp_address = \"127.0.0.1:9999\"\n",
        );
        let c = Config::load_from(f.path()).unwrap();
        assert!(matches!(
            c.transport,
            TransportMode::Both | TransportMode::Http
        ));
        assert!(matches!(c.transport, TransportMode::Http));
        assert_eq!(c.http_address, "127.0.0.1:9999");
        assert_eq!(c.log_level, "info"); // untouched default
    }

    #[test]
    fn legacy_ipc_socket_path_alias_still_works() {
        // settings.json written by the KiCAD plugin dialog uses the alias.
        let f = write_temp("json", r#"{"ipc_socket_path": "ipc://test.sock"}"#);
        let c = Config::load_from(f.path()).unwrap();
        assert_eq!(c.ipc_address, "ipc://test.sock");
    }

    #[test]
    fn unknown_extension_parses_as_toml() {
        let f = write_temp("conf", "log_level = \"debug\"\n");
        let c = Config::load_from(f.path()).unwrap();
        assert_eq!(c.log_level, "debug");
    }
}

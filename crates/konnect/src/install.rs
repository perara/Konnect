//! First-run installer for Konnect.
//!
//! Handles:
//! - `init` — full install with console output
//! - `uninstall` — remove all installed files and hook entries
//! - `status` — show install state with [+]/[-] markers
//! - `skill <name>` — print a skill's markdown to stdout (for hook integration)
//! - Silent install on first MCP launch (no stdout, stderr logging only)
//! - KiCAD auto-detection on Windows, macOS, and Linux

use crate::manifest::{AGENTS, HOOK_SKILLS, SKILLS};
use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

// ─── Public API ──────────────────────────────────────────────────────────────

/// Full install with console output. Called by `init` subcommand or double-click.
pub fn run_install() -> Result<()> {
    println!("Installing Konnect skills, agents, and hooks...\n");

    // Skills
    let skills_dir = claude_skills_dir()?;
    let mut skill_count = 0;
    for skill in SKILLS {
        let dest = skills_dir.join(skill.name);
        fs::create_dir_all(&dest)?;
        fs::write(dest.join("SKILL.md"), skill.content)?;

        // Reference files
        if !skill.references.is_empty() {
            let refs_dir = dest.join("references");
            fs::create_dir_all(&refs_dir)?;
            for (filename, content) in skill.references {
                fs::write(refs_dir.join(filename), content)?;
            }
        }
        skill_count += 1;
        println!("  [+] Skill: {}", skill.name);
    }

    // Agents
    let agents_dir = claude_agents_dir()?;
    fs::create_dir_all(&agents_dir)?;
    let mut agent_count = 0;
    for agent in AGENTS {
        fs::write(agents_dir.join(agent.filename), agent.content)?;
        agent_count += 1;
        println!("  [+] Agent: {}", agent.filename);
    }

    // Hooks
    let exe = std::env::current_exe()?;
    let exe_str = exe.to_string_lossy().to_string();
    let hook_count = patch_claude_settings(&exe_str)?;
    if hook_count > 0 {
        println!(
            "  [+] Hooks: {} entries patched into settings.json",
            hook_count
        );
    } else {
        println!("  [=] Hooks: already installed (no changes)");
    }

    // KiCAD detection
    if let Some(kicad_path) = detect_kicad() {
        println!("\n  [+] Found KiCAD at: {}", kicad_path.display());
    } else {
        println!("\n  [-] KiCAD not found in standard locations");
        println!("      Set kicad_cli path in your config file manually");
    }

    // Write marker
    let data = data_dir()?;
    fs::create_dir_all(&data)?;
    fs::write(data.join(".installed"), env!("CARGO_PKG_VERSION"))?;

    println!(
        "\nDone: {} skills, {} agents, {} hooks installed.",
        skill_count, agent_count, hook_count
    );
    Ok(())
}

/// Silent install — no stdout output (safe for MCP pipe mode).
/// Logs to stderr via tracing.
pub fn run_install_silent() -> Result<()> {
    // Skills
    let skills_dir = claude_skills_dir()?;
    for skill in SKILLS {
        let dest = skills_dir.join(skill.name);
        fs::create_dir_all(&dest)?;
        fs::write(dest.join("SKILL.md"), skill.content)?;
        if !skill.references.is_empty() {
            let refs_dir = dest.join("references");
            fs::create_dir_all(&refs_dir)?;
            for (filename, content) in skill.references {
                fs::write(refs_dir.join(filename), content)?;
            }
        }
    }

    // Agents
    let agents_dir = claude_agents_dir()?;
    fs::create_dir_all(&agents_dir)?;
    for agent in AGENTS {
        fs::write(agents_dir.join(agent.filename), agent.content)?;
    }

    // Hooks
    let exe = std::env::current_exe()?;
    let exe_str = exe.to_string_lossy().to_string();
    let _ = patch_claude_settings(&exe_str);

    // Marker
    let data = data_dir()?;
    fs::create_dir_all(&data)?;
    fs::write(data.join(".installed"), env!("CARGO_PKG_VERSION"))?;

    eprintln!(
        "[konnect] Silent install complete: {} skills, {} agents",
        SKILLS.len(),
        AGENTS.len()
    );
    Ok(())
}

/// Remove all installed files and hook entries.
pub fn run_uninstall() -> Result<()> {
    println!("Uninstalling Konnect skills, agents, and hooks...\n");

    // Skills
    let skills_dir = claude_skills_dir()?;
    for skill in SKILLS {
        let dest = skills_dir.join(skill.name);
        if dest.exists() {
            fs::remove_dir_all(&dest)?;
            println!("  [-] Removed skill: {}", skill.name);
        }
    }

    // Agents
    let agents_dir = claude_agents_dir()?;
    for agent in AGENTS {
        let dest = agents_dir.join(agent.filename);
        if dest.exists() {
            fs::remove_file(&dest)?;
            println!("  [-] Removed agent: {}", agent.filename);
        }
    }

    // Hooks — remove our entries from settings.json
    remove_hooks_from_settings()?;
    println!("  [-] Removed hook entries from settings.json");

    // Marker
    let data = data_dir()?;
    let marker = data.join(".installed");
    if marker.exists() {
        fs::remove_file(&marker)?;
    }

    println!("\nDone.");
    Ok(())
}

/// Print install status with [+]/[-] markers.
pub fn print_status() -> Result<()> {
    println!("Konnect v{} — Install Status\n", env!("CARGO_PKG_VERSION"));

    let skills_dir = claude_skills_dir()?;
    println!("Skills (~/.claude/skills/):");
    for skill in SKILLS {
        let exists = skills_dir.join(skill.name).join("SKILL.md").exists();
        let marker = if exists { "+" } else { "-" };
        println!("  [{}] {}", marker, skill.name);
    }

    let agents_dir = claude_agents_dir()?;
    println!("\nAgents (~/.claude/agents/):");
    for agent in AGENTS {
        let exists = agents_dir.join(agent.filename).exists();
        let marker = if exists { "+" } else { "-" };
        println!("  [{}] {}", marker, agent.filename);
    }

    println!("\nHooks (~/.claude/settings.json):");
    let settings_path = claude_settings_path();
    if settings_path.exists() {
        let raw = fs::read_to_string(&settings_path).unwrap_or_default();
        for hook in HOOK_SKILLS {
            let exists = raw.contains(hook.name);
            let marker = if exists { "+" } else { "-" };
            println!("  [{}] {} ({})", marker, hook.name, hook.event);
        }
    } else {
        for hook in HOOK_SKILLS {
            println!("  [-] {} ({})", hook.name, hook.event);
        }
    }

    // KiCAD detection
    println!("\nKiCAD:");
    if let Some(path) = detect_kicad() {
        println!("  [+] Found: {}", path.display());
    } else {
        println!("  [-] Not found in standard locations");
    }

    let data = data_dir()?;
    let marker = data.join(".installed");
    if marker.exists() {
        let ver = fs::read_to_string(&marker).unwrap_or_default();
        println!("\nInstall marker: v{}", ver.trim());
    } else {
        println!("\nInstall marker: not present (never installed)");
    }

    Ok(())
}

/// Print a skill's content to stdout. Used by hooks:
/// `konnect skill <name>` outputs markdown that Claude Code
/// injects before/after a tool call.
pub fn print_skill_content(name: &str) -> Result<()> {
    // Check hook skills first (they have short inline content)
    for hook in HOOK_SKILLS {
        if hook.name == name {
            print!("{}", hook.content);
            return Ok(());
        }
    }

    // Check regular skills
    for skill in SKILLS {
        if skill.name == name {
            print!("{}", skill.content);
            return Ok(());
        }
    }

    eprintln!("Unknown skill: {}", name);
    std::process::exit(1);
}

/// Check if install has been completed.
pub fn needs_install() -> bool {
    match data_dir() {
        Ok(d) => !d.join(".installed").exists(),
        Err(_) => false,
    }
}

/// Friendly double-click install: shows banner, runs install, prints config snippet.
pub fn run_double_click_install() -> Result<()> {
    println!("===========================================");
    println!("  Konnect v{}", env!("CARGO_PKG_VERSION"));
    println!("  First-time Setup");
    println!("===========================================\n");

    run_install()?;

    // Print MCP config snippet
    let exe = std::env::current_exe()?;
    let exe_str = exe.to_string_lossy().replace('\\', "\\\\");

    println!("\n-------------------------------------------");
    println!("Add this to your Claude MCP config:");
    println!("-------------------------------------------\n");
    println!(r#"  "konnect": {{"#);
    println!(r#"    "command": "{}","#, exe_str);
    println!(r#"    "env": {{ "RUST_LOG": "info" }}"#);
    println!(r#"  }}"#);

    println!("\nConfig locations:");
    print_client_config_locations();
    println!("\nAfter editing the config, restart Claude.\n");

    println!("Press Enter to close...");
    let mut buf = String::new();
    let _ = std::io::stdin().read_line(&mut buf);
    Ok(())
}

// ─── Internal Helpers ────────────────────────────────────────────────────────

fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().context("could not locate home directory")
}

fn data_dir() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        return Ok(home_dir()?.join(".konnect"));
    }
    #[cfg(target_os = "macos")]
    {
        return Ok(home_dir()?.join(".konnect"));
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Ok(std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(dirs::data_local_dir)
            .unwrap_or(home_dir()?.join(".local").join("share"))
            .join("konnect"))
    }
}

fn claude_skills_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(".claude").join("skills"))
}

fn claude_agents_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(".claude").join("agents"))
}

fn claude_settings_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("settings.json")
}

fn print_client_config_locations() {
    #[cfg(target_os = "windows")]
    println!("  Claude Desktop: %APPDATA%\\Claude\\claude_desktop_config.json");

    #[cfg(target_os = "macos")]
    println!("  Claude Desktop: ~/Library/Application Support/Claude/claude_desktop_config.json");

    #[cfg(target_os = "linux")]
    println!("  MCP client:     use the client-specific MCP configuration file");

    println!("  Claude Code:    .mcp.json in your project root");
}

fn hook_command(exe_str: &str, skill_name: &str) -> String {
    // Claude executes hooks through the platform shell. Quoting is required on
    // every platform because PCM and user-data paths may contain spaces. Serde
    // performs the JSON escaping, so do not pre-escape Windows backslashes.
    let quoted_exe = exe_str.replace('"', "\\\"");
    format!("\"{}\" skill {}", quoted_exe, skill_name)
}

fn contains_skill_hook(
    entry: &serde_json::Value,
    hook: &crate::manifest::HookSkillManifest,
) -> bool {
    entry
        .get("hooks")
        .and_then(|hooks| hooks.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("command").and_then(|command| command.as_str()))
        .any(|command| command.ends_with(&format!(" skill {}", hook.name)))
        || entry
            .get("matcher")
            .and_then(|matcher| matcher.as_str())
            .is_some_and(|matcher| matcher == hook.tool_matcher)
}

/// Idempotent hook patching: adds hook entries to `~/.claude/settings.json`.
/// Returns the number of NEW entries added (0 if all already existed).
fn patch_claude_settings(exe_str: &str) -> Result<usize> {
    let path = claude_settings_path();
    fs::create_dir_all(path.parent().unwrap())?;

    let raw = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        "{}".to_string()
    };
    let mut settings: serde_json::Value = serde_json::from_str(&raw)?;

    let hooks_obj = settings
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .context("hooks field is not an object")?;

    let mut added = 0;

    for hook in HOOK_SKILLS {
        let event_arr = hooks_obj
            .entry(hook.event)
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .context("hook event field is not an array")?;

        // Idempotent: skip if this exact Konnect skill hook already exists.
        let already_exists = event_arr
            .iter()
            .any(|entry| contains_skill_hook(entry, hook));

        if !already_exists {
            let entry = serde_json::json!({
                "matcher": hook.tool_matcher,
                "hooks": [{
                    "type": "command",
                    "command": hook_command(exe_str, hook.name)
                }]
            });
            event_arr.push(entry);
            added += 1;
        }
    }

    fs::write(&path, serde_json::to_string_pretty(&settings)?)?;
    Ok(added)
}

/// Remove only our hook entries from settings.json (leave other hooks intact).
fn remove_hooks_from_settings() -> Result<()> {
    let path = claude_settings_path();
    if !path.exists() {
        return Ok(());
    }

    let raw = fs::read_to_string(&path)?;
    let mut settings: serde_json::Value = serde_json::from_str(&raw)?;

    if let Some(hooks_obj) = settings.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        for hook in HOOK_SKILLS {
            if let Some(event_arr) = hooks_obj.get_mut(hook.event).and_then(|a| a.as_array_mut()) {
                event_arr.retain(|h| {
                    let is_ours = h
                        .get("hooks")
                        .and_then(|hooks| hooks.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|h| h.get("command"))
                        .and_then(|c| c.as_str())
                        .map(|c| c.contains("konnect"))
                        .unwrap_or(false);
                    !is_ours
                });
            }
        }
    }

    fs::write(&path, serde_json::to_string_pretty(&settings)?)?;
    Ok(())
}

/// Auto-detect the KiCAD CLI on all supported desktop platforms.
pub fn detect_kicad() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("KICAD_CLI") {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Some(path);
        }
    }

    for path_str in kicad_cli_candidates() {
        let path = PathBuf::from(path_str);
        if path.is_file() {
            return Some(path);
        }
    }

    // Try registry on Windows
    #[cfg(target_os = "windows")]
    {
        if let Some(path) = detect_kicad_from_registry() {
            return Some(path);
        }
    }

    let command = if cfg!(target_os = "windows") {
        "kicad-cli.exe"
    } else {
        "kicad-cli"
    };
    std::process::Command::new(command)
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|_| PathBuf::from(command))
}

#[cfg(target_os = "windows")]
fn kicad_cli_candidates() -> &'static [&'static str] {
    &[
        r"C:\KiCad\10.0\bin\kicad-cli.exe",
        r"C:\Program Files\KiCad\10.0\bin\kicad-cli.exe",
        r"C:\Program Files (x86)\KiCad\10.0\bin\kicad-cli.exe",
        r"C:\KiCad\9.0\bin\kicad-cli.exe",
        r"C:\Program Files\KiCad\9.0\bin\kicad-cli.exe",
        r"C:\Program Files (x86)\KiCad\9.0\bin\kicad-cli.exe",
    ]
}

#[cfg(target_os = "macos")]
fn kicad_cli_candidates() -> &'static [&'static str] {
    &[
        "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli",
        "/opt/homebrew/bin/kicad-cli",
        "/usr/local/bin/kicad-cli",
    ]
}

#[cfg(target_os = "linux")]
fn kicad_cli_candidates() -> &'static [&'static str] {
    &[
        "/usr/bin/kicad-cli",
        "/usr/local/bin/kicad-cli",
        "/snap/bin/kicad-cli",
        "/snap/kicad/current/usr/bin/kicad-cli",
    ]
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn kicad_cli_candidates() -> &'static [&'static str] {
    &[]
}

#[cfg(target_os = "windows")]
fn detect_kicad_from_registry() -> Option<PathBuf> {
    use std::process::Command;

    // Use reg.exe to query the registry (avoids winreg dependency)
    let output = Command::new("reg")
        .args(["query", r"HKLM\SOFTWARE\KiCad\10.0", "/ve"])
        .output()
        .ok()?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Parse the default value which contains the install path
        for line in stdout.lines() {
            if line.contains("REG_SZ") {
                let path_str = line.split("REG_SZ").last()?.trim();
                let cli_path = std::path::Path::new(path_str)
                    .join("bin")
                    .join("kicad-cli.exe");
                if cli_path.exists() {
                    return Some(cli_path);
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_command_quotes_paths_and_leaves_separators_alone() {
        assert_eq!(
            hook_command(r"C:\Program Files\Konnect\konnect.exe", "pre-pcb-ipc"),
            r#""C:\Program Files\Konnect\konnect.exe" skill pre-pcb-ipc"#
        );
        assert_eq!(
            hook_command("/home/test user/konnect", "pre-pcb-ipc"),
            r#""/home/test user/konnect" skill pre-pcb-ipc"#
        );
    }

    #[test]
    fn platform_candidates_prioritize_kicad_10() {
        let candidates = kicad_cli_candidates();
        if !candidates.is_empty() {
            assert!(candidates[0].contains("kicad-cli"));
        }
    }

    #[test]
    fn existing_hook_is_detected_by_command_or_matcher() {
        let hook = &HOOK_SKILLS[0];
        let by_command = serde_json::json!({
            "matcher": "old-matcher",
            "hooks": [{
                "type": "command",
                "command": "\"/opt/Konnect/konnect\" skill pre-pcb-ipc"
            }]
        });
        let by_matcher = serde_json::json!({
            "matcher": hook.tool_matcher,
            "hooks": []
        });
        assert!(contains_skill_hook(&by_command, hook));
        assert!(contains_skill_hook(&by_matcher, hook));
        assert!(!contains_skill_hook(&serde_json::json!({}), hook));
    }
}

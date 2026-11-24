//! Pure-Rust native Windows CPU & Memory monitor with smart parent-process tree attribution.
//!
//! Samples live system-wide CPU and RAM load via Win32 (`GetSystemTimes`, `GlobalMemoryStatusEx`)
//! and enumerates running processes via `CreateToolhelp32Snapshot` + `GetProcessTimes` +
//! `K32GetProcessMemoryInfo` + `QueryFullProcessImageNameW`.
//!
//! Rather than listing 56 separate `node.exe` / `bash.exe` / `conhost.exe` rows, the attribution
//! engine walks each process's parent chain (with PID-reuse protection via creation timestamps) to
//! roll child workers, terminals, compilers, and VM processes back to the real originating program
//! (e.g., `Claude (56)`, `WSL`, `Cyberpunk 2077`, `Chrome (24)`).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Maximum number of top attributed programs returned per list (Top CPU / Top Memory).
pub const MAX_TOP_CONSUMERS: usize = 5;

/// Minimum aggregated CPU percentage to display in the Top CPU breakdown.
const MIN_DISPLAY_CPU_PCT: f64 = 0.5;

/// Minimum aggregated memory bytes (32 MB) to display in the Top Memory breakdown.
const MIN_DISPLAY_MEM_BYTES: u64 = 32 * 1024 * 1024;

/// A traced parent program with its aggregated CPU & memory across all child/session processes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttributedConsumer {
    /// Human-friendly originating program name (e.g. `"Claude"`, `"WSL"`, `"Cyberpunk 2077"`).
    pub name: String,
    /// Number of live processes rolled into this program group.
    pub process_count: usize,
    /// Combined CPU percentage (`0.0..=100.0`) of total machine capacity.
    pub cpu_percent: f64,
    /// Combined private/working memory in bytes.
    pub memory_bytes: u64,
}

impl AttributedConsumer {
    /// Display name with session/process count suffix when multiple processes are rolled up
    /// (e.g. `"Claude (56)"`), or plain name for a single process (e.g. `"WSL"`).
    pub fn display_name(&self) -> String {
        if self.process_count > 1 {
            format!("{} ({})", self.name, self.process_count)
        } else {
            self.name.clone()
        }
    }
}

/// Live local system CPU & Memory telemetry snapshot for the taskbar strip and hover card.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemSnapshot {
    pub cpu_percent: f64,
    pub memory_percent: f64,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    /// Top attributed programs by CPU usage (descending).
    pub top_cpu: Vec<AttributedConsumer>,
    /// Top attributed programs by memory usage (descending).
    pub top_memory: Vec<AttributedConsumer>,
}

/// Raw per-process measurement passed into the pure-Rust tree attribution engine.
#[derive(Clone, Debug)]
pub struct RawProcessSample {
    pub pid: u32,
    pub ppid: u32,
    /// Win32 FILETIME creation timestamp (100ns ticks) used to prevent recycled-PID false parents.
    pub creation_time: u64,
    pub exe_name: String,
    pub exe_path: Option<String>,
    pub cpu_percent: f64,
    pub memory_bytes: u64,
}

/// Format bytes cleanly (`"12.4 GB"`, `"480 MB"`, `"64 GB"`).
pub fn format_bytes_compact(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        let gb = b / GB;
        if gb >= 100.0 || (gb - gb.round()).abs() < 0.05 {
            format!("{:.0} GB", gb.round())
        } else {
            format!("{gb:.1} GB")
        }
    } else {
        let mb = (b / MB).round().max(1.0);
        format!("{mb:.0} MB")
    }
}

/// Compact memory used/total badge (`"24.6 / 64 GB"`).
pub fn format_memory_summary(used_bytes: u64, total_bytes: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    if total_bytes == 0 {
        return String::new();
    }
    let used_gb = used_bytes as f64 / GB;
    let total_gb = total_bytes as f64 / GB;
    let total_str = if (total_gb - total_gb.round()).abs() < 0.15 {
        format!("{:.0}", total_gb.round())
    } else {
        format!("{total_gb:.1}")
    };
    format!("{used_gb:.1} / {total_str} GB")
}

/// Realistic demo snapshot used by `usagedeck.exe --demo` and tests.
pub fn demo_system_snapshot() -> SystemSnapshot {
    const GB: u64 = 1024 * 1024 * 1024;
    SystemSnapshot {
        cpu_percent: 42.0,
        memory_percent: 68.0,
        memory_used_bytes: 43 * GB + 500 * 1024 * 1024,
        memory_total_bytes: 64 * GB,
        top_cpu: vec![
            AttributedConsumer {
                name: "WSL".into(),
                process_count: 3,
                cpu_percent: 24.4,
                memory_bytes: 12 * GB + 400 * 1024 * 1024,
            },
            AttributedConsumer {
                name: "Claude".into(),
                process_count: 56,
                cpu_percent: 11.2,
                memory_bytes: 4 * GB + 100 * 1024 * 1024,
            },
            AttributedConsumer {
                name: "Chrome".into(),
                process_count: 24,
                cpu_percent: 3.8,
                memory_bytes: 6 * GB + 200 * 1024 * 1024,
            },
            AttributedConsumer {
                name: "Cursor".into(),
                process_count: 14,
                cpu_percent: 1.9,
                memory_bytes: 2 * GB + 800 * 1024 * 1024,
            },
        ],
        top_memory: vec![
            AttributedConsumer {
                name: "WSL".into(),
                process_count: 3,
                cpu_percent: 24.4,
                memory_bytes: 12 * GB + 400 * 1024 * 1024,
            },
            AttributedConsumer {
                name: "Chrome".into(),
                process_count: 24,
                cpu_percent: 3.8,
                memory_bytes: 6 * GB + 200 * 1024 * 1024,
            },
            AttributedConsumer {
                name: "Claude".into(),
                process_count: 56,
                cpu_percent: 11.2,
                memory_bytes: 4 * GB + 100 * 1024 * 1024,
            },
            AttributedConsumer {
                name: "Cursor".into(),
                process_count: 14,
                cpu_percent: 1.9,
                memory_bytes: 2 * GB + 800 * 1024 * 1024,
            },
        ],
    }
}

/// Normalize an executable name to lowercase without trailing `.exe`.
fn exe_stem_lower(exe_name: &str) -> String {
    let lower = exe_name.trim().to_ascii_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
}

/// True when the process is part of WSL / WSL2 (including `vmmemWSL` / `vmmem`).
fn is_wsl_process(stem: &str, path_lower: Option<&str>) -> bool {
    if matches!(
        stem,
        "vmmem"
            | "vmmemwsl"
            | "wsl"
            | "wslhost"
            | "wslrelay"
            | "wslservice"
            | "wslg"
            | "wslconfig"
            | "ubuntu"
            | "ubuntu2004"
            | "ubuntu2204"
            | "ubuntu2404"
            | "debian"
            | "kali"
            | "arch"
            | "opensuse-leap"
            | "fedora"
    ) || stem.starts_with("vmmem")
        || stem.starts_with("ubuntu")
    {
        return true;
    }
    path_lower.is_some_and(|p| {
        p.contains("windowssubsystemforlinux") || p.contains("\\wsl\\") || p.contains("\\wslg\\")
    })
}

/// Extract a clean game name if the executable resides inside a known PC game library folder.
fn extract_game_name_from_path(path: &str) -> Option<String> {
    let lower = path.to_ascii_lowercase();
    const MARKERS: &[(&str, bool)] = &[
        ("\\steamapps\\common\\", false),
        ("\\epic games\\", false),
        ("\\riot games\\", true),
        ("\\gog galaxy\\games\\", false),
        ("\\xboxgames\\", false),
        ("\\ea games\\", false),
        ("\\origin games\\", false),
        ("\\ubisoft game launcher\\games\\", false),
    ];
    for &(marker, skip_riot_client) in MARKERS {
        if let Some(pos) = lower.find(marker) {
            let after = &path[pos + marker.len()..];
            let folder = after.split(['\\', '/']).next()?.trim();
            if folder.is_empty() {
                continue;
            }
            if skip_riot_client && folder.eq_ignore_ascii_case("Riot Client") {
                return Some("Riot Client".to_string());
            }
            if folder.eq_ignore_ascii_case("Launcher") || folder.eq_ignore_ascii_case("DirectX") {
                continue;
            }
            return Some(folder.to_string());
        }
    }
    None
}

/// Match an explicit, high-confidence application root name from executable name or install path.
fn match_named_app_root(stem: &str, path_lower: Option<&str>) -> Option<&'static str> {
    // Check path-based AI coding tools first so Node-hosted CLIs (e.g. Claude Code) still map cleanly.
    if let Some(p) = path_lower {
        if p.contains("anthropicclaude") || p.contains("claude-code") || p.contains("\\.claude\\") {
            return Some("Claude");
        }
        if p.contains("\\opencode\\") {
            return Some("OpenCode");
        }
        if p.contains("\\antigravity\\") {
            return Some("Antigravity");
        }
    }

    match stem {
        // AI & Developer IDEs
        "claude" => Some("Claude"),
        "codex" => Some("Codex"),
        "opencode" => Some("OpenCode"),
        "cursor" => Some("Cursor"),
        "antigravity" => Some("Antigravity"),
        "code" | "code - insiders" => Some("VS Code"),
        "windsurf" => Some("Windsurf"),
        "zed" => Some("Zed"),
        "devenv" => Some("Visual Studio"),
        "idea64" => Some("IntelliJ IDEA"),
        "pycharm64" => Some("PyCharm"),
        "rustrover64" => Some("RustRover"),
        "webstorm64" => Some("WebStorm"),
        "goland64" => Some("GoLand"),
        "clion64" => Some("CLion"),
        "rider64" => Some("Rider"),
        "androidstudio64" | "studio64" => Some("Android Studio"),
        "usagedeck" => Some("UsageDeck"),

        // Browsers
        "chrome" => Some("Chrome"),
        "msedge" => Some("Edge"),
        "firefox" => Some("Firefox"),
        "brave" => Some("Brave"),
        "arc" => Some("Arc"),
        "opera" => Some("Opera"),
        "vivaldi" => Some("Vivaldi"),
        "zen" => Some("Zen"),

        // Containers & VMs
        "docker desktop" | "com.docker.backend" | "dockerd" | "docker" => Some("Docker"),
        "vmware-vmx" | "vmware" => Some("VMware"),
        "virtualboxvm" | "virtualbox" => Some("VirtualBox"),

        // Communication, Productivity & Creative
        "discord" | "discordptb" | "discordcanary" => Some("Discord"),
        "slack" => Some("Slack"),
        "ms-teams" | "teams" => Some("Teams"),
        "telegram" => Some("Telegram"),
        "wechat" | "weixin" => Some("WeChat"),
        "qq" => Some("QQ"),
        "spotify" => Some("Spotify"),
        "obsidian" => Some("Obsidian"),
        "notion" => Some("Notion"),
        "figma" => Some("Figma"),
        "obs64" | "obs32" => Some("OBS"),
        "blender" => Some("Blender"),
        "photoshop" => Some("Photoshop"),
        "premiere pro" | "adobe premiere pro" => Some("Premiere Pro"),
        "afterfx" => Some("After Effects"),
        "winword" => Some("Word"),
        "excel" => Some("Excel"),
        "powerpnt" => Some("PowerPoint"),
        "olk" | "outlook" => Some("Outlook"),

        // Game Clients & Launchers
        "steam" | "steamwebhelper" | "steamservice" | "gameoverlayui" => Some("Steam"),
        "epicgameslauncher" | "epicwebhelper" => Some("Epic Games"),
        "riotclientservices" | "riotclientux" | "riotclientuxrender" => Some("Riot Client"),
        "battle.net" | "agent" if path_lower.is_some_and(|p| p.contains("battle.net")) => {
            Some("Battle.net")
        }
        "battle.net" => Some("Battle.net"),
        "eadesktop" | "eabackgroundservice" | "ealauncher" => Some("EA App"),
        "upc" | "ubisoftconnect" => Some("Ubisoft Connect"),
        "galaxyclient" => Some("GOG Galaxy"),
        "playnite.desktopapp" | "playnite.fullscreenapp" => Some("Playnite"),
        "valorant-win64-shipping" | "valorant" => Some("VALORANT"),
        "leagueclient" | "leagueclientux" | "league of legends" => Some("League of Legends"),
        "genshinimpact" | "yuanshen" => Some("Genshin Impact"),
        "starrail" => Some("Honkai: Star Rail"),
        "zenlesszonezero" => Some("Zenless Zone Zero"),
        "minecraft" | "minecraftlauncher" => Some("Minecraft"),

        // Windows security (separate from generic Windows OS so Defender scans are obvious!)
        "msmpeng" | "mpdefendercoreservice" | "nissrv" => Some("Windows Defender"),

        _ => None,
    }
}

/// Windows OS infrastructure boundaries where parent-chain climbing must stop.
fn is_os_boundary(stem: &str) -> bool {
    matches!(
        stem,
        "system"
            | "system idle process"
            | "registry"
            | "memcompression"
            | "smss"
            | "csrss"
            | "wininit"
            | "winlogon"
            | "services"
            | "lsass"
            | "lsaiso"
            | "svchost"
            | "explorer"
            | "sihost"
            | "userinit"
            | "runtimebroker"
            | "applicationframehost"
            | "taskhostw"
            | "dllhost"
            | "dwm"
            | "fontdrvhost"
            | "ctfmon"
            | "searchhost"
            | "searchindexer"
            | "startmenuexperiencehost"
            | "shellexperiencehost"
            | "textinputhost"
            | "widgetservice"
            | "widgets"
            | "audiodg"
            | "spoolsv"
            | "wmiprvse"
            | "smartscreen"
            | "securityhealthservice"
            | "msedgewebview2"
            | "backgroundtaskhost"
            | "conhost"
            | "openconsole"
            | "taskmgr"
    )
}

/// Interactive terminals & shells that are transparent when hosting a real app (e.g. Claude),
/// but serve as fallback labels when no higher app is present.
fn is_shell_or_terminal(stem: &str) -> bool {
    matches!(
        stem,
        "windowsterminal"
            | "wt"
            | "cmd"
            | "powershell"
            | "pwsh"
            | "bash"
            | "sh"
            | "zsh"
            | "fish"
            | "mintty"
            | "git-bash"
            | "wezterm-gui"
            | "alacritty"
            | "warp"
            | "hyper"
            | "tabby"
    )
}

/// Generic runtimes, compilers, and helper workers that should roll up into their parent app
/// whenever an app root exists above them.
fn is_generic_worker_or_tool(stem: &str) -> bool {
    matches!(
        stem,
        "node"
            | "bun"
            | "deno"
            | "npm"
            | "npx"
            | "pnpm"
            | "yarn"
            | "tsx"
            | "ts-node"
            | "python"
            | "python3"
            | "pythonw"
            | "uv"
            | "pip"
            | "pytest"
            | "mypy"
            | "ruff"
            | "cargo"
            | "rustc"
            | "rust-analyzer"
            | "rustfmt"
            | "clippy-driver"
            | "cc1"
            | "cc1plus"
            | "gcc"
            | "g++"
            | "clang"
            | "clang++"
            | "clangd"
            | "cl"
            | "link"
            | "lld"
            | "lld-link"
            | "mspdbsrv"
            | "msbuild"
            | "make"
            | "cmake"
            | "ninja"
            | "dotnet"
            | "java"
            | "javaw"
            | "go"
            | "esbuild"
            | "swc"
            | "tsc"
            | "vite"
            | "webpack"
            | "eslint"
            | "prettier"
            | "git"
            | "git-remote-https"
            | "git-upload-pack"
            | "rg"
            | "fd"
            | "grep"
            | "egrep"
            | "fgrep"
            | "find"
            | "findstr"
            | "sed"
            | "awk"
            | "gawk"
            | "xargs"
            | "timeout"
            | "sleep"
            | "cat"
            | "head"
            | "tail"
            | "sort"
            | "uniq"
            | "wc"
            | "jq"
            | "diff"
            | "ls"
            | "cp"
            | "mv"
            | "rm"
            | "mkdir"
            | "tr"
            | "cut"
            | "tee"
            | "env"
            | "cygpath"
            | "uname"
            | "which"
            | "where"
            | "ssh"
            | "ssh-agent"
            | "curl"
            | "tar"
            | "7z"
            | "qtwebengineprocess"
            | "crashpad_handler"
            | "cefsharp.browsersubprocess"
            | "electron"
            | "msedgewebview2"
    )
}

fn friendly_tool_or_shell_name(stem: &str) -> Option<&'static str> {
    match stem {
        "windowsterminal" | "wt" => Some("Windows Terminal"),
        "wezterm-gui" => Some("WezTerm"),
        "alacritty" => Some("Alacritty"),
        "warp" => Some("Warp"),
        "powershell" | "pwsh" => Some("PowerShell"),
        "cmd" => Some("Command Prompt"),
        "bash" | "sh" | "zsh" | "mintty" | "git-bash" => Some("Git Bash"),
        "cargo" | "rustc" | "rust-analyzer" | "clippy-driver" => Some("Rust / Cargo"),
        "node" => Some("Node.js"),
        "bun" => Some("Bun"),
        "deno" => Some("Deno"),
        "python" | "python3" | "pythonw" | "uv" => Some("Python"),
        "java" | "javaw" => Some("Java"),
        "dotnet" => Some(".NET"),
        "msbuild" | "cl" | "link" | "mspdbsrv" => Some("MSVC Build"),
        "clang" | "clang++" | "clangd" | "gcc" | "g++" => Some("C/C++ Compiler"),
        "go" => Some("Go"),
        "git" => Some("Git"),
        "msedgewebview2" => Some("WebView2"),
        _ => None,
    }
}

fn prettify_exe_name(exe_name: &str) -> String {
    let trimmed = exe_name.trim();
    let base = trimmed
        .strip_suffix(".exe")
        .or_else(|| trimmed.strip_suffix(".EXE"))
        .unwrap_or(trimmed);
    if base.is_empty() {
        return "Unknown".to_string();
    }
    // Preserve camel/mixed-case names like `GameClient` or `OBS`, otherwise capitalize first letter.
    if base.chars().any(|c| c.is_ascii_uppercase()) {
        base.to_string()
    } else {
        let mut chars = base.chars();
        match chars.next() {
            Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
            None => "Unknown".to_string(),
        }
    }
}

/// Trace a single process back to its originating human-friendly program name using the live
/// process map.
pub fn attribute_process_name(
    proc_sample: &RawProcessSample,
    by_pid: &HashMap<u32, &RawProcessSample>,
) -> String {
    // Walk up the ancestor chain [self, parent, grandparent, ...] stopping before OS boundaries.
    let mut chain: Vec<&RawProcessSample> = Vec::with_capacity(8);
    let mut cur = proc_sample;
    let mut visited: Vec<u32> = Vec::with_capacity(8);
    for _ in 0..16 {
        if visited.contains(&cur.pid) {
            break;
        }
        visited.push(cur.pid);
        chain.push(cur);

        if cur.ppid == 0 || cur.ppid == cur.pid {
            break;
        }
        let Some(&parent) = by_pid.get(&cur.ppid) else {
            break;
        };
        // Guard against PID reuse: a real parent must have started at or before its child.
        if parent.creation_time > 0
            && cur.creation_time > 0
            && parent.creation_time > cur.creation_time
        {
            break;
        }
        let parent_stem = exe_stem_lower(&parent.exe_name);
        if is_os_boundary(&parent_stem) && !is_wsl_process(&parent_stem, None) {
            break;
        }
        cur = parent;
    }

    // Pass 1: WSL anywhere in the chain -> "WSL".
    for node in &chain {
        let stem = exe_stem_lower(&node.exe_name);
        let path_lower = node.exe_path.as_deref().map(str::to_ascii_lowercase);
        if is_wsl_process(&stem, path_lower.as_deref()) {
            return "WSL".to_string();
        }
    }

    // Pass 2: Known PC game library path (nearest to self so a Steam-launched game displays its
    // own title while Steam's own `steamwebhelper.exe` still rolls up to "Steam").
    for node in &chain {
        if let Some(ref path) = node.exe_path {
            if let Some(game_name) = extract_game_name_from_path(path) {
                return game_name;
            }
        }
    }

    // Pass 3: Nearest explicit named App Root walking upward from self.
    // This ensures e.g. `node.exe` -> `claude.exe` inside `Cursor`/`WindowsTerminal` attributes to
    // "Claude", while `node.exe` directly under `Cursor.exe` attributes to "Cursor".
    for node in &chain {
        let stem = exe_stem_lower(&node.exe_name);
        let path_lower = node.exe_path.as_deref().map(str::to_ascii_lowercase);
        if let Some(app) = match_named_app_root(&stem, path_lower.as_deref()) {
            return app.to_string();
        }
    }

    // Pass 4: Highest non-OS, non-shell, non-generic desktop executable in the chain
    // (e.g. `Helper.exe` / `QtWebEngineProcess.exe` spawned by `MyGameClient.exe` -> `MyGameClient`).
    for node in chain.iter().rev() {
        let stem = exe_stem_lower(&node.exe_name);
        if !is_os_boundary(&stem)
            && !is_shell_or_terminal(&stem)
            && !is_generic_worker_or_tool(&stem)
        {
            return prettify_exe_name(&node.exe_name);
        }
    }

    // Pass 5: Highest generic build tool / runtime (e.g. standalone `cargo` -> `rustc` inside
    // Windows Terminal -> "Rust / Cargo").
    for node in chain.iter().rev() {
        let stem = exe_stem_lower(&node.exe_name);
        if is_generic_worker_or_tool(&stem) {
            return friendly_tool_or_shell_name(&stem)
                .map(str::to_string)
                .unwrap_or_else(|| prettify_exe_name(&node.exe_name));
        }
    }

    // Pass 6: Terminal / shell fallback, or Windows OS group.
    for node in chain.iter().rev() {
        let stem = exe_stem_lower(&node.exe_name);
        if is_shell_or_terminal(&stem) {
            return friendly_tool_or_shell_name(&stem)
                .map(str::to_string)
                .unwrap_or_else(|| prettify_exe_name(&node.exe_name));
        }
    }

    let self_stem = exe_stem_lower(&proc_sample.exe_name);
    if is_os_boundary(&self_stem) {
        "Windows".to_string()
    } else {
        prettify_exe_name(&proc_sample.exe_name)
    }
}

/// Group raw process samples by traced originating program and return `(top_cpu, top_memory)`.
pub fn attribute_processes(
    procs: &[RawProcessSample],
) -> (Vec<AttributedConsumer>, Vec<AttributedConsumer>) {
    let mut by_pid: HashMap<u32, &RawProcessSample> = HashMap::with_capacity(procs.len());
    for p in procs {
        by_pid.insert(p.pid, p);
    }

    let mut groups: HashMap<String, AttributedConsumer> = HashMap::new();
    for p in procs {
        if p.pid == 0 {
            continue;
        }
        let stem = exe_stem_lower(&p.exe_name);
        if stem == "system idle process" {
            continue;
        }
        let name = attribute_process_name(p, &by_pid);
        let entry = groups.entry(name.clone()).or_insert(AttributedConsumer {
            name,
            process_count: 0,
            cpu_percent: 0.0,
            memory_bytes: 0,
        });
        entry.process_count += 1;
        if p.cpu_percent.is_finite() && p.cpu_percent > 0.0 {
            entry.cpu_percent += p.cpu_percent;
        }
        entry.memory_bytes = entry.memory_bytes.saturating_add(p.memory_bytes);
    }

    let all: Vec<AttributedConsumer> = groups.into_values().collect();

    let mut top_cpu: Vec<AttributedConsumer> = all
        .iter()
        .filter(|c| c.cpu_percent >= MIN_DISPLAY_CPU_PCT)
        .cloned()
        .collect();
    top_cpu.sort_by(|a, b| {
        b.cpu_percent
            .total_cmp(&a.cpu_percent)
            .then_with(|| b.memory_bytes.cmp(&a.memory_bytes))
    });
    top_cpu.truncate(MAX_TOP_CONSUMERS);

    let mut top_memory: Vec<AttributedConsumer> = all
        .into_iter()
        .filter(|c| c.memory_bytes >= MIN_DISPLAY_MEM_BYTES)
        .collect();
    top_memory.sort_by(|a, b| {
        b.memory_bytes
            .cmp(&a.memory_bytes)
            .then_with(|| b.cpu_percent.total_cmp(&a.cpu_percent))
    });
    top_memory.truncate(MAX_TOP_CONSUMERS);

    (top_cpu, top_memory)
}

/// Reconcile per-process CPU% and memory bytes so aggregated process rows can never exceed
/// system-wide CPU% or physical used RAM (e.g. from minor snapshot clock skew or shared pages).
pub fn reconcile_process_totals(
    procs: &mut [RawProcessSample],
    cpu_percent: f64,
    memory_used_bytes: u64,
) {
    let sum_cpu: f64 = procs
        .iter()
        .filter(|p| p.cpu_percent.is_finite() && p.cpu_percent > 0.0)
        .map(|p| p.cpu_percent)
        .sum();
    if sum_cpu > cpu_percent && cpu_percent > 0.0 {
        let scale = cpu_percent / sum_cpu;
        for p in procs.iter_mut() {
            if p.cpu_percent.is_finite() && p.cpu_percent > 0.0 {
                p.cpu_percent *= scale;
            }
        }
    } else if cpu_percent <= 0.0 {
        for p in procs.iter_mut() {
            p.cpu_percent = 0.0;
        }
    }

    let sum_mem: u64 = procs
        .iter()
        .fold(0u64, |acc, p| acc.saturating_add(p.memory_bytes));
    if sum_mem > memory_used_bytes && memory_used_bytes > 0 {
        let scale = memory_used_bytes as f64 / sum_mem as f64;
        for p in procs.iter_mut() {
            if p.memory_bytes > 0 {
                p.memory_bytes = ((p.memory_bytes as f64) * scale).round() as u64;
            }
        }
    }
}

/// Official `psapi.h` `PROCESS_MEMORY_COUNTERS_EX2` layout (Windows 10/11 22H2+).
/// Exposes `private_working_set_size` (exact physical RAM privately owned by a process, matching
/// Windows Task Manager's Memory column and excluding pagefile commit charge & shared DLLs).
#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct ProcessMemoryCountersEx2 {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_usage: usize,
    private_working_set_size: usize,
    shared_commit_usage: u64,
}

/// Exact 64-bit Windows `UNICODE_STRING` header inside `SYSTEM_PROCESS_INFORMATION`.
#[cfg(all(windows, target_pointer_width = "64"))]
#[allow(dead_code)]
#[repr(C)]
#[derive(Clone, Copy)]
struct NtUnicodeString {
    length: u16,
    maximum_length: u16,
    _pad: u32,
    buffer: *const u16,
}

/// Exact 64-bit Windows `SYSTEM_PROCESS_INFORMATION` (`0x100` bytes) returned by
/// `NtQuerySystemInformation(SystemProcessInformation = 5)` — unprivileged syscall used by
/// Windows Task Manager to read CPU times & private working sets for ALL processes including
/// `System` (PID 4), `vmmemWSL`, `MsMpEng`, `MemCompression`, and elevated apps.
#[cfg(all(windows, target_pointer_width = "64"))]
#[allow(dead_code)]
#[repr(C)]
#[derive(Clone, Copy)]
struct SystemProcessInfo64 {
    next_entry_offset: u32,
    number_of_threads: u32,
    working_set_private_size: i64,
    hard_fault_count: u32,
    number_of_threads_high_watermark: u32,
    cycle_time: u64,
    create_time: i64,
    user_time: i64,
    kernel_time: i64,
    image_name: NtUnicodeString,
    base_priority: i32,
    _pad_priority: u32,
    unique_process_id: usize,
    inherited_from_unique_process_id: usize,
    handle_count: u32,
    session_id: u32,
    unique_process_key: usize,
    peak_virtual_size: usize,
    virtual_size: usize,
    page_fault_count: u32,
    _pad_fault: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_page_count: usize,
    io_counters: [u64; 6],
}

#[cfg(all(windows, target_pointer_width = "64"))]
const _: () = {
    assert!(std::mem::size_of::<SystemProcessInfo64>() == 0x100);
    assert!(std::mem::offset_of!(SystemProcessInfo64, working_set_private_size) == 0x08);
    assert!(std::mem::offset_of!(SystemProcessInfo64, create_time) == 0x20);
    assert!(std::mem::offset_of!(SystemProcessInfo64, image_name) == 0x38);
    assert!(std::mem::offset_of!(SystemProcessInfo64, unique_process_id) == 0x50);
    assert!(std::mem::offset_of!(SystemProcessInfo64, inherited_from_unique_process_id) == 0x58);
    assert!(std::mem::offset_of!(SystemProcessInfo64, working_set_size) == 0x90);
};

#[cfg(all(windows, target_pointer_width = "64"))]
#[link(name = "ntdll")]
extern "system" {
    fn NtQuerySystemInformation(
        system_information_class: u32,
        system_information: *mut std::ffi::c_void,
        system_information_length: u32,
        return_length: *mut u32,
    ) -> i32;
}

/// Raw process entry retrieved from either `NtQuerySystemInformation` or `ToolHelp32`.
#[cfg(windows)]
struct RawWinProc {
    pid: u32,
    ppid: u32,
    creation_time: u64,
    total_proc_time: u64,
    memory_bytes: u64,
    exe_name: String,
    exe_path: Option<String>,
}

/// Enumerate every live Windows process (including protected/elevated OS & WSL processes) via
/// `NtQuerySystemInformation(SystemProcessInformation = 5)`.
#[cfg(all(windows, target_pointer_width = "64"))]
fn query_processes_nt() -> Option<Vec<RawWinProc>> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    const SYSTEM_PROCESS_INFORMATION: u32 = 5;
    const STATUS_INFO_LENGTH_MISMATCH: i32 = -1_073_741_820; // 0xC0000004
    const MAX_BUF_BYTES: usize = 64 * 1024 * 1024;

    // Allocate as `Vec<u64>` to guarantee 8-byte alignment for `SystemProcessInfo64`.
    let mut cap_bytes: usize = 256 * 1024;
    let mut words: Vec<u64> = Vec::new();
    let mut ok = false;
    for _ in 0..8 {
        words.resize(cap_bytes.div_ceil(8), 0);
        let buf_len = (words.len() * 8).min(u32::MAX as usize) as u32;
        let mut ret_len: u32 = 0;
        // SAFETY: `words` is valid writable memory of `buf_len` bytes and `ret_len` is stack-owned.
        let status = unsafe {
            NtQuerySystemInformation(
                SYSTEM_PROCESS_INFORMATION,
                words.as_mut_ptr().cast::<std::ffi::c_void>(),
                buf_len,
                &mut ret_len,
            )
        };
        if status >= 0 {
            ok = true;
            break;
        }
        if status != STATUS_INFO_LENGTH_MISMATCH {
            return None;
        }
        let needed = (ret_len as usize).max(cap_bytes.saturating_mul(2)) + 65_536;
        if needed > MAX_BUF_BYTES {
            return None;
        }
        cap_bytes = needed;
    }
    if !ok {
        return None;
    }

    let buf_start = words.as_ptr().cast::<u8>() as usize;
    let buf_bytes = words.len() * 8;
    let buf_end = buf_start.saturating_add(buf_bytes);

    let mut out = Vec::with_capacity(384);
    let mut path_buf = [0u16; 520];
    let mut offset: usize = 0;

    while offset + std::mem::size_of::<SystemProcessInfo64>() <= buf_bytes {
        // SAFETY: bounds checked above; `read_unaligned` handles arbitrary `NextEntryOffset`.
        let info =
            unsafe { std::ptr::read_unaligned((buf_start + offset) as *const SystemProcessInfo64) };
        let pid = info.unique_process_id as u32;
        let ppid = info.inherited_from_unique_process_id as u32;
        let creation_time = info.create_time.max(0) as u64;
        let total_proc_time =
            (info.kernel_time.max(0) as u64).saturating_add(info.user_time.max(0) as u64);
        let memory_bytes = if info.working_set_private_size > 0 {
            info.working_set_private_size as u64
        } else {
            info.working_set_size as u64
        };

        let exe_name = if pid == 0 {
            "System Idle Process".to_string()
        } else if !info.image_name.buffer.is_null() && info.image_name.length >= 2 {
            let str_addr = info.image_name.buffer as usize;
            let byte_len = usize::from(info.image_name.length);
            if str_addr >= buf_start && str_addr.saturating_add(byte_len) <= buf_end {
                let u16_len = byte_len / 2;
                let mut wide = Vec::with_capacity(u16_len);
                // SAFETY: `str_addr..str_addr+byte_len` verified inside `words` buffer.
                unsafe {
                    for i in 0..u16_len {
                        wide.push(std::ptr::read_unaligned(info.image_name.buffer.add(i)));
                    }
                }
                String::from_utf16_lossy(&wide)
            } else if pid == 4 {
                "System".to_string()
            } else {
                String::new()
            }
        } else if pid == 4 {
            "System".to_string()
        } else {
            String::new()
        };

        // Optional path lookup (for Steam/Epic game folders & Claude Code install path).
        // Failing OpenProcess here is fine because CPU & RAM are already read above!
        let mut exe_path: Option<String> = None;
        if pid > 4 {
            // SAFETY: best-effort limited query handle always closed immediately.
            unsafe {
                if let Ok(hproc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                    let mut path_len = path_buf.len() as u32;
                    if QueryFullProcessImageNameW(
                        hproc,
                        PROCESS_NAME_FORMAT(0),
                        PWSTR(path_buf.as_mut_ptr()),
                        &mut path_len,
                    )
                    .is_ok()
                        && path_len > 0
                    {
                        exe_path = Some(String::from_utf16_lossy(&path_buf[..path_len as usize]));
                    }
                    let _ = CloseHandle(hproc);
                }
            }
        }

        out.push(RawWinProc {
            pid,
            ppid,
            creation_time,
            total_proc_time,
            memory_bytes,
            exe_name,
            exe_path,
        });

        if info.next_entry_offset == 0 {
            break;
        }
        let Some(next) = offset.checked_add(info.next_entry_offset as usize) else {
            break;
        };
        if next <= offset {
            break;
        }
        offset = next;
    }

    (!out.is_empty()).then_some(out)
}

#[cfg(all(windows, not(target_pointer_width = "64")))]
fn query_processes_nt() -> Option<Vec<RawWinProc>> {
    None
}

/// Stateful native system monitor that computes delta CPU% across samples and attributes live
/// CPU & Memory usage back to originating applications.
#[derive(Default)]
pub struct SystemMonitor {
    #[cfg(windows)]
    prev_sys_times: Option<(u64, u64, u64)>,
    #[cfg(windows)]
    prev_sample_ft: u64,
    #[cfg(windows)]
    prev_proc_times: HashMap<u32, (u64, u64)>,
}

impl SystemMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take a live CPU + Memory + attributed-process sample.
    ///
    /// On the very first call, performs a brief 140ms warmup delta so initial CPU% and Top CPU
    /// attribution are populated immediately instead of showing `0%`.
    #[cfg(windows)]
    pub fn sample(&mut self) -> Option<SystemSnapshot> {
        if self.prev_sys_times.is_none() {
            let _ = self.sample_once();
            std::thread::sleep(std::time::Duration::from_millis(140));
        }
        self.sample_once()
    }

    #[cfg(not(windows))]
    pub fn sample(&mut self) -> Option<SystemSnapshot> {
        None
    }

    #[cfg(windows)]
    fn sample_once(&mut self) -> Option<SystemSnapshot> {
        use windows::core::PWSTR;
        use windows::Win32::Foundation::{CloseHandle, FILETIME};
        use windows::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        };
        use windows::Win32::System::ProcessStatus::{
            K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
        };
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        use windows::Win32::System::Threading::{
            GetProcessTimes, GetSystemTimes, OpenProcess, QueryFullProcessImageNameW,
            PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        let filetime_u64 = |ft: FILETIME| -> u64 {
            (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)
        };

        // 1. System-wide memory load.
        let mut mem = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        // SAFETY: valid MEMORYSTATUSEX with dwLength initialized.
        unsafe {
            GlobalMemoryStatusEx(&mut mem).ok()?;
        }
        let memory_total_bytes = mem.ullTotalPhys;
        let memory_used_bytes = mem.ullTotalPhys.saturating_sub(mem.ullAvailPhys);
        let memory_percent = if memory_total_bytes > 0 {
            ((memory_used_bytes as f64 / memory_total_bytes as f64) * 100.0).clamp(0.0, 100.0)
        } else {
            f64::from(mem.dwMemoryLoad).clamp(0.0, 100.0)
        };

        // 2. System-wide CPU times (kernel includes idle) & wall-clock FILETIME (100ns since 1601).
        const WINDOWS_EPOCH_OFFSET_SECS: u64 = 11_644_473_600;
        let now_ft = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| {
                d.as_secs()
                    .saturating_add(WINDOWS_EPOCH_OFFSET_SECS)
                    .saturating_mul(10_000_000)
                    .saturating_add(u64::from(d.subsec_nanos() / 100))
            })
            .unwrap_or(0);
        let (mut idle_ft, mut kernel_ft, mut user_ft) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        // SAFETY: valid out pointers for GetSystemTimes.
        unsafe {
            GetSystemTimes(
                Some(std::ptr::from_mut(&mut idle_ft)),
                Some(std::ptr::from_mut(&mut kernel_ft)),
                Some(std::ptr::from_mut(&mut user_ft)),
            )
            .ok()?;
        }
        let curr_sys = (
            filetime_u64(idle_ft),
            filetime_u64(kernel_ft),
            filetime_u64(user_ft),
        );
        let (d_total_sys, cpu_percent) = match self.prev_sys_times {
            Some((pi, pk, pu)) => {
                let d_idle = curr_sys.0.saturating_sub(pi);
                let d_kernel = curr_sys.1.saturating_sub(pk);
                let d_user = curr_sys.2.saturating_sub(pu);
                let d_total = d_kernel.saturating_add(d_user);
                let pct = if d_total > 0 {
                    ((d_total.saturating_sub(d_idle) as f64 / d_total as f64) * 100.0)
                        .clamp(0.0, 100.0)
                } else {
                    0.0
                };
                (d_total, pct)
            }
            None => (0, 0.0),
        };
        let prev_sample_ft = self.prev_sample_ft;
        self.prev_sys_times = Some(curr_sys);
        self.prev_sample_ft = now_ft;

        // 3. Snapshot all live processes (preferring unprivileged NtQuerySystemInformation so
        //    System, vmmemWSL, Defender, and elevated apps are all measured accurately).
        let enumerated: Vec<RawWinProc> = if let Some(nt_procs) = query_processes_nt() {
            nt_procs
        } else {
            // Fallback to ToolHelp32 if NtQuerySystemInformation ever fails.
            // SAFETY: ToolHelp snapshot handle is always closed before returning.
            let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()? };
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut fallback = Vec::with_capacity(256);
            let mut path_buf = [0u16; 520];

            // SAFETY: iterates valid PROCESSENTRY32W entries in our snapshot handle.
            unsafe {
                if Process32FirstW(snap, &mut entry).is_ok() {
                    loop {
                        let pid = entry.th32ProcessID;
                        let ppid = entry.th32ParentProcessID;
                        let end = entry
                            .szExeFile
                            .iter()
                            .position(|&c| c == 0)
                            .unwrap_or(entry.szExeFile.len());
                        let exe_name = String::from_utf16_lossy(&entry.szExeFile[..end]);

                        let mut creation_time = 0u64;
                        let mut total_proc_time = 0u64;
                        let mut memory_bytes = 0u64;
                        let mut exe_path: Option<String> = None;

                        if pid > 4 {
                            if let Ok(hproc) =
                                OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
                            {
                                let (mut c_ft, mut e_ft, mut k_ft, mut u_ft) = (
                                    FILETIME::default(),
                                    FILETIME::default(),
                                    FILETIME::default(),
                                    FILETIME::default(),
                                );
                                if GetProcessTimes(
                                    hproc, &mut c_ft, &mut e_ft, &mut k_ft, &mut u_ft,
                                )
                                .is_ok()
                                {
                                    creation_time = filetime_u64(c_ft);
                                    total_proc_time =
                                        filetime_u64(k_ft).saturating_add(filetime_u64(u_ft));
                                }

                                let mut pmc2 = ProcessMemoryCountersEx2 {
                                    cb: std::mem::size_of::<ProcessMemoryCountersEx2>() as u32,
                                    ..Default::default()
                                };
                                if K32GetProcessMemoryInfo(
                                    hproc,
                                    std::ptr::from_mut(&mut pmc2).cast::<PROCESS_MEMORY_COUNTERS>(),
                                    std::mem::size_of::<ProcessMemoryCountersEx2>() as u32,
                                )
                                .as_bool()
                                {
                                    memory_bytes = if pmc2.private_working_set_size > 0 {
                                        pmc2.private_working_set_size as u64
                                    } else {
                                        pmc2.working_set_size as u64
                                    };
                                } else {
                                    let mut pmc = PROCESS_MEMORY_COUNTERS_EX {
                                        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>()
                                            as u32,
                                        ..Default::default()
                                    };
                                    if K32GetProcessMemoryInfo(
                                        hproc,
                                        std::ptr::from_mut(&mut pmc)
                                            .cast::<PROCESS_MEMORY_COUNTERS>(),
                                        std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
                                    )
                                    .as_bool()
                                    {
                                        memory_bytes = pmc.WorkingSetSize as u64;
                                    }
                                }

                                let mut path_len = path_buf.len() as u32;
                                if QueryFullProcessImageNameW(
                                    hproc,
                                    PROCESS_NAME_FORMAT(0),
                                    PWSTR(path_buf.as_mut_ptr()),
                                    &mut path_len,
                                )
                                .is_ok()
                                    && path_len > 0
                                {
                                    exe_path = Some(String::from_utf16_lossy(
                                        &path_buf[..path_len as usize],
                                    ));
                                }

                                let _ = CloseHandle(hproc);
                            }
                        }

                        fallback.push(RawWinProc {
                            pid,
                            ppid,
                            creation_time,
                            total_proc_time,
                            memory_bytes,
                            exe_name,
                            exe_path,
                        });

                        if Process32NextW(snap, &mut entry).is_err() {
                            break;
                        }
                    }
                }
                let _ = CloseHandle(snap);
            }
            fallback
        };

        let mut raw_procs: Vec<RawProcessSample> = Vec::with_capacity(enumerated.len());
        let mut next_proc_times: HashMap<u32, (u64, u64)> =
            HashMap::with_capacity(enumerated.len());

        for proc in enumerated {
            let mut proc_cpu_pct = 0.0f64;
            if proc.pid != 0 && d_total_sys > 0 {
                let d_proc = if let Some(&(prev_c, prev_t)) = self.prev_proc_times.get(&proc.pid) {
                    if prev_c == proc.creation_time {
                        proc.total_proc_time.saturating_sub(prev_t)
                    } else if prev_sample_ft > 0 && proc.creation_time >= prev_sample_ft {
                        // Recycled PID started within this interval: all its CPU time is new.
                        proc.total_proc_time
                    } else {
                        0
                    }
                } else if prev_sample_ft > 0 && proc.creation_time >= prev_sample_ft {
                    // Newly spawned process within this interval: count its CPU time immediately!
                    proc.total_proc_time
                } else {
                    0
                };
                if d_proc > 0 {
                    proc_cpu_pct = ((d_proc as f64 / d_total_sys as f64) * 100.0).clamp(0.0, 100.0);
                }
            }
            if proc.pid != 0 {
                next_proc_times.insert(proc.pid, (proc.creation_time, proc.total_proc_time));
            }
            raw_procs.push(RawProcessSample {
                pid: proc.pid,
                ppid: proc.ppid,
                creation_time: proc.creation_time,
                exe_name: proc.exe_name,
                exe_path: proc.exe_path,
                cpu_percent: proc_cpu_pct,
                memory_bytes: proc.memory_bytes,
            });
        }

        self.prev_proc_times = next_proc_times;

        reconcile_process_totals(&mut raw_procs, cpu_percent, memory_used_bytes);
        let (top_cpu, top_memory) = attribute_processes(&raw_procs);
        Some(SystemSnapshot {
            cpu_percent,
            memory_percent,
            memory_used_bytes,
            memory_total_bytes,
            top_cpu,
            top_memory,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * MB;

    fn proc_sample(
        pid: u32,
        ppid: u32,
        t: u64,
        exe: &str,
        path: Option<&str>,
        cpu: f64,
        mem: u64,
    ) -> RawProcessSample {
        RawProcessSample {
            pid,
            ppid,
            creation_time: t,
            exe_name: exe.to_string(),
            exe_path: path.map(str::to_string),
            cpu_percent: cpu,
            memory_bytes: mem,
        }
    }

    #[test]
    fn traces_56_claude_sessions_wsl_build_and_steam_game_back_to_real_programs() {
        let mut procs = vec![
            proc_sample(100, 4, 10, "explorer.exe", None, 0.2, 120 * MB),
            proc_sample(200, 100, 20, "WindowsTerminal.exe", None, 0.3, 90 * MB),
            proc_sample(201, 200, 21, "pwsh.exe", None, 0.1, 60 * MB),
            // Root Claude session inside Windows Terminal + 55 child node/bash/rg sessions.
            proc_sample(
                300,
                201,
                30,
                "claude.exe",
                Some(r"C:\Users\me\.local\bin\claude.exe"),
                1.5,
                200 * MB,
            ),
            // WSL2 compiling heavy project.
            proc_sample(400, 4, 40, "vmmemWSL", None, 38.0, 10 * GB),
            proc_sample(401, 201, 41, "wsl.exe", None, 0.4, 40 * MB),
            // Steam launcher + webhelper + a real game under steamapps\common.
            proc_sample(
                500,
                100,
                50,
                "steam.exe",
                Some(r"C:\Program Files (x86)\Steam\steam.exe"),
                0.4,
                180 * MB,
            ),
            proc_sample(
                501,
                500,
                51,
                "steamwebhelper.exe",
                Some(r"C:\Program Files (x86)\Steam\bin\cef\steamwebhelper.exe"),
                0.8,
                320 * MB,
            ),
            proc_sample(
                502,
                500,
                52,
                "Cyberpunk2077.exe",
                Some(
                    r"C:\Program Files (x86)\Steam\steamapps\common\Cyberpunk 2077\bin\x64\Cyberpunk2077.exe",
                ),
                22.0,
                7 * GB,
            ),
        ];

        for i in 0..55u32 {
            let child_exe = if i % 3 == 0 {
                "bash.exe"
            } else if i % 3 == 1 {
                "node.exe"
            } else {
                "rg.exe"
            };
            procs.push(proc_sample(
                1000 + i,
                300,
                100 + u64::from(i),
                child_exe,
                None,
                0.25,
                60 * MB,
            ));
        }

        let (top_cpu, top_mem) = attribute_processes(&procs);
        assert_eq!(top_cpu[0].name, "WSL");
        assert_eq!(top_cpu[0].process_count, 2);
        assert_eq!(top_cpu[1].name, "Cyberpunk 2077");
        assert_eq!(top_cpu[1].process_count, 1);
        assert_eq!(top_cpu[2].name, "Claude");
        assert_eq!(top_cpu[2].process_count, 56);
        assert_eq!(top_cpu[2].display_name(), "Claude (56)");
        assert!(top_cpu[2].cpu_percent > 14.0);

        // Steam helpers rolled into Steam without swallowing Cyberpunk 2077.
        assert!(top_mem.iter().any(|c| c.name == "WSL"));
        assert!(top_mem.iter().any(|c| c.name == "Cyberpunk 2077"));
        let claude_mem = top_mem.iter().find(|c| c.name == "Claude").unwrap();
        assert_eq!(claude_mem.process_count, 56);
        assert!(claude_mem.memory_bytes > 3 * GB);
        let steam_mem = top_mem.iter().find(|c| c.name == "Steam").unwrap();
        assert_eq!(steam_mem.process_count, 2);
    }

    #[test]
    fn claude_inside_cursor_terminal_still_attributes_to_claude() {
        let procs = vec![
            proc_sample(10, 4, 1, "explorer.exe", None, 0.0, 50 * MB),
            proc_sample(20, 10, 2, "Cursor.exe", None, 1.0, 600 * MB),
            proc_sample(21, 20, 3, "node.exe", None, 2.0, 400 * MB), // Cursor's own helper -> Cursor
            proc_sample(22, 20, 4, "pwsh.exe", None, 0.1, 50 * MB),
            proc_sample(23, 22, 5, "claude.exe", None, 1.0, 150 * MB),
            proc_sample(24, 23, 6, "cargo.exe", None, 4.0, 300 * MB),
            proc_sample(25, 24, 7, "rustc.exe", None, 18.0, 900 * MB),
        ];

        let (top_cpu, _) = attribute_processes(&procs);
        assert_eq!(top_cpu[0].name, "Claude");
        assert_eq!(top_cpu[0].process_count, 3); // claude + cargo + rustc
        assert!((top_cpu[0].cpu_percent - 23.0).abs() < 0.01);
        assert_eq!(top_cpu[1].name, "Cursor");
        assert_eq!(top_cpu[1].process_count, 3); // Cursor + node + pwsh
    }

    #[test]
    fn guards_against_recycled_pid_parent_timestamps() {
        // Parent pid 50 started AFTER child pid 60 -> recycled PID, do not attribute to Chrome.
        let procs = vec![
            proc_sample(50, 4, 500, "chrome.exe", None, 1.0, 500 * MB),
            proc_sample(60, 50, 100, "Blender.exe", None, 25.0, 2 * GB),
        ];
        let (top_cpu, _) = attribute_processes(&procs);
        assert_eq!(top_cpu[0].name, "Blender");
    }

    #[test]
    fn formats_bytes_and_memory_summaries_cleanly() {
        assert_eq!(format_bytes_compact(480 * MB), "480 MB");
        assert_eq!(format_bytes_compact(12 * GB + 410 * MB), "12.4 GB");
        assert_eq!(format_bytes_compact(64 * GB), "64 GB");
        assert_eq!(
            format_memory_summary(24 * GB + 614 * MB, 64 * GB),
            "24.6 / 64 GB"
        );
    }

    #[test]
    fn timeout_and_grep_under_claude_roll_up_to_claude() {
        let procs = vec![
            proc_sample(10, 4, 1, "explorer.exe", None, 0.2, 100 * MB),
            proc_sample(
                100,
                10,
                10,
                "claude.exe",
                Some(r"C:\Users\me\.local\bin\claude.exe"),
                12.0,
                4 * GB,
            ),
            proc_sample(101, 100, 11, "bash.exe", None, 1.0, 80 * MB),
            proc_sample(102, 101, 12, "timeout.exe", None, 12.0, 400 * MB),
            proc_sample(103, 102, 13, "grep.exe", None, 6.4, 250 * MB),
            proc_sample(104, 101, 14, "sed.exe", None, 1.2, 40 * MB),
            proc_sample(105, 101, 15, "xargs.exe", None, 0.8, 30 * MB),
            proc_sample(200, 10, 20, "chrome.exe", None, 2.0, GB),
        ];

        let (top_cpu, top_mem) = attribute_processes(&procs);
        assert_eq!(top_cpu[0].name, "Claude");
        assert_eq!(top_cpu[0].process_count, 6);
        assert!((top_cpu[0].cpu_percent - 33.4).abs() < 0.01);
        assert!(!top_cpu
            .iter()
            .any(|c| c.name == "Timeout" || c.name == "Grep"));
        assert!(!top_mem
            .iter()
            .any(|c| c.name == "Timeout" || c.name == "Grep"));
    }

    #[test]
    fn reconcile_process_totals_caps_cpu_and_memory_proportionally() {
        let mut procs = vec![
            proc_sample(10, 4, 1, "claude.exe", None, 60.0, 30 * GB),
            proc_sample(20, 4, 2, "chrome.exe", None, 40.0, 20 * GB),
        ];
        // System reports 75% CPU and 40 GB RAM used -> scale down 100% -> 75% and 50 GB -> 40 GB.
        reconcile_process_totals(&mut procs, 75.0, 40 * GB);
        assert!((procs[0].cpu_percent - 45.0).abs() < 0.01);
        assert!((procs[1].cpu_percent - 30.0).abs() < 0.01);
        assert_eq!(procs[0].memory_bytes, 24 * GB);
        assert_eq!(procs[1].memory_bytes, 16 * GB);

        // When already under system totals, values are preserved unchanged.
        reconcile_process_totals(&mut procs, 80.0, 48 * GB);
        assert!((procs[0].cpu_percent - 45.0).abs() < 0.01);
        assert_eq!(procs[0].memory_bytes, 24 * GB);
    }
}

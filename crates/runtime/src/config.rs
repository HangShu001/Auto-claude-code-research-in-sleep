use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};

use crate::json::JsonValue;
use crate::sandbox::{FilesystemIsolationMode, SandboxConfig};

pub const CLAUDE_CODE_SETTINGS_SCHEMA_NAME: &str = "SettingsSchema";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfigSource {
    User,
    Project,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedPermissionMode {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigEntry {
    pub source: ConfigSource,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    merged: BTreeMap<String, JsonValue>,
    loaded_entries: Vec<ConfigEntry>,
    feature_config: RuntimeFeatureConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RuntimeFeatureConfig {
    hooks: RuntimeHookConfig,
    mcp: McpConfigCollection,
    oauth: Option<OAuthConfig>,
    model: Option<String>,
    permission_mode: Option<ResolvedPermissionMode>,
    sandbox: SandboxConfig,
    meta_logging: String,
}

impl RuntimeFeatureConfig {
    /// Get the meta-logging level string (off/metadata/content).
    #[must_use]
    pub fn meta_logging(&self) -> &str {
        if self.meta_logging.is_empty() {
            "off"
        } else {
            &self.meta_logging
        }
    }

    /// Set meta-logging level.
    pub fn set_meta_logging(&mut self, level: impl Into<String>) {
        self.meta_logging = level.into();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RuntimeHookConfig {
    pre_tool_use: Vec<RuntimeHookSpec>,
    post_tool_use: Vec<RuntimeHookSpec>,
}

/// v0.4.17 Phase 2 (SCHEMA-1): one configured hook command with the Claude
/// Code object-style metadata preserved instead of flattened away.
///
/// * `matcher` lives at the GROUP level in the settings schema
///   (`{matcher, hooks:[...]}`); the parser copies it onto every command in
///   the group. `None` (string-style entry / absent field) and `Some("")`
///   both mean "match every tool" — that is what `aris init` writes
///   (`matcher: ""`).
/// * `timeout_secs` is the per-hook `timeout` field in SECONDS (Claude Code
///   units). `None` means "use the default" (30s); clamping to 1..=600
///   happens at consumption time in `hooks.rs` so the parse stays lossless.
/// * `async_flag` is parsed and stored only — execution is still
///   synchronous in v0.4.17 (known-unsupported, see CHANGELOG).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeHookSpec {
    pub matcher: Option<String>,
    pub command: String,
    pub timeout_secs: Option<u64>,
    pub async_flag: Option<bool>,
}

impl RuntimeHookSpec {
    /// Upgrade a string-style hook entry (bare command string) into a spec:
    /// all metadata absent, command preserved verbatim.
    #[must_use]
    pub fn from_command(command: impl Into<String>) -> Self {
        Self {
            matcher: None,
            command: command.into(),
            timeout_secs: None,
            async_flag: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct McpConfigCollection {
    servers: BTreeMap<String, ScopedMcpServerConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedMcpServerConfig {
    pub scope: ConfigSource,
    pub config: McpServerConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpTransport {
    Stdio,
    Sse,
    Http,
    Ws,
    Sdk,
    ClaudeAiProxy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServerConfig {
    Stdio(McpStdioServerConfig),
    Sse(McpRemoteServerConfig),
    Http(McpRemoteServerConfig),
    Ws(McpWebSocketServerConfig),
    Sdk(McpSdkServerConfig),
    ClaudeAiProxy(McpClaudeAiProxyServerConfig),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpStdioServerConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    /// v0.4.13 P1.D: per-server override for the read timeout used by
    /// `McpStdioProcess::request`. When `Some(secs)`, this takes
    /// precedence over the global `MCP_REQUEST_TIMEOUT_SECS` env and
    /// the 300s default. Clamping to 1..=1800 happens at consumption
    /// time (`mcp_request_timeout` in `mcp_stdio.rs`) so the parse
    /// stays lossless.
    pub request_timeout_secs: Option<u64>,
    /// v0.4.17 (T5): per-server trust flag. `Some(true)` means the
    /// user has pre-approved this server's tools so the CLI's MCP
    /// approval gate can skip the per-call confirmation. `None` (the
    /// default — absent in config) is treated as untrusted. The runtime
    /// only parses/stores it; the consuming approval logic lands with
    /// Track C.
    pub trust: Option<bool>,
}

impl McpStdioServerConfig {
    /// v0.4.17 (T5): whether the user pre-trusted this server. Absent in
    /// config (`None`) is treated as untrusted.
    #[must_use]
    pub fn trust(&self) -> Option<bool> {
        self.trust
    }

    /// Convenience: resolved trust as a plain bool (absent => `false`).
    #[must_use]
    pub fn is_trusted(&self) -> bool {
        self.trust.unwrap_or(false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpRemoteServerConfig {
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub headers_helper: Option<String>,
    pub oauth: Option<McpOAuthConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpWebSocketServerConfig {
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub headers_helper: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpSdkServerConfig {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpClaudeAiProxyServerConfig {
    pub url: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpOAuthConfig {
    pub client_id: Option<String>,
    pub callback_port: Option<u16>,
    pub auth_server_metadata_url: Option<String>,
    pub xaa: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthConfig {
    pub client_id: String,
    pub authorize_url: String,
    pub token_url: String,
    pub callback_port: Option<u16>,
    pub manual_redirect_url: Option<String>,
    pub scopes: Vec<String>,
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(String),
}

impl Display for ConfigError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Parse(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigLoader {
    cwd: PathBuf,
    config_home: PathBuf,
}

impl ConfigLoader {
    #[must_use]
    pub fn new(cwd: impl Into<PathBuf>, config_home: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            config_home: config_home.into(),
        }
    }

    #[must_use]
    pub fn default_for(cwd: impl Into<PathBuf>) -> Self {
        let cwd = cwd.into();
        let config_home = std::env::var_os("CLAUDE_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .or_else(|| std::env::var_os("USERPROFILE"))
                    .map(|home| PathBuf::from(home).join(".claude"))
            })
            .unwrap_or_else(|| PathBuf::from(".claude"));
        Self { cwd, config_home }
    }

    #[must_use]
    pub fn discover(&self) -> Vec<ConfigEntry> {
        let user_legacy_path = self.config_home.parent().map_or_else(
            || PathBuf::from(".claude.json"),
            |parent| parent.join(".claude.json"),
        );
        vec![
            ConfigEntry {
                source: ConfigSource::User,
                path: user_legacy_path,
            },
            ConfigEntry {
                source: ConfigSource::User,
                path: self.config_home.join("settings.json"),
            },
            ConfigEntry {
                source: ConfigSource::Project,
                path: self.cwd.join(".claude.json"),
            },
            ConfigEntry {
                source: ConfigSource::Project,
                path: self.cwd.join(".claude").join("settings.json"),
            },
            ConfigEntry {
                source: ConfigSource::Local,
                path: self.cwd.join(".claude").join("settings.local.json"),
            },
        ]
    }

    pub fn load(&self) -> Result<RuntimeConfig, ConfigError> {
        let mut merged = BTreeMap::new();
        let mut loaded_entries = Vec::new();
        let mut mcp_servers = BTreeMap::new();

        for entry in self.discover() {
            let Some(value) = read_optional_json_object(&entry.path)? else {
                continue;
            };
            merge_mcp_servers(&mut mcp_servers, entry.source, &value, &entry.path)?;
            deep_merge_objects(&mut merged, &value);
            loaded_entries.push(entry);
        }

        let merged_value = JsonValue::Object(merged.clone());

        let feature_config = RuntimeFeatureConfig {
            hooks: parse_optional_hooks_config(&merged_value)?,
            mcp: McpConfigCollection {
                servers: mcp_servers,
            },
            oauth: parse_optional_oauth_config(&merged_value, "merged settings.oauth")?,
            model: parse_optional_model(&merged_value),
            permission_mode: parse_optional_permission_mode(&merged_value)?,
            sandbox: parse_optional_sandbox_config(&merged_value)?,
            meta_logging: String::new(),
        };

        Ok(RuntimeConfig {
            merged,
            loaded_entries,
            feature_config,
        })
    }
}

impl RuntimeConfig {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            merged: BTreeMap::new(),
            loaded_entries: Vec::new(),
            feature_config: RuntimeFeatureConfig::default(),
        }
    }

    #[must_use]
    pub fn merged(&self) -> &BTreeMap<String, JsonValue> {
        &self.merged
    }

    #[must_use]
    pub fn loaded_entries(&self) -> &[ConfigEntry] {
        &self.loaded_entries
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        self.merged.get(key)
    }

    /// `autoCompactEnabled` from the merged settings; only an explicit
    /// `false` turns automatic compaction off.
    #[must_use]
    pub fn auto_compact_enabled(&self) -> bool {
        !matches!(self.get("autoCompactEnabled"), Some(JsonValue::Bool(false)))
    }

    #[must_use]
    pub fn as_json(&self) -> JsonValue {
        JsonValue::Object(self.merged.clone())
    }

    #[must_use]
    pub fn feature_config(&self) -> &RuntimeFeatureConfig {
        &self.feature_config
    }

    #[must_use]
    pub fn mcp(&self) -> &McpConfigCollection {
        &self.feature_config.mcp
    }

    #[must_use]
    pub fn hooks(&self) -> &RuntimeHookConfig {
        &self.feature_config.hooks
    }

    #[must_use]
    pub fn oauth(&self) -> Option<&OAuthConfig> {
        self.feature_config.oauth.as_ref()
    }

    #[must_use]
    pub fn model(&self) -> Option<&str> {
        self.feature_config.model.as_deref()
    }

    #[must_use]
    pub fn permission_mode(&self) -> Option<ResolvedPermissionMode> {
        self.feature_config.permission_mode
    }

    #[must_use]
    pub fn sandbox(&self) -> &SandboxConfig {
        &self.feature_config.sandbox
    }
}

impl RuntimeFeatureConfig {
    #[must_use]
    pub fn with_hooks(mut self, hooks: RuntimeHookConfig) -> Self {
        self.hooks = hooks;
        self
    }

    #[must_use]
    pub fn hooks(&self) -> &RuntimeHookConfig {
        &self.hooks
    }

    #[must_use]
    pub fn mcp(&self) -> &McpConfigCollection {
        &self.mcp
    }

    #[must_use]
    pub fn oauth(&self) -> Option<&OAuthConfig> {
        self.oauth.as_ref()
    }

    #[must_use]
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    #[must_use]
    pub fn permission_mode(&self) -> Option<ResolvedPermissionMode> {
        self.permission_mode
    }

    #[must_use]
    pub fn sandbox(&self) -> &SandboxConfig {
        &self.sandbox
    }
}

impl RuntimeHookConfig {
    #[must_use]
    pub fn new(pre_tool_use: Vec<RuntimeHookSpec>, post_tool_use: Vec<RuntimeHookSpec>) -> Self {
        Self {
            pre_tool_use,
            post_tool_use,
        }
    }

    #[must_use]
    pub fn pre_tool_use(&self) -> &[RuntimeHookSpec] {
        &self.pre_tool_use
    }

    #[must_use]
    pub fn post_tool_use(&self) -> &[RuntimeHookSpec] {
        &self.post_tool_use
    }
}

impl McpConfigCollection {
    #[must_use]
    pub fn servers(&self) -> &BTreeMap<String, ScopedMcpServerConfig> {
        &self.servers
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ScopedMcpServerConfig> {
        self.servers.get(name)
    }
}

impl ScopedMcpServerConfig {
    #[must_use]
    pub fn transport(&self) -> McpTransport {
        self.config.transport()
    }
}

impl McpServerConfig {
    #[must_use]
    pub fn transport(&self) -> McpTransport {
        match self {
            Self::Stdio(_) => McpTransport::Stdio,
            Self::Sse(_) => McpTransport::Sse,
            Self::Http(_) => McpTransport::Http,
            Self::Ws(_) => McpTransport::Ws,
            Self::Sdk(_) => McpTransport::Sdk,
            Self::ClaudeAiProxy(_) => McpTransport::ClaudeAiProxy,
        }
    }
}

fn read_optional_json_object(
    path: &Path,
) -> Result<Option<BTreeMap<String, JsonValue>>, ConfigError> {
    let is_legacy_config = path.file_name().and_then(|name| name.to_str()) == Some(".claude.json");
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ConfigError::Io(error)),
    };

    if contents.trim().is_empty() {
        return Ok(Some(BTreeMap::new()));
    }

    let parsed = match JsonValue::parse(&contents) {
        Ok(parsed) => parsed,
        Err(_error) if is_legacy_config => return Ok(None),
        Err(error) => return Err(ConfigError::Parse(format!("{}: {error}", path.display()))),
    };
    let Some(object) = parsed.as_object() else {
        if is_legacy_config {
            return Ok(None);
        }
        return Err(ConfigError::Parse(format!(
            "{}: top-level settings value must be a JSON object",
            path.display()
        )));
    };
    Ok(Some(object.clone()))
}

fn merge_mcp_servers(
    target: &mut BTreeMap<String, ScopedMcpServerConfig>,
    source: ConfigSource,
    root: &BTreeMap<String, JsonValue>,
    path: &Path,
) -> Result<(), ConfigError> {
    let Some(mcp_servers) = root.get("mcpServers") else {
        return Ok(());
    };
    let servers = expect_object(mcp_servers, &format!("{}: mcpServers", path.display()))?;
    for (name, value) in servers {
        let parsed = parse_mcp_server_config(
            name,
            value,
            &format!("{}: mcpServers.{name}", path.display()),
        )?;
        target.insert(
            name.clone(),
            ScopedMcpServerConfig {
                scope: source,
                config: parsed,
            },
        );
    }
    Ok(())
}

fn parse_optional_model(root: &JsonValue) -> Option<String> {
    root.as_object()
        .and_then(|object| object.get("model"))
        .and_then(JsonValue::as_str)
        .map(ToOwned::to_owned)
}

fn parse_optional_hooks_config(root: &JsonValue) -> Result<RuntimeHookConfig, ConfigError> {
    let Some(object) = root.as_object() else {
        return Ok(RuntimeHookConfig::default());
    };
    let Some(hooks_value) = object.get("hooks") else {
        return Ok(RuntimeHookConfig::default());
    };
    let hooks = expect_object(hooks_value, "merged settings.hooks")?;
    Ok(RuntimeHookConfig {
        pre_tool_use: optional_hook_specs(hooks, "PreToolUse", "merged settings.hooks")?
            .unwrap_or_default(),
        post_tool_use: optional_hook_specs(hooks, "PostToolUse", "merged settings.hooks")?
            .unwrap_or_default(),
    })
}

fn parse_optional_permission_mode(
    root: &JsonValue,
) -> Result<Option<ResolvedPermissionMode>, ConfigError> {
    let Some(object) = root.as_object() else {
        return Ok(None);
    };
    if let Some(mode) = object.get("permissionMode").and_then(JsonValue::as_str) {
        return parse_permission_mode_label(mode, "merged settings.permissionMode").map(Some);
    }
    let Some(mode) = object
        .get("permissions")
        .and_then(JsonValue::as_object)
        .and_then(|permissions| permissions.get("defaultMode"))
        .and_then(JsonValue::as_str)
    else {
        return Ok(None);
    };
    parse_permission_mode_label(mode, "merged settings.permissions.defaultMode").map(Some)
}

fn parse_permission_mode_label(
    mode: &str,
    context: &str,
) -> Result<ResolvedPermissionMode, ConfigError> {
    match mode {
        "default" | "plan" | "read-only" => Ok(ResolvedPermissionMode::ReadOnly),
        "acceptEdits" | "auto" | "workspace-write" => Ok(ResolvedPermissionMode::WorkspaceWrite),
        "dontAsk" | "danger-full-access" => Ok(ResolvedPermissionMode::DangerFullAccess),
        other => Err(ConfigError::Parse(format!(
            "{context}: unsupported permission mode {other}"
        ))),
    }
}

fn parse_optional_sandbox_config(root: &JsonValue) -> Result<SandboxConfig, ConfigError> {
    let Some(object) = root.as_object() else {
        return Ok(SandboxConfig::default());
    };
    let Some(sandbox_value) = object.get("sandbox") else {
        return Ok(SandboxConfig::default());
    };
    let sandbox = expect_object(sandbox_value, "merged settings.sandbox")?;
    let filesystem_mode = optional_string(sandbox, "filesystemMode", "merged settings.sandbox")?
        .map(parse_filesystem_mode_label)
        .transpose()?;
    Ok(SandboxConfig {
        enabled: optional_bool(sandbox, "enabled", "merged settings.sandbox")?,
        namespace_restrictions: optional_bool(
            sandbox,
            "namespaceRestrictions",
            "merged settings.sandbox",
        )?,
        network_isolation: optional_bool(sandbox, "networkIsolation", "merged settings.sandbox")?,
        filesystem_mode,
        allowed_mounts: optional_string_array(sandbox, "allowedMounts", "merged settings.sandbox")?
            .unwrap_or_default(),
        // v0.4.12 #238 — `strictMode: true` hard-locks sandbox policy
        // so LLM tool calls can't bypass via `dangerouslyDisableSandbox`.
        // See SandboxConfig::resolve_request docs.
        strict_mode: optional_bool(sandbox, "strictMode", "merged settings.sandbox")?,
    })
}

fn parse_filesystem_mode_label(value: &str) -> Result<FilesystemIsolationMode, ConfigError> {
    match value {
        "off" => Ok(FilesystemIsolationMode::Off),
        "workspace-only" => Ok(FilesystemIsolationMode::WorkspaceOnly),
        "allow-list" => Ok(FilesystemIsolationMode::AllowList),
        other => Err(ConfigError::Parse(format!(
            "merged settings.sandbox.filesystemMode: unsupported filesystem mode {other}"
        ))),
    }
}

fn parse_optional_oauth_config(
    root: &JsonValue,
    context: &str,
) -> Result<Option<OAuthConfig>, ConfigError> {
    let Some(oauth_value) = root.as_object().and_then(|object| object.get("oauth")) else {
        return Ok(None);
    };
    let object = expect_object(oauth_value, context)?;
    let client_id = expect_string(object, "clientId", context)?.to_string();
    let authorize_url = expect_string(object, "authorizeUrl", context)?.to_string();
    let token_url = expect_string(object, "tokenUrl", context)?.to_string();
    let callback_port = optional_u16(object, "callbackPort", context)?;
    let manual_redirect_url =
        optional_string(object, "manualRedirectUrl", context)?.map(str::to_string);
    let scopes = optional_string_array(object, "scopes", context)?.unwrap_or_default();
    Ok(Some(OAuthConfig {
        client_id,
        authorize_url,
        token_url,
        callback_port,
        manual_redirect_url,
        scopes,
    }))
}

fn parse_mcp_server_config(
    server_name: &str,
    value: &JsonValue,
    context: &str,
) -> Result<McpServerConfig, ConfigError> {
    let object = expect_object(value, context)?;
    let server_type = optional_string(object, "type", context)?.unwrap_or("stdio");
    match server_type {
        "stdio" => Ok(McpServerConfig::Stdio(McpStdioServerConfig {
            command: expect_string(object, "command", context)?.to_string(),
            args: optional_string_array(object, "args", context)?.unwrap_or_default(),
            env: optional_string_map(object, "env", context)?.unwrap_or_default(),
            request_timeout_secs: optional_u64(object, "requestTimeoutSecs", context)?,
            // v0.4.17 (T5): `trust` is a per-server bool. Absent =>
            // `None` (untrusted). A non-bool value is a hard parse
            // error (matches the strictness of every other typed field)
            // so a typo can't silently disable the approval gate.
            trust: optional_bool(object, "trust", context)?,
        })),
        "sse" => Ok(McpServerConfig::Sse(parse_mcp_remote_server_config(
            object, context,
        )?)),
        "http" => Ok(McpServerConfig::Http(parse_mcp_remote_server_config(
            object, context,
        )?)),
        "ws" => Ok(McpServerConfig::Ws(McpWebSocketServerConfig {
            url: expect_string(object, "url", context)?.to_string(),
            headers: optional_string_map(object, "headers", context)?.unwrap_or_default(),
            headers_helper: optional_string(object, "headersHelper", context)?.map(str::to_string),
        })),
        "sdk" => Ok(McpServerConfig::Sdk(McpSdkServerConfig {
            name: expect_string(object, "name", context)?.to_string(),
        })),
        "claudeai-proxy" => Ok(McpServerConfig::ClaudeAiProxy(
            McpClaudeAiProxyServerConfig {
                url: expect_string(object, "url", context)?.to_string(),
                id: expect_string(object, "id", context)?.to_string(),
            },
        )),
        other => Err(ConfigError::Parse(format!(
            "{context}: unsupported MCP server type for {server_name}: {other}"
        ))),
    }
}

fn parse_mcp_remote_server_config(
    object: &BTreeMap<String, JsonValue>,
    context: &str,
) -> Result<McpRemoteServerConfig, ConfigError> {
    Ok(McpRemoteServerConfig {
        url: expect_string(object, "url", context)?.to_string(),
        headers: optional_string_map(object, "headers", context)?.unwrap_or_default(),
        headers_helper: optional_string(object, "headersHelper", context)?.map(str::to_string),
        oauth: parse_optional_mcp_oauth_config(object, context)?,
    })
}

fn parse_optional_mcp_oauth_config(
    object: &BTreeMap<String, JsonValue>,
    context: &str,
) -> Result<Option<McpOAuthConfig>, ConfigError> {
    let Some(value) = object.get("oauth") else {
        return Ok(None);
    };
    let oauth = expect_object(value, &format!("{context}.oauth"))?;
    Ok(Some(McpOAuthConfig {
        client_id: optional_string(oauth, "clientId", context)?.map(str::to_string),
        callback_port: optional_u16(oauth, "callbackPort", context)?,
        auth_server_metadata_url: optional_string(oauth, "authServerMetadataUrl", context)?
            .map(str::to_string),
        xaa: optional_bool(oauth, "xaa", context)?,
    }))
}

fn expect_object<'a>(
    value: &'a JsonValue,
    context: &str,
) -> Result<&'a BTreeMap<String, JsonValue>, ConfigError> {
    value
        .as_object()
        .ok_or_else(|| ConfigError::Parse(format!("{context}: expected JSON object")))
}

fn expect_string<'a>(
    object: &'a BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<&'a str, ConfigError> {
    object
        .get(key)
        .and_then(JsonValue::as_str)
        .ok_or_else(|| ConfigError::Parse(format!("{context}: missing string field {key}")))
}

fn optional_string<'a>(
    object: &'a BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<Option<&'a str>, ConfigError> {
    match object.get(key) {
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| ConfigError::Parse(format!("{context}: field {key} must be a string"))),
        None => Ok(None),
    }
}

fn optional_bool(
    object: &BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<Option<bool>, ConfigError> {
    match object.get(key) {
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| ConfigError::Parse(format!("{context}: field {key} must be a boolean"))),
        None => Ok(None),
    }
}

fn optional_u16(
    object: &BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<Option<u16>, ConfigError> {
    match object.get(key) {
        Some(value) => {
            let Some(number) = value.as_i64() else {
                return Err(ConfigError::Parse(format!(
                    "{context}: field {key} must be an integer"
                )));
            };
            let number = u16::try_from(number).map_err(|_| {
                ConfigError::Parse(format!("{context}: field {key} is out of range"))
            })?;
            Ok(Some(number))
        }
        None => Ok(None),
    }
}

fn optional_u64(
    object: &BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<Option<u64>, ConfigError> {
    match object.get(key) {
        Some(value) => {
            let Some(number) = value.as_i64() else {
                return Err(ConfigError::Parse(format!(
                    "{context}: field {key} must be an integer"
                )));
            };
            let number = u64::try_from(number).map_err(|_| {
                ConfigError::Parse(format!(
                    "{context}: field {key} must be a non-negative integer"
                ))
            })?;
            Ok(Some(number))
        }
        None => Ok(None),
    }
}

fn optional_string_array(
    object: &BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<Option<Vec<String>>, ConfigError> {
    match object.get(key) {
        Some(value) => {
            let Some(array) = value.as_array() else {
                return Err(ConfigError::Parse(format!(
                    "{context}: field {key} must be an array"
                )));
            };
            array
                .iter()
                .map(|item| {
                    item.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                        ConfigError::Parse(format!(
                            "{context}: field {key} must contain only strings"
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Some)
        }
        None => Ok(None),
    }
}

/// Parse one hook event array into [`RuntimeHookSpec`]s.
///
/// v0.4.17 Phase 2 (SCHEMA-2): the parser no longer flattens object-style
/// entries into bare command strings — group-level `matcher` plus per-hook
/// `timeout` / `async` are preserved on each spec. Three pre-existing
/// behaviors are kept verbatim (locked by Phase 0 characterization tests):
///   * string-style entries upgrade to a spec with all metadata `None`;
///   * non-`command` `type` sub-hooks are skipped, an ABSENT `type` is
///     treated as `command`;
///   * unknown event keys are never read (caller only asks for
///     `PreToolUse` / `PostToolUse`), so they stay silently ignored.
///
/// `matcher` / `timeout` / `async` are parsed LENIENTLY (wrong JSON type =>
/// `None`): before Phase 2 these fields were ignored entirely, so a
/// malformed value in an existing settings.json must not turn a config that
/// loaded yesterday into a hard load error today.
fn optional_hook_specs(
    object: &BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<Option<Vec<RuntimeHookSpec>>, ConfigError> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    let Some(array) = value.as_array() else {
        return Err(ConfigError::Parse(format!(
            "{context}: field {key} must be an array"
        )));
    };

    let mut specs = Vec::new();
    for item in array {
        if let Some(command) = item.as_str() {
            specs.push(RuntimeHookSpec::from_command(command));
            continue;
        }

        let item_object = item.as_object().ok_or_else(|| {
            ConfigError::Parse(format!(
                "{context}: field {key} must contain strings or Claude Code hook objects"
            ))
        })?;
        // Group-level matcher: copied onto every command in this group.
        let matcher = item_object
            .get("matcher")
            .and_then(JsonValue::as_str)
            .map(ToOwned::to_owned);
        let Some(nested_hooks) = item_object.get("hooks") else {
            return Err(ConfigError::Parse(format!(
                "{context}: field {key} hook object missing hooks array"
            )));
        };
        let Some(nested_array) = nested_hooks.as_array() else {
            return Err(ConfigError::Parse(format!(
                "{context}: field {key} hook object hooks must be an array"
            )));
        };
        for nested in nested_array {
            let nested_object = nested.as_object().ok_or_else(|| {
                ConfigError::Parse(format!(
                    "{context}: field {key} hook entries must be objects"
                ))
            })?;
            let hook_type = nested_object
                .get("type")
                .and_then(JsonValue::as_str)
                .unwrap_or("command");
            if hook_type != "command" {
                continue;
            }
            let Some(command) = nested_object.get("command").and_then(JsonValue::as_str) else {
                return Err(ConfigError::Parse(format!(
                    "{context}: field {key} command hook missing command string"
                )));
            };
            let timeout_secs = nested_object
                .get("timeout")
                .and_then(JsonValue::as_i64)
                .and_then(|number| u64::try_from(number).ok());
            let async_flag = nested_object.get("async").and_then(JsonValue::as_bool);
            specs.push(RuntimeHookSpec {
                matcher: matcher.clone(),
                command: command.to_string(),
                timeout_secs,
                async_flag,
            });
        }
    }

    Ok(Some(specs))
}

fn optional_string_map(
    object: &BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<Option<BTreeMap<String, String>>, ConfigError> {
    match object.get(key) {
        Some(value) => {
            let Some(map) = value.as_object() else {
                return Err(ConfigError::Parse(format!(
                    "{context}: field {key} must be an object"
                )));
            };
            map.iter()
                .map(|(entry_key, entry_value)| {
                    entry_value
                        .as_str()
                        .map(|text| (entry_key.clone(), text.to_string()))
                        .ok_or_else(|| {
                            ConfigError::Parse(format!(
                                "{context}: field {key} must contain only string values"
                            ))
                        })
                })
                .collect::<Result<BTreeMap<_, _>, _>>()
                .map(Some)
        }
        None => Ok(None),
    }
}

fn deep_merge_objects(
    target: &mut BTreeMap<String, JsonValue>,
    source: &BTreeMap<String, JsonValue>,
) {
    for (key, value) in source {
        match (target.get_mut(key), value) {
            (Some(JsonValue::Object(existing)), JsonValue::Object(incoming)) => {
                deep_merge_objects(existing, incoming);
            }
            _ => {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}

/// `autoCompactEnabled` for the current directory's merged settings. Settings
/// are read when a session (or subagent) starts, so a change applies to the
/// next one. Unreadable settings leave automatic compaction on.
#[must_use]
pub fn auto_compact_enabled_from_settings() -> bool {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| ConfigLoader::default_for(cwd).load().ok())
        .is_none_or(|config| config.auto_compact_enabled())
}

#[cfg(test)]
mod tests {
    use super::{
        parse_mcp_server_config, ConfigLoader, ConfigSource, McpServerConfig, McpTransport,
        ResolvedPermissionMode, RuntimeHookSpec, CLAUDE_CODE_SETTINGS_SCHEMA_NAME,
    };
    use crate::json::JsonValue;
    use crate::sandbox::FilesystemIsolationMode;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Project the command strings out of parsed hook specs — used by tests
    /// that only care about WHICH commands were extracted (the pre-Phase-2
    /// flatten semantics, still locked).
    fn hook_commands(specs: &[RuntimeHookSpec]) -> Vec<&str> {
        specs.iter().map(|spec| spec.command.as_str()).collect()
    }

    fn temp_dir() -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("runtime-config-{nanos}"))
    }

    #[test]
    fn auto_compact_enabled_is_off_only_for_an_explicit_false() {
        for (settings, expected) in [
            (None, true),
            (Some(r#"{"autoCompactEnabled": true}"#), true),
            (Some(r#"{"autoCompactEnabled": false}"#), false),
            (Some(r#"{"model": "opus"}"#), true),
        ] {
            let root = temp_dir().join(format!("compact-{expected}-{}", settings.map_or(0, str::len)));
            let cwd = root.join("project");
            let home = root.join("home").join(".claude");
            fs::create_dir_all(&cwd).expect("project dir");
            fs::create_dir_all(&home).expect("home config dir");
            if let Some(settings) = settings {
                fs::write(home.join("settings.json"), settings).expect("write settings");
            }
            let config = ConfigLoader::new(&cwd, &home).load().expect("load");
            assert_eq!(config.auto_compact_enabled(), expected, "{settings:?}");
            fs::remove_dir_all(root).expect("cleanup");
        }
    }

    #[test]
    fn rejects_non_object_settings_files() {
        let root = temp_dir();
        let cwd = root.join("project");
        let home = root.join("home").join(".claude");
        fs::create_dir_all(&home).expect("home config dir");
        fs::create_dir_all(&cwd).expect("project dir");
        fs::write(home.join("settings.json"), "[]").expect("write bad settings");

        let error = ConfigLoader::new(&cwd, &home)
            .load()
            .expect_err("config should fail");
        assert!(error
            .to_string()
            .contains("top-level settings value must be a JSON object"));

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn loads_and_merges_claude_code_config_files_by_precedence() {
        let root = temp_dir();
        let cwd = root.join("project");
        let home = root.join("home").join(".claude");
        fs::create_dir_all(cwd.join(".claude")).expect("project config dir");
        fs::create_dir_all(&home).expect("home config dir");

        fs::write(
            home.parent().expect("home parent").join(".claude.json"),
            r#"{"model":"haiku","env":{"A":"1"},"mcpServers":{"home":{"command":"uvx","args":["home"]}}}"#,
        )
        .expect("write user compat config");
        fs::write(
            home.join("settings.json"),
            r#"{"model":"sonnet","env":{"A2":"1"},"hooks":{"PreToolUse":["base"]},"permissions":{"defaultMode":"plan"}}"#,
        )
        .expect("write user settings");
        fs::write(
            cwd.join(".claude.json"),
            r#"{"model":"project-compat","env":{"B":"2"}}"#,
        )
        .expect("write project compat config");
        fs::write(
            cwd.join(".claude").join("settings.json"),
            r#"{"env":{"C":"3"},"hooks":{"PostToolUse":["project"]},"mcpServers":{"project":{"command":"uvx","args":["project"]}}}"#,
        )
        .expect("write project settings");
        fs::write(
            cwd.join(".claude").join("settings.local.json"),
            r#"{"model":"opus","permissionMode":"acceptEdits"}"#,
        )
        .expect("write local settings");

        let loaded = ConfigLoader::new(&cwd, &home)
            .load()
            .expect("config should load");

        assert_eq!(CLAUDE_CODE_SETTINGS_SCHEMA_NAME, "SettingsSchema");
        assert_eq!(loaded.loaded_entries().len(), 5);
        assert_eq!(loaded.loaded_entries()[0].source, ConfigSource::User);
        assert_eq!(
            loaded.get("model"),
            Some(&JsonValue::String("opus".to_string()))
        );
        assert_eq!(loaded.model(), Some("opus"));
        assert_eq!(
            loaded.permission_mode(),
            Some(ResolvedPermissionMode::WorkspaceWrite)
        );
        assert_eq!(
            loaded
                .get("env")
                .and_then(JsonValue::as_object)
                .expect("env object")
                .len(),
            4
        );
        assert!(loaded
            .get("hooks")
            .and_then(JsonValue::as_object)
            .expect("hooks object")
            .contains_key("PreToolUse"));
        assert!(loaded
            .get("hooks")
            .and_then(JsonValue::as_object)
            .expect("hooks object")
            .contains_key("PostToolUse"));
        assert_eq!(hook_commands(loaded.hooks().pre_tool_use()), ["base"]);
        assert_eq!(hook_commands(loaded.hooks().post_tool_use()), ["project"]);
        assert!(loaded.mcp().get("home").is_some());
        assert!(loaded.mcp().get("project").is_some());

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn parses_sandbox_config() {
        let root = temp_dir();
        let cwd = root.join("project");
        let home = root.join("home").join(".claude");
        fs::create_dir_all(cwd.join(".claude")).expect("project config dir");
        fs::create_dir_all(&home).expect("home config dir");

        fs::write(
            cwd.join(".claude").join("settings.local.json"),
            r#"{
              "sandbox": {
                "enabled": true,
                "namespaceRestrictions": false,
                "networkIsolation": true,
                "filesystemMode": "allow-list",
                "allowedMounts": ["logs", "tmp/cache"]
              }
            }"#,
        )
        .expect("write local settings");

        let loaded = ConfigLoader::new(&cwd, &home)
            .load()
            .expect("config should load");

        assert_eq!(loaded.sandbox().enabled, Some(true));
        assert_eq!(loaded.sandbox().namespace_restrictions, Some(false));
        assert_eq!(loaded.sandbox().network_isolation, Some(true));
        assert_eq!(
            loaded.sandbox().filesystem_mode,
            Some(FilesystemIsolationMode::AllowList)
        );
        assert_eq!(loaded.sandbox().allowed_mounts, vec!["logs", "tmp/cache"]);

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    // v0.4.13 regression — issue #238. parse_optional_sandbox_config must
    // round-trip the `strictMode` field (camelCase per the merged settings
    // schema) into the new SandboxConfig::strict_mode option, in all three
    // states: true (hard-lock policy), false (explicit non-strict), and
    // missing (default permissive / pre-v0.4.12 behaviour). Each state is
    // exercised in an isolated temp dir to avoid cross-fixture state.
    #[test]
    fn parses_sandbox_strict_mode() {
        fn load_with(local_json: &str) -> crate::sandbox::SandboxConfig {
            let root = temp_dir();
            let cwd = root.join("project");
            let home = root.join("home").join(".claude");
            fs::create_dir_all(cwd.join(".claude")).expect("project config dir");
            fs::create_dir_all(&home).expect("home config dir");
            fs::write(cwd.join(".claude").join("settings.local.json"), local_json)
                .expect("write local settings");
            let loaded = ConfigLoader::new(&cwd, &home)
                .load()
                .expect("config should load");
            let sandbox = loaded.sandbox().clone();
            fs::remove_dir_all(root).expect("cleanup temp dir");
            sandbox
        }

        let strict_true = load_with(r#"{"sandbox":{"enabled":true,"strictMode":true}}"#);
        assert_eq!(strict_true.strict_mode, Some(true));
        assert!(strict_true.is_strict());

        let strict_false = load_with(r#"{"sandbox":{"enabled":true,"strictMode":false}}"#);
        assert_eq!(strict_false.strict_mode, Some(false));
        assert!(!strict_false.is_strict());

        let strict_missing = load_with(r#"{"sandbox":{"enabled":true}}"#);
        assert_eq!(strict_missing.strict_mode, None);
        assert!(!strict_missing.is_strict());
    }

    #[test]
    fn parses_typed_mcp_and_oauth_config() {
        let root = temp_dir();
        let cwd = root.join("project");
        let home = root.join("home").join(".claude");
        fs::create_dir_all(cwd.join(".claude")).expect("project config dir");
        fs::create_dir_all(&home).expect("home config dir");

        fs::write(
            home.join("settings.json"),
            r#"{
              "mcpServers": {
                "stdio-server": {
                  "command": "uvx",
                  "args": ["mcp-server"],
                  "env": {"TOKEN": "secret"}
                },
                "remote-server": {
                  "type": "http",
                  "url": "https://example.test/mcp",
                  "headers": {"Authorization": "Bearer token"},
                  "headersHelper": "helper.sh",
                  "oauth": {
                    "clientId": "mcp-client",
                    "callbackPort": 7777,
                    "authServerMetadataUrl": "https://issuer.test/.well-known/oauth-authorization-server",
                    "xaa": true
                  }
                }
              },
              "oauth": {
                "clientId": "runtime-client",
                "authorizeUrl": "https://console.test/oauth/authorize",
                "tokenUrl": "https://console.test/oauth/token",
                "callbackPort": 54545,
                "manualRedirectUrl": "https://console.test/oauth/callback",
                "scopes": ["org:read", "user:write"]
              }
            }"#,
        )
        .expect("write user settings");
        fs::write(
            cwd.join(".claude").join("settings.local.json"),
            r#"{
              "mcpServers": {
                "remote-server": {
                  "type": "ws",
                  "url": "wss://override.test/mcp",
                  "headers": {"X-Env": "local"}
                }
              }
            }"#,
        )
        .expect("write local settings");

        let loaded = ConfigLoader::new(&cwd, &home)
            .load()
            .expect("config should load");

        let stdio_server = loaded
            .mcp()
            .get("stdio-server")
            .expect("stdio server should exist");
        assert_eq!(stdio_server.scope, ConfigSource::User);
        assert_eq!(stdio_server.transport(), McpTransport::Stdio);

        let remote_server = loaded
            .mcp()
            .get("remote-server")
            .expect("remote server should exist");
        assert_eq!(remote_server.scope, ConfigSource::Local);
        assert_eq!(remote_server.transport(), McpTransport::Ws);
        match &remote_server.config {
            McpServerConfig::Ws(config) => {
                assert_eq!(config.url, "wss://override.test/mcp");
                assert_eq!(
                    config.headers.get("X-Env").map(String::as_str),
                    Some("local")
                );
            }
            other => panic!("expected ws config, got {other:?}"),
        }

        let oauth = loaded.oauth().expect("oauth config should exist");
        assert_eq!(oauth.client_id, "runtime-client");
        assert_eq!(oauth.callback_port, Some(54_545));
        assert_eq!(oauth.scopes, vec!["org:read", "user:write"]);

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn rejects_invalid_mcp_server_shapes() {
        let root = temp_dir();
        let cwd = root.join("project");
        let home = root.join("home").join(".claude");
        fs::create_dir_all(&home).expect("home config dir");
        fs::create_dir_all(&cwd).expect("project dir");
        fs::write(
            home.join("settings.json"),
            r#"{"mcpServers":{"broken":{"type":"http","url":123}}}"#,
        )
        .expect("write broken settings");

        let error = ConfigLoader::new(&cwd, &home)
            .load()
            .expect_err("config should fail");
        assert!(error
            .to_string()
            .contains("mcpServers.broken: missing string field url"));

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn loads_object_style_claude_code_hooks_as_commands() {
        let root = temp_dir();
        let cwd = root.join("project");
        let home = root.join("home").join(".claude");
        fs::create_dir_all(cwd.join(".claude")).expect("project config dir");
        fs::create_dir_all(&home).expect("home config dir");

        fs::write(
            cwd.join(".claude").join("settings.json"),
            r#"{
              "hooks": {
                "PreToolUse": [
                  {
                    "matcher": "Bash",
                    "hooks": [
                      {
                        "type": "command",
                        "command": "echo pre",
                        "timeout": 5,
                        "async": true
                      }
                    ]
                  }
                ],
                "PostToolUse": [
                  {
                    "matcher": "",
                    "hooks": [
                      {
                        "type": "command",
                        "command": "echo post"
                      }
                    ]
                  }
                ]
              }
            }"#,
        )
        .expect("write object-style hooks settings");

        let loaded = ConfigLoader::new(&cwd, &home)
            .load()
            .expect("object-style hooks should load");

        assert_eq!(hook_commands(loaded.hooks().pre_tool_use()), ["echo pre"]);
        assert_eq!(
            hook_commands(loaded.hooks().post_tool_use()),
            ["echo post"]
        );
        // v0.4.17 Phase 2 (SCHEMA-1/2): the object-style metadata is now
        // preserved alongside the command.
        let pre = loaded.hooks().pre_tool_use();
        assert_eq!(pre[0].matcher.as_deref(), Some("Bash"));
        assert_eq!(pre[0].timeout_secs, Some(5));
        assert_eq!(pre[0].async_flag, Some(true));
        let post = loaded.hooks().post_tool_use();
        assert_eq!(post[0].matcher.as_deref(), Some(""));
        assert_eq!(post[0].timeout_secs, None);
        assert_eq!(post[0].async_flag, None);

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    // ---------------------------------------------------------------
    // v0.4.17 Phase 0 — CHARACTERIZATION TESTS (hooks parse truth table)
    //
    // Phase 2 (SCHEMA-1/2) replaced the flatten-to-`Vec<String>` parse with
    // `RuntimeHookSpec`, which retains matcher/timeout/async. The command-
    // extraction semantics locked here are UNCHANGED (string-style verbatim,
    // non-command type skipped, unknown events ignored); only the
    // field-dropping test was deliberately flipped (annotated below).
    // ---------------------------------------------------------------

    /// String-style hook arrays round-trip verbatim into the command list.
    #[test]
    fn char_string_style_hooks_round_trip_verbatim() {
        let root = temp_dir();
        let cwd = root.join("project");
        let home = root.join("home").join(".claude");
        fs::create_dir_all(&home).expect("home config dir");
        fs::create_dir_all(&cwd).expect("project dir");

        fs::write(
            home.join("settings.json"),
            r#"{"hooks":{"PreToolUse":["echo a","echo b"],"PostToolUse":["echo c"]}}"#,
        )
        .expect("write string-style hooks");

        let loaded = ConfigLoader::new(&cwd, &home)
            .load()
            .expect("string-style hooks should load");

        assert_eq!(
            hook_commands(loaded.hooks().pre_tool_use()),
            ["echo a", "echo b"]
        );
        assert_eq!(hook_commands(loaded.hooks().post_tool_use()), ["echo c"]);
        // String-style entries carry no metadata: all fields stay None.
        assert!(loaded
            .hooks()
            .pre_tool_use()
            .iter()
            .all(|spec| spec.matcher.is_none()
                && spec.timeout_secs.is_none()
                && spec.async_flag.is_none()));

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    /// deliberately flipped in v0.4.17 Phase 2: schema now preserves
    /// matcher/timeout. The pre-Phase-2 parser flattened object-style
    /// entries to bare command strings, DROPPING the group matcher and the
    /// per-hook timeouts; the same fixture now parses into specs that keep
    /// the group matcher on every command plus each command's own timeout.
    /// Command extraction order is unchanged.
    #[test]
    fn char_object_style_hooks_drop_matcher_and_timeout_fields() {
        let root = temp_dir();
        let cwd = root.join("project");
        let home = root.join("home").join(".claude");
        fs::create_dir_all(&home).expect("home config dir");
        fs::create_dir_all(&cwd).expect("project dir");

        fs::write(
            home.join("settings.json"),
            r#"{
              "hooks": {
                "PreToolUse": [
                  {
                    "matcher": "Bash|Edit",
                    "hooks": [
                      { "type": "command", "command": "first", "timeout": 42 },
                      { "type": "command", "command": "second", "timeout": 7 }
                    ]
                  }
                ]
              }
            }"#,
        )
        .expect("write object-style hooks");

        let loaded = ConfigLoader::new(&cwd, &home)
            .load()
            .expect("object-style hooks should load");

        // Commands still extract in declaration order...
        let pre = loaded.hooks().pre_tool_use();
        assert_eq!(hook_commands(pre), ["first", "second"]);
        // ...and (flipped) matcher "Bash|Edit" is preserved on BOTH specs
        // (group-level matcher copied onto every command in the group),
        // with each spec keeping its own timeout.
        assert_eq!(pre[0].matcher.as_deref(), Some("Bash|Edit"));
        assert_eq!(pre[0].timeout_secs, Some(42));
        assert_eq!(pre[1].matcher.as_deref(), Some("Bash|Edit"));
        assert_eq!(pre[1].timeout_secs, Some(7));

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    /// Non-`command` `type` sub-hooks are silently skipped; the default
    /// `type` (absent) is treated AS `command`. Locks both branches of
    /// `optional_hook_specs`'s type handling.
    #[test]
    fn char_non_command_hook_type_skipped_absent_type_kept() {
        let root = temp_dir();
        let cwd = root.join("project");
        let home = root.join("home").join(".claude");
        fs::create_dir_all(&home).expect("home config dir");
        fs::create_dir_all(&cwd).expect("project dir");

        fs::write(
            home.join("settings.json"),
            r#"{
              "hooks": {
                "PostToolUse": [
                  {
                    "matcher": "",
                    "hooks": [
                      { "type": "notification", "command": "ignored-non-command" },
                      { "command": "kept-default-type" },
                      { "type": "command", "command": "kept-explicit" }
                    ]
                  }
                ]
              }
            }"#,
        )
        .expect("write mixed-type hooks");

        let loaded = ConfigLoader::new(&cwd, &home)
            .load()
            .expect("mixed-type hooks should load");

        // The `notification`-type entry is dropped; the type-absent entry is
        // treated as a command and kept.
        assert_eq!(
            hook_commands(loaded.hooks().post_tool_use()),
            ["kept-default-type", "kept-explicit"]
        );

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    /// Unknown event names (anything other than PreToolUse / PostToolUse)
    /// are silently ignored: the parser only reads those two keys, so a
    /// config with ONLY unknown events yields empty command lists and still
    /// loads successfully (no error).
    #[test]
    fn char_unknown_hook_event_names_silently_ignored() {
        let root = temp_dir();
        let cwd = root.join("project");
        let home = root.join("home").join(".claude");
        fs::create_dir_all(&home).expect("home config dir");
        fs::create_dir_all(&cwd).expect("project dir");

        fs::write(
            home.join("settings.json"),
            r#"{
              "hooks": {
                "SessionStart": ["echo start"],
                "UserPromptSubmit": ["echo prompt"],
                "PostToolUseFailure": [
                  { "matcher": "", "hooks": [ { "type": "command", "command": "echo fail" } ] }
                ]
              }
            }"#,
        )
        .expect("write unknown-event hooks");

        let loaded = ConfigLoader::new(&cwd, &home)
            .load()
            .expect("unknown-event hooks must still load without error");

        assert!(
            loaded.hooks().pre_tool_use().is_empty(),
            "unknown events must not populate PreToolUse"
        );
        assert!(
            loaded.hooks().post_tool_use().is_empty(),
            "unknown events must not populate PostToolUse"
        );

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    /// v0.4.17 Phase 2 (SCHEMA-2): matcher/timeout/async are parsed
    /// LENIENTLY — a wrong JSON type degrades to `None` instead of failing
    /// the whole config load. Pre-Phase-2 these fields were ignored
    /// entirely, so a settings.json that loaded yesterday (even with, say,
    /// `"timeout": "fast"`) must still load today.
    #[test]
    fn hook_metadata_with_wrong_json_types_degrades_to_none_not_load_error() {
        let root = temp_dir();
        let cwd = root.join("project");
        let home = root.join("home").join(".claude");
        fs::create_dir_all(&home).expect("home config dir");
        fs::create_dir_all(&cwd).expect("project dir");

        fs::write(
            home.join("settings.json"),
            r#"{
              "hooks": {
                "PreToolUse": [
                  {
                    "matcher": 42,
                    "hooks": [
                      {
                        "type": "command",
                        "command": "echo lenient",
                        "timeout": "fast",
                        "async": "yes"
                      },
                      { "type": "command", "command": "echo negative", "timeout": -5 }
                    ]
                  }
                ]
              }
            }"#,
        )
        .expect("write malformed-metadata hooks");

        let loaded = ConfigLoader::new(&cwd, &home)
            .load()
            .expect("malformed hook metadata must not fail the config load");

        let pre = loaded.hooks().pre_tool_use();
        assert_eq!(hook_commands(pre), ["echo lenient", "echo negative"]);
        assert_eq!(pre[0].matcher, None, "non-string matcher degrades to None");
        assert_eq!(
            pre[0].timeout_secs, None,
            "non-integer timeout degrades to None (default applies)"
        );
        assert_eq!(pre[0].async_flag, None, "non-bool async degrades to None");
        assert_eq!(
            pre[1].timeout_secs, None,
            "negative timeout degrades to None (default applies)"
        );

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn parses_per_server_mcp_request_timeout() {
        // v0.4.13 P1.D: `requestTimeoutSecs` on a stdio MCP server
        // entry should round-trip into
        // `McpStdioServerConfig.request_timeout_secs`. Entries
        // without it must leave the field `None` so the global env /
        // 300s default still applies.
        let root = temp_dir();
        let cwd = root.join("project");
        let home = root.join("home").join(".claude");
        fs::create_dir_all(cwd.join(".claude")).expect("project config dir");
        fs::create_dir_all(&home).expect("home config dir");

        fs::write(
            home.join("settings.json"),
            r#"{
              "mcpServers": {
                "fast": {
                  "command": "uvx",
                  "args": ["fast-mcp"]
                },
                "slow-agent": {
                  "command": "codex",
                  "args": ["mcp-server"],
                  "requestTimeoutSecs": 900
                }
              }
            }"#,
        )
        .expect("write settings");

        let loaded = ConfigLoader::new(&cwd, &home)
            .load()
            .expect("config should load");

        let fast = loaded.mcp().get("fast").expect("fast server");
        match &fast.config {
            McpServerConfig::Stdio(stdio) => {
                assert_eq!(
                    stdio.request_timeout_secs, None,
                    "absent requestTimeoutSecs should round-trip to None"
                );
            }
            other => panic!("expected stdio config, got {other:?}"),
        }

        let slow = loaded.mcp().get("slow-agent").expect("slow-agent server");
        match &slow.config {
            McpServerConfig::Stdio(stdio) => {
                assert_eq!(stdio.request_timeout_secs, Some(900));
            }
            other => panic!("expected stdio config, got {other:?}"),
        }

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn parses_per_server_mcp_trust_flag() {
        // v0.4.17 (T5): `trust` on a stdio MCP server entry round-trips
        // into `McpStdioServerConfig.trust`. Absent => `None`
        // (untrusted); `true`/`false` round-trip; a non-bool value is a
        // hard parse error so a typo can't silently disable the
        // approval gate.
        let root = temp_dir();
        let cwd = root.join("project");
        let home = root.join("home").join(".claude");
        fs::create_dir_all(cwd.join(".claude")).expect("project config dir");
        fs::create_dir_all(&home).expect("home config dir");

        fs::write(
            home.join("settings.json"),
            r#"{
              "mcpServers": {
                "untrusted": {
                  "command": "uvx",
                  "args": ["fast-mcp"]
                },
                "trusted": {
                  "command": "codex",
                  "args": ["mcp-server"],
                  "trust": true
                },
                "explicit-false": {
                  "command": "tool",
                  "args": [],
                  "trust": false
                }
              }
            }"#,
        )
        .expect("write settings");

        let loaded = ConfigLoader::new(&cwd, &home)
            .load()
            .expect("config should load");

        match &loaded.mcp().get("untrusted").expect("untrusted").config {
            McpServerConfig::Stdio(stdio) => {
                assert_eq!(stdio.trust, None, "absent trust => None");
                assert!(!stdio.is_trusted(), "absent trust resolves to false");
            }
            other => panic!("expected stdio config, got {other:?}"),
        }
        match &loaded.mcp().get("trusted").expect("trusted").config {
            McpServerConfig::Stdio(stdio) => {
                assert_eq!(stdio.trust(), Some(true));
                assert!(stdio.is_trusted());
            }
            other => panic!("expected stdio config, got {other:?}"),
        }
        match &loaded
            .mcp()
            .get("explicit-false")
            .expect("explicit-false")
            .config
        {
            McpServerConfig::Stdio(stdio) => {
                assert_eq!(stdio.trust(), Some(false));
                assert!(!stdio.is_trusted());
            }
            other => panic!("expected stdio config, got {other:?}"),
        }

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn rejects_non_bool_mcp_trust_flag() {
        // A non-boolean `trust` must be a hard parse error, not a
        // silently-ignored field.
        let value = JsonValue::parse(
            r#"{"command": "tool", "args": [], "trust": "yes"}"#,
        )
        .expect("test json should parse");
        let err = parse_mcp_server_config("bad", &value, "test")
            .expect_err("non-bool trust should fail to parse");
        let message = err.to_string();
        assert!(
            message.contains("trust") && message.contains("boolean"),
            "unexpected error: {message}"
        );
    }
}

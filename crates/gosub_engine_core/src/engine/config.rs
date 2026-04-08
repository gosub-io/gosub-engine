//! Engine configuration.
//!
//! The [`EngineConfig`] struct defines global configuration for the
//! Gosub engine. It controls engine-wide resources, networking,
//! rendering, scripting, and security policies. While [`ZoneConfig`]
//! applies to a single zone, `EngineConfig` governs the entire engine
//! instance.
//!
//! Use [`EngineConfig::default()`] for sensible defaults, or
//! [`EngineConfig::builder()`] for a fluent builder API with
//! validation.
//!
//! # Examples
//!
//! ## Default engine configuration
//! ```rust
//! use gosub_engine::EngineConfig;
//!
//! let engine_cfg = EngineConfig::default();
//! assert_eq!(engine_cfg.max_zones, 8);
//! ```
//!
//! ## Customized configuration with builder
//! ```rust
//! use std::time::Duration;
//! use gosub_engine::EngineConfig;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let cfg = EngineConfig::builder()
//!     .user_agent("Gosub/0.2 (+https://gosub.dev)")
//!     .max_zones(12)
//!     .connect_timeout(Duration::from_secs(5))
//!     .redirect_policy(gosub_engine::config::RedirectPolicy::Follow(5))
//!     .gpu(gosub_engine::config::GpuOptions {
//!         prefer_low_power: false,
//!         msaa_samples: 4,
//!         vsync: true,
//!         use_srgb_framebuffer: true
//!     })
//!     .javascript_enabled(true)
//!     .lua_enabled(false)
//!     .build()?;
//! # Ok(()) }
//! ```
//!
//! # Field categories
//!
//! - **Zones**
//!   - `max_zones`: Maximum number of zones per engine.
//!   - `default_zone_config`: Zone defaults if no config is supplied.
//!
//! - **Concurrency**
//!   - `worker_threads`: Engine thread-pool size.
//!   - `io_concurrency`: Max concurrent network/disk tasks.
//!   - `script_concurrency`: Max concurrent JS/WASM tasks.
//!
//! - **Networking**
//!   - `user_agent`: Default UA string.
//!   - `connect_timeout`, `request_timeout`: Timeouts.
//!   - `redirect_policy`: Redirect handling.
//!   - `http2`: Enable HTTP/2.
//!   - `max_connections_per_host`: Connection cap per host.
//!   - `proxy`: Optional [`ProxyConfig`].
//!   - `tls`: [`TlsConfig`] (roots, client certs, HTTP/3).
//!
//! - **Cache & storage**
//!   - `disk_cache_dir`, `disk_cache_bytes`: On-disk cache.
//!   - `memory_cache_bytes`: In-memory cache size.
//!   - `storage_root`: Root for per-zone storage (localStorage, IndexedDB…).
//!   - `quota_per_zone_bytes`: Per-zone storage cap.
//!   - `persist_cookies`: Save cookies to disk.
//!   - `cookie_jar_partitioning`: [`CookiePartitioning`] policy.
//!
//! - **Security / privacy**
//!   - `sandbox_mode`: [`SandboxMode`] for zones.
//!   - `cors_enforcement`: Enforce CORS.
//!   - `disable_networking`: Disable networking completely.
//!   - `blocked_domains`, `allowlist_domains`: Domain filters.
//!
//! - **Rendering**
//!   - `gpu`: [`GpuOptions`] (MSAA, vsync, etc.).
//!   - `target_fps`: Limit FPS, or `None` for uncapped.
//!   - `pixel_snap`: Align to pixels for sharper text.
//!
//! - **Fonts**
//!   - `font_search_paths`: Extra font directories.
//!   - `fallback_fonts`: Font fallback list.
//!   - `font_cache_bytes`: Font cache cap.
//!
//! - **Scripting**
//!   - `javascript_enabled`: Enable JS engine.
//!   - `lua_enabled`: Enable Lua scripting.
//!   - `wasm_enabled`: Enable WASM execution.
//!   - `max_script_cpu_ms_per_frame`: Script budget per frame.
//!
//! - **Telemetry / logging**
//!   - `log_level`: [`LogLevel`] verbosity.
//!   - `metrics_enabled`: Collect metrics.
//!   - `trace_enabled`: Collect tracing spans.
//!
//! # Notes
//!
//! Note that most of these fields are not implemented but are here to show
//! the intended design. The actual implementation may change without notice.
//!
//! # Errors
//!
//! Builder validation may return [`EngineConfigError`] if values are
//! nonsensical (e.g. `max_zones == 0`, invalid MSAA samples, zero
//! timeouts).
//!
//! # See also
//!
//! - [`ZoneConfig`] for per-zone settings.

use std::{fmt, path::PathBuf, time::Duration};

use crate::zone::ZoneConfig;
// adjust path if needed

/// Redirect handling policy for the engine.
#[derive(Debug, Clone)]
pub enum RedirectPolicy {
    /// Follow up to N redirects (engine-wide cap).
    Follow(u8),
    /// Treat redirects as an error.
    Error,
    /// Expose redirects to the caller; do nothing automatically.
    Manual,
}

/// Proxy configuration for the engine.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Proxy URL for HTTP requests
    pub http: Option<String>, // e.g. "http://user:pass@host:port"
    /// Proxy URL for HTTPS requests
    pub https: Option<String>, // e.g. "https://user:pass@host:port"
    /// Proxy URL for SOCKS5 (TCP) requests
    pub socks5: Option<String>, // e.g. "socks5://host:1080"
    /// Domains to bypass the proxy for (exact match)
    pub no_proxy: Vec<String>, // domains to bypass proxy
}

/// TLS configuration settings
#[derive(Debug, Clone)]
pub struct TlsConfig {
    /// Whether to use the system root certificates
    pub use_system_roots: bool,
    /// Additional root certificates in PEM format
    pub extra_roots_pem: Vec<u8>, // concatenated PEM
    /// Optional client certificate in PKCS#12 / PFX format
    pub client_cert_pfx: Option<Vec<u8>>, // PKCS#12 / PFX bytes
    /// Optional password for the client certificate
    pub client_cert_password: Option<String>,
    /// Whether to enable HTTP/3 support (if the backend supports it)
    pub enable_http3: bool,
}

/// Cookie partitioning mode
#[derive(Debug, Clone)]
pub enum CookiePartitioning {
    /// No partitioning; all cookies shared globally.
    Disabled,
    /// Partition cookies by top-level site (default).
    TopLevel,
    /// Partition cookies by strict site (full URL origin).
    StrictSite,
}

/// GPU Rendering backend config (if applicable for the backend renderer)
#[derive(Debug, Clone)]
pub struct GpuOptions {
    /// Whether to prefer low-power GPUs (e.g. integrated) on multi-GPU systems.
    pub prefer_low_power: bool,
    /// Number of MSAA samples for anti-aliasing.
    pub msaa_samples: u32, // {1,2,4,8}
    /// Whether to enable vsync (may affect latency).
    pub vsync: bool,
    /// Whether to use an sRGB framebuffer (if supported).
    pub use_srgb_framebuffer: bool,
}

/// Log verbosity for the engine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

/// Overall engine configuration (engine-wide knobs).
///
/// Use [`EngineConfig::default()`] for sensible defaults, or
/// [`EngineConfig::builder()`] to customize with validation.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Maximum number of zones that can be created within this engine.
    pub max_zones: usize,
    /// Default zone configuration used when creating zones without an explicit config.
    pub default_zone_config: ZoneConfig,

    // --- threads / concurrency ---
    /// Number of worker threads for the engine's thread pool (default: num_cpus::get().max(2)).
    pub worker_threads: usize,
    /// Number of concurrent I/O tasks (e.g. network, disk).
    pub io_concurrency: usize,
    /// Number of concurrent script tasks (e.g. JS, WASM).
    pub script_concurrency: usize,

    // --- networking / HTTP ---
    /// User agent string used for outgoing HTTP requests (default is Gosub-UA).
    pub user_agent: String,
    /// Connection timeout duration.
    pub connect_timeout: Duration,
    /// Overall request timeout duration.
    pub request_timeout: Duration,
    /// Redirect handling policy.
    pub redirect_policy: RedirectPolicy,
    /// Whether to enable HTTP/2 support.
    pub http2: bool,
    /// Maximum simultaneous connections per host.
    pub max_connections_per_host: u32,
    /// Optional proxy configuration.
    pub proxy: Option<ProxyConfig>,
    /// TLS configuration.
    pub tls: TlsConfig,

    // --- cache / storage ---
    /// (disk cache is shared across zones; storage is per-zone)
    pub disk_cache_dir: PathBuf,
    /// Maximum disk cache size in bytes.
    pub disk_cache_bytes: u64,
    /// Maximum memory cache size in bytes.
    pub memory_cache_bytes: u64,
    /// Root directory for per-zone storage (IndexedDB, localStorage, etc).
    pub storage_root: PathBuf,
    /// Maximum storage quota per zone in bytes.
    pub quota_per_zone_bytes: u64,
    /// Whether to persist cookies to disk (in storage_root).
    pub persist_cookies: bool,
    /// Cookie partitioning mode.
    pub cookie_jar_partitioning: CookiePartitioning,

    // --- security / privacy ---
    /// Sandboxing mode for zones (network, filesystem, etc).
    pub sandbox_mode: SandboxMode,
    /// Whether to enforce CORS policies.
    pub cors_enforcement: bool,
    /// Whether to disable all networking (for testing).
    pub disable_networking: bool,
    /// List of blocked domains (exact match).
    pub blocked_domains: Vec<String>,
    /// List of allowlisted domains (exact match).
    pub allowlist_domains: Vec<String>,

    // --- rendering ---
    /// GPU Options (if applicable for the chosen backend)
    pub gpu: GpuOptions,
    /// FPS target for rendering (None = uncapped).
    pub target_fps: Option<u16>,
    /// Pixel snapping for sharper text (if supported by backend).
    pub pixel_snap: bool,

    // --- fonts ---
    /// List of additional font search paths.
    pub font_search_paths: Vec<PathBuf>,
    /// List of fallback font family names (e.g. ["Inter", "Noto Sans"]).
    pub fallback_fonts: Vec<String>,
    /// Maximum font cache size in bytes.
    pub font_cache_bytes: u64,

    // --- scripting ---
    /// Whether to enable JavaScript execution.
    pub javascript_enabled: bool,
    /// Whether to enable Lua scripting.
    pub lua_enabled: bool,
    /// Whether to enable WebAssembly execution.
    pub wasm_enabled: bool,
    /// Maximum CPU time for scripts per frame in milliseconds.
    pub max_script_cpu_ms_per_frame: u32,

    // --- telemetry / logging ---
    /// Logging verbosity level.
    pub log_level: LogLevel,
    /// Whether to enable metrics
    pub metrics_enabled: bool,
    /// Whether to enable tracing
    pub trace_enabled: bool,
}

#[derive(Debug, Clone)]
pub enum SandboxMode {
    Off,
    Balanced,
    Strict,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            user_agent: "Gosub/0.1 (+https://gosub.dev)".to_owned(),
            max_zones: 8,
            default_zone_config: ZoneConfig::default(),

            worker_threads: num_cpus::get().max(2),
            io_concurrency: 64,
            script_concurrency: 8,

            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            redirect_policy: RedirectPolicy::Follow(10),
            http2: true,
            max_connections_per_host: 6,
            proxy: None,
            tls: TlsConfig {
                use_system_roots: true,
                extra_roots_pem: Vec::new(),
                client_cert_pfx: None,
                client_cert_password: None,
                enable_http3: false,
            },

            disk_cache_dir: std::env::temp_dir().join("gosub-cache"),
            disk_cache_bytes: 512 * 1024 * 1024, // 512 MB
            memory_cache_bytes: 128 * 1024 * 1024,
            storage_root: std::env::temp_dir().join("gosub-storage"),
            quota_per_zone_bytes: 256 * 1024 * 1024,
            persist_cookies: true,
            cookie_jar_partitioning: CookiePartitioning::TopLevel,

            sandbox_mode: SandboxMode::Balanced,
            cors_enforcement: true,
            disable_networking: false,
            blocked_domains: Vec::new(),
            allowlist_domains: Vec::new(),

            gpu: GpuOptions {
                prefer_low_power: false,
                msaa_samples: 1,
                vsync: true,
                use_srgb_framebuffer: true,
            },
            target_fps: None,
            pixel_snap: true,

            font_search_paths: Vec::new(),
            fallback_fonts: vec!["Inter".into(), "Noto Sans".into()],
            font_cache_bytes: 64 * 1024 * 1024,

            javascript_enabled: true,
            lua_enabled: true,
            wasm_enabled: true,
            max_script_cpu_ms_per_frame: 8,

            log_level: LogLevel::Info,
            metrics_enabled: false,
            trace_enabled: false,
        }
    }
}

impl EngineConfig {
    /// Start building an `EngineConfig` from defaults using a fluent builder.
    pub fn builder() -> EngineConfigBuilder {
        EngineConfigBuilder::default()
    }
}

/// Fluent builder for [`EngineConfig`] with validation.
#[derive(Debug, Clone)]
pub struct EngineConfigBuilder {
    inner: EngineConfig,
}

impl Default for EngineConfigBuilder {
    fn default() -> Self {
        Self {
            inner: EngineConfig::default(),
        }
    }
}

impl EngineConfigBuilder {
    #[inline]
    fn map(mut self, f: impl FnOnce(&mut EngineConfig)) -> Self {
        f(&mut self.inner);
        self
    }

    pub fn user_agent<S: Into<String>>(self, ua: S) -> Self {
        self.map(|c| c.user_agent = ua.into())
    }
    pub fn max_zones(self, n: usize) -> Self {
        self.map(|c| c.max_zones = n)
    }
    pub fn default_zone_config(self, z: ZoneConfig) -> Self {
        self.map(|c| c.default_zone_config = z)
    }

    pub fn worker_threads(self, n: usize) -> Self {
        self.map(|c| c.worker_threads = n)
    }
    pub fn io_concurrency(self, n: usize) -> Self {
        self.map(|c| c.io_concurrency = n)
    }
    pub fn script_concurrency(self, n: usize) -> Self {
        self.map(|c| c.script_concurrency = n)
    }

    pub fn connect_timeout(self, d: Duration) -> Self {
        self.map(|c| c.connect_timeout = d)
    }
    pub fn request_timeout(self, d: Duration) -> Self {
        self.map(|c| c.request_timeout = d)
    }
    pub fn redirect_policy(self, p: RedirectPolicy) -> Self {
        self.map(|c| c.redirect_policy = p)
    }
    pub fn http2(self, on: bool) -> Self {
        self.map(|c| c.http2 = on)
    }
    pub fn max_connections_per_host(self, n: u32) -> Self {
        self.map(|c| c.max_connections_per_host = n)
    }
    pub fn proxy(self, p: ProxyConfig) -> Self {
        self.map(|c| c.proxy = Some(p))
    }
    pub fn tls(self, t: TlsConfig) -> Self {
        self.map(|c| c.tls = t)
    }

    pub fn disk_cache_dir<P: Into<PathBuf>>(self, p: P) -> Self {
        self.map(|c| c.disk_cache_dir = p.into())
    }
    pub fn disk_cache_bytes(self, n: u64) -> Self {
        self.map(|c| c.disk_cache_bytes = n)
    }
    pub fn memory_cache_bytes(self, n: u64) -> Self {
        self.map(|c| c.memory_cache_bytes = n)
    }
    pub fn storage_root<P: Into<PathBuf>>(self, p: P) -> Self {
        self.map(|c| c.storage_root = p.into())
    }
    pub fn quota_per_zone_bytes(self, n: u64) -> Self {
        self.map(|c| c.quota_per_zone_bytes = n)
    }
    pub fn persist_cookies(self, on: bool) -> Self {
        self.map(|c| c.persist_cookies = on)
    }
    pub fn cookie_jar_partitioning(self, m: CookiePartitioning) -> Self {
        self.map(|c| c.cookie_jar_partitioning = m)
    }

    pub fn sandbox_mode(self, m: SandboxMode) -> Self {
        self.map(|c| c.sandbox_mode = m)
    }
    pub fn cors_enforcement(self, on: bool) -> Self {
        self.map(|c| c.cors_enforcement = on)
    }
    pub fn disable_networking(self, on: bool) -> Self {
        self.map(|c| c.disable_networking = on)
    }
    pub fn blocked_domains(self, list: Vec<String>) -> Self {
        self.map(|c| c.blocked_domains = list)
    }
    pub fn allowlist_domains(self, list: Vec<String>) -> Self {
        self.map(|c| c.allowlist_domains = list)
    }

    pub fn gpu(self, opts: GpuOptions) -> Self {
        self.map(|c| c.gpu = opts)
    }
    pub fn target_fps(self, fps: Option<u16>) -> Self {
        self.map(|c| c.target_fps = fps)
    }
    pub fn pixel_snap(self, on: bool) -> Self {
        self.map(|c| c.pixel_snap = on)
    }

    pub fn font_search_paths(self, v: Vec<PathBuf>) -> Self {
        self.map(|c| c.font_search_paths = v)
    }
    pub fn fallback_fonts(self, v: Vec<String>) -> Self {
        self.map(|c| c.fallback_fonts = v)
    }
    pub fn font_cache_bytes(self, n: u64) -> Self {
        self.map(|c| c.font_cache_bytes = n)
    }

    pub fn javascript_enabled(self, on: bool) -> Self {
        self.map(|c| c.javascript_enabled = on)
    }
    pub fn lua_enabled(self, on: bool) -> Self {
        self.map(|c| c.lua_enabled = on)
    }
    pub fn wasm_enabled(self, on: bool) -> Self {
        self.map(|c| c.wasm_enabled = on)
    }
    pub fn max_script_cpu_ms_per_frame(self, n: u32) -> Self {
        self.map(|c| c.max_script_cpu_ms_per_frame = n)
    }

    pub fn log_level(self, lvl: LogLevel) -> Self {
        self.map(|c| c.log_level = lvl)
    }
    pub fn metrics_enabled(self, on: bool) -> Self {
        self.map(|c| c.metrics_enabled = on)
    }
    pub fn trace_enabled(self, on: bool) -> Self {
        self.map(|c| c.trace_enabled = on)
    }

    /// Apply multiple mutations in one go.
    pub fn with(self, f: impl FnOnce(&mut EngineConfig)) -> Self {
        self.map(f)
    }

    /// Validate and build the final `EngineConfig`.
    pub fn build(self) -> Result<EngineConfig, EngineConfigError> {
        validate(&self.inner)?;
        Ok(self.inner)
    }
}

#[derive(Debug, Clone)]
pub enum EngineConfigError {
    ZeroZones,
    ZeroThreads(&'static str),
    InvalidConnectionsPerHost(u32),
    InvalidTimeout(&'static str, Duration),
    InvalidMsaa(u32),
    NegativeBytes(&'static str), // (we still use u64, but keep for future signed fields)
}

impl fmt::Display for EngineConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use EngineConfigError::*;
        match self {
            ZeroZones => write!(f, "max_zones must be at least 1"),
            ZeroThreads(who) => write!(f, "{who} must be at least 1"),
            InvalidConnectionsPerHost(n) => {
                write!(f, "max_connections_per_host must be >= 1 (got {n})")
            }
            InvalidTimeout(name, d) => write!(f, "{name} must be > 0 (got {:?})", d),
            InvalidMsaa(s) => write!(f, "msaa_samples must be one of {{1,2,4,8}} (got {s})"),
            NegativeBytes(name) => write!(f, "{name} must be non-negative"),
        }
    }
}
impl std::error::Error for EngineConfigError {}

fn validate(c: &EngineConfig) -> Result<(), EngineConfigError> {
    if c.max_zones == 0 {
        return Err(EngineConfigError::ZeroZones);
    }
    if c.worker_threads == 0 {
        return Err(EngineConfigError::ZeroThreads("worker_threads"));
    }
    if c.io_concurrency == 0 {
        return Err(EngineConfigError::ZeroThreads("io_concurrency"));
    }
    if c.script_concurrency == 0 {
        return Err(EngineConfigError::ZeroThreads("script_concurrency"));
    }

    if c.max_connections_per_host == 0 {
        return Err(EngineConfigError::InvalidConnectionsPerHost(
            c.max_connections_per_host,
        ));
    }
    if c.connect_timeout == Duration::from_millis(0) {
        return Err(EngineConfigError::InvalidTimeout(
            "connect_timeout",
            c.connect_timeout,
        ));
    }
    if c.request_timeout == Duration::from_millis(0) {
        return Err(EngineConfigError::InvalidTimeout(
            "request_timeout",
            c.request_timeout,
        ));
    }
    match c.gpu.msaa_samples {
        1 | 2 | 4 | 8 => {}
        other => return Err(EngineConfigError::InvalidMsaa(other)),
    }
    // (bytes are u64 already; if you later switch to i64, keep NegativeBytes)
    Ok(())
}

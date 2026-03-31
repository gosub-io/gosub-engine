/// Policy settings for the user agent
#[derive(Debug, Clone)]
pub struct UaPolicy {
    /// Enable sniffing of content to determine MIME type
    pub enable_sniffing: bool,
    /// Enable sniffing of navigation to upgrade HTTP to HTTPS
    pub enable_sniffing_navigation_upgrade: bool,
    /// Enable PDF viewer
    pub enable_pdf_viewer: bool,
    /// Render unknown text files in a tab (otherwise download)
    pub render_unknown_text_in_tab: bool,
    /// Allow downloads without user activation
    pub allow_download_without_user_activation: bool,
}

impl Default for UaPolicy {
    fn default() -> Self {
        Self {
            enable_sniffing: true,
            enable_sniffing_navigation_upgrade: true,
            enable_pdf_viewer: true,
            render_unknown_text_in_tab: true,
            allow_download_without_user_activation: false,
        }
    }
}

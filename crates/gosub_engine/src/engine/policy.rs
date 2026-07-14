/// Policy settings for the user agent
#[derive(Debug, Clone)]
pub struct UaPolicy {
    /// Enable sniffing of content to determine MIME type
    pub enable_sniffing: bool,
    /// Allow mislabelled document navigations (e.g. `text/plain` or `application/octet-stream`
    /// bodies that sniff as HTML) to be upgraded to the HTML parser.
    pub enable_sniffing_navigation_upgrade: bool,
    /// Enable PDF viewer
    pub enable_pdf_viewer: bool,
    /// Allow downloads without user activation
    pub allow_download_without_user_activation: bool,
}

impl Default for UaPolicy {
    fn default() -> Self {
        Self {
            enable_sniffing: true,
            enable_sniffing_navigation_upgrade: true,
            enable_pdf_viewer: true,
            allow_download_without_user_activation: false,
        }
    }
}

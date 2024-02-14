use core::fmt::{Display, Formatter};
use std::time::{SystemTime, UNIX_EPOCH};

pub enum Timing {
    DnsLookup,
    TcpConnection,
    TlsHandshake,
    ServerProcessing,
    ContentTransfer,
    HtmlParse,
    CssParse,
    JsParse,
    RenderTree,
}

#[derive(Default, Debug, Clone)]
pub struct Timer {
    start: u64,
    end: u64,
}

impl Timer {
    pub fn new() -> Timer {
        Timer { start: 0, end: 0 }
    }

    pub fn start(&mut self) {
        let current_time = SystemTime::now();
        let duration_since_epoch = current_time
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards");
        let milliseconds_since_epoch =
            duration_since_epoch.as_secs() * 1000 + u64::from(duration_since_epoch.subsec_millis());

        self.start = milliseconds_since_epoch;
    }

    pub fn end(&mut self) {
        let current_time = SystemTime::now();
        let duration_since_epoch = current_time
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards");
        let milliseconds_since_epoch =
            duration_since_epoch.as_secs() * 1000 + u64::from(duration_since_epoch.subsec_millis());

        self.end = milliseconds_since_epoch;
    }

    pub fn duration(&self) -> u64 {
        self.end - self.start
    }
}

/// Timing information for a single request. Timing is measured in milliseconds.
#[derive(Default, Debug, Clone)]
pub struct TimingTable {
    /// Time spent making DNS queries.
    pub dns_lookup: Timer,
    /// Time spent establishing TCP connection.
    pub tcp_connection: Timer,
    /// Time spent performing TLS handshake.
    pub tls_handshake: Timer,
    /// Time spent sending HTTP request.
    pub server_processing: Timer,
    /// Time spent waiting for HTTP response.
    pub content_transfer: Timer,
    /// Time spent parsing HTML.
    pub html_parse: Timer,
    /// Time spent parsing CSS.
    pub css_parse: Timer,
    /// Time spent parsing JS.
    pub js_parse: Timer,
    /// Time spent generating render_tree
    pub render_tree: Timer,
}

impl Display for TimingTable {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "DNS lookup:        {:6}ms", self.dns_lookup.duration())?;
        writeln!(
            f,
            "TCP connection:    {:6}ms",
            self.tcp_connection.duration()
        )?;
        writeln!(
            f,
            "TLS handshake:     {:6}ms",
            self.tls_handshake.duration()
        )?;
        writeln!(
            f,
            "Server processing: {:6}ms",
            self.server_processing.duration()
        )?;
        writeln!(
            f,
            "Content transfer:  {:6}ms",
            self.content_transfer.duration()
        )?;
        writeln!(f, "HTML parse:        {:6}ms", self.html_parse.duration())?;
        writeln!(f, "CSS parse:         {:6}ms", self.css_parse.duration())?;
        writeln!(f, "JS parse:          {:6}ms", self.js_parse.duration())?;
        writeln!(f, "Render tree:       {:6}ms", self.render_tree.duration())?;

        Ok(())
    }
}

impl TimingTable {
    /// Starts a timing table, setting the start time to the current time.
    pub fn start(&mut self, timing: Timing) {
        match timing {
            Timing::DnsLookup => self.dns_lookup.start(),
            Timing::TcpConnection => self.tcp_connection.start(),
            Timing::TlsHandshake => self.tls_handshake.start(),
            Timing::ServerProcessing => self.server_processing.start(),
            Timing::ContentTransfer => self.content_transfer.start(),
            Timing::HtmlParse => self.html_parse.start(),
            Timing::CssParse => self.css_parse.start(),
            Timing::JsParse => self.js_parse.start(),
            Timing::RenderTree => self.render_tree.start(),
        }
    }

    pub fn end(&mut self, timing: Timing) {
        match timing {
            Timing::DnsLookup => self.dns_lookup.end(),
            Timing::TcpConnection => self.tcp_connection.end(),
            Timing::TlsHandshake => self.tls_handshake.end(),
            Timing::ServerProcessing => self.server_processing.end(),
            Timing::ContentTransfer => self.content_transfer.end(),
            Timing::HtmlParse => self.html_parse.end(),
            Timing::CssParse => self.css_parse.end(),
            Timing::JsParse => self.js_parse.end(),
            Timing::RenderTree => self.render_tree.end(),
        }
    }

    pub fn duration(&self, timing: Timing) -> u64 {
        match timing {
            Timing::DnsLookup => self.dns_lookup.duration(),
            Timing::TcpConnection => self.tcp_connection.duration(),
            Timing::TlsHandshake => self.tls_handshake.duration(),
            Timing::ServerProcessing => self.server_processing.duration(),
            Timing::ContentTransfer => self.content_transfer.duration(),
            Timing::HtmlParse => self.html_parse.duration(),
            Timing::CssParse => self.css_parse.duration(),
            Timing::JsParse => self.js_parse.duration(),
            Timing::RenderTree => self.render_tree.duration(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn test_timing_defaults() {
        let timing_table = TimingTable::default();

        assert_eq!(0, timing_table.duration(Timing::DnsLookup));
        assert_eq!(0, timing_table.duration(Timing::TcpConnection));
        assert_eq!(0, timing_table.duration(Timing::RenderTree));
    }

    #[test]
    fn test_timings() {
        let mut timingtable = TimingTable::default();

        timingtable.start(Timing::DnsLookup);
        timingtable.start(Timing::TcpConnection);
        sleep(std::time::Duration::from_millis(150));
        timingtable.end(Timing::DnsLookup);
        sleep(std::time::Duration::from_millis(10));
        timingtable.end(Timing::TcpConnection);

        assert!(timingtable.duration(Timing::DnsLookup) > 100);
        assert!(
            timingtable.duration(Timing::TcpConnection) > timingtable.duration(Timing::DnsLookup)
        );
    }
}

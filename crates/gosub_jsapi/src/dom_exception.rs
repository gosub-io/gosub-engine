//! DOMException as described by <https://webidl.spec.whatwg.org/#idl-DOMException>

use std::error::Error;
use std::fmt;

/// The standard error names from the WebIDL error names table
/// (<https://webidl.spec.whatwg.org/#dfn-error-names-table>).
///
/// A `DomException` can carry an arbitrary name (the constructor accepts any
/// string), but these are the names the platform specs throw and the only ones
/// that map to a non-zero legacy `code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorName {
    /// Deprecated: use `RangeError` instead
    IndexSizeError,
    HierarchyRequestError,
    WrongDocumentError,
    InvalidCharacterError,
    NoModificationAllowedError,
    NotFoundError,
    NotSupportedError,
    InUseAttributeError,
    InvalidStateError,
    SyntaxError,
    InvalidModificationError,
    NamespaceError,
    /// Deprecated: use `TypeError`, `NotSupportedError` or `NotAllowedError` instead
    InvalidAccessError,
    /// Deprecated: use `TypeError` instead
    TypeMismatchError,
    SecurityError,
    NetworkError,
    AbortError,
    UrlMismatchError,
    QuotaExceededError,
    TimeoutError,
    InvalidNodeTypeError,
    DataCloneError,
    EncodingError,
    NotReadableError,
    UnknownError,
    ConstraintError,
    DataError,
    TransactionInactiveError,
    ReadOnlyError,
    VersionError,
    OperationError,
    NotAllowedError,
}

impl ErrorName {
    /// The name as it appears on the JS side (`exception.name`)
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::IndexSizeError => "IndexSizeError",
            Self::HierarchyRequestError => "HierarchyRequestError",
            Self::WrongDocumentError => "WrongDocumentError",
            Self::InvalidCharacterError => "InvalidCharacterError",
            Self::NoModificationAllowedError => "NoModificationAllowedError",
            Self::NotFoundError => "NotFoundError",
            Self::NotSupportedError => "NotSupportedError",
            Self::InUseAttributeError => "InUseAttributeError",
            Self::InvalidStateError => "InvalidStateError",
            Self::SyntaxError => "SyntaxError",
            Self::InvalidModificationError => "InvalidModificationError",
            Self::NamespaceError => "NamespaceError",
            Self::InvalidAccessError => "InvalidAccessError",
            Self::TypeMismatchError => "TypeMismatchError",
            Self::SecurityError => "SecurityError",
            Self::NetworkError => "NetworkError",
            Self::AbortError => "AbortError",
            Self::UrlMismatchError => "URLMismatchError",
            Self::QuotaExceededError => "QuotaExceededError",
            Self::TimeoutError => "TimeoutError",
            Self::InvalidNodeTypeError => "InvalidNodeTypeError",
            Self::DataCloneError => "DataCloneError",
            Self::EncodingError => "EncodingError",
            Self::NotReadableError => "NotReadableError",
            Self::UnknownError => "UnknownError",
            Self::ConstraintError => "ConstraintError",
            Self::DataError => "DataError",
            Self::TransactionInactiveError => "TransactionInactiveError",
            Self::ReadOnlyError => "ReadOnlyError",
            Self::VersionError => "VersionError",
            Self::OperationError => "OperationError",
            Self::NotAllowedError => "NotAllowedError",
        }
    }

    /// Resolve a name string to a standard error name. Matching is exact, per
    /// the spec's "error names table" lookup.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "IndexSizeError" => Some(Self::IndexSizeError),
            "HierarchyRequestError" => Some(Self::HierarchyRequestError),
            "WrongDocumentError" => Some(Self::WrongDocumentError),
            "InvalidCharacterError" => Some(Self::InvalidCharacterError),
            "NoModificationAllowedError" => Some(Self::NoModificationAllowedError),
            "NotFoundError" => Some(Self::NotFoundError),
            "NotSupportedError" => Some(Self::NotSupportedError),
            "InUseAttributeError" => Some(Self::InUseAttributeError),
            "InvalidStateError" => Some(Self::InvalidStateError),
            "SyntaxError" => Some(Self::SyntaxError),
            "InvalidModificationError" => Some(Self::InvalidModificationError),
            "NamespaceError" => Some(Self::NamespaceError),
            "InvalidAccessError" => Some(Self::InvalidAccessError),
            "TypeMismatchError" => Some(Self::TypeMismatchError),
            "SecurityError" => Some(Self::SecurityError),
            "NetworkError" => Some(Self::NetworkError),
            "AbortError" => Some(Self::AbortError),
            "URLMismatchError" => Some(Self::UrlMismatchError),
            "QuotaExceededError" => Some(Self::QuotaExceededError),
            "TimeoutError" => Some(Self::TimeoutError),
            "InvalidNodeTypeError" => Some(Self::InvalidNodeTypeError),
            "DataCloneError" => Some(Self::DataCloneError),
            "EncodingError" => Some(Self::EncodingError),
            "NotReadableError" => Some(Self::NotReadableError),
            "UnknownError" => Some(Self::UnknownError),
            "ConstraintError" => Some(Self::ConstraintError),
            "DataError" => Some(Self::DataError),
            "TransactionInactiveError" => Some(Self::TransactionInactiveError),
            "ReadOnlyError" => Some(Self::ReadOnlyError),
            "VersionError" => Some(Self::VersionError),
            "OperationError" => Some(Self::OperationError),
            "NotAllowedError" => Some(Self::NotAllowedError),
            _ => None,
        }
    }

    /// The legacy numeric code for this name, or 0 if the name has none
    #[must_use]
    pub fn legacy_code(&self) -> u16 {
        match self {
            Self::IndexSizeError => DomException::INDEX_SIZE_ERR,
            Self::HierarchyRequestError => DomException::HIERARCHY_REQUEST_ERR,
            Self::WrongDocumentError => DomException::WRONG_DOCUMENT_ERR,
            Self::InvalidCharacterError => DomException::INVALID_CHARACTER_ERR,
            Self::NoModificationAllowedError => DomException::NO_MODIFICATION_ALLOWED_ERR,
            Self::NotFoundError => DomException::NOT_FOUND_ERR,
            Self::NotSupportedError => DomException::NOT_SUPPORTED_ERR,
            Self::InUseAttributeError => DomException::INUSE_ATTRIBUTE_ERR,
            Self::InvalidStateError => DomException::INVALID_STATE_ERR,
            Self::SyntaxError => DomException::SYNTAX_ERR,
            Self::InvalidModificationError => DomException::INVALID_MODIFICATION_ERR,
            Self::NamespaceError => DomException::NAMESPACE_ERR,
            Self::InvalidAccessError => DomException::INVALID_ACCESS_ERR,
            Self::TypeMismatchError => DomException::TYPE_MISMATCH_ERR,
            Self::SecurityError => DomException::SECURITY_ERR,
            Self::NetworkError => DomException::NETWORK_ERR,
            Self::AbortError => DomException::ABORT_ERR,
            Self::UrlMismatchError => DomException::URL_MISMATCH_ERR,
            Self::QuotaExceededError => DomException::QUOTA_EXCEEDED_ERR,
            Self::TimeoutError => DomException::TIMEOUT_ERR,
            Self::InvalidNodeTypeError => DomException::INVALID_NODE_TYPE_ERR,
            Self::DataCloneError => DomException::DATA_CLONE_ERR,
            _ => 0,
        }
    }
}

impl fmt::Display for ErrorName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An exception as thrown by web platform APIs. Holds a name from the error
/// names table (or any other string, when constructed from script) and a
/// human-readable message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomException {
    name: String,
    message: String,
}

impl DomException {
    pub const INDEX_SIZE_ERR: u16 = 1;
    pub const DOMSTRING_SIZE_ERR: u16 = 2;
    pub const HIERARCHY_REQUEST_ERR: u16 = 3;
    pub const WRONG_DOCUMENT_ERR: u16 = 4;
    pub const INVALID_CHARACTER_ERR: u16 = 5;
    pub const NO_DATA_ALLOWED_ERR: u16 = 6;
    pub const NO_MODIFICATION_ALLOWED_ERR: u16 = 7;
    pub const NOT_FOUND_ERR: u16 = 8;
    pub const NOT_SUPPORTED_ERR: u16 = 9;
    pub const INUSE_ATTRIBUTE_ERR: u16 = 10;
    pub const INVALID_STATE_ERR: u16 = 11;
    pub const SYNTAX_ERR: u16 = 12;
    pub const INVALID_MODIFICATION_ERR: u16 = 13;
    pub const NAMESPACE_ERR: u16 = 14;
    pub const INVALID_ACCESS_ERR: u16 = 15;
    pub const VALIDATION_ERR: u16 = 16;
    pub const TYPE_MISMATCH_ERR: u16 = 17;
    pub const SECURITY_ERR: u16 = 18;
    pub const NETWORK_ERR: u16 = 19;
    pub const ABORT_ERR: u16 = 20;
    pub const URL_MISMATCH_ERR: u16 = 21;
    pub const QUOTA_EXCEEDED_ERR: u16 = 22;
    pub const TIMEOUT_ERR: u16 = 23;
    pub const INVALID_NODE_TYPE_ERR: u16 = 24;
    pub const DATA_CLONE_ERR: u16 = 25;

    /// The `new DOMException(message, name)` constructor. Both arguments are
    /// optional in the spec; the defaults are an empty message and the name
    /// "Error".
    #[must_use]
    pub fn new(message: &str, name: &str) -> Self {
        Self {
            name: name.to_owned(),
            message: message.to_owned(),
        }
    }

    /// Create an exception with a standard name, the way specs (and our own
    /// APIs) throw them: "throw a `NotFoundError` `DOMException`"
    #[must_use]
    pub fn with_name(name: ErrorName, message: &str) -> Self {
        Self::new(message, name.as_str())
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The legacy code attribute: the code listed for the name in the error
    /// names table, or 0 if the name has none
    #[must_use]
    pub fn code(&self) -> u16 {
        ErrorName::from_name(&self.name).map_or(0, |n| n.legacy_code())
    }
}

impl Default for DomException {
    fn default() -> Self {
        Self::new("", "Error")
    }
}

impl fmt::Display for DomException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.message.is_empty() {
            f.write_str(&self.name)
        } else {
            write!(f, "{}: {}", self.name, self.message)
        }
    }
}

impl Error for DomException {}

impl From<ErrorName> for DomException {
    fn from(name: ErrorName) -> Self {
        Self::with_name(name, "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructor_defaults() {
        let e = DomException::default();
        assert_eq!(e.name(), "Error");
        assert_eq!(e.message(), "");
        assert_eq!(e.code(), 0);
    }

    #[test]
    fn standard_name_roundtrip() {
        let e = DomException::with_name(ErrorName::NotFoundError, "no such node");
        assert_eq!(e.name(), "NotFoundError");
        assert_eq!(e.message(), "no such node");
        assert_eq!(e.code(), DomException::NOT_FOUND_ERR);
    }

    #[test]
    fn url_mismatch_uses_platform_spelling() {
        let e = DomException::from(ErrorName::UrlMismatchError);
        assert_eq!(e.name(), "URLMismatchError");
        assert_eq!(e.code(), DomException::URL_MISMATCH_ERR);
        assert_eq!(
            ErrorName::from_name("URLMismatchError"),
            Some(ErrorName::UrlMismatchError)
        );
    }

    #[test]
    fn code_lookup() {
        // Spot-check both ends of the legacy table
        assert_eq!(DomException::with_name(ErrorName::IndexSizeError, "").code(), 1);
        assert_eq!(DomException::with_name(ErrorName::DataCloneError, "").code(), 25);
        // Names without a legacy code report 0
        assert_eq!(DomException::with_name(ErrorName::EncodingError, "").code(), 0);
        assert_eq!(DomException::with_name(ErrorName::NotAllowedError, "").code(), 0);
        // Arbitrary names report 0
        assert_eq!(DomException::new("boom", "MyCustomError").code(), 0);
    }

    #[test]
    fn every_standard_name_resolves_back() {
        let names = [
            ErrorName::IndexSizeError,
            ErrorName::HierarchyRequestError,
            ErrorName::WrongDocumentError,
            ErrorName::InvalidCharacterError,
            ErrorName::NoModificationAllowedError,
            ErrorName::NotFoundError,
            ErrorName::NotSupportedError,
            ErrorName::InUseAttributeError,
            ErrorName::InvalidStateError,
            ErrorName::SyntaxError,
            ErrorName::InvalidModificationError,
            ErrorName::NamespaceError,
            ErrorName::InvalidAccessError,
            ErrorName::TypeMismatchError,
            ErrorName::SecurityError,
            ErrorName::NetworkError,
            ErrorName::AbortError,
            ErrorName::UrlMismatchError,
            ErrorName::QuotaExceededError,
            ErrorName::TimeoutError,
            ErrorName::InvalidNodeTypeError,
            ErrorName::DataCloneError,
            ErrorName::EncodingError,
            ErrorName::NotReadableError,
            ErrorName::UnknownError,
            ErrorName::ConstraintError,
            ErrorName::DataError,
            ErrorName::TransactionInactiveError,
            ErrorName::ReadOnlyError,
            ErrorName::VersionError,
            ErrorName::OperationError,
            ErrorName::NotAllowedError,
        ];
        for name in names {
            assert_eq!(ErrorName::from_name(name.as_str()), Some(name));
        }
    }

    #[test]
    fn display_and_error_trait() {
        let e = DomException::with_name(ErrorName::AbortError, "operation was cancelled");
        assert_eq!(e.to_string(), "AbortError: operation was cancelled");

        let no_message = DomException::from(ErrorName::AbortError);
        assert_eq!(no_message.to_string(), "AbortError");

        let boxed: Box<dyn Error> = Box::new(e);
        assert_eq!(boxed.to_string(), "AbortError: operation was cancelled");
    }
}

impl LonghandId {
    /// The CSS name of this property.
    #[must_use]
    pub fn name(self) -> &'static str {
        LONGHANDS[self as usize].name
    }

    /// Whether the property inherits by default.
    #[must_use]
    pub fn inherited(self) -> bool {
        LONGHANDS[self as usize].inherited
    }

    /// The initial value as the definition data writes it, before it is parsed. `None` where the
    /// data gives a list rather than a value.
    #[must_use]
    pub fn initial_source(self) -> Option<&'static str> {
        LONGHANDS[self as usize].initial
    }

    /// Whether a percentage specified for this property computes to a plain number.
    #[must_use]
    pub fn percentage_is_number(self) -> bool {
        LONGHANDS[self as usize].percentage_is_number
    }

    /// This property's slot, which is also its position in [`ALL_LONGHAND_IDS`].
    #[must_use]
    pub fn index(self) -> usize {
        self as usize
    }

    /// The longhand at `index`, or `None` when there is none.
    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        ALL_LONGHAND_IDS.get(index).copied()
    }
}

impl ShorthandId {
    /// The CSS name of this property.
    #[must_use]
    pub fn name(self) -> &'static str {
        SHORTHANDS[self as usize].name
    }

    /// Whether the property inherits by default.
    #[must_use]
    pub fn inherited(self) -> bool {
        SHORTHANDS[self as usize].inherited
    }

    /// The initial value as the definition data writes it, before it is parsed. A shorthand's is
    /// usually `None`: the data gives its longhand list there instead of a value.
    #[must_use]
    pub fn initial_source(self) -> Option<&'static str> {
        SHORTHANDS[self as usize].initial
    }

    /// Whether a percentage specified for this property computes to a plain number.
    #[must_use]
    pub fn percentage_is_number(self) -> bool {
        SHORTHANDS[self as usize].percentage_is_number
    }

    /// The properties this shorthand sets. A few of them are shorthands themselves.
    #[must_use]
    pub fn longhands(self) -> &'static [PropertyId] {
        SHORTHANDS[self as usize].longhands
    }

    /// This property's position in [`ALL_SHORTHAND_IDS`]. Note that a [`PropertyId::index`] puts
    /// the shorthands after the longhands, so the two are not the same number.
    #[must_use]
    pub fn index(self) -> usize {
        self as usize
    }

    /// The shorthand at `index`, or `None` when there is none.
    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        ALL_SHORTHAND_IDS.get(index).copied()
    }
}

impl PropertyId {
    /// A slot number below [`PROPERTY_COUNT`], unique across longhands and shorthands both. The
    /// longhands come first, so an array of [`LONGHAND_COUNT`] entries can be indexed by a
    /// [`LonghandId`] and one of [`PROPERTY_COUNT`] entries by any property.
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            PropertyId::Longhand(id) => id as usize,
            PropertyId::Shorthand(id) => LONGHAND_COUNT + id as usize,
        }
    }

    /// The property whose [`PropertyId::index`] is `index`, or `None` when there is none.
    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        if index < LONGHAND_COUNT {
            return LonghandId::from_index(index).map(PropertyId::Longhand);
        }
        ShorthandId::from_index(index - LONGHAND_COUNT).map(PropertyId::Shorthand)
    }

    /// The CSS name of this property.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            PropertyId::Longhand(id) => id.name(),
            PropertyId::Shorthand(id) => id.name(),
        }
    }

    /// Whether the property inherits by default.
    #[must_use]
    pub fn inherited(self) -> bool {
        match self {
            PropertyId::Longhand(id) => id.inherited(),
            PropertyId::Shorthand(id) => id.inherited(),
        }
    }

    /// The initial value as the definition data writes it, before it is parsed.
    #[must_use]
    pub fn initial_source(self) -> Option<&'static str> {
        match self {
            PropertyId::Longhand(id) => id.initial_source(),
            PropertyId::Shorthand(id) => id.initial_source(),
        }
    }

    /// Whether a percentage specified for this property computes to a plain number.
    #[must_use]
    pub fn percentage_is_number(self) -> bool {
        match self {
            PropertyId::Longhand(id) => id.percentage_is_number(),
            PropertyId::Shorthand(id) => id.percentage_is_number(),
        }
    }

    /// Whether this property distributes its value over others.
    #[must_use]
    pub fn is_shorthand(self) -> bool {
        matches!(self, PropertyId::Shorthand(_))
    }

    /// The properties a shorthand sets, empty for a longhand.
    #[must_use]
    pub fn longhands(self) -> &'static [PropertyId] {
        match self {
            PropertyId::Longhand(_) => &[],
            PropertyId::Shorthand(id) => id.longhands(),
        }
    }

    /// The property `name` denotes, or `None` when this engine has no definition for it.
    ///
    /// Property names are ASCII case-insensitive (css-syntax-3 §3.3), so `COLOR` is `color`. The
    /// table is all lowercase and the common case is a name that is too, so the exact search runs
    /// first and only a name carrying an uppercase byte costs a second one.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        if let Ok(found) = BY_NAME.binary_search_by(|(known, _)| (*known).cmp(name)) {
            return Some(BY_NAME[found].1);
        }
        if !name.bytes().any(|byte| byte.is_ascii_uppercase()) {
            return None;
        }
        let found = BY_NAME
            .binary_search_by(|(known, _)| compare_ascii_lowercase(known, name))
            .ok()?;
        Some(BY_NAME[found].1)
    }
}

/// Compare `known`, which is already lowercase, against `name` lowercased as it is read. Keeps
/// the case-insensitive search allocation-free and consistent with the table's ordering.
fn compare_ascii_lowercase(known: &str, name: &str) -> core::cmp::Ordering {
    let mut lowered = name.bytes().map(|byte| byte.to_ascii_lowercase());
    for byte in known.bytes() {
        match lowered.next() {
            None => return core::cmp::Ordering::Greater,
            Some(other) => match byte.cmp(&other) {
                core::cmp::Ordering::Equal => {}
                other => return other,
            },
        }
    }
    match lowered.next() {
        None => core::cmp::Ordering::Equal,
        Some(_) => core::cmp::Ordering::Less,
    }
}

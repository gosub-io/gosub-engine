use gosub_shared::geo::Point;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputEvent {
    /// The mouse moved to a new position
    MouseMove(Point),
    /// Scroll delta, not a position.
    MouseScroll(Point),
    /// A mouse button was pressed
    MouseDown(MouseButton),
    /// A mouse button was released
    MouseUp(MouseButton),
    /// A key was pressed
    KeyboardDown(char),
    /// A key was released
    KeyboardUp(char),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

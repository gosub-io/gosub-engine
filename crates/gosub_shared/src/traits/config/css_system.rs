use crate::traits::css3::{CssProperty, CssPropertyMap, CssStylesheet, CssSystem, CssValue};

pub trait HasCssSystem:
    Sized
    + HasCssSystemExt<
        Self,
        Stylesheet = <Self::CssSystem as CssSystem>::Stylesheet,
        CssPropertyMap = <Self::CssSystem as CssSystem>::PropertyMap,
        CssProperty = <Self::CssSystem as CssSystem>::Property,
        CssValue = <Self::CssSystem as CssSystem>::Value,
    >
{
    type CssSystem: CssSystem;
}

pub trait HasCssSystemExt<C: HasCssSystem> {
    type Stylesheet: CssStylesheet;
    type CssPropertyMap: CssPropertyMap<C::CssSystem>;
    type CssProperty: CssProperty<C::CssSystem>;
    type CssValue: CssValue;
}

impl<C: HasCssSystem> HasCssSystemExt<C> for C {
    type Stylesheet = <C::CssSystem as CssSystem>::Stylesheet;
    type CssPropertyMap = <C::CssSystem as CssSystem>::PropertyMap;
    type CssProperty = <C::CssSystem as CssSystem>::Property;
    type CssValue = <C::CssSystem as CssSystem>::Value;
}

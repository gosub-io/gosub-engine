use v8::{Array, Local};

use crate::js::v8::{V8Context, V8Value};
use crate::js::{JSArray, JSError};
use crate::types::{Error, Result};

pub struct V8Array<'a> {
    value: Local<'a, Array>,
    ctx: V8Context<'a>,
}

impl<'a> JSArray for V8Array<'a> {
    type Value = V8Value<'a>;

    type Index = u32;

    fn get<T: Into<Self::Index>>(&self, index: T) -> Result<Self::Value> {
        let Some(value) = self
            .value
            .get_index(self.ctx.borrow_mut().scope(), index.into())
        else {
            return Err(Error::JS(JSError::Generic(
                "failed to get a value from an array".to_owned(),
            )));
        };

        Ok(V8Value::from_value(self.ctx.clone(), value))
    }

    fn set<T: Into<Self::Index>>(&self, index: T, value: &Self::Value) -> Result<()> {
        match self
            .value
            .set_index(self.ctx.borrow_mut().scope(), index.into(), value.value)
        {
            Some(_) => Ok(()),
            None => Err(Error::JS(JSError::Conversion(
                "failed to set a value in an array".to_owned(),
            ))),
        }
    }

    fn push(&self, value: Self::Value) -> Result<()> {
        let index = self.value.length();

        match self
            .value
            .set_index(self.ctx.borrow_mut().scope(), index, value.value)
        {
            Some(_) => Ok(()),
            None => Err(Error::JS(JSError::Conversion(
                "failed to push to an array".to_owned(),
            ))),
        }
    }

    fn pop(&self) -> Result<Self::Value> {
        let index = self.value.length() - 1;

        let Some(value) = self.value.get_index(self.ctx.borrow_mut().scope(), index) else {
            return Err(Error::JS(JSError::Generic(
                "failed to get a value from an array".to_owned(),
            )));
        };

        if self
            .value
            .delete_index(self.ctx.borrow_mut().scope(), index)
            .is_none()
        {
            return Err(Error::JS(JSError::Generic(
                "failed to delete a value from an array".to_owned(),
            )));
        }

        Ok(V8Value::from_value(self.ctx.clone(), value))
    }

    fn remove<T: Into<Self::Index>>(&self, index: T) -> Result<()> {
        if self
            .value
            .delete_index(self.ctx.borrow_mut().scope(), index.into())
            .is_none()
        {
            return Err(Error::JS(JSError::Generic(
                "failed to delete a value from an array".to_owned(),
            )));
        }

        Ok(())
    }

    fn length(&self) -> Result<Self::Index> {
        Ok(self.value.length())
    }
}

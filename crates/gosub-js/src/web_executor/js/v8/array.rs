use v8::{Array, Local};

use crate::web_executor::js::v8::{V8Context, V8Engine, V8Value};
use crate::web_executor::js::{JSArray, JSError, JSRuntime};
use crate::Error;
use gosub_shared::types::Result;

pub struct V8Array<'a> {
    value: Local<'a, Array>,
    ctx: V8Context<'a>,
}

impl<'a> JSArray for V8Array<'a> {
    type RT = V8Engine<'a>;

    fn get<T: Into<u32>>(&self, index: T) -> Result<<Self::RT as JSRuntime>::Value> {
        let Some(value) = self
            .value
            .get_index(self.ctx.borrow_mut().scope(), index.into())
        else {
            return Err(Error::JS(JSError::Generic(
                "failed to get a value from an array".to_owned(),
            ))
            .into());
        };

        Ok(V8Value::from_value(self.ctx.clone(), value))
    }

    fn set<T: Into<u32>>(&self, index: T, value: &V8Value) -> Result<()> {
        match self
            .value
            .set_index(self.ctx.borrow_mut().scope(), index.into(), value.value)
        {
            Some(_) => Ok(()),
            None => Err(Error::JS(JSError::Conversion(
                "failed to set a value in an array".to_owned(),
            ))
            .into()),
        }
    }

    fn push(&self, value: V8Value) -> Result<()> {
        let index = self.value.length();

        match self
            .value
            .set_index(self.ctx.borrow_mut().scope(), index, value.value)
        {
            Some(_) => Ok(()),
            None => {
                Err(Error::JS(JSError::Conversion("failed to push to an array".to_owned())).into())
            }
        }
    }

    fn pop(&self) -> Result<<Self::RT as JSRuntime>::Value> {
        let index = self.value.length() - 1;

        let Some(value) = self.value.get_index(self.ctx.borrow_mut().scope(), index) else {
            return Err(Error::JS(JSError::Generic(
                "failed to get a value from an array".to_owned(),
            ))
            .into());
        };

        if self
            .value
            .delete_index(self.ctx.borrow_mut().scope(), index)
            .is_none()
        {
            return Err(Error::JS(JSError::Generic(
                "failed to delete a value from an array".to_owned(),
            ))
            .into());
        }

        Ok(V8Value::from_value(self.ctx.clone(), value))
    }

    fn remove<T: Into<u32>>(&self, index: T) -> Result<()> {
        if self
            .value
            .delete_index(self.ctx.borrow_mut().scope(), index.into())
            .is_none()
        {
            return Err(Error::JS(JSError::Generic(
                "failed to delete a value from an array".to_owned(),
            ))
            .into());
        }

        Ok(())
    }

    fn length(&self) -> Result<u32> {
        Ok(self.value.length())
    }
}

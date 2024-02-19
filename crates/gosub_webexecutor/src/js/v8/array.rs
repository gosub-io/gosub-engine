use v8::{Array, Local};

use crate::js::v8::{V8Context, V8Engine, V8Value};
use crate::js::{JSArray, JSError, JSRuntime};
use crate::Error;
use gosub_shared::types::Result;

pub struct V8Array<'a> {
    value: Local<'a, Array>,
    ctx: V8Context<'a>,
}

impl<'a> V8Array<'a> {
    pub fn new(ctx: &V8Context<'a>, len: u32) -> Result<Self> {
        let value = Array::new(ctx.borrow_mut().scope(), len as i32);

        Ok(Self {
            value,
            ctx: ctx.clone(),
        })
    }
}

impl<'a> JSArray for V8Array<'a> {
    type RT = V8Engine<'a>;

    fn get(&self, index: u32) -> Result<<Self::RT as JSRuntime>::Value> {
        let Some(value) = self
            .value
            .get_index(self.ctx.borrow_mut().scope(), index)
        else {
            return Err(Error::JS(JSError::Generic(
                "failed to get a value from an array".to_owned(),
            ))
            .into());
        };

        Ok(V8Value::from_value(self.ctx.clone(), value))
    }

    fn set(&self, index: u32, value: &V8Value) -> Result<()> {
        match self
            .value
            .set_index(self.ctx.borrow_mut().scope(), index, value.value)
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

#[cfg(test)]
mod tests {
    use crate::web_executor::js::v8::{V8Array, V8Engine};
    use crate::web_executor::js::{JSArray, JSRuntime, JSValue, ValueConversion};

    #[test]
    fn set() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let array = V8Array::new(&context, 2).unwrap();
        array
            .set(0, &1234.0.to_js_value(context.clone()).unwrap())
            .unwrap();
        array
            .set(1, &"Hello World!".to_js_value(context).unwrap())
            .unwrap();

        assert_eq!(array.length().unwrap(), 2);
    }

    #[test]
    fn get() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let array = V8Array::new(&context, 2).unwrap();

        array
            .set(0, &1234.0.to_js_value(context.clone()).unwrap())
            .unwrap();
        array
            .set(1, &"Hello World!".to_js_value(context).unwrap())
            .unwrap();

        assert_eq!(array.get(0).unwrap().as_number().unwrap(), 1234.0);
        assert_eq!(
            array.get(1).unwrap().as_string().unwrap(),
            "Hello World!"
        );
    }

    #[test]
    fn push() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let array = V8Array::new(&context, 2).unwrap();

        array
            .push(1234.0.to_js_value(context.clone()).unwrap())
            .unwrap();
        array
            .push("Hello World!".to_js_value(context).unwrap())
            .unwrap();

        assert_eq!(array.length().unwrap(), 4);
    }

    #[test]
    fn out_of_bounds() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let array = V8Array::new(&context, 2).unwrap();

        array
            .set(0, &1234.0.to_js_value(context.clone()).unwrap())
            .unwrap();
        array
            .set(1, &"Hello World!".to_js_value(context).unwrap())
            .unwrap();

        assert!(array.get(2).unwrap().is_undefined());
    }

    #[test]
    fn pop() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let array = V8Array::new(&context, 2).unwrap();

        array
            .set(0, &1234.0.to_js_value(context.clone()).unwrap())
            .unwrap();
        array
            .set(1, &"Hello World!".to_js_value(context).unwrap())
            .unwrap();

        assert_eq!(array.pop().unwrap().as_string().unwrap(), "Hello World!");
        assert_eq!(array.get(0u32).unwrap().as_number().unwrap(), 1234.0);
        assert!(array.get(1u32).unwrap().is_undefined());
    }

    #[test]
    fn dynamic_resize() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let array = V8Array::new(&context, 2).unwrap();

        array
            .set(0, &1234.0.to_js_value(context.clone()).unwrap())
            .unwrap();
        array
            .set(1, &"Hello World!".to_js_value(context.clone()).unwrap())
            .unwrap();
        array
            .set(2, &1234.0.to_js_value(context.clone()).unwrap())
            .unwrap();
        array
            .set(3, &"Hello World!".to_js_value(context.clone()).unwrap())
            .unwrap();

        assert_eq!(array.length().unwrap(), 4);
    }
}

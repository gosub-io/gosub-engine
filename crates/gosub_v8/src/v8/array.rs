use v8::{Array, Local};

use gosub_shared::types::Result;
use gosub_webexecutor::js::{AsArray, JSArray, JSError, JSRuntime, Ref};
use gosub_webexecutor::Error;

use crate::{FromContext, V8Context, V8Engine, V8Value};

pub struct V8Array<'a> {
    pub value: Local<'a, Array>,
    pub ctx: V8Context<'a>,
    next: u32,
}

impl<'a> FromContext<'a, Local<'a, Array>> for V8Array<'a> {
    fn from_ctx(ctx: V8Context<'a>, value: Local<'a, Array>) -> Self {
        Self {
            value,
            ctx,
            next: 0,
        }
    }
}

impl<'a> Iterator for V8Array<'a> {
    type Item = V8Value<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.value.length() {
            return None;
        }
        let value = self.value.get_index(self.ctx.scope(), self.next);

        self.next += 1;

        value.map(|v| V8Value::from_value(self.ctx.clone(), v))
    }
}

impl<'a> AsArray for V8Array<'a> {
    type Runtime = V8Engine<'a>;

    fn array(&self) -> Result<Ref<<Self::Runtime as JSRuntime>::Array>> {
        Ok(Ref::Ref(self))
    }
}

impl<'a> JSArray for V8Array<'a> {
    type RT = V8Engine<'a>;

    fn get(&self, index: usize) -> Result<<Self::RT as JSRuntime>::Value> {
        let Some(value) = self.value.get_index(self.ctx.scope(), index as u32) else {
            return Err(Error::JS(JSError::Generic(
                "failed to get a value from an array".to_owned(),
            ))
            .into());
        };

        Ok(V8Value::from_value(self.ctx.clone(), value))
    }

    fn set(&self, index: usize, value: &V8Value) -> Result<()> {
        match self
            .value
            .set_index(self.ctx.scope(), index as u32, value.value)
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

        match self.value.set_index(self.ctx.scope(), index, value.value) {
            Some(_) => Ok(()),
            None => {
                Err(Error::JS(JSError::Conversion("failed to push to an array".to_owned())).into())
            }
        }
    }

    fn pop(&self) -> Result<<Self::RT as JSRuntime>::Value> {
        let index = self.value.length() - 1;

        let Some(value) = self.value.get_index(self.ctx.scope(), index) else {
            return Err(Error::JS(JSError::Generic(
                "failed to get a value from an array".to_owned(),
            ))
            .into());
        };

        if self.value.delete_index(self.ctx.scope(), index).is_none() {
            return Err(Error::JS(JSError::Generic(
                "failed to delete a value from an array".to_owned(),
            ))
            .into());
        }

        Ok(V8Value::from_value(self.ctx.clone(), value))
    }

    fn remove(&self, index: usize) -> Result<()> {
        if self
            .value
            .delete_index(self.ctx.scope(), index as u32)
            .is_none()
        {
            return Err(Error::JS(JSError::Generic(
                "failed to delete a value from an array".to_owned(),
            ))
            .into());
        }

        Ok(())
    }

    fn len(&self) -> usize {
        self.value.length() as usize
    }

    fn is_empty(&self) -> bool {
        self.value.length() == 0
    }

    fn new(ctx: V8Context<'a>, cap: usize) -> Result<Self> {
        let value = Array::new(ctx.scope(), cap as i32);

        Ok(Self {
            value,
            ctx: ctx.clone(),
            next: 0,
        })
    }

    fn new_with_data(ctx: V8Context<'a>, data: &[V8Value]) -> Result<Self> {
        let elements = data.iter().map(|v| v.value).collect::<Vec<_>>();
        let value = Array::new_with_elements(ctx.scope(), &elements);

        Ok(Self {
            value,
            ctx: ctx.clone(),
            next: 0,
        })
    }

    fn as_value(&self) -> <Self::RT as JSRuntime>::Value {
        V8Value::from_value(self.ctx.clone(), Local::from(self.value))
    }

    fn as_vec(&self) -> Vec<<Self::RT as JSRuntime>::Value> {
        let mut vec = Vec::with_capacity(self.len());
        for i in 0..self.len() {
            vec.push(self.get(i).unwrap());
        }
        vec
    }
}

#[cfg(test)]
mod tests {
    use gosub_webexecutor::js::{
        ArrayConversion, IntoJSValue, IntoRustValue, JSArray, JSContext, JSObject, JSRuntime,
        JSValue,
    };

    use crate::v8::{V8Array, V8Engine, V8Value};

    #[test]
    fn set() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        let array = V8Array::new(context.clone(), 2).unwrap();
        array
            .set(0, &1234.0.to_js_value(context.clone()).unwrap())
            .unwrap();
        array
            .set(1, &"Hello World!".to_js_value(context).unwrap())
            .unwrap();

        assert_eq!(array.len(), 2);
    }

    #[test]
    fn get() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        let array = V8Array::new(context.clone(), 2).unwrap();

        array
            .set(0, &1234.0.to_js_value(context.clone()).unwrap())
            .unwrap();
        array
            .set(1, &"Hello World!".to_js_value(context).unwrap())
            .unwrap();

        assert_eq!(array.get(0).unwrap().as_number().unwrap(), 1234.0);
        assert_eq!(array.get(1).unwrap().as_string().unwrap(), "Hello World!");
    }

    #[test]
    fn push() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        let array = V8Array::new(context.clone(), 2).unwrap();

        array
            .push(1234.0.to_js_value(context.clone()).unwrap())
            .unwrap();
        array
            .push("Hello World!".to_js_value(context).unwrap())
            .unwrap();

        assert_eq!(array.len(), 4);
    }

    #[test]
    fn out_of_bounds() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        let array = V8Array::new(context.clone(), 2).unwrap();

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
        let context = engine.new_context().unwrap();

        let array = V8Array::new(context.clone(), 2).unwrap();

        array
            .set(0, &1234.0.to_js_value(context.clone()).unwrap())
            .unwrap();
        array
            .set(1, &"Hello World!".to_js_value(context).unwrap())
            .unwrap();

        assert_eq!(array.pop().unwrap().as_string().unwrap(), "Hello World!");
        assert_eq!(array.get(0).unwrap().as_number().unwrap(), 1234.0);
        assert!(array.get(1).unwrap().is_undefined());
    }

    #[test]
    fn dynamic_resize() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        let array = V8Array::new(context.clone(), 2).unwrap();

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

        assert_eq!(array.len(), 4);
    }

    #[test]
    fn rust_to_js() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        let array: V8Array = [42, 1337, 1234].to_js_array(context.clone()).unwrap();

        assert_eq!(array.len(), 3);
        assert_eq!(array.get(0).unwrap().as_number().unwrap(), 42.0);
        assert_eq!(array.get(1).unwrap().as_number().unwrap(), 1337.0);
        assert_eq!(array.get(2).unwrap().as_number().unwrap(), 1234.0);
    }

    #[test]
    fn rust_to_js_value() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        let array: V8Value = [42, 1337, 1234].to_js_value(context.clone()).unwrap();

        assert!(array.is_array());
        let array = array.as_array().unwrap();
        assert_eq!(array.len(), 3);
        assert_eq!(array.get(0).unwrap().as_number().unwrap(), 42.0);
        assert_eq!(array.get(1).unwrap().as_number().unwrap(), 1337.0);
        assert_eq!(array.get(2).unwrap().as_number().unwrap(), 1234.0);
    }

    #[test]
    fn rust_to_js_global() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let array: V8Array = [42, 1337, 1234].to_js_array(context.clone()).unwrap();

        let test_obj = context.new_global_object("test").unwrap();

        test_obj.set_property("array", &array.into()).unwrap();

        {
            let val = context
                .run(
                    r#"
                test.array
            "#,
                )
                .unwrap();

            assert!(val.is_array());
            let array = val.as_array().unwrap();
            assert_eq!(array.len(), 3);

            assert_eq!(array.get(0).unwrap().as_number().unwrap(), 42.0);
            assert_eq!(array.get(1).unwrap().as_number().unwrap(), 1337.0);
            assert_eq!(array.get(2).unwrap().as_number().unwrap(), 1234.0);
        }

        {
            let val = context
                .run(
                    r#"
                test.array[0]
            "#,
                )
                .unwrap();

            assert!(val.is_number());
            assert_eq!(val.as_number().unwrap(), 42.0);
        }

        {
            let val = context
                .run(
                    r#"
                test.array[1]
            "#,
                )
                .unwrap();

            assert!(val.is_number());
            assert_eq!(val.as_number().unwrap(), 1337.0);
        }

        {
            let val = context
                .run(
                    r#"
                test.array[2]
            "#,
                )
                .unwrap();

            assert!(val.is_number());
            assert_eq!(val.as_number().unwrap(), 1234.0);
        }

        {
            let val = context
                .run(
                    r#"
                test.array.push(5678)
                
                test.array
            "#,
                )
                .unwrap();

            assert!(val.is_array());
            let array = val.as_array().unwrap();
            assert_eq!(array.len(), 4);
        }
    }

    #[test]
    fn js_to_rust() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let array = context
            .run(
                r#"
            [42, 1337, 1234]
        "#,
            )
            .unwrap();

        assert!(array.is_array());
        let array = array.as_array().unwrap();
        assert_eq!(array.len(), 3);
        assert_eq!(array.get(0).unwrap().as_number().unwrap(), 42.0);
        assert_eq!(array.get(1).unwrap().as_number().unwrap(), 1337.0);
        assert_eq!(array.get(2).unwrap().as_number().unwrap(), 1234.0);
    }

    #[test]
    fn rust_vec_to_js() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        #[allow(clippy::useless_vec)]
        let vec = vec![42, 1337, 1234];

        let array: V8Array = vec.to_js_array(context.clone()).unwrap();

        assert_eq!(array.len(), 3);
        assert_eq!(array.get(0).unwrap().as_number().unwrap(), 42.0);
        assert_eq!(array.get(1).unwrap().as_number().unwrap(), 1337.0);
        assert_eq!(array.get(2).unwrap().as_number().unwrap(), 1234.0);
    }

    #[test]
    fn js_vec_to_rust() {
        let mut engine = V8Engine::new();
        let mut context = engine.new_context().unwrap();

        let array = context
            .run(
                r#"
            [42, 1337, 1234]
        "#,
            )
            .unwrap();

        let vec: Vec<u32> = array.as_array().unwrap().to_rust_value().unwrap();

        assert_eq!(vec, vec![42, 1337, 1234]);
    }
}

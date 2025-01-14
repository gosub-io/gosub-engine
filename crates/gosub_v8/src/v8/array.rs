use v8::{Array, Global, Local};

use gosub_shared::types::Result;
use gosub_webexecutor::js::{AsArray, JSError, Ref, WebArray, WebRuntime};
use gosub_webexecutor::Error;

use crate::{FromContext, V8Context, V8Engine, V8Value};

#[derive(Clone)]
pub struct V8Array {
    pub value: Global<Array>,
    pub ctx: V8Context,
    next: u32, //TODO; this should not be in the array itself
}

impl FromContext<Local<'_, Array>> for V8Array {
    fn from_ctx(ctx: V8Context, value: Local<Array>) -> Self {
        let value = Global::new(&mut ctx.isolate(), value);

        Self { value, ctx, next: 0 }
    }
}

impl FromContext<Global<Array>> for V8Array {
    fn from_ctx(ctx: V8Context, value: Global<Array>) -> Self {
        Self { value, ctx, next: 0 }
    }
}

impl Iterator for V8Array {
    type Item = V8Value;
    fn next(&mut self) -> Option<Self::Item> {
        let scope = &mut self.ctx.scope();

        let array = self.value.open(scope);

        if self.next >= array.length() {
            return None;
        }
        let value = array.get_index(scope, self.next);

        self.next += 1;

        value.map(|v| {
            let global = Global::new(scope, v);
            V8Value::from_value(self.ctx.clone(), global)
        })
    }
}

impl AsArray for V8Array {
    type Runtime = V8Engine;

    fn array(&self) -> Result<Ref<<Self::Runtime as WebRuntime>::Array>> {
        Ok(Ref::Ref(self))
    }
}

impl WebArray for V8Array {
    type RT = V8Engine;

    fn get(&self, index: usize) -> Result<<Self::RT as WebRuntime>::Value> {
        let scope = &mut self.ctx.scope();

        let array = self.value.open(scope);

        let Some(value) = array.get_index(scope, index as u32) else {
            return Err(Error::JS(JSError::Generic("failed to get a value from an array".to_owned())).into());
        };

        let value = Global::new(scope, value);

        Ok(V8Value::from_value(self.ctx.clone(), value))
    }

    fn set(&self, index: usize, value: &V8Value) -> Result<()> {
        let scope = &mut self.ctx.scope();

        let array = self.value.open(scope);

        let value = Local::new(scope, value.value.clone());

        match array.set_index(scope, index as u32, value) {
            Some(_) => Ok(()),
            None => Err(Error::JS(JSError::Conversion("failed to set a value in an array".to_owned())).into()),
        }
    }

    fn push(&self, value: V8Value) -> Result<()> {
        let scope = &mut self.ctx.scope();

        let array = self.value.open(scope);

        let value = Local::new(scope, value.value.clone());

        let index = array.length();

        match array.set_index(scope, index, value) {
            Some(_) => Ok(()),
            None => Err(Error::JS(JSError::Conversion("failed to push to an array".to_owned())).into()),
        }
    }

    fn pop(&self) -> Result<<Self::RT as WebRuntime>::Value> {
        let scope = &mut self.ctx.scope();

        let array = self.value.open(scope);

        let index = array.length() - 1;

        let Some(value) = array.get_index(scope, index) else {
            return Err(Error::JS(JSError::Generic("failed to get a value from an array".to_owned())).into());
        };

        if array.delete_index(scope, index).is_none() {
            return Err(Error::JS(JSError::Generic("failed to delete a value from an array".to_owned())).into());
        }

        let value = Global::new(scope, value);

        Ok(V8Value::from_value(self.ctx.clone(), value))
    }

    fn remove(&self, index: usize) -> Result<()> {
        let scope = &mut self.ctx.scope();

        let array = self.value.open(scope);

        if array.delete_index(scope, index as u32).is_none() {
            return Err(Error::JS(JSError::Generic("failed to delete a value from an array".to_owned())).into());
        }

        Ok(())
    }

    fn len(&self) -> usize {
        let scope = &mut self.ctx.scope();

        let array = self.value.open(scope);
        array.length() as usize
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn new(ctx: V8Context, cap: usize) -> Result<Self> {
        let mut scope = ctx.scope();

        let value = Array::new(&mut scope, cap as i32);
        let value = Global::new(&mut scope, value);

        drop(scope);

        Ok(Self { value, ctx, next: 0 })
    }

    fn new_with_data(ctx: V8Context, data: &[V8Value]) -> Result<Self> {
        let mut scope = ctx.scope();

        let elements = data
            .iter()
            .map(|v| Local::new(&mut scope, v.value.clone()))
            .collect::<Vec<_>>();

        let value = Array::new_with_elements(&mut scope, &elements);
        let value = Global::new(&mut scope, value);

        drop(scope);

        Ok(Self { value, ctx, next: 0 })
    }

    fn as_value(&self) -> <Self::RT as WebRuntime>::Value {
        V8Value::from(self.clone())
    }

    fn as_vec(&self) -> Vec<<Self::RT as WebRuntime>::Value> {
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
        ArrayConversion, IntoRustValue, IntoWebValue, WebArray, WebContext, WebObject, WebRuntime, WebValue,
    };

    use crate::v8::{V8Array, V8Engine, V8Value};
    use crate::V8Object;

    #[test]
    fn set() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        let array = V8Array::new(context.clone(), 2).unwrap();
        array.set(0, &1234.0.to_web_value(context.clone()).unwrap()).unwrap();
        array.set(1, &"Hello World!".to_web_value(context).unwrap()).unwrap();

        assert_eq!(array.len(), 2);
    }

    #[test]
    fn get() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        let array = V8Array::new(context.clone(), 2).unwrap();

        array.set(0, &1234.0.to_web_value(context.clone()).unwrap()).unwrap();
        array.set(1, &"Hello World!".to_web_value(context).unwrap()).unwrap();

        assert_eq!(array.get(0).unwrap().as_number().unwrap(), 1234.0);
        assert_eq!(array.get(1).unwrap().as_string().unwrap(), "Hello World!");
    }

    #[test]
    fn push() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        let array = V8Array::new(context.clone(), 2).unwrap();

        array.push(1234.0.to_web_value(context.clone()).unwrap()).unwrap();
        array.push("Hello World!".to_web_value(context).unwrap()).unwrap();

        assert_eq!(array.len(), 4);
    }

    #[test]
    fn out_of_bounds() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        let array = V8Array::new(context.clone(), 2).unwrap();

        array.set(0, &1234.0.to_web_value(context.clone()).unwrap()).unwrap();
        array.set(1, &"Hello World!".to_web_value(context).unwrap()).unwrap();

        assert!(array.get(2).unwrap().is_undefined());
    }

    #[test]
    fn pop() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        let array = V8Array::new(context.clone(), 2).unwrap();

        array.set(0, &1234.0.to_web_value(context.clone()).unwrap()).unwrap();
        array.set(1, &"Hello World!".to_web_value(context).unwrap()).unwrap();

        assert_eq!(array.pop().unwrap().as_string().unwrap(), "Hello World!");
        assert_eq!(array.get(0).unwrap().as_number().unwrap(), 1234.0);
        assert!(array.get(1).unwrap().is_undefined());
    }

    #[test]
    fn dynamic_resize() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        let array = V8Array::new(context.clone(), 2).unwrap();

        array.set(0, &1234.0.to_web_value(context.clone()).unwrap()).unwrap();
        array
            .set(1, &"Hello World!".to_web_value(context.clone()).unwrap())
            .unwrap();
        array.set(2, &1234.0.to_web_value(context.clone()).unwrap()).unwrap();
        array
            .set(3, &"Hello World!".to_web_value(context.clone()).unwrap())
            .unwrap();

        assert_eq!(array.len(), 4);
    }

    #[test]
    fn rust_to_js() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        let array: V8Array = [42, 1337, 1234].to_web_array(context.clone()).unwrap();

        assert_eq!(array.len(), 3);
        assert_eq!(array.get(0).unwrap().as_number().unwrap(), 42.0);
        assert_eq!(array.get(1).unwrap().as_number().unwrap(), 1337.0);
        assert_eq!(array.get(2).unwrap().as_number().unwrap(), 1234.0);
    }

    #[test]
    fn rust_to_web_value() {
        let mut engine = V8Engine::new();
        let context = engine.new_context().unwrap();

        let array: V8Value = [42, 1337, 1234].to_web_value(context.clone()).unwrap();

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

        let array: V8Array = [42, 1337, 1234].to_web_array(context.clone()).unwrap();

        // let test_obj = context.new_global_object("test").unwrap();

        let test_obj = V8Object::new(context.clone()).unwrap();

        context.set_on_global_object("test", test_obj.clone().into()).unwrap();

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

        let array: V8Array = vec.to_web_array(context.clone()).unwrap();

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
